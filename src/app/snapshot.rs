//! Baseline snapshots: freeze every metric at a point in time so that later
//! readings can be shown as movement away from that moment rather than as
//! bare numbers.
//!
//! The baseline is fixed when it is taken — it is *not* a comparison against
//! the previous tick. Every subsequent refresh is compared against the same
//! frozen values until a new snapshot is taken (`s`) or the current one is
//! dropped (`S`), which is what makes "how much has this index grown while I
//! was watching?" answerable at a glance.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local};

use crate::models::{IndexRate, NodeStats};

/// Which way a metric has moved relative to its snapshot baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trend {
    #[default]
    Unchanged,
    Up,
    Down,
}

impl Trend {
    /// The arrow a metric cell carries, or nothing at all when the value
    /// hasn't moved — a table that has held still looks exactly as it did
    /// before the snapshot was taken, rather than growing a column of
    /// placeholder glyphs.
    ///
    /// `↑`/`↓` deliberately differ from the `▲`/`▼` a header shows for sort
    /// direction: the two mean entirely different things and appear only a
    /// row apart.
    pub fn arrow(self) -> &'static str {
        match self {
            Trend::Up => " ↑",
            Trend::Down => " ↓",
            Trend::Unchanged => "",
        }
    }

    /// Compares a current reading against its baseline. Values that don't
    /// order against each other (a NaN rate) count as unchanged rather than
    /// picking a direction arbitrarily.
    fn compare<T: PartialOrd>(current: T, baseline: T) -> Self {
        match current.partial_cmp(&baseline) {
            Some(Ordering::Greater) => Trend::Up,
            Some(Ordering::Less) => Trend::Down,
            _ => Trend::Unchanged,
        }
    }
}

/// Per-column trends for one row of the indices table. The name and health
/// columns have no entry: neither is a magnitude, so "increased" is not a
/// thing either of them can do.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexTrends {
    pub doc_count: Trend,
    pub rate: Trend,
    pub size: Trend,
}

/// Per-column trends for one row of the nodes table.
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeTrends {
    pub cpu: Trend,
    pub heap: Trend,
    pub index_failed: Trend,
    pub bulk_avg_size: Trend,
    pub breaker_tripped: Trend,
}

/// The frozen numbers for one index. Only the metrics are kept — the name is
/// already the map key, and duplicating it would double the memory this costs
/// on a 50k-index cluster for nothing.
#[derive(Debug, Clone, Copy)]
struct IndexBaseline {
    doc_count: u64,
    rate_per_sec: f64,
    size_bytes: u64,
}

impl From<&IndexRate> for IndexBaseline {
    fn from(index: &IndexRate) -> Self {
        Self {
            doc_count: index.doc_count,
            rate_per_sec: index.rate_per_sec,
            size_bytes: index.size_bytes,
        }
    }
}

/// The frozen numbers for one node, kept for the same reason as
/// `IndexBaseline`.
#[derive(Debug, Clone, Copy)]
struct NodeBaseline {
    cpu_percent: u64,
    heap_used_percent: u64,
    index_failed: u64,
    bulk_avg_size_bytes: u64,
    breaker_parent_tripped: u64,
}

impl From<&NodeStats> for NodeBaseline {
    fn from(node: &NodeStats) -> Self {
        Self {
            cpu_percent: node.cpu_percent,
            heap_used_percent: node.heap_used_percent,
            index_failed: node.index_failed,
            bulk_avg_size_bytes: node.bulk_avg_size_bytes,
            breaker_parent_tripped: node.breaker_parent_tripped,
        }
    }
}

/// The active baseline, if there is one, for both panels.
///
/// `taken_at` doubles as the "is a snapshot active" flag: with no snapshot
/// there is nothing to compare against and every trend comes back
/// `Unchanged`, so the tables render exactly as they did before this feature
/// existed.
#[derive(Debug, Default)]
pub struct SnapshotState {
    taken_at: Option<DateTime<Local>>,
    indices: HashMap<String, IndexBaseline>,
    nodes: HashMap<String, NodeBaseline>,
}

impl SnapshotState {
    pub fn is_active(&self) -> bool {
        self.taken_at.is_some()
    }

    pub fn taken_at(&self) -> Option<DateTime<Local>> {
        self.taken_at
    }

    /// Freezes the readings currently on screen as the new baseline. Taking a
    /// second snapshot re-baselines from now rather than stacking.
    pub fn capture(&mut self, indices: &[IndexRate], nodes: &[NodeStats]) {
        self.taken_at = Some(Local::now());
        self.indices = indices
            .iter()
            .map(|index| (index.name.clone(), IndexBaseline::from(index)))
            .collect();
        self.nodes = nodes
            .iter()
            .map(|node| (node.name.clone(), NodeBaseline::from(node)))
            .collect();
    }

