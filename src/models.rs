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

#[derive(Debug, Clone)]
pub struct IndexSnapshot {
    pub doc_count: u64,
    pub index_total: u64,
    pub size_bytes: u64,
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
    pub is_frozen: bool,
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
