use ratatui::{
    layout::{Constraint, Layout, Rect},
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

/// The main-screen areas, computed once from the visibility toggles so
/// `draw()` and the keyboard page-size calculations (see `table_page_size`)
/// always agree on where the table actually is.
pub struct Areas {
    pub header: Rect,
    pub chart: Option<Rect>,
    pub health: Option<Rect>,
    pub table: Option<Rect>,
    pub footer: Rect,
}

pub fn compute_areas(area: Rect, app: &App) -> Areas {
    // Build dynamic layout based on visibility settings
    let mut constraints = vec![Constraint::Length(3)]; // Header always visible

    if app.show_graph || app.show_health {
        constraints.push(Constraint::Length(8)); // Row for graph/health
    }
    if app.show_indices {
        constraints.push(Constraint::Min(0)); // Table
    }
    constraints.push(Constraint::Length(3)); // Footer always visible

    let layout = Layout::vertical(constraints).split(area);
    let mut area_iter = layout.iter().copied();

    let header = area_iter.next().unwrap_or_default();

    let mut chart = None;
    let mut health = None;
    if app.show_graph || app.show_health {
        if let Some(area) = area_iter.next() {
            match (app.show_graph, app.show_health) {
                (true, true) => {
                    let [chart_area, health_area] = Layout::horizontal([
                        Constraint::Percentage(70),
                        Constraint::Percentage(30),
                    ])
                    .areas(area);
                    chart = Some(chart_area);
                    health = Some(health_area);
                }
                (true, false) => chart = Some(area),
                (false, true) => health = Some(area),
                _ => unreachable!(),
            }
        }
    }

    let table = if app.show_indices {
        area_iter.next()
    } else {
        None
    };

    let footer = area_iter.next().unwrap_or_default();

    Areas {
        header,
        chart,
        health,
        table,
        footer,
    }
}

/// Number of index rows that fit in the table for the given terminal area,
/// matching the height the table widget itself reserves for its border and
/// header row. Used to size Page Up/Down navigation to what's actually on
/// screen instead of a magic constant.
pub fn table_page_size(terminal_area: Rect, app: &App) -> usize {
    let areas = compute_areas(terminal_area, app);
    areas
        .table
        .map(|a| a.height.saturating_sub(3) as usize)
        .unwrap_or(0)
        .max(1)
}

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = compute_areas(frame.area(), app);
    // Filter once per frame and share the result across every widget that
    // needs it, instead of each widget re-running the (potentially jq-based)
    // visibility filter over every index independently.
    let summary = app.visible_summary();

    frame.render_widget(Header::new(app, summary.metrics), areas.header);

    if let Some(area) = areas.chart {
        frame.render_widget(RateChart::new(app), area);
    }
    if let Some(area) = areas.health {
        frame.render_widget(ClusterHealthWidget::new(app), area);
    }

    if let Some(area) = areas.table {
        let mut state = TableState::default().with_selected(app.selected_index);
        frame.render_stateful_widget(IndicesTable::new(app, &summary.indices), area, &mut state);
    }

    frame.render_widget(
        Footer::new(app, summary.indices.len(), app.indices.len()),
        areas.footer,
    );

    // Details popup overlay
    if app.details.show_popup {
        frame.render_widget(DetailsPopup::new(app), frame.area());
    }

    // Help popup overlay
    if app.show_help_popup {
        frame.render_widget(HelpPopup::new(app), frame.area());
    }
}
