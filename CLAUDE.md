# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Run against a local/dev cluster
cargo run -- -u http://localhost:9200

# Build
cargo build            # debug
cargo build --release  # release

# Test (all tests are unit tests colocated in `#[cfg(test)] mod tests` blocks)
cargo test
cargo test test_selection_movement       # single test by name
cargo test --lib app::filter::tests      # a module's tests

# Lint / format — CI runs these with zero tolerance for warnings/diffs
cargo clippy -- -D warnings
cargo fmt --all -- --check
cargo fmt --all         # apply formatting
```

CI (`.github/workflows/ci.yml`) runs `cargo check`, `cargo test`, `cargo fmt --all -- --check`, and `cargo clippy -- -D warnings` as separate jobs on every push/PR to `main`. A change isn't done until all four pass.

## Architecture

EstiCLI is a `top`-like ratatui TUI that polls Elasticsearch on a timer and renders indexing rates. The whole app runs on a single-threaded event loop in `main.rs`; concurrency is limited to two background fetch tasks that report back over channels.

### Event loop (`main.rs`)

`run()` is a tight loop: poll async results (non-blocking) → tick spinner → draw → poll keyboard (50ms timeout) → maybe start a new fetch. There is no dirty-tracking — every iteration redraws unconditionally. `map_key_to_action()` translates a `KeyEvent` into an `app::actions::Action`; popups (`help_popup`, `details.show_popup`) and filter-input mode each get their own key-mapping branch checked in priority order before the default keymap. `Action` is a plain data enum (some variants carry values, e.g. `SelectPageUp(usize)`) dispatched through `App::handle_action` — this indirection exists so key bindings and behavior stay decoupled and testable without a terminal.

Page-scroll sizes (`SelectPageUp/Down`, `DetailsScrollPageUp/Down`) are computed fresh each loop iteration from the *actual* rendered geometry (`ui::table_page_size`, `ui::details_popup::visible_rows`) rather than hardcoded — see "Layout" below for why this must stay in sync with `ui::compute_areas`.

### App state (`app/mod.rs`)

`App` owns all state and is passed by reference into every UI widget (read-only) and mutated only via `Action` handlers. Sub-concerns are split into their own state structs, each with delegating methods on `App`: `SortState` (`app/sort.rs`), `FilterState` (`app/filter.rs`), `DetailsState` (`app/details.rs`).

**Visibility filtering is centralized.** `App::is_visible()` is the single predicate (exclusions + system-index toggle + name/jq filter) that both `filtered_indices()` and `visible_summary()` delegate to — never reimplement this filter inline. `visible_summary()` computes the filtered index list *and* aggregated `ClusterMetrics` (total rate, total bytes/sec) in one pass; `ui::draw()` calls it once per frame and threads the result into every widget that needs it (header, table, footer) instead of each widget re-filtering independently. When adding a new widget that needs the visible set, take it as a constructor argument rather than calling `app.filtered_indices()` inside `render()`.

**Filter mode compiles down to two different representations, deliberately.** `FilterState::recompile()` (`app/filter.rs`) produces either a `regex_lite::Regex` (default, `/`) matched directly against `.name`, or a compiled jq `select(...)` filter (`//`, toggled while the input is empty) matched against the full serialized index. This split exists because jq's `test()` builtin recompiles its regex argument from scratch on *every call* — routing plain name search through `select(.name | test(...))` would mean recompiling the same regex once per index on every redraw (measured ~28x slower on 5k indices). Never route the common name-search case through jq's `test`/`match` for this reason; only jq mode should touch `jaq_core`/`serde_json` per match.

**Rate calculation is a two-stage average.** The ES client (`elasticsearch/stats.rs`) computes a raw docs/sec delta between consecutive `_stats` snapshots (`EsClient::previous_snapshot`, keyed by index name, with `(Instant, HashMap<String, IndexSnapshot>)`). `App::update_indices_with_rates()` then smooths that raw rate over a rolling window (`--rate-samples`, per-index `VecDeque<f64>` in `index_rate_history`) before it's stored in `App.indices`. The cluster-wide rate graph (`rate_history`, capped at `MAX_HISTORY_POINTS` = 60) is a *separate* smoothing window over the already-smoothed cluster total — the two window sizes are independent knobs.

**Background fetches use mpsc, not shared mutable state.** `App::start_fetch()` spawns a task that locks `es_client` (`Arc<Mutex<EsClient>>`), fetches rates + cluster health concurrently, and sends the combined result down `fetch_tx`; `poll_fetch_result()` drains `fetch_rx` non-blockingly each loop tick. `DetailsState` follows the identical pattern with its own channel pair, initially triggered by `Enter` and then kept alive: `App::refresh_open_details()` (called at the end of every successful `poll_fetch_result`) re-fetches the open popup's index by name, in step with the main list's refresh cadence. `DetailsState::fetch()` distinguishes "opening a new index" (resets `data`/`scroll`/`loading` for a clean slate) from "refreshing the same one already on screen" (leaves them untouched so the popup updates seamlessly, with `refreshing` driving a small in-title indicator instead) — and refuses to start a second fetch while one's still in flight. Don't block the render loop waiting on either channel.

### Elasticsearch layer (`elasticsearch/`)

`EsClient` (`client.rs`) wraps `reqwest` with a `send_json<T>` helper that maps non-2xx responses to `EstiCliError::Api`. `stats.rs` handles the polling path (`_stats`, `_cluster/health`); `details.rs` handles the on-demand path (`fetch_index_details`), which fans out 7 parallel requests via `tokio::join!` (settings, ILM explain, segments, cat/shards, index templates, cat/indices, data streams) and degrades each field independently with `.unwrap_or_default()`/`.ok()` rather than failing the whole popup if one sub-request errors. Response shapes live in `types.rs`, deliberately mirroring ES's JSON rather than the app's internal `models.rs` structs — conversion happens explicitly in `stats.rs`/`details.rs`, keeping wire format decoupled from what the UI consumes.

### UI layer (`ui/`)

`ui::compute_areas()` is the one place that knows the screen layout (header / graph+health / table / footer, with graph+health further split horizontally); it's used both by `draw()` for rendering and by `main.rs` for computing page-scroll sizes, so they can't drift apart. Each widget module (`header.rs`, `table.rs`, `footer.rs`, `chart.rs`, `health.rs`, `details_popup.rs`, `help_popup.rs`) implements ratatui's `Widget`/`StatefulWidget` and takes `&App` (plus any precomputed data like `ClusterMetrics` or the filtered index slice) as constructor arguments — no widget mutates state. Colors driven by data (table gradient, health status) go through `Colormap::color_at()` (`ui/types.rs`, backed by `colorgrad` presets) or fixed thresholds in `theme.rs`; keep new data-driven coloring consistent with one of those two mechanisms rather than inventing a third.

Popups (`details_popup.rs`, `help_popup.rs`) each expose their own `visible_rows`/height-calculation logic derived from the same percentage-of-terminal sizing they use to render, for the same reason as `compute_areas` — so keyboard paging matches what's on screen.
