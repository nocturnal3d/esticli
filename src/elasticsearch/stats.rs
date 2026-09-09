use super::client::EsClient;
use super::types::{ClusterHealthResponse, StatsResponse};
use crate::error::Result;
use crate::models::{ClusterHealth, IndexRate, IndexSnapshot};
use std::collections::HashMap;
use std::time::Instant;

/// A point-in-time `_stats` reading: when it was taken, and the per-index
/// counters it contained.
pub type Snapshot = (Instant, HashMap<String, IndexSnapshot>);

/// Fetches a fresh `_stats` snapshot.
///
/// Deliberately takes `&EsClient` rather than `&mut`: the rate calculation
/// needs `&mut` (it swaps in the new baseline), but keeping the *request*
/// immutable is what lets it be `tokio::join!`ed with the cluster-health
/// request instead of running after it. The `&mut` half happens once both
/// responses are back, in `rates_from_snapshot`.
pub async fn fetch_snapshot(client: &EsClient) -> Result<Snapshot> {
    let url = client.base_url.join("_stats/indexing,docs,store")?;
    let request = client.client.get(url);

    let stats: StatsResponse = client.send_json(request).await?;

    let now = Instant::now();

    // Map stats to internal models
    let snapshot: HashMap<String, IndexSnapshot> = stats
        .indices
        .iter()
        .map(|(name, entry)| {
            (
                name.clone(),
                IndexSnapshot {
                    doc_count: entry.primaries.docs.count,
                    index_total: entry.primaries.indexing.index_total,
                    size_bytes: entry.primaries.store.size_in_bytes,
                    health: entry.health.clone(),
                },
            )
        })
        .collect();

    Ok((now, snapshot))
}

/// Turns a fresh snapshot into per-index rates by diffing it against the
/// previous one, then stores it as the baseline for the next call. The very
/// first snapshot has nothing to diff against, so every rate is 0.
pub fn rates_from_snapshot(
    client: &mut EsClient,
    now: Instant,
    current: HashMap<String, IndexSnapshot>,
) -> Vec<IndexRate> {
    let rates: Vec<IndexRate> = if let Some((prev_time, prev_snapshot)) = &client.previous_snapshot
    {
        let elapsed = now.duration_since(*prev_time).as_secs_f64();

        current
            .iter()
            .map(|(name, entry)| {
                let rate = prev_snapshot
                    .get(name)
                    .filter(|prev| elapsed > 0.0 && entry.index_total >= prev.index_total)
                    .map(|prev| (entry.index_total - prev.index_total) as f64 / elapsed)
                    .unwrap_or(0.0);

                IndexRate {
                    name: name.clone(),
                    doc_count: entry.doc_count,
                    rate_per_sec: rate,
                    size_bytes: entry.size_bytes,
                    health: entry.health.clone(),
                }
            })
            .collect()
    } else {
        // First fetch, no rate data yet
        current
            .iter()
            .map(|(name, entry)| IndexRate {
                name: name.clone(),
                doc_count: entry.doc_count,
                rate_per_sec: 0.0,
                size_bytes: entry.size_bytes,
                health: entry.health.clone(),
            })
            .collect()
    };

    // Store current snapshot for the next calculation
    client.previous_snapshot = Some((now, current));

    rates
}

/// Fetches cluster health. Takes `&EsClient` for the same reason as
/// `fetch_snapshot` — so the two can be issued concurrently.
pub async fn fetch_cluster_health(client: &EsClient) -> Result<ClusterHealth> {
    let url = client.base_url.join("_cluster/health")?;
    let request = client.client.get(url);

    let health: ClusterHealthResponse = client.send_json(request).await?;

    Ok(ClusterHealth {
        cluster_name: health.cluster_name,
        status: health.status,
        number_of_nodes: health.number_of_nodes,
        number_of_data_nodes: health.number_of_data_nodes,
        active_primary_shards: health.active_primary_shards,
        active_shards: health.active_shards,
        relocating_shards: health.relocating_shards,
        initializing_shards: health.initializing_shards,
        unassigned_shards: health.unassigned_shards,
        active_shards_percent: health.active_shards_percent_as_number,
        number_of_pending_tasks: health.number_of_pending_tasks,
    })
}
