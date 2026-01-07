use ratatui::{
    layout::{Constraint, Layout},
    widgets::TableState,
    Frame,
};

pub mod chart;
pub mod details_popup;
pub mod footer;
pub mod header;
pub mod health;
pub mod help_popup;
pub mod table;
pub mod theme;
pub mod types;

use crate::app::App;
use chart::RateChart;
use details_popup::DetailsPopup;
use footer::Footer;
use header::Header;
use health::ClusterHealthWidget;
use help_popup::HelpPopup;
use table::IndicesTable;

pub fn draw(frame: &mut Frame, app: &App) {
    // Build dynamic layout based on visibility settings
    let mut constraints = vec![Constraint::Length(3)]; // Header always visible

    if app.show_graph || app.show_health {
        constraints.push(Constraint::Length(8)); // Row for graph/health
    }
    if app.show_indices {
        constraints.push(Constraint::Min(0)); // Table
    }
    constraints.push(Constraint::Length(3)); // Footer always visible

    let areas = Layout::vertical(constraints).split(frame.area());
    let mut area_iter = areas.iter();

    // Header
    if let Some(&area) = area_iter.next() {
        frame.render_widget(Header::new(app), area);
    }

    // Charts and Health (if visible)
    if app.show_graph || app.show_health {
        if let Some(&area) = area_iter.next() {
            match (app.show_graph, app.show_health) {
                (true, true) => {
                    let [chart_area, health_area] = Layout::horizontal([
                        Constraint::Percentage(70),
                        Constraint::Percentage(30),
                    ])
                    .areas(area);
                    frame.render_widget(RateChart::new(app), chart_area);
                    frame.render_widget(ClusterHealthWidget::new(app), health_area);
                }
                (true, false) => {
                    frame.render_widget(RateChart::new(app), area);
                }
                (false, true) => {
                    frame.render_widget(ClusterHealthWidget::new(app), area);
                }
                _ => unreachable!(),
            }
        }
    }

    // Table (if visible)
    if app.show_indices {
        if let Some(&area) = area_iter.next() {
            let mut state = TableState::default().with_selected(app.selected_index);
            frame.render_stateful_widget(IndicesTable::new(app), area, &mut state);
        }
    }

    // Footer
    if let Some(&area) = area_iter.next() {
        frame.render_widget(Footer::new(app), area);
    }

    // Details popup overlay
    if app.details.show_popup {
        frame.render_widget(DetailsPopup::new(app), frame.area());
    }

    // Help popup overlay
    if app.show_help_popup {
        frame.render_widget(HelpPopup::new(app), frame.area());
    }
}
