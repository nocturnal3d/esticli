use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

use super::theme;
use crate::app::App;

pub struct ClusterHealthWidget<'a> {
    app: &'a App,
}

impl<'a> ClusterHealthWidget<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for ClusterHealthWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let health = &self.app.cluster_health;

        let status_color = match health.status.as_str() {
            "green" => Color::Green,
            "yellow" => Color::Yellow,
            "red" => Color::Red,
            _ => Color::Gray,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::BORDER)
            .title(Span::styled(
                " Cluster Health ",
                Style::new().add_modifier(Modifier::BOLD),
            ));

        let inner_area = block.inner(area);
        block.render(area, buf);

        if inner_area.height == 0 || inner_area.width == 0 {
            return;
        }

        // Divide inner area into rows for metrics
        let [name_area, status_nodes_area, shards_area, moving_unassigned_area, pending_tasks_area, _] =
            Layout::vertical([
                Constraint::Length(1), // Cluster Name
                Constraint::Length(1), // Status and Nodes
                Constraint::Length(1), // Shards
                Constraint::Length(1), // Relocating and Unassigned
                Constraint::Length(1), // Pending Tasks
                Constraint::Min(0),
            ])
            .areas(inner_area);

        // Row: Cluster Name
        let name_line = Line::from(vec![
            Span::styled("󰆼 ", Style::new().fg(Color::Gray)),
            Span::styled(
                &health.cluster_name,
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(name_area.x, name_area.y, &name_line, name_area.width);

        // Row: Status and Nodes
        let [status_area, nodes_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(status_nodes_area);

        // Status
        let status_line = Line::from(vec![
            Span::styled("♥ ", Style::new().fg(status_color)),
            Span::styled(
                health.status.to_uppercase(),
                Style::new().fg(status_color).add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            status_area.x,
            status_area.y,
            &status_line,
            status_area.width,
        );

        // Nodes: Total / Data
        let nodes_line = Line::from(vec![
            Span::styled("󰄳 ", Style::new().fg(Color::Cyan)),
            Span::styled(
                format!("{}", health.number_of_nodes),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / ", Style::new().fg(Color::Gray)),
            Span::styled("󰋊 ", Style::new().fg(Color::Blue)),
            Span::styled(
                format!("{}", health.number_of_data_nodes),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(nodes_area.x, nodes_area.y, &nodes_line, nodes_area.width);

        // Row: Shards
        let [shards_left_area, shards_right_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(shards_area);

        // Shards: P Pri / A Total Active
        let shards_line = Line::from(vec![
            Span::styled("P ", Style::new().fg(Color::Green)),
            Span::styled(
                format!("{}", health.active_primary_shards),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / ", Style::new().fg(Color::Gray)),
            Span::styled("A ", Style::new().fg(Color::Magenta)),
            Span::styled(
                format!("{}", health.active_shards),
                Style::new().add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            shards_left_area.x,
            shards_left_area.y,
            &shards_line,
            shards_left_area.width,
        );

        // Active %
        let percent_color = if health.active_shards_percent >= 100.0 {
            Color::Green
        } else if health.active_shards_percent >= 90.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let active_pct_line = Line::from(vec![
            Span::styled("% ", Style::new().fg(percent_color)),
            Span::styled(
                format!("{:.1}", health.active_shards_percent),
                Style::new().fg(percent_color).add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            shards_right_area.x,
            shards_right_area.y,
            &active_pct_line,
            shards_right_area.width,
        );

        // Row: Relocating and Unassigned
        let [moving_area, unassigned_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(moving_unassigned_area);

        // Relocating and Initializing
        let relocating_color = if health.relocating_shards > 0 {
            Color::Cyan
        } else {
            Color::Gray
        };
        let initializing_color = if health.initializing_shards > 0 {
            Color::Yellow
        } else {
            Color::Gray
        };
        let moving_line = Line::from(vec![
            Span::styled("󰪹 ", Style::new().fg(relocating_color)),
            Span::styled(
                format!("{}", health.relocating_shards),
                Style::new()
                    .fg(relocating_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" / ", Style::new().fg(Color::Gray)),
            Span::styled("󰗖 ", Style::new().fg(initializing_color)),
            Span::styled(
                format!("{}", health.initializing_shards),
                Style::new()
                    .fg(initializing_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            moving_area.x,
            moving_area.y,
            &moving_line,
            moving_area.width,
        );

        // Unassigned
        let unassigned_color = if health.unassigned_shards > 0 {
            Color::Red
        } else {
            Color::Gray
        };
        let unassigned_line = Line::from(vec![
            Span::styled("󰀦 ", Style::new().fg(unassigned_color)),
            Span::styled(
                format!("{}", health.unassigned_shards),
                Style::new()
                    .fg(unassigned_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            unassigned_area.x,
            unassigned_area.y,
            &unassigned_line,
            unassigned_area.width,
        );

        // Row: Pending Tasks
        let [pending_area] =
            Layout::horizontal([Constraint::Percentage(100)]).areas(pending_tasks_area);

        // Pending Tasks: 󱎫
        let pending_color = if health.number_of_pending_tasks > 0 {
            Color::Yellow
        } else {
            Color::Gray
        };
        let pending_line = Line::from(vec![
            Span::styled("󱎫 ", Style::new().fg(pending_color)),
            Span::styled(
                format!("{}", health.number_of_pending_tasks),
                Style::new().fg(pending_color).add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(
            pending_area.x,
            pending_area.y,
            &pending_line,
            pending_area.width,
        );
    }
}
