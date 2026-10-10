use super::client::EsClient;
use super::types::{ClusterHealthResponse, StatsResponse};
use crate::error::Result;
use crate::models::{ClusterHealth, IndexRate, IndexSnapshot};
use std::collections::HashMap;
use std::time::Instant;

/// A point-in-time `_stats` reading: when it was taken, and the per-index
/// counters it contained.
pub type Snapshot = (Instant, HashMap<String, IndexSnapshot>);

/// The only leaves of `_stats` the app reads. Without this, every tick pulls
/// both the `primaries` and `total` sections with ~15 indexing counters each,
/// plus `_all` — about 8x the bytes for nothing (measured on ES 9.5: 46 KB
/// vs 5.7 KB for 38 indices), which at tens of thousands of indices is tens
/// of megabytes per tick for the cluster to build and us to parse.
///
/// Index names containing dots (every `.ds-…-2026.10.10-000001` backing
/// index) are matched correctly by `*` on 7.17 and 9.x alike.
const STATS_FILTER_PATH: &str = "indices.*.health,\
indices.*.primaries.docs.count,\
indices.*.primaries.indexing.index_total,\
indices.*.primaries.store.size_in_bytes";

/// Fetches a fresh `_stats` snapshot.
///
/// Deliberately takes `&EsClient` and does no bookkeeping: turning readings
/// into rates needs the previous reading, which lives in `RateTracker`, so
/// the request itself can be `tokio::join!`ed with the cluster-health
/// request and never holds a lock while it waits on the network.
pub async fn fetch_snapshot(client: &EsClient) -> Result<Snapshot> {
    let mut url = client.endpoint(&["_stats", "indexing,docs,store"]);
    url.query_pairs_mut()
        .append_pair("filter_path", STATS_FILTER_PATH);
    let request = client.client.get(url);

    let stats: StatsResponse = client.send_json(request).await?;

    let now = Instant::now();

    // Map stats to internal models
    let snapshot: HashMap<String, IndexSnapshot> = stats
        .indices
        .into_iter()
        .map(|(name, entry)| {
            let snapshot = IndexSnapshot {
                // `primaries` is missing (or empty) when no primary shard
                // copy answered this time; that is a gap in this reading, not
                // an index that has emptied out.
                counters: entry.primaries.map(|primaries| {
                    (
                        primaries.docs.count,
                        primaries.indexing.index_total,
                        primaries.store.size_in_bytes,
                    )
                }),
                // The per-index `health` field only exists from ES 8.1.
                health: entry.health.unwrap_or_default(),
            };
            (name, snapshot)
        })
        .collect();

    Ok((now, snapshot))
}

/// Turns successive `_stats` readings into per-index indexing rates.
///
/// The previous reading is the only state polling needs, which is why it
/// lives here rather than in `EsClient`: keeping the client stateless lets
/// the details popup use it concurrently.
#[derive(Default)]
pub struct RateTracker {
    previous: Option<(Instant, HashMap<String, IndexSnapshot>)>,
}