    pub fn clear(&mut self) {
        self.taken_at = None;
        self.indices.clear();
        self.nodes.clear();
    }

    /// Reconciles the baseline with what the cluster now reports, once per
    /// refresh tick.
    ///
    /// Rows that have appeared since the snapshot adopt their first observed
    /// reading as their baseline, so they start unmarked and drift from
    /// there; there is no honest way to compare them against a moment they
    /// didn't exist in. This is what makes the nodes panel work at all when a
    /// snapshot is taken before it has ever been focused: `_nodes/stats` is
    /// only requested while that panel is on screen (see `App::start_fetch`),
    /// so the node baseline is otherwise empty and would never fill in.
    ///
    /// Rows that have disappeared are dropped so neither map outlives the
    /// cluster. This is the only part of the feature with a cost that scales
    /// with the cluster — ~2.5ms at 50k indices — and it deliberately sits
    /// here, on the refresh tick, rather than in the render path: the tables
    /// redraw ~20x a second and only ever do a hash lookup per on-screen row,
    /// which measures as zero against the render they were already doing. `nodes` is `None` when `_nodes/stats` wasn't requested this
    /// tick, which must not be mistaken for "the cluster has no nodes" — in
    /// that case the node baseline is left completely alone.
    pub fn observe(&mut self, indices: &[IndexRate], nodes: Option<&[NodeStats]>) {
        if !self.is_active() {
            return;
        }

        for index in indices {
            if !self.indices.contains_key(&index.name) {
                self.indices
                    .insert(index.name.clone(), IndexBaseline::from(index));
            }
        }
        // Membership goes through a set rather than a scan per surviving
        // entry: this runs on every refresh tick, and the quadratic version
        // would be 2.5 billion comparisons on a 50k-index cluster.
        let present: HashSet<&str> = indices.iter().map(|index| index.name.as_str()).collect();
        self.indices
            .retain(|name, _| present.contains(name.as_str()));

        let Some(nodes) = nodes else {
            return;
        };
        for node in nodes {
            if !self.nodes.contains_key(&node.name) {
                self.nodes
                    .insert(node.name.clone(), NodeBaseline::from(node));
            }
        }
        let present: HashSet<&str> = nodes.iter().map(|node| node.name.as_str()).collect();
        self.nodes.retain(|name, _| present.contains(name.as_str()));
    }

    /// How each of an index's numeric columns has moved. Everything comes
    /// back `Unchanged` when no snapshot is active, so the tables need no
    /// special case for the common state.
    pub fn index_trends(&self, index: &IndexRate) -> IndexTrends {
        let Some(baseline) = self.indices.get(&index.name) else {
            return IndexTrends::default();
        };

        IndexTrends {
            doc_count: Trend::compare(index.doc_count, baseline.doc_count),
            rate: Trend::compare(index.rate_per_sec, baseline.rate_per_sec),
            size: Trend::compare(index.size_bytes, baseline.size_bytes),
        }
    }

    /// How each of a node's numeric columns has moved.
    pub fn node_trends(&self, node: &NodeStats) -> NodeTrends {
        let Some(baseline) = self.nodes.get(&node.name) else {
            return NodeTrends::default();
        };

        NodeTrends {
            cpu: Trend::compare(node.cpu_percent, baseline.cpu_percent),
            heap: Trend::compare(node.heap_used_percent, baseline.heap_used_percent),
            index_failed: Trend::compare(node.index_failed, baseline.index_failed),
            bulk_avg_size: Trend::compare(node.bulk_avg_size_bytes, baseline.bulk_avg_size_bytes),
            breaker_tripped: Trend::compare(
                node.breaker_parent_tripped,
                baseline.breaker_parent_tripped,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(name: &str, doc_count: u64, rate_per_sec: f64, size_bytes: u64) -> IndexRate {
        IndexRate {
            name: name.to_string(),
            doc_count,
            rate_per_sec,
            size_bytes,
            health: "green".to_string(),
        }
    }

    fn node(name: &str, cpu_percent: u64, index_failed: u64) -> NodeStats {
        NodeStats {
            name: name.to_string(),
            cpu_percent,
            index_failed,
            ..NodeStats::default()
        }
    }

    #[test]
    fn test_no_snapshot_means_no_arrows() {
        let snapshot = SnapshotState::default();
        assert!(!snapshot.is_active());

        let trends = snapshot.index_trends(&index("idx", 100, 1.0, 1024));
        assert_eq!(trends.doc_count, Trend::Unchanged);
        assert_eq!(trends.rate, Trend::Unchanged);
        assert_eq!(trends.size, Trend::Unchanged);
        assert_eq!(Trend::Unchanged.arrow(), "");
    }

    #[test]
    fn test_index_trends_point_each_way_independently() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("idx", 100, 5.0, 2048)], &[]);

        // Docs grew, the rate fell off, the size held exactly still.
        let trends = snapshot.index_trends(&index("idx", 150, 2.5, 2048));
        assert_eq!(trends.doc_count, Trend::Up);
        assert_eq!(trends.rate, Trend::Down);
        assert_eq!(trends.size, Trend::Unchanged);
        assert_eq!(trends.doc_count.arrow(), " ↑");
        assert_eq!(trends.rate.arrow(), " ↓");
    }

