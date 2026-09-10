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

/// How a metric behaves over time, which is what decides how a move away from
/// the baseline should be read.
///
/// This is a property of the *column*, not of any one row, and it exists
/// because the two kinds mean opposite things when they fall. It is the piece
/// a configurable column list would carry alongside a label, an accessor and
/// a formatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetricKind {
    /// An instantaneous reading that moves both ways — CPU %, heap %, the
    /// indexing rate, an average bulk size. A fall is a genuine fall, and
    /// returning to the baseline number genuinely means "back where it
    /// started", so the arrow clears. Doc counts and store sizes are gauges
    /// too: they trend upwards, but deletes, merges and ILM shrink them for
    /// real, and reporting that as anything other than a decrease would hide
    /// it.
    #[default]
    Gauge,
    /// A total that only ever accumulates, and goes backwards solely when
    /// whatever owns it restarts — failed indexing operations and
    /// circuit-breaker trips, both of which reset when a node does. A drop is
    /// therefore not an improvement but a lost history, and must not be drawn
    /// as a down arrow: `0 ↓` after a node restart reads as "failures went
    /// away" when the truth is the opposite.
    Counter,
}

/// Which way a metric has moved relative to its snapshot baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trend {
    #[default]
    Unchanged,
    Up,
    Down,
    /// A counter has restarted since the snapshot: it was seen to go
    /// backwards, so its baseline no longer refers to the same run of the
    /// thing being counted.
    Reset,
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
            Trend::Reset => " ⟲",
            Trend::Unchanged => "",
        }
    }

    /// Compares a current reading against its baseline, reading a fall
    /// according to what kind of metric it is. Values that don't order
    /// against each other (a NaN rate) count as unchanged rather than picking
    /// a direction arbitrarily.
    fn compare<T: PartialOrd>(kind: MetricKind, current: T, baseline: T) -> Self {
        match current.partial_cmp(&baseline) {
            Some(Ordering::Greater) => Trend::Up,
            Some(Ordering::Less) => match kind {
                MetricKind::Gauge => Trend::Down,
                MetricKind::Counter => Trend::Reset,
            },
            _ => Trend::Unchanged,
        }
    }
}

/// One cumulative counter's baseline, plus whether it has been seen to
/// restart since the snapshot was taken.
///
/// The flag is needed because a restart is handled by re-baselining to the
/// post-restart value: without it, the very next comparison would be "equal
/// to baseline" and report `Unchanged`, claiming nothing had happened at the
/// exact moment something did.
#[derive(Debug, Clone, Copy)]
struct CounterBaseline {
    baseline: u64,
    reset: bool,
}

impl CounterBaseline {
    fn new(current: u64) -> Self {
        Self {
            baseline: current,
            reset: false,
        }
    }

    /// Reconciles the counter against a fresh reading, once per refresh tick.
    ///
    /// A counter that has gone backwards has restarted, so it is re-baselined
    /// to the value it restarted from. Without that, growth after a node
    /// restart would stay invisible until the counter climbed all the way
    /// back past its pre-restart total — which for a node that had logged
    /// thousands of failures could be never, hiding exactly the failures
    /// somebody watching would most want to see.
    fn observe(&mut self, current: u64) {
        if current < self.baseline {
            self.baseline = current;
            self.reset = true;
        }
    }