impl RateTracker {
    /// Diffs a fresh snapshot against the previous one, then keeps it as the
    /// baseline for the next call.
    ///
    /// `rate_per_sec` is `None` where there is nothing honest to report yet:
    /// on the very first reading, for an index seen for the first time, and
    /// for an index whose counter went backwards (it was deleted and
    /// recreated under the same name, or a shard copy restarted), and for a
    /// tick where no primary shard answered for the index. Reporting
    /// 0 there instead would feed a fake idle sample into the smoothing
    /// window and drag every average down while it warmed up.
    pub fn rates_from_snapshot(
        &mut self,
        now: Instant,
        mut current: HashMap<String, IndexSnapshot>,
    ) -> Vec<(IndexRate, Option<f64>)> {
        let previous = self.previous.take();
        let (prev_time, prev_snapshot) = match &previous {
            Some((time, snapshot)) => (Some(*time), Some(snapshot)),
            None => (None, None),
        };
        let elapsed = prev_time.map(|time| now.duration_since(time).as_secs_f64());

        let rates = current
            .iter_mut()
            .map(|(name, entry)| {
                let prev = prev_snapshot.and_then(|prev| prev.get(name));
                let rate = match (entry.counters, prev.and_then(|prev| prev.counters), elapsed) {
                    (Some((_, total, _)), Some((_, prev_total, _)), Some(elapsed))
                        if elapsed > 0.0 && total >= prev_total =>
                    {
                        Some((total - prev_total) as f64 / elapsed)
                    }
                    _ => None,
                };

                // An index missing its counters this tick keeps the last ones
                // seen, so the table holds steady and the next delta is
                // measured from a real reading instead of from zero (which
                // would show the whole lifetime counter as one tick's worth
                // of indexing).
                if entry.counters.is_none() {
                    entry.counters = prev.and_then(|prev| prev.counters);
                }
                let (doc_count, _, size_bytes) = entry.counters.unwrap_or_default();

                let index = IndexRate {
                    name: name.clone(),
                    doc_count,
                    rate_per_sec: rate.unwrap_or(0.0),
                    size_bytes,
                    health: entry.health.clone(),
                };
                (index, rate)
            })
            .collect();

        // Store current snapshot for the next calculation
        self.previous = Some((now, current));

        rates
    }
}

/// Fetches cluster health. Takes `&EsClient` for the same reason as
/// `fetch_snapshot` — so the two can be issued concurrently.
pub async fn fetch_cluster_health(client: &EsClient) -> Result<ClusterHealth> {
    let url = client.endpoint(&["_cluster", "health"]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    type Counters = Option<(u64, u64, u64)>;

    fn reading(entries: &[(&str, Counters)]) -> HashMap<String, IndexSnapshot> {
        entries
            .iter()
            .map(|(name, counters)| {
                (
                    name.to_string(),
                    IndexSnapshot {
                        counters: *counters,
                        health: "green".to_string(),
                    },
                )
            })
            .collect()
    }

    fn rate_of(rates: &[(IndexRate, Option<f64>)], name: &str) -> Option<f64> {
        rates
            .iter()
            .find(|(index, _)| index.name == name)
            .unwrap()
            .1
    }

    #[test]
    fn test_first_reading_and_new_indices_have_no_rate_yet() {
        let mut tracker = RateTracker::default();
        let start = Instant::now();

        let rates = tracker.rates_from_snapshot(start, reading(&[("a", Some((10, 100, 1)))]));
        assert_eq!(rate_of(&rates, "a"), None);

        let rates = tracker.rates_from_snapshot(
            start + Duration::from_secs(2),
            reading(&[("a", Some((10, 300, 1))), ("b", Some((1, 50, 1)))]),
        );
        assert_eq!(rate_of(&rates, "a"), Some(100.0));
        assert_eq!(rate_of(&rates, "b"), None);
    }

    #[test]
    fn test_counter_going_backwards_reports_no_rate() {
        let mut tracker = RateTracker::default();
        let start = Instant::now();
        tracker.rates_from_snapshot(start, reading(&[("a", Some((10, 500, 1)))]));
        let rates = tracker.rates_from_snapshot(
            start + Duration::from_secs(1),
            reading(&[("a", Some((1, 5, 1)))]),
        );
        assert_eq!(rate_of(&rates, "a"), None);
    }

    #[test]
    fn test_missing_primaries_carry_the_last_reading_forward() {
        let mut tracker = RateTracker::default();
        let start = Instant::now();
        tracker.rates_from_snapshot(start, reading(&[("a", Some((10, 100, 7)))]));

        // A tick where no primary answered: last counters are kept, not zeros.
        let rates =
            tracker.rates_from_snapshot(start + Duration::from_secs(1), reading(&[("a", None)]));
        assert_eq!(rates[0].0.doc_count, 10);
        assert_eq!(rates[0].0.size_bytes, 7);
        assert_eq!(rate_of(&rates, "a"), None);

        // When the primary is back, the delta is from the last real reading,
        // not the whole lifetime counter.
        let rates = tracker.rates_from_snapshot(
            start + Duration::from_secs(2),
            reading(&[("a", Some((10, 110, 7)))]),
        );
        assert_eq!(rate_of(&rates, "a"), Some(10.0));
    }
}
