mod app;
mod elasticsearch;
mod error;
mod models;
#[cfg(test)]
mod perf;
mod ui;
mod utils;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::{layout::Rect, DefaultTerminal};

use app::actions::Action;
use app::{App, MAX_REFRESH_SECS, MIN_REFRESH_SECS};
use elasticsearch::AuthConfig;
use ui::types::Colormap;

// No `Debug`: it would print the password and API key if the struct were
// ever logged.
#[derive(Parser)]
#[command(name = "esticli", version)]
#[command(about = "A top-like TUI for monitoring Elasticsearch")]
struct Args {
    /// Elasticsearch URL, including any path prefix a gateway serves it under
    #[arg(
        short = 'u',
        long,
        env = "ESTICLI_URL",
        default_value = "http://localhost:9200"
    )]
    url: String,

    /// Basic auth username (requires --password)
    #[arg(long, env = "ESTICLI_USERNAME", requires = "password")]
    username: Option<String>,

    /// Basic auth password. Prefer ESTICLI_PASSWORD: arguments are visible to
    /// every user on the machine in `ps`
    #[arg(
        long,
        env = "ESTICLI_PASSWORD",
        hide_env_values = true,
        requires = "username"
    )]
    password: Option<String>,

    /// API key, as the base64 "encoded" value Elasticsearch returns. Prefer
    /// ESTICLI_API_KEY over the argument
    #[arg(
        long,
        env = "ESTICLI_API_KEY",
        hide_env_values = true,
        conflicts_with_all = ["username", "password"]
    )]
    api_key: Option<String>,

    /// Skip TLS certificate verification
    #[arg(short = 'k', long)]
    insecure: bool,

    /// CA certificate (PEM) to trust in addition to the system roots
    #[arg(long, value_name = "FILE", env = "ESTICLI_CA_CERT")]
    ca_cert: Option<PathBuf>,

    /// Refresh interval in seconds
    #[arg(
        long,
        default_value_t = 5,
        value_parser = clap::value_parser!(u64).range(MIN_REFRESH_SECS..=MAX_REFRESH_SECS)
    )]
    refresh: u64,

    /// Colormap for the table and graph gradients: turbo, spectral, inferno,
    /// magma, plasma, viridis, rainbow, cividis, warm or cool
    // The default comes from `Colormap::default()` rather than being spelled
    // out again here, so the two can't drift apart.
    #[arg(long, default_value_t = Colormap::default())]
    colormap: Colormap,

    /// Number of refresh samples each index's rate is averaged over
    #[arg(
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u64).range(1..=1000)
    )]
    rate_samples: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let auth = if let Some(api_key) = args.api_key {
        AuthConfig::ApiKey(api_key)
    } else if let (Some(username), Some(password)) = (args.username, args.password) {
        AuthConfig::Basic { username, password }
    } else {
        AuthConfig::None
    };

    let mut app = App::new(
        args.url,
        auth,
        args.insecure,
        args.ca_cert,
        args.refresh,
        args.colormap,
        args.rate_samples as usize,
    )?;

    let terminal = ratatui::init();
    // Without bracketed paste, a paste arrives as a stream of keystrokes:
    // its first newline leaves the filter box and the rest runs as commands.
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste);
    let result = run(terminal, &mut app).await;
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    result
}

async fn run(mut terminal: DefaultTerminal, app: &mut App) -> Result<()> {
    // Initial data fetch
    app.start_fetch();

    while app.running {
        // Poll for fetch results (non-blocking)
        app.poll_fetch_result();

        // Poll for details results (non-blocking)
        app.poll_details_result();

        // Advance spinner animation
        app.tick_spinner();

        terminal.draw(|frame| ui::draw(frame, app))?;

        // Page sizes derived from the actual rendered areas, so Page Up/Down
        // scrolls by what's really on screen instead of a magic constant.
        let size = terminal.size()?;
        let terminal_area = Rect::new(0, 0, size.width, size.height);
        let table_page_size = ui::table_page_size(terminal_area, app);
        let details_page_size = ui::details_popup::visible_rows(terminal_area).max(1);

        // Poll for keyboard events with a short timeout
        if event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            // A paste only ever goes into the filter box; anywhere else it
            // is dropped rather than replayed as a burst of commands.
            if let Event::Paste(text) = &event {
                if app.filter.active {
                    app.filter.paste(text);
                }
            }
            if let Event::Key(key) = event {
                if key.kind == KeyEventKind::Press {
                    if let Some(action) =
                        map_key_to_action(app, key, table_page_size, details_page_size)
                    {
                        app.handle_action(action);
                    } else if app.filter.active {
                        // Filter mode special handling for text input
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter => {
                                app.handle_action(Action::ExitFilterMode)
                            }
                            _ => app.filter.handle_key(&Event::Key(key)),
                        }
                    }
                }
            }
        }

        // Check if we need to start a new fetch
        if app.should_refresh() && !app.loading {
            app.start_fetch();
        }
    }

    Ok(())
}

