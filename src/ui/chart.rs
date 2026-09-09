use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

use super::theme;
use crate::app::App;
use crate::utils::format_number;

/// Partial block glyphs, 1/8 of a cell through 8/8.
///
/// Drawing the top of each column with one of these is what gives the graph
/// eight vertical steps per terminal row instead of one, which is the whole
/// reason it reads as a smooth curve rather than a staircase.
const EIGHTHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Height of a column, in eighths of a cell, for `value` against `max` over a
/// column `height` rows tall.
///
/// A non-zero rate always returns at least one eighth: rounding a small but
/// real rate down to nothing would render an idle cluster and a barely-active
/// one identically.
fn column_eighths(value: u64, max: u64, height: u16) -> usize {
    if max == 0 || height == 0 {
        return 0;
    }

    let full_scale = height as f64 * 8.0;
    let scaled = (value as f64 / max as f64 * full_scale).round() as usize;

    if value > 0 {
        scaled.clamp(1, full_scale as usize)
    } else {
        0
    }
}

pub struct RateChart<'a> {
    app: &'a App,
}

impl<'a> RateChart<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for RateChart<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let max_rate = self.app.rate_history.iter().max().copied().unwrap_or(0);
        let current_rate = self.app.rate_history.back().copied().unwrap_or(0);

        let title = Line::from(vec![
            Span::raw(" Cluster Indexing Rate "),
            Span::styled(
                format!("{} /s", format_number(current_rate as f64)),
                theme::RATE,
            ),
            Span::raw(" "),
        ]);
        let scale = Line::from(vec![
            Span::styled(
                format!("peak {} /s", format_number(max_rate as f64)),
                theme::TIME,
            ),
            Span::raw(" "),
        ])
        .right_aligned();

        let border_style = if self.app.paused {
            Style::new().fg(Color::Yellow)
        } else {
            theme::BORDER
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title_top(title)
            .title_top(scale);
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // One column per sample, newest at the right, so history scrolls in
        // from the right edge the way every other time-series graph does.
        let width = inner.width as usize;
        let points: Vec<u64> = self
            .app
            .rate_history
            .iter()
            .rev()
            .take(width)
            .rev()
            .copied()
            .collect();

        // Right-align: a partly-filled history leaves the left side blank and
        // grows rightwards rather than starting at the left edge and shifting.
        let x_offset = inner.width.saturating_sub(points.len() as u16);

        for (i, &value) in points.iter().enumerate() {
            let x = inner.x + x_offset + i as u16;
            let eighths = column_eighths(value, max_rate, inner.height);

            // A sample of zero still gets a floor tick, so a quiet cluster
            // reads as "measured, and idle" rather than "no data".
            if eighths == 0 {
                let y = inner.y + inner.height - 1;
                buf[(x, y)]
                    .set_char('▁')
                    .set_style(Style::new().fg(Color::DarkGray));
                continue;
            }

            for row in 0..inner.height {
                // `row` counts up from the bottom of the plot.
                let floor = row as usize * 8;
                let glyph = if eighths >= floor + 8 {
                    EIGHTHS[7]
                } else if eighths > floor {
                    EIGHTHS[eighths - floor - 1]
                } else {
                    break; // nothing above this row in this column
                };

                // Colored by how high the cell sits rather than by the
                // column's value, so each column shades bottom-to-top and the
                // peaks stand out — and it follows the active colormap, so
                // `c`/`C` restyles the graph along with the tables.
                let height_fraction = (row as f32 + 0.5) / inner.height as f32;
                let color = self.app.colormap.color_at(1.0 - height_fraction);

                let y = inner.y + inner.height - 1 - row;
                // No BOLD here: the glyphs are already solid, and some
                // terminals brighten bold foregrounds, which would distort
                // the gradient.
                buf[(x, y)]
                    .set_char(glyph)
                    .set_style(Style::new().fg(color));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_eighths_scales_to_full_height() {
        // The peak fills the column; half the peak fills half of it.
        assert_eq!(column_eighths(100, 100, 4), 32);
        assert_eq!(column_eighths(50, 100, 4), 16);
        assert_eq!(column_eighths(0, 100, 4), 0);
    }

    #[test]
    fn test_column_eighths_keeps_small_values_visible() {
        // A rate that rounds to nothing must still draw a sliver, otherwise a
        // barely-active cluster looks identical to an idle one.
        assert_eq!(column_eighths(1, 100_000, 4), 1);
    }

    #[test]
    fn test_column_eighths_handles_empty_history() {
        assert_eq!(column_eighths(0, 0, 4), 0);
        assert_eq!(column_eighths(5, 0, 4), 0);
        assert_eq!(column_eighths(5, 10, 0), 0);
    }
}
