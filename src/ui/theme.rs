use ratatui::style::{Color, Modifier, Style};

pub const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const ERROR: Style = Style::new().fg(Color::Red);
pub const TIME: Style = Style::new().fg(Color::DarkGray);
pub const URL: Style = Style::new().fg(Color::Green);
pub const RATE: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const BORDER: Style = Style::new().fg(Color::DarkGray);

// Snapshot trend arrows. Green/red reads as *direction* here, not as merit —
// a node whose failed-operation count has climbed gets a green arrow like
// anything else that went up. The glyph is what carries the meaning; the
// color is only there to make it findable while scanning a column.
pub const TREND_UP: Style = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);
pub const TREND_DOWN: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
pub const SNAPSHOT: Style = Style::new().fg(Color::Magenta);
