use super::client::EsClient;
use super::types::NodesStatsResponse;
use crate::error::Result;
use crate::models::NodeStats;

/// Fetches per-node stats.
///
/// The metric list is narrowed to the four sections the nodes panel actually
/// renders — asking for the default "everything" returns several times the
/// payload per node for data nothing displays.
///
/// Takes `&EsClient` (like `stats::fetch_snapshot`) so it can be joined with
/// the other per-tick requests rather than run after them.
pub async fn fetch_node_stats(client: &EsClient) -> Result<Vec<NodeStats>> {
    let url = client
        .base_url
        .join("_nodes/stats/indices,jvm,process,breaker")?;
    let request = client.client.get(url);

    let response: NodesStatsResponse = client.send_json(request).await?;

    // Node id is the map key and is not shown anywhere, so it's dropped here;
    // a node that somehow reports no name falls back to its id so the row is
    // still identifiable (and, since selection is keyed by name, selectable).
    let mut nodes: Vec<NodeStats> = response
        .nodes
        .into_iter()
        .map(|(node_id, entry)| NodeStats {
            name: if entry.name.is_empty() {
                node_id
            } else {
                entry.name
            },
            heap_used_percent: entry.jvm.mem.heap_used_percent,
            index_failed: entry.indices.indexing.index_failed,
            bulk_avg_size_bytes: entry.indices.bulk.avg_size_in_bytes,
            cpu_percent: entry.process.cpu.percent,
            breaker_parent_tripped: entry.breakers.parent.tripped,
        })
        .collect();

    // `_nodes/stats` returns a map, so iteration order is arbitrary. Sort by
    // name for a stable baseline before the panel's own sort is applied.
    nodes.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(nodes)
}
