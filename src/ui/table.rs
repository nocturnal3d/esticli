use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, StatefulWidget, Table, TableState},
};

use super::theme;
use crate::app::App;
use crate::models::IndexRate;
use crate::ui::types::{gradient_position, metric_cell, SortColumn, SortOrder};

pub struct IndicesTable<'a> {
    app: &'a App,
    /// Every index that survives the filter — used for the row count in the
    /// title and for the gradient's scale, both of which must reflect the
    /// whole list rather than the slice on screen.
    visible: &'a [&'a IndexRate],
    /// First row to draw. Only `visible[offset..offset + capacity]` is turned
    /// into `Row`s; building the rest is what made large clusters unusable.
    offset: usize,
}

impl<'a> IndicesTable<'a> {
    pub fn new(app: &'a App, visible: &'a [&'a IndexRate], offset: usize) -> Self {
        Self {
            app,
            visible,
            offset,
        }
    }
}

impl<'a> StatefulWidget for IndicesTable<'a> {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let filtered_count = self.visible.len();
        let total_count = self.app.indices.len();

        // Only the rows that will actually be on screen get built. Each row
        // clones two strings and formats three numbers, so doing this for
        // every index on a 50k-index cluster cost ~93ms per frame — and the
        // loop redraws unconditionally, ~20x a second.
        let end = self
            .offset
            .saturating_add(super::visible_row_capacity(area))
            .min(self.visible.len());
        let window = self.visible.get(self.offset..end).unwrap_or(&[]);

        let header_cells = [
            ("Index Name", SortColumn::Name),
            ("Docs Count", SortColumn::DocCount),
            ("Rate (/s)", SortColumn::Rate),
            ("Size", SortColumn::Size),
            ("Health", SortColumn::Health),
        ]
        .iter()
        .map(|(name, col)| {
            let mut style = Style::new().add_modifier(Modifier::BOLD);
            let mut text = name.to_string();

            if *col == self.app.sort.column {
                style = style.fg(Color::Yellow);
                let arrow = match self.app.sort.order {
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

        // Scale the gradient across the whole filtered list, not just the
        // window: normalising against the visible slice would make a row's
        // color change as you scroll past it.
        let max_value: f64 = self
            .visible
            .iter()
            .map(|i| match self.app.sort.column {
                SortColumn::Name | SortColumn::Health => 0.0,
                SortColumn::DocCount => i.doc_count as f64,
                SortColumn::Rate => i.rate_per_sec,
                SortColumn::Size => i.size_bytes as f64,
            })
            .fold(0.0_f64, f64::max);

        let rows: Vec<Row> = window
            .iter()
            .map(|index| {
                let style = match self.app.sort.column {
                    SortColumn::Name | SortColumn::Health => {
                        let color = match index.health.as_str() {
                            "green" => Color::Green,
                            "yellow" => Color::Yellow,
                            "red" => Color::Red,
                            _ => Color::default(),
                        };
                        Style::new().fg(color)
                    }
                    _ => {
                        // Calculate gradient position based on current sort column value
                        let current_value = match self.app.sort.column {
                            SortColumn::DocCount => index.doc_count as f64,
                            SortColumn::Rate => index.rate_per_sec,
                            SortColumn::Size => index.size_bytes as f64,
                            _ => 0.0,
                        };

                        // Shared with the nodes table so a row's color means
                        // the same thing in both.
                        let position = gradient_position(current_value, max_value);
                        let color = self.app.colormap.color_at(position);
                        Style::new().fg(color)
                    }
                };

                // Movement away from the snapshot baseline, if one has been
                // taken. Looked up per on-screen row rather than precomputed
                // for the whole list, for the same reason the rows
                // themselves are: only the window is ever built.
                let trends = self.app.snapshot.index_trends(index);

                let cells = [
                    Cell::from(index.name.clone()),
                    metric_cell(index.doc_count_human(), trends.doc_count),
                    metric_cell(index.rate_human(), trends.rate),
                    metric_cell(index.size_human(), trends.size),
                    Cell::from(index.health.clone()),
                ];

                Row::new(cells).style(style)
            })
            .collect();

        // Fixed widths for the metric columns, as in the nodes table: each
        // has to fit its header plus a sort arrow *and* a value plus a
        // snapshot trend arrow, and percentages that look generous silently
        // truncate both at 80 columns. The name takes whatever is left.
        let widths = [
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(13),
            Constraint::Length(9),
        ];

        // Left half of the title is the shared tab strip plus the filter
        // box; the spinner, fetch duration and row count are right-aligned by
        // `panel_title::status`.
        let mut title_spans = super::panel_title::tabs(self.app);
        title_spans.extend(super::panel_title::filter_spans(self.app));

        if self.app.paused {
            title_spans.push(Span::styled(
                " ⏸ PAUSED",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        title_spans.push(Span::raw(" "));
        let title = Line::from(title_spans);
        let status =
            super::panel_title::status(self.app, format!("({}/{})", filtered_count, total_count));

        let border_style = if self.app.paused {
            Style::new().fg(Color::Yellow)
        } else {
            theme::BORDER
        };

        // No offset adjustment here on purpose: the incoming `state` carries
        // the previous frame's offset (see `ui::draw`) and `Table` scrolls it
        // just far enough to keep the selection visible.
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title_top(title)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::elasticsearch::AuthConfig;
    use crate::ui::types::Colormap;
    use ratatui::buffer::Buffer;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 8,
    };

    fn app_with(doc_counts: [u64; 2]) -> App {
        let mut app = App::new(
            "http://localhost:9200".to_string(),
            AuthConfig::None,
            false,
            None,
            5,
            Colormap::default(),
            10,
        )
        .unwrap();

        app.indices = doc_counts
            .iter()
            .enumerate()
            .map(|(i, doc_count)| IndexRate {
                name: format!("index-{}", i),
                doc_count: *doc_count,
                rate_per_sec: 1.0,
                size_bytes: 1024,
                health: "green".to_string(),
            })
            .collect();
        app
    }

    fn rendered(app: &App) -> String {
        let visible = app.filtered_indices();
        let mut buf = Buffer::empty(AREA);
        let mut state = TableState::default();
        IndicesTable::new(app, &visible, 0).render(AREA, &mut buf, &mut state);

        (0..AREA.height)
            .map(|y| {
                (0..AREA.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_trend_arrows_reach_the_screen_and_only_where_earned() {
        let mut app = app_with([100, 100]);

        // Without a snapshot the table renders exactly as it always did.
        let before = rendered(&app);
        assert!(!before.contains('↑'), "unexpected arrow:\n{}", before);
        assert!(!before.contains('↓'), "unexpected arrow:\n{}", before);

        app.take_snapshot();
        app.indices[0].doc_count = 500;
        app.indices[1].doc_count = 50;

        let after = rendered(&app);
        // One row went up, the other down, and the columns are wide enough
        // that neither arrow gets truncated away at 80 columns.
        assert!(after.contains('↑'), "missing up arrow:\n{}", after);
        assert!(after.contains('↓'), "missing down arrow:\n{}", after);

        // The rate and size held still, so those cells stay bare — an
        // unchanged metric must not grow a placeholder.
        assert_eq!(after.matches('↑').count(), 1, "{}", after);
        assert_eq!(after.matches('↓').count(), 1, "{}", after);
    }
}
