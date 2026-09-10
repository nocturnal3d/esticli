//! Performance harness for the indices panel at cluster scale.
//!
//! Not run by CI — these are timing measurements, not assertions:
//!
//! ```text
//! cargo test --release perf -- --ignored --nocapture
//! ```

use std::hint::black_box;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{StatefulWidget, TableState};

use crate::app::App;
use crate::elasticsearch::AuthConfig;
use crate::models::IndexRate;
use crate::ui::table::IndicesTable;
use crate::ui::types::Colormap;

const INDEX_COUNT: usize = 50_000;
const TERMINAL: Rect = Rect {
    x: 0,
    y: 0,
    width: 160,
    height: 50,
};

fn app_with_indices(count: usize) -> App {
    let mut app = App::new(
        "http://localhost:9200".to_string(),
        AuthConfig::None,
        false,
        None,
        5,
        Colormap::default(),
        10,
    )
    .unwrap();

    app.indices = (0..count)
        .map(|i| IndexRate {
            name: format!("logs-app-{:06}-2026.09.09", i),
            doc_count: (i as u64) * 37,
            rate_per_sec: (i % 997) as f64,
            size_bytes: (i as u64) * 4096,
            health: if i % 11 == 0 { "yellow" } else { "green" }.to_string(),
        })
        .collect();
    app
}

fn time<F: FnMut()>(label: &str, iterations: u32, mut op: F) {
    // Warm up so the first allocation isn't charged to the measurement.
    for _ in 0..3 {
        op();
    }

    let start = Instant::now();
    for _ in 0..iterations {
        op();
    }
    let per_op = start.elapsed() / iterations;
    println!("  {:<38} {:>10.3?}", label, per_op);
}

#[test]
#[ignore = "performance harness; run with --release --ignored --nocapture"]
fn perf_indices_panel() {
    let mut app = app_with_indices(INDEX_COUNT);
    app.select_first();

    println!(
        "\n{} indices, {}x{} terminal\n",
        INDEX_COUNT, TERMINAL.width, TERMINAL.height
    );

    println!("per keypress:");
    time("select_down()", 200, || {
        app.select_down();
    });

    println!("\nper frame (redraw runs unconditionally, ~20x/sec):");
    time("visible_summary()", 200, || {
        black_box(app.visible_summary());
    });

    let summary = app.visible_summary();
    time("IndicesTable::render()", 200, || {
        let mut buf = Buffer::empty(TERMINAL);
        let mut state = TableState::default().with_offset(0).with_selected(Some(0));
        IndicesTable::new(&app, &summary.indices, 0).render(TERMINAL, &mut buf, &mut state);
    });
    drop(summary);

    // A snapshot adds a hash lookup per on-screen row to the render, and one
    // reconciliation pass over the whole list per refresh tick. Both need to
    // stay off the critical path at cluster scale.
    println!("\nwith an active snapshot baseline:");
    app.take_snapshot();
    time("snapshot.observe() [per refresh tick]", 50, || {
        app.snapshot.observe(&app.indices, None);
    });

    let summary = app.visible_summary();
    time("IndicesTable::render() [snapshot]", 200, || {
        let mut buf = Buffer::empty(TERMINAL);
        let mut state = TableState::default().with_offset(0).with_selected(Some(0));
        IndicesTable::new(&app, &summary.indices, 0).render(TERMINAL, &mut buf, &mut state);
    });
    drop(summary);
    app.snapshot.clear();

    println!("\nwith an active jq filter:");
    app.filter.toggle_mode();
    app.filter.input = app
        .filter
        .input
        .clone()
        .with_value(".doc_count > 1000".to_string());
    app.filter.recompile();
    assert!(app.filter.error.is_none(), "jq filter failed to compile");
    time("visible_summary() [jq]", 5, || {
        black_box(app.visible_summary());
    });
    app.filter.clear();

    println!("\nwith an active filter:");
    app.filter.input = app.filter.input.clone().with_value("app-0001".to_string());
    app.filter.recompile();
    time("visible_summary() [plain substring]", 100, || {
        black_box(app.visible_summary());
    });
    time("select_down() [plain substring]", 100, || {
        app.select_down();
    });

    app.filter.input = app
        .filter
        .input
        .clone()
        .with_value("app-00[0-9]1".to_string());
    app.filter.recompile();
    assert!(app.filter.error.is_none(), "regex failed to compile");
    time("visible_summary() [true regex]", 50, || {
        black_box(app.visible_summary());
    });
    println!();
}
