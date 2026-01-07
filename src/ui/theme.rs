use ratatui::style::{Color, Modifier, Style};

pub const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const ERROR: Style = Style::new().fg(Color::Red);
pub const TIME: Style = Style::new().fg(Color::DarkGray);
pub const URL: Style = Style::new().fg(Color::Green);
pub const RATE: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
pub const BORDER: Style = Style::new().fg(Color::DarkGray);
