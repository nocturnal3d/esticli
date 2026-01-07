use crate::models::IndexRate;
use crate::ui::types::{SortColumn, SortOrder};

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
                SortColumn::Health => index_a.health.cmp(&index_b.health),
            };

            match self.order {
                SortOrder::Ascending => cmp,
                SortOrder::Descending => cmp.reverse(),
            }
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
