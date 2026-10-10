use super::client::EsClient;
use super::types::{
    CatIndexEntry, CatShardEntry, DataStreamsResponse, IlmExplainResponse, IndexSettingsResponse,
    IndexTemplateResponse, SegmentsStatsResponse,
};
use crate::error::Result;
use crate::models::{DataStreamDetails, IndexDetails, ShardInfo};

pub async fn fetch_index_details(
    client: &EsClient,
    index_name: &str,
    doc_count: u64,
    rate_per_sec: f64,
    size_bytes: u64,
) -> Result<IndexDetails> {
    // Prepare all requests. The index name always goes in as its own
    // encoded path segment — see `EsClient::endpoint`.
    let settings_req = client
        .client
        .get(client.endpoint(&[index_name, "_settings"]));
    let ilm_req = client
        .client
        .get(client.endpoint(&[index_name, "_ilm", "explain"]));
    let segments_req = client
        .client
        .get(client.endpoint(&[index_name, "_stats", "segments"]));

    let mut shards_url = client.endpoint(&["_cat", "shards", index_name]);
    shards_url
        .query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("h", "index,shard,prirep,state,docs,store,node");
    let shards_req = client.client.get(shards_url);

    // The two cluster-wide listings are narrowed to the fields read below:
    // on a Fleet-managed cluster each is hundreds of kilobytes otherwise,
    // fetched again on every refresh while the popup is open.
    let mut templates_url = client.endpoint(&["_index_template"]);
    templates_url.query_pairs_mut().append_pair(
        "filter_path",
        "index_templates.name,\
index_templates.index_template.index_patterns,\
index_templates.index_template.priority",
    );
    let templates_req = client.client.get(templates_url);

    let mut cat_url = client.endpoint(&["_cat", "indices", index_name]);
    cat_url
        .query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("h", "health,status,index");
    let cat_req = client.client.get(cat_url);

    let mut ds_url = client.endpoint(&["_data_stream"]);
    ds_url.query_pairs_mut().append_pair(
        "filter_path",
        "data_streams.name,\
data_streams.timestamp_field.name,\
data_streams.indices.index_name,\
data_streams.generation,\
data_streams.template,\
data_streams.lifecycle.data_retention",
    );
    let ds_req = client.client.get(ds_url);

    // Execute requests in parallel
    let (settings_res, ilm_res, segments_res, shards_res, templates_res, cat_res, ds_res) = tokio::join!(
        client.send_json::<IndexSettingsResponse>(settings_req),
        client.send_json::<IlmExplainResponse>(ilm_req),
        client.send_json::<SegmentsStatsResponse>(segments_req),
        client.send_json::<Vec<CatShardEntry>>(shards_req),
        client.send_json::<IndexTemplateResponse>(templates_req),
        client.send_json::<Vec<CatIndexEntry>>(cat_req),
        client.send_json::<DataStreamsResponse>(ds_req),
    );

    // Process settings (required for most other things)
    let settings = settings_res.unwrap_or_default();
    let index_settings = settings.indices.get(index_name);

    // Process ILM
    let (ilm_policy, ilm_phase) = ilm_res
        .ok()
        .and_then(|ilm| {
            ilm.indices
                .get(index_name)
                .map(|s| (s.policy.clone(), s.phase.clone()))
        })
        .unwrap_or((None, None));

    // Fallback ILM policy from settings
    let ilm_policy = ilm_policy.or_else(|| {
        index_settings
            .and_then(|s| s.settings.index.lifecycle.as_ref())
            .and_then(|l| l.name.clone())
    });

    // Process segments
    let total_segments = segments_res
        .ok()
        .and_then(|s| {
            s.indices
                .get(index_name)
                .map(|stats| stats.primaries.segments.count)
        })
        .unwrap_or(0);

    // Process shards
    let shard_allocation = shards_res
        .unwrap_or_default()
        .into_iter()
        .map(|s| ShardInfo {
            shard_id: s.shard.parse().unwrap_or(0),
            primary: s.prirep == "p",
            state: s.state,
            node: s.node.unwrap_or_else(|| "unassigned".to_string()),
            docs: s.docs.and_then(|d| d.parse().ok()),
            size: s.store,
        })
        .collect();

    // Process health/status
    let (health, status) = cat_res
        .ok()
        .and_then(|entries| {
            entries
                .first()
                .map(|e| (e.health.clone(), e.status.clone()))
        })
        .unwrap_or((None, None));

    // Process data stream
    let data_stream = ds_res.ok().and_then(|ds_response| {
        ds_response.data_streams.iter().find_map(|ds| {
            ds.indices
                .iter()
                .position(|idx| idx.index_name == index_name)
                .map(|pos| {
                    let total = ds.indices.len();
                    DataStreamDetails {
                        name: ds.name.clone(),
                        timestamp_field: ds.timestamp_field.name.clone(),
                        generation: ds.generation,
                        total_backing_indices: total,
                        backing_index_position: pos + 1,
                        is_write_index: pos == total - 1,
                        template: ds.template.clone(),
                        data_retention: ds
                            .lifecycle
                            .as_ref()
                            .and_then(|l| l.data_retention.clone()),
                    }
                })
        })
    });

    // Process templates. A data stream's backing indices were created from
    // the template that matched the *stream's* name, which the data stream
    // reports itself; matching the `.ds-…` backing index name against the
    // patterns would never find it. Otherwise list every template whose
    // patterns match, the one Elasticsearch actually applies (highest
    // priority) first.
    let templates = match data_stream.as_ref().and_then(|ds| ds.template.clone()) {
        Some(template) => vec![template],
        None => {
            let mut matching: Vec<(u64, String)> = templates_res
                .map(|tmpl_resp| {
                    tmpl_resp
                        .index_templates
                        .into_iter()
                        .filter(|t| {
                            t.index_template
                                .index_patterns
                                .iter()
                                .any(|pattern| pattern_matches(pattern, index_name))
                        })
                        .map(|t| (t.index_template.priority.unwrap_or(0), t.name))
                        .collect()
                })
                .unwrap_or_default();
            matching.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            matching.into_iter().map(|(_, name)| name).collect()
        }
    };

    // Parse specific settings fields
    let creation_date = index_settings
        .and_then(|s| s.settings.index.creation_date.as_ref())
        .and_then(|ts| ts.parse::<i64>().ok())
        .and_then(|ts| {
            chrono::DateTime::from_timestamp_millis(ts)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        });

    let primary_shards = index_settings
        .and_then(|s| s.settings.index.number_of_shards.as_ref())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    let replica_shards = index_settings
        .and_then(|s| s.settings.index.number_of_replicas.as_ref())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    let is_frozen = index_settings
        .and_then(|s| s.settings.index.frozen.as_ref())
        .map(|f| f == "true")
        .unwrap_or(false);

    let is_searchable_snapshot = index_settings
        .and_then(|s| s.settings.index.store.as_ref())
        .and_then(|store| store.store_type.as_deref())
        .is_some_and(|store_type| store_type == "snapshot");

    // Only a partially mounted (frozen tier) searchable snapshot sets
    // `index.store.snapshot.partial`; a fully mounted one has the same
    // `snapshot` store type without it.
    let is_partial = index_settings
        .and_then(|s| s.settings.index.store.as_ref())
        .and_then(|store| store.snapshot.as_ref())
        .and_then(|snapshot| snapshot.partial.as_deref())
        .is_some_and(|partial| partial == "true");

    let uuid = index_settings.and_then(|s| s.settings.index.uuid.clone());
    let provided_name = index_settings.and_then(|s| s.settings.index.provided_name.clone());

    Ok(IndexDetails {
        name: index_name.to_string(),
        provided_name,
        creation_date,
        primary_shards,
        replica_shards,
        is_frozen,
        is_searchable_snapshot,
        is_partial,
        ilm_policy,
        ilm_phase,
        total_segments,
        shard_allocation,
        templates,
        uuid,
        health,
        status,
        doc_count,
        rate_per_sec,
        size_bytes,
        data_stream,
    })
}

