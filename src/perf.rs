//! Performance harness for the indices panel at cluster scale, plus the
//! details popup at large shard counts.
//!
//! Not run by CI — these are timing measurements, not assertions:
//!
//! ```text
//! cargo test --release perf -- --ignored --nocapture
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{StatefulWidget, TableState, Widget};
use ratatui::Terminal;

use crate::app::{App, FetchPayload};
use crate::elasticsearch::AuthConfig;
use crate::models::{ClusterHealth, IndexRate};
use crate::ui;
use crate::ui::chart::RateChart;
use crate::ui::table::IndicesTable;
use crate::ui::types::Colormap;

const INDEX_COUNT: usize = 50_000;
const TERMINAL: Rect = Rect {
    x: 0,
    y: 0,
    width: 160,
    height: 50,
};

fn index(i: usize) -> IndexRate {
    IndexRate {
        name: format!("logs-app-{:06}-2026.09.09", i),
        doc_count: (i as u64) * 37,
        rate_per_sec: (i % 997) as f64,
        size_bytes: (i as u64) * 4096,
        health: if i.is_multiple_of(11) {
            "yellow"
        } else {
            "green"
        }
        .to_string(),
    }
}

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

    app.indices = (0..count).map(index).collect::<Vec<_>>().into();
    app
}

/// One refresh tick's worth of `_stats` results, as the polling task hands
/// them to the main thread. Most indices are idle, as on a real cluster.
fn tick_payload(count: usize, tick: u64) -> FetchPayload {
    let indices = (0..count)
        .map(|i| {
            let mut index = index(i);
            index.doc_count += tick * (i as u64 % 7);
            let sample = i
                .is_multiple_of(13)
                .then_some(((i as u64 + tick) % 997) as f64);
            (index, sample)
        })
        .collect();
    FetchPayload {
        indices: Ok(indices),
        health: Ok(ClusterHealth::default()),
        nodes: None,
    }
}

fn report(label: &str, per_op: Duration) {
    println!("  {:<44} {:>10.3?}", label, per_op);
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
    report(label, start.elapsed() / iterations);
}

/// Like `time`, but builds each iteration's input outside the measurement,
/// for operations that consume what they are given.
fn time_with_setup<T, S: FnMut(u32) -> T, F: FnMut(T)>(
    label: &str,
    iterations: u32,
    mut setup: S,
    mut op: F,
) {
    for i in 0..3 {
        op(setup(i));
    }

    let mut total = Duration::ZERO;
    for i in 0..iterations {
        let input = setup(i + 3);
        let start = Instant::now();
        op(input);
        total += start.elapsed();
    }
    report(label, total / iterations);
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(TERMINAL.width, TERMINAL.height)).unwrap()
}

fn draw(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
}

#[test]
#[ignore = "performance harness; run with --release --ignored --nocapture"]
fn perf_indices_panel() {
    let mut app = app_with_indices(INDEX_COUNT);
    let mut terminal = terminal();
    app.select_first();

    println!(
        "\n{} indices, {}x{} terminal\n",
        INDEX_COUNT, TERMINAL.width, TERMINAL.height
    );

    println!("per keypress:");
    time("select_down()", 200, || {
        app.select_down();
    });
    // Cursor deep in the list, as after End or a long scroll: anything that
    // locates the selection by scanning pays for every row above it.
    app.select_last();
    time("select_up() [cursor near the end]", 200, || {
        app.select_up();
    });

    println!("\nper frame (redraw runs unconditionally, ~20x/sec):");
    app.select_first();
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

    // The whole frame as the event loop draws it: every widget plus
    // ratatui's diff against the previous frame.
    app.rate_history = (0..300).map(|i| (i * 37 % 1000) as f64).collect();
    time("ui::draw() [full frame]", 200, || {
        draw(&mut terminal, &mut app);
    });
    app.select_last();
    time("ui::draw() [cursor near the end]", 200, || {
        draw(&mut terminal, &mut app);
    });
    app.select_first();

    // The graph is drawn cell by cell; the spline-based colormaps are the
    // expensive ones to sample.
    let chart_area = ui::compute_areas(TERMINAL, &app).chart.unwrap();
    for colormap in [Colormap::Warm, Colormap::Inferno] {
        app.colormap = colormap;
        time(&format!("RateChart::render() [{}]", colormap), 200, || {
            let mut buf = Buffer::empty(chart_area);
            RateChart::new(&app).render(chart_area, &mut buf);
        });
    }
    app.colormap = Colormap::default();

    println!("\nper refresh tick (main thread):");
    time_with_setup(
        "apply_fetch()",
        20,
        |i| tick_payload(INDEX_COUNT, i as u64),
        |payload| app.apply_fetch(payload),
    );

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
    time("ui::draw() [jq]", 5, || {
        draw(&mut terminal, &mut app);
    });
    time("select_down() [jq]", 5, || {
        app.select_down();
    });
    time_with_setup(
        "apply_fetch() [jq]",
        5,
        |i| tick_payload(INDEX_COUNT, i as u64),
        |payload| app.apply_fetch(payload),
    );
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
    time("ui::draw() [true regex]", 50, || {
        draw(&mut terminal, &mut app);
    });
    time("select_down() [true regex]", 50, || {
        app.select_down();
    });
    println!();

    details_popup();
}

/// Part of the single harness test rather than a test of its own, so the two
/// never run concurrently and skew each other's timings.
fn details_popup() {
    use crate::models::{IndexDetails, ShardInfo};

    let mut app = app_with_indices(100);
    let mut terminal = terminal();
    app.details.show_popup = true;

    // The popup lists every shard copy, so its cost per frame grows with the
    // index's shard count; an index with hundreds is uncommon but real.
    println!("per frame, details popup open:");
    for shards in [10, 200, 1000] {
        app.details.data = Some(IndexDetails {
            name: "big-index".to_string(),
            provided_name: None,
            creation_date: None,
            primary_shards: shards / 2,
            replica_shards: 1,
            is_frozen: false,
            is_searchable_snapshot: false,
            is_partial: false,
            ilm_policy: None,
            ilm_phase: None,
            total_segments: 0,
            shard_allocation: (0..shards)
                .map(|i| ShardInfo {
                    shard_id: i / 2,
                    primary: i.is_multiple_of(2),
                    state: "STARTED".to_string(),
                    node: format!("es-data-{}", i % 30),
                    docs: Some(123_456),
                    size: Some("12.3gb".to_string()),
                })
                .collect(),
            templates: Vec::new(),
            uuid: None,
            health: None,
            status: None,
            doc_count: 1,
            rate_per_sec: 1.0,
            size_bytes: 1,
            data_stream: None,
        });
        time(&format!("ui::draw() [{} shard copies]", shards), 50, || {
            draw(&mut terminal, &mut app);
        });
    }
    println!();
}
