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
    // Prepare all requests
    let settings_req = client
        .client
        .get(client.base_url.join(&format!("{}/_settings", index_name))?);
    let ilm_req = client.client.get(
        client
            .base_url
            .join(&format!("_ilm/explain/{}", index_name))?,
    );
    let segments_req = client.client.get(
        client
            .base_url
            .join(&format!("{}/_stats/segments", index_name))?,
    );
    let shards_req = client.client.get(client.base_url.join(&format!(
        "_cat/shards/{}?format=json&h=index,shard,prirep,state,docs,store,node",
        index_name
    ))?);
    let templates_req = client.client.get(client.base_url.join("_index_template")?);
    let cat_req = client.client.get(client.base_url.join(&format!(
        "_cat/indices/{}?format=json&h=health,status,index",
        index_name
    ))?);
    let ds_req = client.client.get(client.base_url.join("_data_stream")?);

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

    // Process templates
    let templates = templates_res
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
                .map(|t| t.name)
                .collect()
        })
        .unwrap_or_default();

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

    let is_partial = index_settings
        .and_then(|s| s.settings.index.store.as_ref())
        .and_then(|store| store.store_type.as_ref())
        .map(|t| t.contains("snapshot") || t.contains("searchable"))
        .unwrap_or(false);

    let uuid = index_settings.and_then(|s| s.settings.index.uuid.clone());
    let provided_name = index_settings.and_then(|s| s.settings.index.provided_name.clone());

    Ok(IndexDetails {
        name: index_name.to_string(),
        provided_name,
        creation_date,
        primary_shards,
        replica_shards,
        is_frozen,
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

// Simple glob pattern matching for index templates
fn pattern_matches(pattern: &str, index_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if let Some(prefix) = pattern.strip_suffix('*') {
        return index_name.starts_with(prefix);
    }

    if let Some(suffix) = pattern.strip_prefix('*') {
        return index_name.ends_with(suffix);
    }

    pattern == index_name
}
