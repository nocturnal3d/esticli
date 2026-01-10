mod app;
mod elasticsearch;
mod error;
mod models;
mod ui;
mod utils;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use tui_input::backend::crossterm::EventHandler;

use app::actions::Action;
use app::App;
use elasticsearch::AuthConfig;
use ui::types::Colormap;

#[derive(Parser, Debug)]
#[command(name = "esticli")]
#[command(about = "A top-like TUI for monitoring Elasticsearch")]
struct Args {
    // Elasticsearch URL
    #[arg(short = 'u', long, default_value = "http://localhost:9200")]
    url: String,

    // Basic auth username
    #[arg(long)]
    username: Option<String>,

    // Basic auth password
    #[arg(long)]
    password: Option<String>,

    // API key for authentication
    #[arg(long)]
    api_key: Option<String>,

    // Skip TLS certificate verification
    #[arg(short = 'k', long)]
    insecure: bool,

    // Path to CA certificate file (PEM format) for TLS verification
    #[arg(long, value_name = "FILE")]
    ca_cert: Option<PathBuf>,

    // Refresh interval in seconds
    #[arg(long, default_value = "5")]
    refresh: u64,

    // Colormap for gradient visualization
    // Options: turbo, spectral, inferno, magma, plasma, viridis, rainbow, cividis, warm, cool
    #[arg(long, default_value = "warm")]
    colormap: Colormap,

    // Number of samples to average for rate calculation
    #[arg(long, default_value = "10")]
    rate_samples: usize,
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
        args.rate_samples,
    )?;

    let terminal = ratatui::init();
    let result = run(terminal, &mut app).await;
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

        // Poll for keyboard events with a short timeout
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(action) = map_key_to_action(app, key) {
                        app.handle_action(action);
                    } else if app.filter.active {
                        // Filter mode special handling for text input
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter => {
                                app.handle_action(Action::ExitFilterMode)
                            }
                            _ => {
                                app.filter.input.handle_event(&Event::Key(key));
                                app.filter.recompile();
                            }
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

fn map_key_to_action(app: &App, key: event::KeyEvent) -> Option<Action> {
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
            KeyCode::PageUp => Some(Action::DetailsScrollPageUp),
            KeyCode::PageDown => Some(Action::DetailsScrollPageDown),
            _ => None,
        };
    }

    if app.filter.active {
        return match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::ClearFilter)
            }
            // Other keys handled by input component in run loop
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Char(' ') => Some(Action::TogglePause),
        KeyCode::Char('/') => Some(Action::EnterFilterMode),
        KeyCode::Enter => Some(Action::ShowDetails),
        KeyCode::Char('x') => Some(Action::ToggleExclude),
        KeyCode::Char('X') => Some(Action::ClearExclusions),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::NextColumn),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::PrevColumn),
        KeyCode::Char('r') => Some(Action::ToggleSortOrder),
        KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::DecreaseRefreshRate),
        KeyCode::Char('-') | KeyCode::Char('_') => Some(Action::IncreaseRefreshRate),
        KeyCode::Char('1') => Some(Action::ToggleGraph),
        KeyCode::Char('2') => Some(Action::ToggleHealth),
        KeyCode::Char('3') => Some(Action::ToggleIndices),
        KeyCode::Char('.') => Some(Action::ToggleSystemIndices),
        KeyCode::Char('c') => Some(Action::NextColormap),
        KeyCode::Char('C') => Some(Action::PrevColormap),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::SelectUp),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::SelectDown),
        KeyCode::PageUp | KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::SelectPageUp)
        }
        KeyCode::PageUp => Some(Action::SelectPageUp),
        KeyCode::PageDown | KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::SelectPageDown)
        }
        KeyCode::PageDown => Some(Action::SelectPageDown),
        KeyCode::Home | KeyCode::Char('g') => Some(Action::SelectFirst),
        KeyCode::End | KeyCode::Char('G') => Some(Action::SelectLast),
        _ => None,
    }
}
