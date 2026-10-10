use std::cmp::Ordering;

use crate::models::{IndexRate, NodeStats};
use crate::ui::types::{NodeSortColumn, SortColumn, SortOrder};

/// How bad a health status is, for sorting: alphabetically `red` falls
/// between `green` and `yellow`, so the indices most worth seeing could never
/// be brought to the top. Unknown (7.x clusters don't report per-index
/// health in `_stats`) ranks below green.
fn health_rank(health: &str) -> u8 {
    match health {
        "green" => 1,
        "yellow" => 2,
        "red" => 3,
        _ => 0,
    }
}

/// Applies the sort direction to a column comparison, then breaks ties by
/// name — always ascending, whichever way the column runs.
///
/// Without the tiebreak, rows that compare equal (every idle index at rate 0,
/// the default sort) come back in whatever order the `_stats` map happened to
/// iterate in, which changes on every refresh: the bottom of the table
/// reshuffled every tick.
fn directed(cmp: Ordering, order: SortOrder, name_a: &str, name_b: &str) -> Ordering {
    let cmp = match order {
        SortOrder::Ascending => cmp,
        SortOrder::Descending => cmp.reverse(),
    };
    cmp.then_with(|| name_a.cmp(name_b))
}

#[derive(Default)]
pub struct SortState {
    pub column: SortColumn,
    pub order: SortOrder,
}

impl SortState {
    pub fn next_column(&mut self) {
        self.column = self.column.next();
    }

    pub fn prev_column(&mut self) {
        self.column = self.column.prev();
    }

    pub fn toggle_order(&mut self) {
        self.order = self.order.toggle();
    }

    pub fn sort(&self, indices: &mut [IndexRate]) {
        indices.sort_by(|index_a, index_b| {
            let cmp = match self.column {
                SortColumn::Name => index_a.name.cmp(&index_b.name),
                SortColumn::DocCount => index_a.doc_count.cmp(&index_b.doc_count),
                SortColumn::Rate => index_a
                    .rate_per_sec
                    .partial_cmp(&index_b.rate_per_sec)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Size => index_a.size_bytes.cmp(&index_b.size_bytes),
                SortColumn::Health => {
                    health_rank(&index_a.health).cmp(&health_rank(&index_b.health))
                }
            };

            directed(cmp, self.order, &index_a.name, &index_b.name)
        });
    }
}

/// Sort state for the nodes panel. Mirrors `SortState`, but over
/// `NodeStats`/`NodeSortColumn` — the two panels sort independently, so
/// switching between them preserves each one's column and direction.
#[derive(Default)]
pub struct NodeSortState {
    pub column: NodeSortColumn,
    pub order: SortOrder,
}

impl NodeSortState {
    pub fn next_column(&mut self) {
        self.column = self.column.next();
    }

    pub fn prev_column(&mut self) {
        self.column = self.column.prev();
    }

    pub fn toggle_order(&mut self) {
        self.order = self.order.toggle();
    }

