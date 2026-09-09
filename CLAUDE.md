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

**The main panel holds one of two tables.** `App.show_main_panel` controls whether the panel is on screen at all (`3`); `App.main_panel: MainPanel` (`Indices` | `Nodes`, toggled with `n`) controls which table it holds. They are alternatives occupying the same `Areas::table` rect, not stacked panels. Navigation, sorting and scroll offset all dispatch on `main_panel` — `select_up`/`next_column`/etc. act on the focused panel only, and each panel keeps its own cursor (`selected_name` / `selected_node`), sort state (`sort` / `node_sort`) and offset (`table_offset` / `node_table_offset`) so switching preserves both. When adding a panel-scoped behavior, extend the dispatch rather than reading `main_panel` inside a widget.

**`_nodes/stats` is only requested while its panel is focused.** `App::nodes_focused()` gates the request in `start_fetch()`, and `FetchPayload.nodes` is `Option`: `None` means "not requested, keep what's on screen", which is deliberately distinct from `Some(vec![])` ("the cluster reported no nodes"). The indices and health requests stay unconditional because the header rate and cluster graph are driven by them regardless of which panel is showing. `EsClient::fetch_tick(want_nodes)` joins all three.

**Selection is tracked by index name, not by row position.** `App.selected_name: Option<String>` is the cursor; the row it lands on is derived on demand via `selected_position_in(&visible)` (or `selected_position()`, which filters first). This is deliberate: the list is re-sorted on every refresh tick, so a `selected_index: usize` would silently slide onto a different index between polls. Anything acting on the selection should resolve it by name rather than by indexing into `filtered_indices()`. A selection that is merely filtered out is kept (so it returns when the filter is cleared) — only `clamp_selection()`, called after each successful refresh, drops it, and only when the index has actually disappeared from the cluster.

**Filter mode compiles down to two different representations, deliberately.** `FilterState::recompile()` (`app/filter.rs`) produces either a `regex_lite::Regex` (default, `/`) matched directly against `.name`, or a compiled jq `select(...)` filter (`//`, toggled while the input is empty) matched against the full serialized index. This split exists because jq's `test()` builtin recompiles its regex argument from scratch on *every call* — routing plain name search through `select(.name | test(...))` would mean recompiling the same regex once per index on every redraw (measured ~28x slower on 5k indices). Never route the common name-search case through jq's `test`/`match` for this reason; only jq mode should touch `jaq_core`/`serde_json` per match.

**Filter box input goes through `FilterState::handle_key`, not raw `tui_input` calls.** It only calls `recompile()` when `tui_input`'s `StateChanged.value` is true — cursor-only movement (arrows, Home/End, word-jumps) reports `value: false` and is intentionally cheap, so navigating the box stays instant regardless of how expensive the current filter is to (re)compile. It also drives the filter's cursor blink (`cursor_visible()`, rendered in `ui/table.rs`) off its own `Instant`-based clock rather than the terminal's SGR blink attribute, which most modern terminal emulators ignore.

**Rate calculation is a two-stage average.** The ES client (`elasticsearch/stats.rs`) computes a raw docs/sec delta between consecutive `_stats` snapshots (`EsClient::previous_snapshot`, keyed by index name, with `(Instant, HashMap<String, IndexSnapshot>)`). `App::update_indices_with_rates()` then smooths that raw rate over a rolling window (`--rate-samples`, per-index `VecDeque<f64>` in `index_rate_history`) before it's stored in `App.indices`. The cluster-wide rate graph (`rate_history`, capped at `MAX_HISTORY_POINTS` = 60) is a *separate* smoothing window over the already-smoothed cluster total — the two window sizes are independent knobs.

