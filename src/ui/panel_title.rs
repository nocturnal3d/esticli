use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::theme;
use crate::app::filter::FilterMode;
use crate::app::{App, MainPanel};

/// The tab strip both main-panel tables carry in their border title:
/// `Indices │ Nodes`, with whichever one is on screen highlighted.
///
/// Shared by `table.rs` and `nodes.rs` so the strip can't drift between the
/// two views — it's the only on-screen indication that the nodes view exists
/// at all, and which of the two you're currently looking at.
pub fn tabs(app: &App) -> Vec<Span<'static>> {
    let tab = |label: &'static str, panel: MainPanel| {
        if app.main_panel == panel {
            Span::styled(label, theme::TITLE)
        } else {
            Span::styled(label, Style::new().fg(Color::DarkGray))
        }
    };

    vec![
        Span::raw(" "),
        tab("Indices", MainPanel::Indices),
        Span::styled(" │ ", Style::new().fg(Color::DarkGray)),
        tab("Nodes", MainPanel::Nodes),
    ]
}

/// The filter box as rendered in a panel title: `| Filter: <text>`, with the
/// caret drawn while the box has focus.
///
/// Shared by both tables because the filter drives whichever panel is on
/// screen — showing it on only one of them would leave `/` looking inert.
pub fn filter_spans(app: &App) -> Vec<Span<'static>> {
    let filter_value = app.filter.input.value().to_string();
    if !app.filter.active && filter_value.is_empty() {
        return Vec::new();
    }

    let mut spans = vec![Span::raw(" | ")];

    // jq mode is visually distinct so it's obvious the input is no longer a
    // plain name regex.
    let (label, label_color) = match app.filter.mode {
        FilterMode::Regex => ("Filter: ", Color::Yellow),
        FilterMode::Jq => ("jq: ", Color::Magenta),
    };
    spans.push(Span::styled(
        label,
        Style::new().fg(label_color).add_modifier(Modifier::BOLD),
    ));

    let filter_style = if app.filter.error.is_some() {
        theme::ERROR
    } else if app.filter.active {
        Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Green)
    };

    if app.filter.active {
        let cursor = app.filter.input.cursor();
        let (before, after) = filter_value.split_at(cursor);
        if !before.is_empty() {
            spans.push(Span::styled(before.to_string(), filter_style));
        }
        // Blink driven by our own clock (see FilterState::cursor_visible)
        // rather than the terminal's SGR blink attribute, which most modern
        // terminal emulators ignore. A plain space in the "off" phase keeps
        // the column width stable so text doesn't jitter.
        if app.filter.cursor_visible() {
            spans.push(Span::styled("▏", Style::new().fg(Color::White)));
        } else {
            spans.push(Span::raw(" "));
        }
        if !after.is_empty() {
            spans.push(Span::styled(after.to_string(), filter_style));
        }
    } else {
        spans.push(Span::styled(filter_value, filter_style));
    }

    spans
}

/// Right-aligned half of the title: the refresh spinner and how long the last
/// fetch took, then how many rows the panel is showing.
///
/// Kept flush right so it holds a fixed position instead of being pushed
/// around by the filter text growing and shrinking to its left.
pub fn status(app: &App, count: String) -> Line<'static> {
    let spinner_color = if app.loading {
        Color::Cyan
    } else {
        Color::Green
    };

    Line::from(vec![
        Span::styled("(", theme::TIME),
        Span::styled(
            app.spinner_char().to_string(),
            Style::new().fg(spinner_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {})", app.fetch_duration_display()), theme::TIME),
        Span::raw(" "),
        Span::styled(count, theme::TIME),
        Span::raw(" "),
    ])
    .right_aligned()
}