/// Matches an index name against an index template pattern, where `*` stands
/// for any run of characters (including none) — Elasticsearch's own rule.
/// Built-in patterns put wildcards anywhere (`logs-*-*`,
/// `metrics-*.otel-*`), so handling only a leading or trailing one misses
/// them.
fn pattern_matches(pattern: &str, index_name: &str) -> bool {
    let mut parts = pattern.split('*');
    // `split` always yields at least one part.
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = index_name.strip_prefix(first) else {
        return false;
    };

    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // No `*` at all: the whole pattern had to match exactly.
        return rest.is_empty();
    };

    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::pattern_matches;

    #[test]
    fn test_pattern_matches_wildcards_anywhere() {
        assert!(pattern_matches("*", "anything"));
        assert!(pattern_matches("logs-*", "logs-app"));
        assert!(pattern_matches("*-logs", "app-logs"));
        assert!(pattern_matches("logs-*-*", "logs-nginx.access-default"));
        assert!(pattern_matches(
            "metrics-*.otel-*",
            "metrics-hostmetrics.otel-default"
        ));
        assert!(pattern_matches("exact", "exact"));
        assert!(pattern_matches("a*b*c", "abc"));

        assert!(!pattern_matches("logs-*-*", "logs-nodash"));
        assert!(!pattern_matches("exact", "exactly"));
        assert!(!pattern_matches("a*b*c", "acb"));
        // The prefix and suffix may not overlap.
        assert!(!pattern_matches("ab*ba", "aba"));
    }
}