fn map_key_to_action(
    app: &App,
    key: event::KeyEvent,
    table_page_size: usize,
    details_page_size: usize,
) -> Option<Action> {
    // In raw mode Ctrl+C is an ordinary key event rather than a signal, so it
    // has to be mapped explicitly — everywhere, since it's what people press
    // to get out of a terminal program whatever state it is in.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    if app.show_help_popup {
        return match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
                Some(Action::ToggleHelp)
            }
            KeyCode::Up | KeyCode::Char('k') => Some(Action::HelpScrollUp),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::HelpScrollDown),
            _ => None,
        };
    }

    if app.details.show_popup {
        return match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => Some(Action::CloseDetails),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::DetailsScrollUp),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::DetailsScrollDown),
            KeyCode::PageUp => Some(Action::DetailsScrollPageUp(details_page_size)),
            KeyCode::PageDown => Some(Action::DetailsScrollPageDown(details_page_size)),
            _ => None,
        };
    }

    if app.filter.active {
        return match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ClearFilter)
            }
            // A second '/' while the box is still empty ("//") toggles
            // regex/jq syntax instead of being inserted as text.
            KeyCode::Char('/') if app.filter.input.value().is_empty() => {
                Some(Action::ToggleFilterMode)
            }
            // Other keys handled by input component in run loop
            _ => None,
        };
    }

    // Modified keys only do something where a binding asks for the modifier.
    // Matching on the bare character instead made Ctrl+S re-take the
    // snapshot, Ctrl+N switch panels, Alt+Q quit, and so on.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('b') | KeyCode::PageUp => Some(Action::SelectPageUp(table_page_size)),
            KeyCode::Char('f') | KeyCode::PageDown => Some(Action::SelectPageDown(table_page_size)),
            _ => None,
        };
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('/') => Some(Action::EnterFilterMode),
        KeyCode::Enter => Some(Action::ShowDetails),
        KeyCode::Char('x') => Some(Action::ToggleExclude),
        KeyCode::Char('X') => Some(Action::ClearExclusions),
        KeyCode::Char('s') => Some(Action::TakeSnapshot),
        KeyCode::Char('S') => Some(Action::ClearSnapshot),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::NextColumn),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::PrevColumn),
        KeyCode::Char('r') => Some(Action::ToggleSortOrder),
        KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::DecreaseRefreshRate),
        KeyCode::Char('-') | KeyCode::Char('_') => Some(Action::IncreaseRefreshRate),
        KeyCode::Char('1') => Some(Action::ToggleGraph),
        KeyCode::Char('2') => Some(Action::ToggleHealth),
        KeyCode::Char('3') => Some(Action::ToggleIndices),
        KeyCode::Char('n') => Some(Action::ToggleNodes),
        KeyCode::Char('.') => Some(Action::ToggleSystemIndices),
        KeyCode::Char('c') => Some(Action::NextColormap),
        KeyCode::Char('C') => Some(Action::PrevColormap),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectDown),
        KeyCode::PageUp => Some(Action::SelectPageUp(table_page_size)),
        KeyCode::PageDown => Some(Action::SelectPageDown(table_page_size)),
        KeyCode::Home | KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::End | KeyCode::Char('G') => Some(Action::SelectLast),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

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

    fn map(app: &App, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
        map_key_to_action(app, KeyEvent::new(code, modifiers), 10, 10)
    }

    #[test]
    fn test_ctrl_c_quits_from_anywhere() {
        let mut app = app();
        let ctrl_c = |app: &App| map(app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(ctrl_c(&app), Some(Action::Quit)));

        app.filter.enter();
        assert!(matches!(ctrl_c(&app), Some(Action::Quit)));
        app.filter.exit();

        app.show_help_popup = true;
        assert!(matches!(ctrl_c(&app), Some(Action::Quit)));
    }

    #[test]
    fn test_modified_keys_do_not_fall_through_to_plain_bindings() {
        let app = app();
        for (c, modifiers) in [
            ('s', KeyModifiers::CONTROL),
            ('n', KeyModifiers::CONTROL),
            ('q', KeyModifiers::ALT),
        ] {
            assert!(
                map(&app, KeyCode::Char(c), modifiers).is_none(),
                "{:?}+{}",
                modifiers,
                c
            );
        }

        // Bindings that do ask for Ctrl still work, as do shifted letters.
        assert!(matches!(
            map(&app, KeyCode::Char('f'), KeyModifiers::CONTROL),
            Some(Action::SelectPageDown(10))
        ));
        assert!(matches!(
            map(&app, KeyCode::Char('G'), KeyModifiers::SHIFT),
            Some(Action::SelectLast)
        ));
        assert!(matches!(
            map(&app, KeyCode::PageUp, KeyModifiers::NONE),
            Some(Action::SelectPageUp(10))
        ));
    }
}
