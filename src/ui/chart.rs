use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Widget},
};

use super::theme;
use crate::app::App;
use crate::utils::format_number;

pub struct RateChart<'a> {
    app: &'a App,
}

impl<'a> RateChart<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for RateChart<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let history = self.app.rate_history_vec();

        // Calculate max for display
        let max_rate = history.iter().max().copied().unwrap_or(1);
        let current_rate = history.last().copied().unwrap_or(0);

        let title = format!(
            " Cluster Indexing Rate History (current: {} /s, max: {} /s) ",
            format_number(current_rate as f64),
            format_number(max_rate as f64)
        );

        // Calculate how many bars we can fit based on available width
        let available_width = area.width.saturating_sub(2) as usize; // Account for borders
        let bar_width = 6_u16;
        let gap = 1_u16;
        let chars_per_bar = (bar_width + gap) as usize;
        let max_bars = available_width / chars_per_bar.max(1);

        // Take only the most recent N values that fit
        let visible_history: Vec<u64> = if history.len() > max_bars {
            history[history.len() - max_bars..].to_vec()
        } else {
            history.clone()
        };

        // Create bars with rate labels
        let bars: Vec<Bar> = visible_history
            .iter()
            .map(|&value| {
                let label = format_number(value as f64);
                Bar::default()
                    .value(value)
                    .label(Line::from(label))
                    .style(Style::new().fg(Color::Green))
            })
            .collect();

        BarChart::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::BORDER)
                    .title(title),
            )
            .data(BarGroup::default().bars(&bars))
            .bar_width(bar_width)
            .value_style(Style::new().bg(Color::Green))
            .bar_gap(gap)
            .max(max_rate)
            .render(area, buf);
    }
}
