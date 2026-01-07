use serde::Deserialize;
use std::collections::HashMap;

// Elasticsearch _stats API response types
#[derive(Debug, Deserialize, Default, Clone)]
pub struct StatsResponse {
    pub indices: HashMap<String, IndexStatsEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexStatsEntry {
    pub primaries: PrimaryStats,
    pub health: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct PrimaryStats {
    pub docs: DocsStats,
    pub indexing: IndexingStats,
    pub store: StoreStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DocsStats {
    pub count: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexingStats {
    pub index_total: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct StoreStats {
    pub size_in_bytes: u64,
}

// API response types for index details

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexSettingsResponse {
    #[serde(flatten)]
    pub indices: HashMap<String, IndexSettingsEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexSettingsEntry {
    pub settings: IndexSettings,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexSettings {
    pub index: IndexSettingsIndex,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexSettingsIndex {
    pub creation_date: Option<String>,
    pub number_of_shards: Option<String>,
    pub number_of_replicas: Option<String>,
    pub uuid: Option<String>,
    #[serde(default)]
    pub frozen: Option<String>,
    #[serde(default)]
    pub store: Option<IndexStoreSettings>,
    #[serde(default)]
    pub lifecycle: Option<IndexLifecycleSettings>,
    pub provided_name: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexLifecycleSettings {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexStoreSettings {
    #[serde(rename = "type")]
    pub store_type: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IlmExplainResponse {
    pub indices: HashMap<String, IlmIndexStatus>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IlmIndexStatus {
    pub _managed: bool,
    pub policy: Option<String>,
    pub phase: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SegmentsStatsResponse {
    pub indices: HashMap<String, SegmentsIndexStats>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SegmentsIndexStats {
    pub primaries: SegmentsPrimaryStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SegmentsPrimaryStats {
    pub segments: SegmentsCount,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct SegmentsCount {
    pub count: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct CatShardEntry {
    #[serde(rename = "index")]
    pub _index: String,
    pub shard: String,
    pub prirep: String,
    pub state: String,
    pub docs: Option<String>,
    pub store: Option<String>,
    pub node: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexTemplateResponse {
    pub index_templates: Vec<IndexTemplateEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexTemplateEntry {
    pub name: String,
    pub index_template: IndexTemplateDetails,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct IndexTemplateDetails {
    pub index_patterns: Vec<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct CatIndexEntry {
    pub health: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "index")]
    pub _index: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DataStreamsResponse {
    pub data_streams: Vec<DataStreamInfo>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DataStreamInfo {
    pub name: String,
    pub timestamp_field: DataStreamTimestampField,
    pub indices: Vec<DataStreamIndex>,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub _status: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub lifecycle: Option<DataStreamLifecycle>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DataStreamTimestampField {
    pub name: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DataStreamIndex {
    pub index_name: String,
    #[serde(rename = "index_uuid")]
    pub _index_uuid: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DataStreamLifecycle {
    #[serde(default)]
    pub data_retention: Option<String>,
}
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ClusterHealthResponse {
    pub cluster_name: String,
    pub status: String,
    pub number_of_nodes: u32,
    pub number_of_data_nodes: u32,
    pub active_primary_shards: u32,
    pub active_shards: u32,
    pub relocating_shards: u32,
    pub initializing_shards: u32,
    pub unassigned_shards: u32,
    pub active_shards_percent_as_number: f64,
    pub number_of_pending_tasks: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_cat_shard_entry() {
        let json_data = json!({
            "index": "test-index",
            "shard": "0",
            "prirep": "p",
            "state": "STARTED",
            "docs": "100",
            "store": "10kb",
            "node": "node-1"
        });
        let entry: CatShardEntry = serde_json::from_value(json_data).unwrap();
        assert_eq!(entry._index, "test-index");
        assert_eq!(entry.shard, "0");
        assert_eq!(entry.prirep, "p");
        assert_eq!(entry.state, "STARTED");
        assert_eq!(entry.docs, Some("100".to_string()));
        assert_eq!(entry.store, Some("10kb".to_string()));
        assert_eq!(entry.node, Some("node-1".to_string()));
    }

    #[test]
    fn test_deserialize_cat_index_entry() {
        let json_data = json!({
            "health": "green",
            "status": "open",
            "index": "test-index"
        });
        let entry: CatIndexEntry = serde_json::from_value(json_data).unwrap();
        assert_eq!(entry.health, Some("green".to_string()));
        assert_eq!(entry.status, Some("open".to_string()));
        assert_eq!(entry._index, "test-index");
    }

    #[test]
    fn test_deserialize_data_stream_index() {
        let json_data = json!({
            "index_name": ".ds-test-2023.01.01-000001",
            "index_uuid": "abc-123"
        });
        let entry: DataStreamIndex = serde_json::from_value(json_data).unwrap();
        assert_eq!(entry.index_name, ".ds-test-2023.01.01-000001");
        assert_eq!(entry._index_uuid, "abc-123");
    }
}