    fn trend(&self, current: u64) -> Trend {
        match Trend::compare(MetricKind::Counter, current, self.baseline) {
            // Sitting exactly on the baseline after a restart is not the same
            // as never having moved, so it keeps saying so rather than
            // silently reading as unchanged.
            Trend::Unchanged if self.reset => Trend::Reset,
            trend => trend,
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
///
/// The two `MetricKind::Counter` metrics carry a little more state than the
/// gauges do, since a counter can restart underneath the snapshot and the
/// baseline has to follow it when it does.
#[derive(Debug, Clone, Copy)]
struct NodeBaseline {
    cpu_percent: u64,
    heap_used_percent: u64,
    index_failed: CounterBaseline,
    bulk_avg_size_bytes: u64,
    breaker_parent_tripped: CounterBaseline,
}

impl From<&NodeStats> for NodeBaseline {
    fn from(node: &NodeStats) -> Self {
        Self {
            cpu_percent: node.cpu_percent,
            heap_used_percent: node.heap_used_percent,
            index_failed: CounterBaseline::new(node.index_failed),
            bulk_avg_size_bytes: node.bulk_avg_size_bytes,
            breaker_parent_tripped: CounterBaseline::new(node.breaker_parent_tripped),
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
            match self.nodes.get_mut(&node.name) {
                // An existing baseline keeps its gauges frozen, but its
                // counters have to be followed: if the node restarted, they
                // are now counting from zero and the old totals no longer
                // refer to the same run.
                Some(baseline) => {
                    baseline.index_failed.observe(node.index_failed);
                    baseline
                        .breaker_parent_tripped
                        .observe(node.breaker_parent_tripped);
                }
                None => {
                    self.nodes
                        .insert(node.name.clone(), NodeBaseline::from(node));
                }
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

        // Every indices-panel metric is a gauge. Doc count and size trend
        // upwards but genuinely shrink — deletes, segment merges, ILM — and a
        // real decrease there is worth seeing rather than being explained
        // away as a counter restart.
        IndexTrends {
            doc_count: Trend::compare(MetricKind::Gauge, index.doc_count, baseline.doc_count),
            rate: Trend::compare(MetricKind::Gauge, index.rate_per_sec, baseline.rate_per_sec),
            size: Trend::compare(MetricKind::Gauge, index.size_bytes, baseline.size_bytes),
        }
    }

    /// How each of a node's numeric columns has moved.
    pub fn node_trends(&self, node: &NodeStats) -> NodeTrends {
        let Some(baseline) = self.nodes.get(&node.name) else {
            return NodeTrends::default();
        };

        // CPU, heap and the average bulk size are instantaneous readings;
        // failed indexing operations and breaker trips are cumulative and
        // reset with the node, so they go through `CounterBaseline`.
        NodeTrends {
            cpu: Trend::compare(MetricKind::Gauge, node.cpu_percent, baseline.cpu_percent),
            heap: Trend::compare(
                MetricKind::Gauge,
                node.heap_used_percent,
                baseline.heap_used_percent,
            ),
            index_failed: baseline.index_failed.trend(node.index_failed),
            bulk_avg_size: Trend::compare(
                MetricKind::Gauge,
                node.bulk_avg_size_bytes,
                baseline.bulk_avg_size_bytes,
            ),
            breaker_tripped: baseline
                .breaker_parent_tripped
                .trend(node.breaker_parent_tripped),
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
    fn test_a_restarting_counter_reads_as_a_reset_not_an_improvement() {
        // The case this whole distinction exists for: a node restarts, its
        // failure counter goes back to zero, and the old behaviour drew that
        // as `0 ↓` — indistinguishable from failures genuinely falling away.
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[], &[node("node-1", 10, 500)]);

        // Accumulating normally.
        let tick = node("node-1", 10, 530);
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).index_failed, Trend::Up);

        // Restart: the counter falls, which a counter cannot genuinely do.
        let tick = node("node-1", 10, 0);
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).index_failed, Trend::Reset);
        assert_eq!(Trend::Reset.arrow(), " ⟲");

        // Re-baselined to the restart, so new failures show immediately
        // rather than staying hidden until the count passes 500 again.
        let tick = node("node-1", 10, 14);
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).index_failed, Trend::Up);
    }

    #[test]
    fn test_a_gauge_falling_is_still_a_plain_decrease() {
        // Counters and gauges must part company only on the way down: CPU is
        // an instantaneous reading, so a fall is a fall.
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[], &[node("node-1", 80, 0)]);

        let tick = node("node-1", 20, 0);
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).cpu, Trend::Down);

        // Doc count and size are gauges too: they trend up, but deletes and
        // merges shrink them for real and that must not read as a reset.
        snapshot.capture(&[index("idx", 1000, 1.0, 8192)], &[]);
        let trends = snapshot.index_trends(&index("idx", 400, 1.0, 2048));
        assert_eq!(trends.doc_count, Trend::Down);
        assert_eq!(trends.size, Trend::Down);
    }

    #[test]
    fn test_a_reset_counter_sitting_on_its_new_baseline_still_says_so() {
        // After a restart the counter equals its re-baselined value, and
        // "unchanged" would claim nothing had happened at the exact moment
        // something did.
        let mut snapshot = SnapshotState::default();
        snapshot.capture(&[], &[node("node-1", 10, 42)]);

        let tick = node("node-1", 10, 0);
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).index_failed, Trend::Reset);

        // Still quiet several ticks later: it stays flagged for the life of
        // the snapshot rather than quietly reverting to "unchanged".
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));
        assert_eq!(snapshot.node_trends(&tick).index_failed, Trend::Reset);

        // A fresh snapshot starts clean.
        snapshot.capture(&[], &[node("node-1", 10, 0)]);
        assert_eq!(
            snapshot.node_trends(&node("node-1", 10, 0)).index_failed,
            Trend::Unchanged
        );
    }

    #[test]
    fn test_both_node_counters_reset_independently() {
        let mut snapshot = SnapshotState::default();
        let mut base = node("node-1", 10, 500);
        base.breaker_parent_tripped = 7;
        snapshot.capture(&[], &[base]);

        // Only the breaker counter falls; the failure counter keeps climbing.
        let mut tick = node("node-1", 10, 600);
        tick.breaker_parent_tripped = 0;
        snapshot.observe(&[], Some(std::slice::from_ref(&tick)));

        let trends = snapshot.node_trends(&tick);
        assert_eq!(trends.index_failed, Trend::Up);
        assert_eq!(trends.breaker_tripped, Trend::Reset);
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
