use super::client::EsClient;
use super::types::{ClusterHealthResponse, StatsResponse};
use crate::error::Result;
use crate::models::{ClusterHealth, IndexRate, IndexSnapshot};
use std::collections::HashMap;
use std::time::Instant;

pub async fn fetch_index_rates(client: &mut EsClient) -> Result<Vec<IndexRate>> {
    let url = client.base_url.join("_stats/indexing,docs,store")?;
    let request = client.client.get(url);

    let stats: StatsResponse = client.send_json(request).await?;

    let now = Instant::now();

    // Map stats to internal models
    let current_snapshot: HashMap<String, IndexSnapshot> = stats
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

    // Calculate rates based on the previous snapshot
    let rates: Vec<IndexRate> = if let Some((prev_time, prev_snapshot)) = &client.previous_snapshot
    {
        let elapsed = now.duration_since(*prev_time).as_secs_f64();

        current_snapshot
            .iter()
            .map(|(name, current)| {
                let rate = prev_snapshot
                    .get(name)
                    .filter(|prev| elapsed > 0.0 && current.index_total >= prev.index_total)
                    .map(|prev| (current.index_total - prev.index_total) as f64 / elapsed)
                    .unwrap_or(0.0);

                IndexRate {
                    name: name.clone(),
                    doc_count: current.doc_count,
                    rate_per_sec: rate,
                    size_bytes: current.size_bytes,
                    health: current.health.clone(),
                }
            })
            .collect()
    } else {
        // First fetch, no rate data yet
        current_snapshot
            .iter()
            .map(|(name, current)| IndexRate {
                name: name.clone(),
                doc_count: current.doc_count,
                rate_per_sec: 0.0,
                size_bytes: current.size_bytes,
                health: current.health.clone(),
            })
            .collect()
    };

    // Store current snapshot for the next calculation
    client.previous_snapshot = Some((now, current_snapshot));

    Ok(rates)
}

pub async fn fetch_cluster_health(client: &mut EsClient) -> Result<ClusterHealth> {
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