    #[test]
    fn test_baseline_stays_fixed_rather_than_tracking_the_previous_tick() {
        // The defining behaviour: comparison is always against the frozen
        // moment, so a value that rises and then partially falls back is
        // still marked as up relative to the snapshot.
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("idx", 100, 1.0, 1024)], &[]);

        let tick = index("idx", 400, 1.0, 1024);
        snapshot.observe(std::slice::from_ref(&tick), None);
        assert_eq!(snapshot.index_trends(&tick).doc_count, Trend::Up);

        let tick = index("idx", 200, 1.0, 1024);
        snapshot.observe(std::slice::from_ref(&tick), None);
        assert_eq!(snapshot.index_trends(&tick).doc_count, Trend::Up);

        // ...and only reads as down once it drops below the baseline itself.
        let tick = index("idx", 50, 1.0, 1024);
        snapshot.observe(std::slice::from_ref(&tick), None);
        assert_eq!(snapshot.index_trends(&tick).doc_count, Trend::Down);
    }

    #[test]
    fn test_rows_appearing_after_the_snapshot_adopt_their_first_reading() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("old", 100, 1.0, 1024)], &[]);

        let fresh = index("new", 500, 9.0, 4096);
        snapshot.observe(&[index("old", 100, 1.0, 1024), fresh.clone()], None);

        // First sighting is the baseline, so it starts unmarked...
        assert_eq!(snapshot.index_trends(&fresh).doc_count, Trend::Unchanged);
        // ...and drifts from there.
        assert_eq!(
            snapshot
                .index_trends(&index("new", 600, 9.0, 4096))
                .doc_count,
            Trend::Up
        );
    }

    #[test]
    fn test_disappearing_rows_are_dropped_from_the_baseline() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("a", 1, 1.0, 1), index("b", 2, 2.0, 2)], &[]);
        assert_eq!(snapshot.indices.len(), 2);

        snapshot.observe(&[index("a", 1, 1.0, 1)], None);
        assert_eq!(snapshot.indices.len(), 1);
        assert!(!snapshot.indices.contains_key("b"));
    }

    #[test]
    fn test_unrequested_nodes_leave_the_node_baseline_untouched() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[], &[node("node-1", 10, 0)]);

        // `None` means the nodes panel wasn't on screen, so `_nodes/stats`
        // was never asked for — quite different from an empty node list.
        snapshot.observe(&[], None);
        assert_eq!(snapshot.node_trends(&node("node-1", 40, 0)).cpu, Trend::Up);

        // An actual empty list does clear it.
        snapshot.observe(&[], Some(&[]));
        assert!(snapshot.nodes.is_empty());
    }

    #[test]
    fn test_nodes_fetched_only_after_the_snapshot_still_get_a_baseline() {
        // A snapshot taken while the indices panel was up has no node
        // baseline at all; the first nodes fetch has to supply one, or the
        // nodes panel would never show an arrow.
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[], &[]);

        snapshot.observe(&[], Some(&[node("node-1", 10, 3)]));
        let trends = snapshot.node_trends(&node("node-1", 55, 4));
        assert_eq!(trends.cpu, Trend::Up);
        assert_eq!(trends.index_failed, Trend::Up);
    }

    #[test]
    fn test_clearing_drops_every_baseline() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("idx", 100, 1.0, 1024)], &[node("node-1", 10, 0)]);
        assert!(snapshot.is_active());

        snapshot.clear();
        assert!(!snapshot.is_active());
        assert!(snapshot.taken_at().is_none());
        assert_eq!(
            snapshot
                .index_trends(&index("idx", 999, 9.0, 9999))
                .doc_count,
            Trend::Unchanged
        );
        assert_eq!(
            snapshot.node_trends(&node("node-1", 99, 9)).cpu,
            Trend::Unchanged
        );
    }

    #[test]
    fn test_a_nan_rate_picks_no_direction() {
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[index("idx", 1, f64::NAN, 1)], &[]);
        assert_eq!(
            snapshot.index_trends(&index("idx", 1, 5.0, 1)).rate,
            Trend::Unchanged
        );
    }
}
