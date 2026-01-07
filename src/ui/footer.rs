use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::theme;
use crate::app::App;

pub struct Footer<'a> {
    app: &'a App,
}

impl<'a> Footer<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for Footer<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut spans = vec![
            Span::styled(" [?] ", Style::new().fg(Color::Yellow)),
            Span::raw("Help"),
        ];

        if self.app.filter.active {
            spans.push(Span::raw("  |  "));
            spans.push(Span::styled(
                "Filter mode: type regex, [Esc] to exit, [Ctrl+u] to clear",
                Style::new().fg(Color::Cyan),
            ));
        } else {
            // Status indicators
            spans.push(Span::raw("  |  "));

            // Pause status
            if self.app.paused {
                spans.push(Span::styled(
                    "⏸ PAUSED",
                    Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw("  "));
            }

            // Refresh interval
            spans.push(Span::styled(
                format!("{}s", self.app.refresh_interval.as_secs()),
                Style::new().fg(Color::Cyan),
            ));

            // Toggle states
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "Graph",
                Style::new().fg(if self.app.show_graph {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ));
            spans.push(Span::raw("/"));
            spans.push(Span::styled(
                "Indices",
                Style::new().fg(if self.app.show_indices {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ));
            spans.push(Span::raw("/"));
            spans.push(Span::styled(
                "Hidden",
                Style::new().fg(if self.app.show_system_indices {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ));

            // Colormap indicator
            spans.push(Span::raw("  |  "));
            spans.push(Span::styled(
                format!("{}", self.app.colormap),
                Style::new().fg(Color::Magenta),
            ));

            // Excluded count
            let excluded = self.app.excluded_count();
            if excluded > 0 {
                spans.push(Span::styled(format!("  ✗{}", excluded), theme::ERROR));
            }

            // Index count
            spans.push(Span::raw("  |  "));
            spans.push(Span::styled(
                format!(
                    "{}/{}",
                    self.app.filtered_indices().len(),
                    self.app.indices.len()
                ),
                Style::new().fg(Color::White),
            ));
        }

        Paragraph::new(Line::from(spans))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::BORDER),
            )
            .render(area, buf);
    }
}
