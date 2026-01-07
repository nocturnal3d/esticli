use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Row, StatefulWidget, Table, TableState},
};

use super::theme;
use crate::app::App;
use crate::ui::types::{SortColumn, SortOrder};

pub struct IndicesTable<'a> {
    app: &'a App,
}

impl<'a> IndicesTable<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> StatefulWidget for IndicesTable<'a> {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Get filtered indices once
        let filtered_indices = self.app.filtered_indices();
        let filtered_count = filtered_indices.len();
        let total_count = self.app.indices.len();

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

        // Find max value for gradient calculation based on current sort column
        let max_value: f64 = filtered_indices
            .iter()
            .map(|i| match self.app.sort.column {
                SortColumn::Name | SortColumn::Health => 0.0,
                SortColumn::DocCount => i.doc_count as f64,
                SortColumn::Rate => i.rate_per_sec,
                SortColumn::Size => i.size_bytes as f64,
            })
            .fold(0.0_f64, f64::max);

        let rows: Vec<Row> = filtered_indices
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

                        // Use logarithmic scale to spread colors more evenly
                        let position = if max_value > 0.0 {
                            let log_current = (1.0 + current_value).ln();
                            let log_max = (1.0 + max_value).ln();
                            1.0 - (log_current / log_max) as f32
                        } else {
                            1.0 // No gradient or zero values
                        };

                        let color = self.app.colormap.color_at(position);
                        Style::new().fg(color)
                    }
                };

                let cells = [
                    Cell::from(index.name.clone()),
                    Cell::from(index.doc_count_human()),
                    Cell::from(index.rate_human()),
                    Cell::from(index.size_human()),
                    Cell::from(index.health.clone()),
                ];

                Row::new(cells).style(style)
            })
            .collect();

        let widths = [
            Constraint::Percentage(60),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
        ];

        // Create title
        let spinner = self.app.spinner_char();
        let duration = self.app.fetch_duration_display();
        let spinner_color = if self.app.loading {
            Color::Cyan
        } else {
            Color::Green
        };

        let mut title_spans = vec![
            Span::raw(" Indices "),
            Span::styled(
                format!("{}", spinner),
                Style::new().fg(spinner_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(format!("({})", duration), theme::TIME),
        ];

        // Add filter display
        let filter_value = self.app.filter.input.value();
        if self.app.filter.active || !filter_value.is_empty() {
            title_spans.push(Span::raw(" | "));
            title_spans.push(Span::styled("Filter: ", Style::new().fg(Color::Yellow)));

            let filter_style = if self.app.filter.error.is_some() {
                theme::ERROR
            } else if self.app.filter.active {
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::Green)
            };

            if self.app.filter.active {
                let cursor = self.app.filter.input.cursor();
                let (before, after) = filter_value.split_at(cursor);
                if !before.is_empty() {
                    title_spans.push(Span::styled(before.to_string(), filter_style));
                }
                title_spans.push(Span::styled(
                    "▏",
                    Style::new()
                        .fg(Color::White)
                        .add_modifier(Modifier::RAPID_BLINK),
                ));
                if !after.is_empty() {
                    title_spans.push(Span::styled(after.to_string(), filter_style));
                }
            } else {
                title_spans.push(Span::styled(filter_value, filter_style));
            }

            // Show match count
            title_spans.push(Span::styled(
                format!(" ({}/{})", filtered_count, total_count),
                theme::TIME,
            ));
        }

        if self.app.paused {
            title_spans.push(Span::styled(
                " ⏸ PAUSED",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }

        title_spans.push(Span::raw(" "));
        let title = Line::from(title_spans);

        let border_style = if self.app.paused {
            Style::new().fg(Color::Yellow)
        } else {
            theme::BORDER
        };

        let available_height = area.height.saturating_sub(3) as usize;

        if let Some(selected) = self.app.selected_index {
            let total_rows = rows.len();
            if total_rows > available_height {
                let center_offset = available_height / 2;
                let ideal_offset = selected.saturating_sub(center_offset);
                let max_offset = total_rows.saturating_sub(available_height);
                let offset = ideal_offset.min(max_offset);

                *state = state.clone().with_offset(offset);
            }
        }

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .row_highlight_style(
                Style::new()
                    .add_modifier(Modifier::REVERSED)
                    .add_modifier(Modifier::BOLD),
            );

        StatefulWidget::render(table, area, buf, state);
    }
}