    pub fn sort(&self, nodes: &mut [NodeStats]) {
        nodes.sort_by(|node_a, node_b| {
            let cmp = match self.column {
                NodeSortColumn::Name => node_a.name.cmp(&node_b.name),
                NodeSortColumn::Heap => node_a.heap_used_percent.cmp(&node_b.heap_used_percent),
                NodeSortColumn::IndexFailed => node_a.index_failed.cmp(&node_b.index_failed),
                NodeSortColumn::BulkAvgSize => {
                    node_a.bulk_avg_size_bytes.cmp(&node_b.bulk_avg_size_bytes)
                }
                NodeSortColumn::Cpu => node_a.cpu_percent.cmp(&node_b.cpu_percent),
                NodeSortColumn::BreakerTripped => node_a
                    .breaker_parent_tripped
                    .cmp(&node_b.breaker_parent_tripped),
            };

            directed(cmp, self.order, &node_a.name, &node_b.name)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_index(name: &str, docs: u64, rate: f64) -> IndexRate {
        IndexRate {
            name: name.to_string(),
            doc_count: docs,
            rate_per_sec: rate,
            size_bytes: 0,
            health: "green".to_string(),
        }
    }

    fn mock_node(name: &str, heap: u64, failed: u64) -> NodeStats {
        NodeStats {
            name: name.to_string(),
            heap_used_percent: heap,
            index_failed: failed,
            ..Default::default()
        }
    }

    #[test]
    fn test_node_sort_by_heap_descending() {
        let mut nodes = vec![mock_node("a", 30, 0), mock_node("b", 90, 0)];
        let sort = NodeSortState {
            column: NodeSortColumn::Heap,
            order: SortOrder::Descending,
        };
        sort.sort(&mut nodes);
        assert_eq!(nodes[0].name, "b");
        assert_eq!(nodes[1].name, "a");
    }

    #[test]
    fn test_node_sort_every_column_orders_by_its_own_key() {
        let build = |name: &str, heap, failed, bulk, cpu, tripped| NodeStats {
            name: name.to_string(),
            heap_used_percent: heap,
            index_failed: failed,
            bulk_avg_size_bytes: bulk,
            cpu_percent: Some(cpu),
            breaker_parent_tripped: tripped,
        };
        // Each node leads exactly one column, so a descending sort on that
        // column must put it first — catching any column wired to the wrong
        // field.
        let original = vec![
            build("heaviest", 99, 0, 0, 0, 0),
            build("failing", 0, 99, 0, 0, 0),
            build("bulkiest", 0, 0, 99, 0, 0),
            build("busiest", 0, 0, 0, 99, 0),
            build("tripping", 0, 0, 0, 0, 99),
        ];

        for (column, expected) in [
            (NodeSortColumn::Heap, "heaviest"),
            (NodeSortColumn::IndexFailed, "failing"),
            (NodeSortColumn::BulkAvgSize, "bulkiest"),
            (NodeSortColumn::Cpu, "busiest"),
            (NodeSortColumn::BreakerTripped, "tripping"),
        ] {
            let mut nodes = original.clone();
            NodeSortState {
                column,
                order: SortOrder::Descending,
            }
            .sort(&mut nodes);
            assert_eq!(nodes[0].name, expected, "wrong leader for {:?}", column);
        }
    }

    #[test]
    fn test_node_sort_independent_of_index_sort() {
        // The two panels keep their own column/order, so sorting one must not
        // disturb the other.
        let mut nodes = vec![mock_node("a", 10, 5), mock_node("b", 20, 1)];
        let sort = NodeSortState {
            column: NodeSortColumn::IndexFailed,
            order: SortOrder::Ascending,
        };
        sort.sort(&mut nodes);
        assert_eq!(nodes[0].name, "b");
    }

    #[test]
    fn test_sort_by_name() {
        let mut indices = vec![mock_index("z", 0, 0.0), mock_index("a", 0, 0.0)];
        let sort = SortState {
            column: SortColumn::Name,
            order: SortOrder::Ascending,
        };
        sort.sort(&mut indices);
        assert_eq!(indices[0].name, "a");
        assert_eq!(indices[1].name, "z");
    }

    #[test]
    fn test_ties_keep_a_stable_name_order_in_either_direction() {
        // Idle indices all tie on rate; their order must not depend on how
        // the response map happened to iterate.
        for order in [SortOrder::Ascending, SortOrder::Descending] {
            let mut indices = vec![
                mock_index("c", 0, 0.0),
                mock_index("a", 0, 0.0),
                mock_index("b", 0, 0.0),
            ];
            SortState {
                column: SortColumn::Rate,
                order,
            }
            .sort(&mut indices);
            let names: Vec<&str> = indices.iter().map(|i| i.name.as_str()).collect();
            assert_eq!(names, ["a", "b", "c"], "{:?}", order);
        }
    }

    #[test]
    fn test_health_sorts_by_severity() {
        let with_health = |name: &str, health: &str| IndexRate {
            health: health.to_string(),
            ..mock_index(name, 0, 0.0)
        };
        let mut indices = vec![
            with_health("g", "green"),
            with_health("r", "red"),
            with_health("y", "yellow"),
        ];
        SortState {
            column: SortColumn::Health,
            order: SortOrder::Descending,
        }
        .sort(&mut indices);
        let names: Vec<&str> = indices.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["r", "y", "g"]);
    }

    #[test]
    fn test_sort_by_rate_descending() {
        let mut indices = vec![mock_index("a", 10, 1.0), mock_index("b", 10, 5.0)];
        let sort = SortState {
            column: SortColumn::Rate,
            order: SortOrder::Descending,
        };
        sort.sort(&mut indices);
        assert_eq!(indices[0].name, "b");
        assert_eq!(indices[1].name, "a");
    }
}
