use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::theme;
use crate::app::filter::FilterMode;
use crate::app::{App, MainPanel};

pub struct Footer<'a> {
    app: &'a App,
    filtered_count: usize,
    total_count: usize,
}

impl<'a> Footer<'a> {
    pub fn new(app: &'a App, filtered_count: usize, total_count: usize) -> Self {
        Self {
            app,
            filtered_count,
            total_count,
        }
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
                match self.app.filter.mode {
                    FilterMode::Regex => {
                        "Filter mode: type regex, [Esc] to exit, [Ctrl+u] to clear"
                    }
                    FilterMode::Jq => {
                        "Filter mode: type jq expression, [Esc] to exit, [Ctrl+u] to clear"
                    }
                },
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
            // Named after whichever table the panel currently holds, so it
            // doubles as an indicator of which view you're in.
            spans.push(Span::styled(
                match self.app.main_panel {
                    MainPanel::Indices => "Indices",
                    MainPanel::Nodes => "Nodes",
                },
                Style::new().fg(if self.app.show_main_panel {
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

            // Snapshot baseline, with the moment it was taken — the trend
            // arrows in the table are meaningless without knowing what they
            // are counting from.
            if let Some(taken_at) = self.app.snapshot.taken_at() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    format!("⇅ {}", taken_at.format("%H:%M:%S")),
                    theme::SNAPSHOT,
                ));
            }

            // Excluded count
            let excluded = self.app.excluded_count();
            if excluded > 0 {
                spans.push(Span::styled(format!("  ✗{}", excluded), theme::ERROR));
            }

            // Index count
            spans.push(Span::raw("  |  "));
            spans.push(Span::styled(
                format!("{}/{}", self.filtered_count, self.total_count),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elasticsearch::AuthConfig;
    use crate::ui::types::Colormap;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 3,
    };

    fn app() -> App {
        App::new(
            "http://localhost:9200".to_string(),
            AuthConfig::None,
            false,
            None,
            5,
            Colormap::default(),
            10,
        )
        .unwrap()
    }

    fn rendered(app: &App) -> String {
        let mut buf = Buffer::empty(AREA);
        Footer::new(app, 0, 0).render(AREA, &mut buf);
        (0..AREA.width).map(|x| buf[(x, 1)].symbol()).collect()
    }

    #[test]
    fn test_filter_hint_names_the_mode_being_typed() {
        let mut app = app();
        app.filter.enter();
        assert!(rendered(&app).contains("type regex"));

        app.filter.toggle_mode();
        let text = rendered(&app);
        assert!(text.contains("type jq expression"), "{}", text);
        assert!(!text.contains("regex"), "{}", text);
    }
}
