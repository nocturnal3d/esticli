use crate::utils::{format_bytes, format_number};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct IndexRate {
    pub name: String,
    pub doc_count: u64,
    pub rate_per_sec: f64,
    pub size_bytes: u64,
    pub health: String,
}

impl IndexRate {
    pub fn size_human(&self) -> String {
        format_bytes(self.size_bytes)
    }

    pub fn rate_human(&self) -> String {
        format_number(self.rate_per_sec)
    }

    pub fn doc_count_human(&self) -> String {
        format_number(self.doc_count as f64)
    }
}

/// Per-node stats shown in the nodes panel, flattened from `_nodes/stats`.
///
/// `Serialize` is what the filter box's jq mode reads, so these field names
/// are the ones a user types in a jq expression against the nodes panel.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NodeStats {
    pub name: String,
    /// `jvm.mem.heap_used_percent`
    pub heap_used_percent: u64,
    /// `indices.indexing.index_failed`
    pub index_failed: u64,
    /// `indices.bulk.avg_size_in_bytes`
    pub bulk_avg_size_bytes: u64,
    /// `process.cpu.percent`; `None` where the node reports it as
    /// unavailable (ES sends -1).
    pub cpu_percent: Option<u64>,
    /// `breakers.parent.tripped`
    pub breaker_parent_tripped: u64,
}

impl NodeStats {
    pub fn bulk_avg_size_human(&self) -> String {
        format_bytes(self.bulk_avg_size_bytes)
    }

    /// Counters are rendered exactly rather than through `format_number`:
    /// "3" failed bulk requests is actionable in a way that "3.0" is not, and
    /// these stay small on a healthy cluster.
    pub fn index_failed_human(&self) -> String {
        self.index_failed.to_string()
    }

    pub fn breaker_tripped_human(&self) -> String {
        self.breaker_parent_tripped.to_string()
    }
}

/// One index's counters from one `_stats` reading.
#[derive(Debug, Clone)]
pub struct IndexSnapshot {
    /// `(doc_count, index_total, size_bytes)` of the primaries, or `None`
    /// when no primary shard copy answered for this reading.
    pub counters: Option<(u64, u64, u64)>,
    pub health: String,
}

// Detailed index information
#[derive(Debug, Clone)]
pub struct IndexDetails {
    pub name: String,
    pub provided_name: Option<String>,
    pub creation_date: Option<String>,
    pub primary_shards: u32,
    pub replica_shards: u32,
    /// Frozen with the legacy (7.x) `_freeze` API.
    pub is_frozen: bool,
    /// Mounted from a snapshot (`index.store.type: snapshot`), fully or
    /// partially.
    pub is_searchable_snapshot: bool,
    /// A partially mounted (frozen tier) searchable snapshot.
    pub is_partial: bool,
    pub ilm_policy: Option<String>,
    pub ilm_phase: Option<String>,
    pub total_segments: u64,
    pub shard_allocation: Vec<ShardInfo>,
    pub templates: Vec<String>,
    pub uuid: Option<String>,
    pub health: Option<String>,
    pub status: Option<String>,
    pub doc_count: u64,
    pub rate_per_sec: f64,
    pub size_bytes: u64,
    pub data_stream: Option<DataStreamDetails>,
}

#[derive(Debug, Clone)]
pub struct ShardInfo {
    pub shard_id: u32,
    pub primary: bool,
    pub state: String,
    pub node: String,
    pub docs: Option<u64>,
    pub size: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DataStreamDetails {
    pub name: String,
    pub timestamp_field: String,
    pub generation: u64,
    pub total_backing_indices: usize,
    pub backing_index_position: usize,
    pub is_write_index: bool,
    pub template: Option<String>,
    pub data_retention: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct ClusterHealth {
    pub cluster_name: String,
    pub status: String,
    pub number_of_nodes: u32,
    pub number_of_data_nodes: u32,
    pub active_primary_shards: u32,
    pub active_shards: u32,
    pub relocating_shards: u32,
    pub initializing_shards: u32,
    pub unassigned_shards: u32,
    pub active_shards_percent: f64,
    pub number_of_pending_tasks: u32,
}
