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

// Elasticsearch _nodes/stats API response types.
//
// Every nested section is `#[serde(default)]` so a node that omits one (an
// older cluster without `indices.bulk`, a node the stats call only partially
// answered for) still deserializes, reporting zeros for what's missing rather
// than failing the whole panel.

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodesStatsResponse {
    #[serde(default)]
    pub nodes: HashMap<String, NodeStatsEntry>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeStatsEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub indices: NodeIndicesStats,
    #[serde(default)]
    pub jvm: NodeJvmStats,
    #[serde(default)]
    pub process: NodeProcessStats,
    #[serde(default)]
    pub breakers: NodeBreakerStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeIndicesStats {
    #[serde(default)]
    pub indexing: NodeIndexingStats,
    /// Present only on ES 7.13+; older clusters simply report zero.
    #[serde(default)]
    pub bulk: NodeBulkStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeIndexingStats {
    #[serde(default)]
    pub index_failed: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeBulkStats {
    #[serde(default)]
    pub avg_size_in_bytes: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeJvmStats {
    #[serde(default)]
    pub mem: NodeJvmMemStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeJvmMemStats {
    #[serde(default)]
    pub heap_used_percent: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeProcessStats {
    #[serde(default)]
    pub cpu: NodeProcessCpuStats,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeProcessCpuStats {
    #[serde(default)]
    pub percent: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeBreakerStats {
    #[serde(default)]
    pub parent: NodeBreakerEntry,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NodeBreakerEntry {
    #[serde(default)]
    pub tripped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_nodes_stats() {
        // Shaped like a real `_nodes/stats` response, including sibling fields
        // we don't read, to confirm they're ignored rather than rejected.
        let json_data = json!({
            "_nodes": {"total": 2, "successful": 2, "failed": 0},
            "cluster_name": "test-cluster",
            "nodes": {
                "abc123": {
                    "name": "node-1",
                    "host": "10.0.0.1",
                    "indices": {
                        "docs": {"count": 42},
                        "indexing": {"index_total": 900, "index_failed": 7},
                        "bulk": {"total_operations": 5, "avg_size_in_bytes": 2048}
                    },
                    "jvm": {"mem": {"heap_used_percent": 73, "heap_used_in_bytes": 1}},
                    "process": {"cpu": {"percent": 41, "total_in_millis": 9}},
                    "breakers": {
                        "parent": {"limit_size_in_bytes": 1, "tripped": 3},
                        "fielddata": {"tripped": 0}
                    }
                }
            }
        });

        let response: NodesStatsResponse = serde_json::from_value(json_data).unwrap();
        let node = &response.nodes["abc123"];
        assert_eq!(node.name, "node-1");
        assert_eq!(node.jvm.mem.heap_used_percent, 73);
        assert_eq!(node.indices.indexing.index_failed, 7);
        assert_eq!(node.indices.bulk.avg_size_in_bytes, 2048);
        assert_eq!(node.process.cpu.percent, 41);
        assert_eq!(node.breakers.parent.tripped, 3);
    }

    #[test]
    fn test_deserialize_nodes_stats_tolerates_missing_sections() {
        // `indices.bulk` only exists on ES 7.13+, and a node can answer
        // partially. Missing sections must read as zero, not fail the parse.
        let json_data = json!({
            "nodes": {
                "abc123": {
                    "name": "old-node",
                    "indices": {"indexing": {"index_failed": 2}}
                }
            }
        });

        let response: NodesStatsResponse = serde_json::from_value(json_data).unwrap();
        let node = &response.nodes["abc123"];
        assert_eq!(node.indices.indexing.index_failed, 2);
        assert_eq!(node.indices.bulk.avg_size_in_bytes, 0);
        assert_eq!(node.jvm.mem.heap_used_percent, 0);
        assert_eq!(node.process.cpu.percent, 0);
        assert_eq!(node.breakers.parent.tripped, 0);
    }

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
