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
pub mod nodes;
pub mod panel_title;
pub mod table;
pub mod theme;
pub mod types;

use crate::app::{App, MainPanel};
use chart::RateChart;
use details_popup::DetailsPopup;
use footer::Footer;
use header::Header;
use health::ClusterHealthWidget;
use help_popup::HelpPopup;
use nodes::NodesTable;
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
    if app.show_main_panel {
        constraints.push(Constraint::Min(0)); // Main table panel
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

    let table = if app.show_main_panel {
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

/// Number of data rows a main-panel table can show in `area`, accounting for
/// the border it draws and its header row.
///
/// Single source of truth for that arithmetic: the tables use it to decide
/// which rows to build, `draw` uses it to compute the scroll offset, and
/// `table_page_size` uses it to size Page Up/Down. If these disagreed, paging
/// would skip or repeat rows.
pub fn visible_row_capacity(area: Rect) -> usize {
    area.height.saturating_sub(3) as usize
}

/// Scroll offset that keeps `selected` on screen while moving as little as
/// possible — the list stays put until the cursor would leave the viewport.
///
/// This used to be ratatui's job, but the tables now hand `Table` only the
/// rows that are actually visible (building 50k `Row`s to show 40 of them cost
/// ~93ms per frame on a large cluster), and it can't scroll past what it's
/// given. So the offset is computed here instead.
pub fn scroll_offset(
    current: usize,
    selected: Option<usize>,
    capacity: usize,
    total: usize,
) -> usize {
    if capacity == 0 || total == 0 {
        return 0;
    }

    let max_offset = total.saturating_sub(capacity);
    let mut offset = current.min(max_offset);

    if let Some(selected) = selected {
        if selected < offset {
            offset = selected;
        } else if selected >= offset + capacity {
            offset = selected + 1 - capacity;
        }
    }

    offset.min(max_offset)
}

/// Number of index rows that fit in the table for the given terminal area.
/// Used to size Page Up/Down navigation to what's actually on screen instead
/// of a magic constant.
pub fn table_page_size(terminal_area: Rect, app: &App) -> usize {
    let areas = compute_areas(terminal_area, app);
    areas.table.map(visible_row_capacity).unwrap_or(0).max(1)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    // Read the persisted scroll offset up front and write it back at the end:
    // everything in between borrows `app` immutably.
    let mut table_offset = app.table_offset;
    let mut node_table_offset = app.node_table_offset;

    {
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
            // Each panel carries its own scroll offset across frames, so the
            // list only moves when the cursor would leave the viewport and
            // switching panels preserves each one's position. The offset is
            // also what the table slices its rows from — only the rows on
            // screen are built.
            let capacity = visible_row_capacity(area);

            match app.main_panel {
                MainPanel::Indices => {
                    let selected = app.selected_position_in(&summary.indices);
                    let offset =
                        scroll_offset(table_offset, selected, capacity, summary.indices.len());

                    // Selection is passed relative to the window, since that's
                    // all the rows `Table` can see.
                    let mut state =
                        TableState::default().with_selected(selected.map(|s| s - offset));

                    frame.render_stateful_widget(
                        IndicesTable::new(app, &summary.indices, offset),
                        area,
                        &mut state,
                    );
                    table_offset = offset;
                }
                MainPanel::Nodes => {
                    // Filtered once here and handed to the widget, mirroring
                    // how `summary.indices` is threaded into the indices table.
                    let visible_nodes = app.visible_nodes();
                    let selected = app.selected_node_position_in(&visible_nodes);
                    let offset =
                        scroll_offset(node_table_offset, selected, capacity, visible_nodes.len());

                    let mut state =
                        TableState::default().with_selected(selected.map(|s| s - offset));

                    frame.render_stateful_widget(
                        NodesTable::new(app, &visible_nodes, offset),
                        area,
                        &mut state,
                    );
                    node_table_offset = offset;
                }
            }
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

    app.table_offset = table_offset;
    app.node_table_offset = node_table_offset;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scroll_offset_keeps_list_still_while_cursor_is_on_screen() {
        // The defining behaviour: the cursor moves within a stable page rather
        // than the list sliding under a pinned cursor.
        assert_eq!(scroll_offset(10, Some(10), 5, 100), 10);
        assert_eq!(scroll_offset(10, Some(12), 5, 100), 10);
        assert_eq!(scroll_offset(10, Some(14), 5, 100), 10);
    }

    #[test]
    fn test_scroll_offset_follows_cursor_off_either_edge() {
        // Moving past the bottom scrolls by exactly one row...
        assert_eq!(scroll_offset(10, Some(15), 5, 100), 11);
        // ...and past the top likewise.
        assert_eq!(scroll_offset(10, Some(9), 5, 100), 9);
        // A jump (Page Down, End) brings the target into view in one step.
        assert_eq!(scroll_offset(10, Some(80), 5, 100), 76);
        assert_eq!(scroll_offset(80, Some(0), 5, 100), 0);
    }

    #[test]
    fn test_scroll_offset_never_scrolls_past_the_end() {
        // The last page is flush with the end of the list, so there is never
        // empty space below the final row.
        assert_eq!(scroll_offset(99, Some(99), 5, 100), 95);
        assert_eq!(scroll_offset(500, None, 5, 100), 95);
        // A list shorter than the viewport never scrolls at all.
        assert_eq!(scroll_offset(3, Some(1), 20, 4), 0);
    }

    #[test]
    fn test_scroll_offset_degenerate_cases() {
        assert_eq!(scroll_offset(7, Some(3), 0, 100), 0);
        assert_eq!(scroll_offset(7, Some(3), 5, 0), 0);
        assert_eq!(scroll_offset(0, None, 5, 100), 0);
    }

    #[test]
    fn test_visible_row_capacity_reserves_border_and_header() {
        // Two border rows plus one header row.
        assert_eq!(visible_row_capacity(Rect::new(0, 0, 80, 10)), 7);
        assert_eq!(visible_row_capacity(Rect::new(0, 0, 80, 3)), 0);
        assert_eq!(visible_row_capacity(Rect::new(0, 0, 80, 1)), 0);
    }
}
