use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, StatefulWidget, Table, TableState},
};

use super::theme;
use crate::app::App;
use crate::models::NodeStats;
use crate::ui::types::{gradient_position, NodeSortColumn, SortOrder};

pub struct NodesTable<'a> {
    app: &'a App,
    /// Every node that survives the filter — used for the row count and the
    /// gradient's scale, which must reflect the whole list, not the window.
    visible: &'a [&'a NodeStats],
    /// First row to draw; only the on-screen window is turned into `Row`s,
    /// matching the indices table.
    offset: usize,
}

impl<'a> NodesTable<'a> {
    pub fn new(app: &'a App, visible: &'a [&'a NodeStats], offset: usize) -> Self {
        Self {
            app,
            visible,
            offset,
        }
    }

    /// The value the gradient is keyed on for a given row: whichever column
    /// the table is currently sorted by, matching how the indices table
    /// colors its rows. Sorting by name has no magnitude to shade, so those
    /// rows are left unshaded.
    fn sort_value(&self, node: &NodeStats) -> Option<f64> {
        match self.app.node_sort.column {
            NodeSortColumn::Name => None,
            NodeSortColumn::Heap => Some(node.heap_used_percent as f64),
            NodeSortColumn::IndexFailed => Some(node.index_failed as f64),
            NodeSortColumn::BulkAvgSize => Some(node.bulk_avg_size_bytes as f64),
            NodeSortColumn::Cpu => Some(node.cpu_percent as f64),
            NodeSortColumn::BreakerTripped => Some(node.breaker_parent_tripped as f64),
        }
    }
}

impl<'a> StatefulWidget for NodesTable<'a> {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let header_cells = [
            ("Node", NodeSortColumn::Name),
            ("CPU %", NodeSortColumn::Cpu),
            ("Heap %", NodeSortColumn::Heap),
            ("IdxFailed", NodeSortColumn::IndexFailed),
            ("BlkAvgSize", NodeSortColumn::BulkAvgSize),
            ("BrkTripd", NodeSortColumn::BreakerTripped),
        ]
        .iter()
        .map(|(name, col)| {
            let mut style = Style::new().add_modifier(Modifier::BOLD);
            let mut text = name.to_string();

            if *col == self.app.node_sort.column {
                style = style.fg(Color::Yellow);
                let arrow = match self.app.node_sort.order {
                    SortOrder::Ascending => " ▲",
                    SortOrder::Descending => " ▼",
                };
                text.push_str(arrow);
            }

            Cell::from(text).style(style)
        });

        let header = Row::new(header_cells)
            .style(Style::new().bg(Color::DarkGray))
            .height(1);

        // Gradient is normalized against the largest value in the sorted
        // column, exactly as the indices table does, so the hottest node is
        // always at the top of the colormap and the rest read relative to it.
        let end = self
            .offset
            .saturating_add(super::visible_row_capacity(area))
            .min(self.visible.len());
        let window = self.visible.get(self.offset..end).unwrap_or(&[]);

        let max_value: f64 = self
            .visible
            .iter()
            .filter_map(|node| self.sort_value(node))
            .fold(0.0_f64, f64::max);

        let rows: Vec<Row> = window
            .iter()
            .map(|node| {
                let style = match self.sort_value(node) {
                    Some(value) => {
                        let position = gradient_position(value, max_value);
                        Style::new().fg(self.app.colormap.color_at(position))
                    }
                    None => Style::new(),
                };

                let cells = [
                    Cell::from(node.name.clone()),
                    Cell::from(format!("{}%", node.cpu_percent)),
                    Cell::from(format!("{}%", node.heap_used_percent)),
                    Cell::from(node.index_failed_human()),
                    Cell::from(node.bulk_avg_size_human()),
                    Cell::from(node.breaker_tripped_human()),
                ];

                Row::new(cells).style(style)
            })
            .collect();

        // Metric columns are fixed-width rather than percentages: each one has
        // to fit its header *plus* the sort arrow, and `Table` also spends a
        // column of spacing between them, so a percentage that looks wide
        // enough silently truncates the arrow at common terminal widths. The
        // node name takes whatever is left over.
        let widths = [
            Constraint::Fill(1),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(12),
            Constraint::Length(13),
            Constraint::Length(11),
        ];

        // Same tab strip, filter box and right-aligned status as the indices
        // table — the filter drives whichever panel is on screen.
        let mut title_spans = super::panel_title::tabs(self.app);
        title_spans.extend(super::panel_title::filter_spans(self.app));

        if self.app.paused {
            title_spans.push(Span::styled(
                " ⏸ PAUSED",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        title_spans.push(Span::raw(" "));
        let status = super::panel_title::status(
            self.app,
            format!("({}/{})", self.visible.len(), self.app.nodes.len()),
        );

        let border_style = if self.app.paused {
            Style::new().fg(Color::Yellow)
        } else {
            theme::BORDER
        };

        // As in the indices table, the offset arrives from `ui::draw` and is
        // left for ratatui to nudge only if the selection would fall outside.
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title_top(Line::from(title_spans))
                    .title_top(status),
            )
            .row_highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .add_modifier(Modifier::BOLD),
            );

        StatefulWidget::render(table, area, buf, state);
    }
}