**Background fetches use mpsc, not shared mutable state.** `App::start_fetch()` spawns a task that locks `es_client` (`Arc<Mutex<EsClient>>`), calls `EsClient::fetch_rates_and_health()`, and sends the combined result down `fetch_tx`. That method `tokio::join!`s the `_stats` and `_cluster/health` requests, which is why `stats.rs` splits the polling path into an immutable `fetch_snapshot()` (joinable) and a `&mut` `rates_from_snapshot()` that runs afterwards — the snapshot bookkeeping needs `&mut EsClient`, and taking it during the request would serialise the two calls. Keep new polling requests on the `&self` side of that split; `poll_fetch_result()` drains `fetch_rx` non-blockingly each loop tick. `DetailsState` follows the identical pattern with its own channel pair, initially triggered by `Enter` and then kept alive: `App::refresh_open_details()` (called at the end of every successful `poll_fetch_result`) re-fetches the open popup's index by name, in step with the main list's refresh cadence. `DetailsState::fetch()` distinguishes "opening a new index" (resets `data`/`scroll`/`loading` for a clean slate) from "refreshing the same one already on screen" (leaves them untouched so the popup updates seamlessly, with `refreshing` driving a small in-title indicator instead) — and refuses to start a second fetch while one's still in flight. Don't block the render loop waiting on either channel.

### Elasticsearch layer (`elasticsearch/`)

`EsClient` (`client.rs`) wraps `reqwest` with a `send_json<T>` helper that maps non-2xx responses to `EstiCliError::Api`. `stats.rs` handles the polling path (`_stats`, `_cluster/health`); `details.rs` handles the on-demand path (`fetch_index_details`), which fans out 7 parallel requests via `tokio::join!` (settings, ILM explain, segments, cat/shards, index templates, cat/indices, data streams) and degrades each field independently with `.unwrap_or_default()`/`.ok()` rather than failing the whole popup if one sub-request errors. Response shapes live in `types.rs`, deliberately mirroring ES's JSON rather than the app's internal `models.rs` structs — conversion happens explicitly in `stats.rs`/`details.rs`, keeping wire format decoupled from what the UI consumes.

### UI layer (`ui/`)

`ui::compute_areas()` is the one place that knows the screen layout (header / graph+health / table / footer, with graph+health further split horizontally); it's used both by `draw()` for rendering and by `main.rs` for computing page-scroll sizes, so they can't drift apart. Each widget module (`header.rs`, `table.rs`, `footer.rs`, `chart.rs`, `health.rs`, `details_popup.rs`, `help_popup.rs`) implements ratatui's `Widget`/`StatefulWidget` and takes `&App` (plus any precomputed data like `ClusterMetrics` or the filtered index slice) as constructor arguments — no widget mutates state. `chart.rs` draws the rate history as a filled area graph, using partial block glyphs for eight vertical steps per terminal row and one sample per column (which is why `MAX_HISTORY_POINTS` has to exceed a wide terminal's width); its `column_eighths()` height math is a free function precisely so it can be unit-tested without a terminal. Colors driven by data (table gradient, chart, health status) go through `Colormap::color_at()` (`ui/types.rs`, backed by `colorgrad` presets) or fixed thresholds in `theme.rs`; both the indices and nodes tables shade rows via the shared `ui::types::gradient_position()` (log-scaled against the largest value in the sorted column) so a row's color means the same thing in either panel — keep new data-driven coloring consistent with one of those two mechanisms rather than inventing a third.

**The indices table's scroll offset lives in `App.table_offset`, not in the widget.** `ui::draw()` takes `&mut App` for exactly this reason: it seeds a `TableState` with the previous frame's offset, renders, then writes back `state.offset()`. Carrying the offset is what makes the table scroll only when the cursor would leave the viewport — ratatui's `Table` does that minimal "scroll into view" adjustment itself given a stable starting offset. `ui/table.rs` must therefore *not* compute an offset from the selection (an earlier version re-centered the selection every frame, which pinned the cursor and slid the whole list under it on every keypress).

Popups (`details_popup.rs`, `help_popup.rs`) each expose their own `visible_rows`/height-calculation logic derived from the same percentage-of-terminal sizing they use to render, for the same reason as `compute_areas` — so keyboard paging matches what's on screen.
