use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

use super::theme;
use crate::app::App;
use crate::utils::{format_bytes, format_number};

pub struct DetailsPopup<'a> {
    app: &'a App,
}

impl<'a> DetailsPopup<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for DetailsPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Calculate popup area (80% width, 80% height, centered)
        let popup_width = (area.width as f32 * 0.8) as u16;
        let popup_height = (area.height as f32 * 0.8) as u16;
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Clear the popup area
        Clear.render(popup_area, buf);

        // Build content
        let mut lines: Vec<Line> = Vec::new();

        if self.app.details.loading {
            lines.push(Line::from(Span::styled(
                "Loading index details...",
                Style::new().fg(Color::Yellow),
            )));
        } else if let Some(ref error) = self.app.details.error {
            lines.push(Line::from(Span::styled(
                format!("Error: {}", error),
                theme::ERROR,
            )));
        } else if let Some(ref details) = self.app.details.data {
            // Index name as header
            lines.push(Line::from(vec![
                Span::styled("Index: ", Style::new().fg(Color::DarkGray)),
                Span::styled(&details.name, theme::TITLE),
            ]));

            // Show provided name if it exists
            if let Some(ref provided_name) = details.provided_name {
                lines.push(Line::from(vec![
                    Span::styled("Provided Name: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(provided_name, theme::TITLE),
                ]));
            }

            // UUID
            if let Some(ref uuid) = details.uuid {
                lines.push(Line::from(vec![
                    Span::styled("UUID: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(uuid, Style::new().fg(Color::White)),
                ]));
            }

            lines.push(Line::from(""));

            // Health and Status
            let health_color = match details.health.as_deref() {
                Some("green") => Color::Green,
                Some("yellow") => Color::Yellow,
                Some("red") => Color::Red,
                _ => Color::DarkGray,
            };
            lines.push(Line::from(vec![
                Span::styled("Health: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    details.health.as_deref().unwrap_or("unknown"),
                    Style::new().fg(health_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Status: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    details.status.as_deref().unwrap_or("unknown"),
                    Style::new().fg(Color::White),
                ),
            ]));

            // Creation date
            lines.push(Line::from(vec![
                Span::styled("Created: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    details.creation_date.as_deref().unwrap_or("unknown"),
                    Style::new().fg(Color::White),
                ),
            ]));

            lines.push(Line::from(""));

            // Document count and size
            lines.push(Line::from(vec![
                Span::styled("Documents: ", Style::new().fg(Color::DarkGray)),
                Span::styled(format_number(details.doc_count as f64), theme::TITLE),
                Span::raw("  "),
                Span::styled("Size: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    format_bytes(details.size_bytes),
                    Style::new().fg(Color::White),
                ),
            ]));

            // Index rate
            let rate_str = format!("{} /s", format_number(details.rate_per_sec));

            let rate_color = if details.rate_per_sec > 10000.0 {
                Color::Red
            } else if details.rate_per_sec > 1000.0 {
                Color::Yellow
            } else if details.rate_per_sec > 0.0 {
                Color::Green
            } else {
                Color::DarkGray
            };

            lines.push(Line::from(vec![
                Span::styled("Index Rate: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    rate_str,
                    Style::new().fg(rate_color).add_modifier(Modifier::BOLD),
                ),
            ]));

            lines.push(Line::from(""));

            // Shards
            let shard_info = if details.is_frozen || details.is_partial {
                format!(
                    "{} primary, {} replicas (Frozen/Searchable Snapshot)",
                    details.primary_shards, details.replica_shards
                )
            } else {
                format!(
                    "{} primary, {} replicas",
                    details.primary_shards, details.replica_shards
                )
            };

            lines.push(Line::from(vec![
                Span::styled("Shards: ", Style::new().fg(Color::DarkGray)),
                Span::styled(shard_info, Style::new().fg(Color::White)),
            ]));

            if details.is_frozen {
                lines.push(Line::from(Span::styled(
                    "  ❄ Index is FROZEN (searchable snapshot from frozen tier)",
                    Style::new().fg(Color::Cyan),
                )));
            }
            if details.is_partial {
                lines.push(Line::from(Span::styled(
                    "  ⚡ Partial index (searchable snapshot)",
                    Style::new().fg(Color::Magenta),
                )));
            }

            // Segments
            lines.push(Line::from(vec![
                Span::styled("Segments: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", details.total_segments),
                    Style::new().fg(Color::White),
                ),
            ]));

            lines.push(Line::from(""));

            // ILM Policy
            lines.push(Line::from(vec![
                Span::styled("ILM Policy: ", Style::new().fg(Color::DarkGray)),
                Span::styled(
                    details.ilm_policy.as_deref().unwrap_or("none"),
                    Style::new().fg(if details.ilm_policy.is_some() {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]));

            if let Some(ref phase) = details.ilm_phase {
                lines.push(Line::from(vec![
                    Span::styled("ILM Phase: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(
                        phase,
                        Style::new().fg(match phase.as_str() {
                            "hot" => Color::Red,
                            "warm" => Color::Yellow,
                            "cold" => Color::Cyan,
                            "frozen" => Color::Blue,
                            "delete" => Color::Magenta,
                            _ => Color::White,
                        }),
                    ),
                ]));
            }

            lines.push(Line::from(""));

            // Data Stream
            if let Some(ref ds) = details.data_stream {
                lines.push(Line::from(Span::styled(
                    "Data Stream:",
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )));

                lines.push(Line::from(vec![
                    Span::styled("  Name: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(&ds.name, theme::TITLE),
                ]));

                let write_indicator = if ds.is_write_index {
                    " (write index)"
                } else {
                    ""
                };
                lines.push(Line::from(vec![
                    Span::styled("  Backing Index: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(
                        format!(
                            "{} of {}{}",
                            ds.backing_index_position, ds.total_backing_indices, write_indicator
                        ),
                        Style::new().fg(if ds.is_write_index {
                            Color::Green
                        } else {
                            Color::White
                        }),
                    ),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("  Generation: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(format!("{}", ds.generation), Style::new().fg(Color::White)),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("  Timestamp Field: ", Style::new().fg(Color::DarkGray)),
                    Span::styled(&ds.timestamp_field, Style::new().fg(Color::White)),
                ]));

                if let Some(ref template) = ds.template {
                    lines.push(Line::from(vec![
                        Span::styled("  Template: ", Style::new().fg(Color::DarkGray)),
                        Span::styled(template, Style::new().fg(Color::Green)),
                    ]));
                }

                if let Some(ref retention) = ds.data_retention {
                    lines.push(Line::from(vec![
                        Span::styled("  Data Retention: ", Style::new().fg(Color::DarkGray)),
                        Span::styled(retention, Style::new().fg(Color::Yellow)),
                    ]));
                }

                lines.push(Line::from(""));
            }

            // Templates
            lines.push(Line::from(vec![
                Span::styled("Templates: ", Style::new().fg(Color::DarkGray)),
                if details.templates.is_empty() {
                    Span::styled("none", Style::new().fg(Color::DarkGray))
                } else {
                    Span::styled(details.templates.join(", "), Style::new().fg(Color::Green))
                },
            ]));

            lines.push(Line::from(""));

            // Shard Allocation
            lines.push(Line::from(Span::styled(
                "Shard Allocation:",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));

            if details.shard_allocation.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No shard information available",
                    Style::new().fg(Color::DarkGray),
                )));
            } else {
                // Group by shard ID
                let mut shards_by_id: std::collections::HashMap<u32, Vec<_>> =
                    std::collections::HashMap::new();
                for shard in &details.shard_allocation {
                    shards_by_id.entry(shard.shard_id).or_default().push(shard);
                }

                let mut shard_ids: Vec<_> = shards_by_id.keys().collect();
                shard_ids.sort();

                for shard_id in shard_ids {
                    if let Some(shards) = shards_by_id.get(shard_id) {
                        let primary = shards.iter().find(|s| s.primary);
                        let replicas: Vec<_> = shards.iter().filter(|s| !s.primary).collect();

                        // Primary shard
                        if let Some(p) = primary {
                            let state_color = match p.state.as_str() {
                                "STARTED" => Color::Green,
                                "RELOCATING" => Color::Yellow,
                                "INITIALIZING" => Color::Cyan,
                                "UNASSIGNED" => Color::Red,
                                _ => Color::White,
                            };

                            let size_str = p.size.as_deref().unwrap_or("-");
                            let docs_str = p
                                .docs
                                .map(|d| format!("{}", d))
                                .unwrap_or_else(|| "-".to_string());

                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("  Shard {} ", shard_id),
                                    Style::new().fg(Color::White),
                                ),
                                Span::styled(
                                    "[P] ",
                                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(&p.node, Style::new().fg(Color::Cyan)),
                                Span::raw(" "),
                                Span::styled(&p.state, Style::new().fg(state_color)),
                                Span::raw(" "),
                                Span::styled(
                                    format!("docs:{} size:{}", docs_str, size_str),
                                    Style::new().fg(Color::DarkGray),
                                ),
                            ]));
                        }

                        // Replica shards
                        for r in replicas {
                            let state_color = match r.state.as_str() {
                                "STARTED" => Color::Green,
                                "RELOCATING" => Color::Yellow,
                                "INITIALIZING" => Color::Cyan,
                                "UNASSIGNED" => Color::Red,
                                _ => Color::White,
                            };

                            let size_str = r.size.as_deref().unwrap_or("-");
                            let docs_str = r
                                .docs
                                .map(|d| format!("{}", d))
                                .unwrap_or_else(|| "-".to_string());

                            lines.push(Line::from(vec![
                                Span::raw("          "),
                                Span::styled("[R] ", Style::new().fg(Color::Yellow)),
                                Span::styled(&r.node, Style::new().fg(Color::Cyan)),
                                Span::raw(" "),
                                Span::styled(&r.state, Style::new().fg(state_color)),
                                Span::raw(" "),
                                Span::styled(
                                    format!("docs:{} size:{}", docs_str, size_str),
                                    Style::new().fg(Color::DarkGray),
                                ),
                            ]));
                        }
                    }
                }
            }
        }

        // Apply scroll offset
        let visible_height = popup_height.saturating_sub(4) as usize; // Account for border and title
        let max_scroll = lines.len().saturating_sub(visible_height);
        let scroll = self.app.details.scroll.min(max_scroll);

        let title = Line::from(vec![
            Span::raw(" Index Details "),
            Span::styled(
                "[Esc/Enter] Close  [j/k] Scroll ",
                Style::new().fg(Color::DarkGray),
            ),
        ]);

        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan)),
            )
            .scroll((scroll as u16, 0))
            .wrap(Wrap { trim: false })
            .render(popup_area, buf);
    }
}
