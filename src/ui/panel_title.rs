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
///
/// `max_width` is how many characters of filter text fit beside the tabs and
/// the right-aligned status. Longer text is shown as a window that keeps the
/// caret in view, since the end being typed at is otherwise exactly the part
/// the title cuts off.
pub fn filter_spans(app: &App, max_width: usize) -> Vec<Span<'static>> {
    let filter_value = app.filter.input.value();
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

    // tui-input's cursor counts characters, not bytes, so the text is split
    // on characters: slicing the string at the cursor directly panics as
    // soon as the filter contains anything outside ASCII.
    let chars: Vec<char> = filter_value.chars().collect();
    let cursor = app.filter.input.cursor().min(chars.len());
    let (start, end) = visible_window(chars.len(), cursor, max_width.max(1));
    let text = |from: usize, to: usize| chars[from..to].iter().collect::<String>();

    if start > 0 {
        spans.push(Span::styled("…", theme::TIME));
    }
    if app.filter.active {
        if cursor > start {
            spans.push(Span::styled(text(start, cursor), filter_style));
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
        if end > cursor {
            spans.push(Span::styled(text(cursor, end), filter_style));
        }
    } else {
        spans.push(Span::styled(text(start, end), filter_style));
    }
    if end < chars.len() {
        spans.push(Span::styled("…", theme::TIME));
    }

    // An invalid filter matches everything, so say why rather than leaving
    // the user to wonder why nothing was filtered.
    if let Some(error) = &app.filter.error {
        spans.push(Span::styled(
            format!(" ✗ {}", summarize(error)),
            theme::ERROR,
        ));
    }

    spans
}

/// The `[start, end)` character range of a `len`-character filter to show
/// in `width` columns, keeping `cursor` (which may sit just past the end)
/// inside it.
fn visible_window(len: usize, cursor: usize, width: usize) -> (usize, usize) {
    if len <= width {
        return (0, len);
    }
    let start = cursor.saturating_sub(width).min(len - width);
    (start, start + width)
}

/// The most informative line of a compile error, shortened for the title.
/// regex errors span several lines (pattern, a caret, then the message); the
/// last one says what is wrong.
fn summarize(error: &str) -> String {
    const MAX_LEN: usize = 48;
    let line = error
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("invalid filter");
    if line.chars().count() > MAX_LEN {
        format!("{}…", line.chars().take(MAX_LEN).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Columns the filter text may use in a panel title `width` wide: what's left
/// beside the border, the tab strip, the filter label, a paused marker, the
/// right-aligned status and an error message.
pub fn filter_width(width: u16) -> usize {
    const RESERVED: u16 = 2 + 18 + 11 + 10 + 28;
    width.saturating_sub(RESERVED).max(8) as usize
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elasticsearch::AuthConfig;
    use crate::ui::types::Colormap;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn rendered(spans: &[Span]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn test_non_ascii_filter_text_renders_with_the_caret_anywhere() {
        let mut app = app();
        app.filter.enter();
        for c in "日本-ü".chars() {
            app.filter.handle_key(&key(KeyCode::Char(c)));
        }
        // Caret at the end, then walked back into the middle of the
        // multi-byte characters: both used to panic in `split_at`.
        assert!(rendered(&filter_spans(&app, 40)).contains("日本-ü"));
        app.filter.handle_key(&key(KeyCode::Left));
        app.filter.handle_key(&key(KeyCode::Left));
        app.filter.handle_key(&key(KeyCode::Left));
        let text = rendered(&filter_spans(&app, 40));
        // The caret may be in either blink phase: a bar or a space.
        assert!(
            text.ends_with("日▏本-ü") || text.ends_with("日 本-ü"),
            "{}",
            text
        );
    }

    #[test]
    fn test_long_filters_keep_the_caret_in_view() {
        assert_eq!(visible_window(5, 5, 10), (0, 5));
        // Typing at the end of a long filter shows its tail.
        assert_eq!(visible_window(30, 30, 10), (20, 30));
        // Moving back to the start shows its head.
        assert_eq!(visible_window(30, 0, 10), (0, 10));
        assert_eq!(visible_window(30, 15, 10), (5, 15));
    }

    #[test]
    fn test_an_invalid_filter_says_why() {
        let mut app = app();
        app.filter.enter();
        for c in "a(".chars() {
            app.filter.handle_key(&key(KeyCode::Char(c)));
        }
        let text = rendered(&filter_spans(&app, 40));
        assert!(text.contains('✗'), "{}", text);
    }
}
