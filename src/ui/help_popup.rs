use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::theme;
use crate::app::App;

pub struct HelpPopup<'a> {
    app: &'a App,
}

impl<'a> HelpPopup<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for HelpPopup<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Calculate popup area
        let popup_width = (area.width as f32 * 0.6).min(70.0) as u16;
        let popup_height = (area.height as f32 * 0.8).min(40.0) as u16;
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;

        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Clear the popup area
        Clear.render(popup_area, buf);

        let help_lines = vec![
            Line::from(Span::styled("Keyboard Shortcuts", theme::TITLE)),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Navigation",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  j/↓       ", Style::new().fg(Color::Green)),
                Span::raw("Move selection down"),
            ]),
            Line::from(vec![
                Span::styled("  k/↑       ", Style::new().fg(Color::Green)),
                Span::raw("Move selection up"),
            ]),
            Line::from(vec![
                Span::styled("  PgUp/PgDn ", Style::new().fg(Color::Green)),
                Span::raw("Page up/down"),
            ]),
            Line::from(vec![
                Span::styled("  g/Home    ", Style::new().fg(Color::Green)),
                Span::raw("Go to first index"),
            ]),
            Line::from(vec![
                Span::styled("  G/End     ", Style::new().fg(Color::Green)),
                Span::raw("Go to last index"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Actions",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  Enter     ", Style::new().fg(Color::Green)),
                Span::raw("Show index details"),
            ]),
            Line::from(vec![
                Span::styled("  x         ", Style::new().fg(Color::Green)),
                Span::raw("Exclude/include selected index from stats"),
            ]),
            Line::from(vec![
                Span::styled("  X         ", Style::new().fg(Color::Green)),
                Span::raw("Clear all exclusions"),
            ]),
            Line::from(vec![
                Span::styled("  /         ", Style::new().fg(Color::Green)),
                Span::raw("Enter filter mode (jq)"),
            ]),
            Line::from(vec![
                Span::styled("  Space     ", Style::new().fg(Color::Green)),
                Span::raw("Pause/resume refresh"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Filter Mode",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  ←/→       ", Style::new().fg(Color::Green)),
                Span::raw("Move cursor left/right"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+←/→  ", Style::new().fg(Color::Green)),
                Span::raw("Move cursor by word"),
            ]),
            Line::from(vec![
                Span::styled("  Home/End  ", Style::new().fg(Color::Green)),
                Span::raw("Jump to start/end of filter"),
            ]),
            Line::from(vec![
                Span::styled("  Backspace ", Style::new().fg(Color::Green)),
                Span::raw("Delete character before cursor"),
            ]),
            Line::from(vec![
                Span::styled("  Delete    ", Style::new().fg(Color::Green)),
                Span::raw("Delete character at cursor"),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+u    ", Style::new().fg(Color::Green)),
                Span::raw("Clear filter"),
            ]),
            Line::from(vec![
                Span::styled("  Esc/Enter ", Style::new().fg(Color::Green)),
                Span::raw("Exit filter input"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Sorting",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  ←(h)/→(l) ", Style::new().fg(Color::Green)),
                Span::raw("Change sort column"),
            ]),
            Line::from(vec![
                Span::styled("  r         ", Style::new().fg(Color::Green)),
                Span::raw("Reverse sort order"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Display",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  1         ", Style::new().fg(Color::Green)),
                Span::raw("Toggle graph visibility"),
            ]),
            Line::from(vec![
                Span::styled("  2         ", Style::new().fg(Color::Green)),
                Span::raw("Toggle cluster health visibility"),
            ]),
            Line::from(vec![
                Span::styled("  3         ", Style::new().fg(Color::Green)),
                Span::raw("Toggle indices table visibility"),
            ]),
            Line::from(vec![
                Span::styled("  .         ", Style::new().fg(Color::Green)),
                Span::raw("Toggle system indices (dot-prefixed)"),
            ]),
            Line::from(vec![
                Span::styled("  +/-       ", Style::new().fg(Color::Green)),
                Span::raw("Increase/decrease refresh interval"),
            ]),
            Line::from(vec![
                Span::styled("  c/C       ", Style::new().fg(Color::Green)),
                Span::raw("Cycle colormap forward/backward"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  q/Esc     ", Style::new().fg(Color::Green)),
                Span::raw("Quit / Close popup"),
            ]),
            Line::from(""),
            Line::from(Span::styled("jq Filter Syntax", theme::TITLE)),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Fields:   ", Style::new().fg(Color::Yellow)),
                Span::raw(".name, .doc_count, .rate_per_sec, .health, .size_bytes"),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Examples",
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    "  select(.name == \"idx-1\")         ",
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw("Exact name match"),
            ]),
            Line::from(vec![
                Span::styled(
                    "  select(.doc_count > 1000)        ",
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw("Docs > 1000"),
            ]),
            Line::from(vec![
                Span::styled(
                    "  select(.health != \"green\")       ",
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw("Problematic health"),
            ]),
            Line::from(vec![
                Span::styled(
                    "  select(.rate_per_sec > 5)        ",
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw("High rate"),
            ]),
            Line::from(vec![
                Span::styled(
                    "  select(.name | contains(\"test\")) ",
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw("Name contains 'test'"),
            ]),
            Line::from(""),
            Line::from("  Combine with: and, or, not (e.g., select(.a > 1 and .b < 5))"),
        ];

        // Apply scroll offset
        let visible_height = popup_height.saturating_sub(2) as usize; // Account for border
        let max_scroll = help_lines.len().saturating_sub(visible_height);
        let scroll = self.app.help_scroll.min(max_scroll);

        Paragraph::new(help_lines)
            .block(
                Block::default()
                    .title(Line::from(vec![
                        Span::raw(" Help "),
                        Span::styled(
                            "[j/k] Scroll  [?/Esc] Close ",
                            Style::new().fg(Color::DarkGray),
                        ),
                    ]))
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Yellow)),
            )
            .scroll((scroll as u16, 0))
            .render(popup_area, buf);
    }
}
