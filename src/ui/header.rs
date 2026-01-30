use chrono::Local;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

use super::theme;
use crate::app::App;

pub struct Header<'a> {
    app: &'a App,
}

impl<'a> Header<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for Header<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let now = Local::now();
        let datetime = now.format("%Y-%m-%d %H:%M:%S").to_string();

        let title = if let Some(ref error) = self.app.error {
            Line::from(vec![
                Span::styled(" EstiCLI ", theme::TITLE),
                Span::raw(" | "),
                Span::styled(format!("Error: {}", error), theme::ERROR),
                Span::raw(" | "),
                Span::styled(datetime, theme::TIME),
            ])
        } else {
            Line::from(vec![
                Span::styled(" EstiCLI ", theme::TITLE),
                Span::raw(" | "),
                Span::styled(&self.app.es_url, theme::URL),
                Span::raw(" | Cluster Rate: "),
                Span::styled(
                    format!("{} /s", self.app.total_cluster_rate_human()),
                    theme::RATE,
                ),
                Span::raw(" ("),
                Span::styled(
                    format!("{}/s", self.app.total_cluster_bytes_per_sec_human()),
                    theme::RATE,
                ),
                Span::raw(")"),
                Span::raw(" | "),
                Span::styled(datetime, Style::new().fg(Color::White)),
            ])
        };

        Paragraph::new(title)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::BORDER),
            )
            .render(area, buf);
    }
}
