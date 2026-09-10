# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-09-10

### Added
- Snapshots: `s` freezes every metric in both the indices and nodes panels at that moment, and every subsequent refresh is compared against that frozen baseline. Metrics that have moved carry an arrow — `↑` higher than the snapshot, `↓` lower, nothing at all when the value is exactly where it was. `s` again re-baselines from now, `S` drops the baseline. Previously the tables showed only the current reading, so answering "how much has this index actually grown while I've been watching?" meant writing the numbers down first.
  - The baseline is fixed, not rolling: this compares against the snapshot, not against the previous tick, so a value that spikes and then partially falls back still reads as `↑` until it drops below where it started.
  - Marked columns are the numeric ones — `Docs Count`, `Rate (/s)` and `Size` on the indices panel, and all five metrics on the nodes panel. The index name and health are left alone: neither is a magnitude, so "increased" isn't something either can do.
  - The arrows are `↑`/`↓` rather than the `▲`/`▼` a column header shows for sort direction, since the two mean different things and appear a row apart. Green for up and red for down is direction, not judgement — a node whose `IdxFailed` count has climbed gets a green arrow like anything else that went up.
  - One press captures both panels, so you can snapshot on the indices view, press `n`, and see node movement from the same moment. When a snapshot is taken before the nodes panel has ever been opened there are no node readings to capture — `_nodes/stats` isn't requested until that panel is on screen — so the node half of the baseline is filled in by the first fetch that does happen.
  - Rows that appear after the snapshot adopt their first observed reading as their baseline and so start unmarked; rows that disappear are dropped from it. There is no honest way to compare an index against a moment it didn't exist in.
  - The footer shows the time the baseline was taken (`⇅ 14:03:21`) while one is active.
  - Rendering is unaffected at cluster scale: the tables do one hash lookup per on-screen row, and `IndicesTable::render()` measures the same 0.24ms at 50,000 indices with a snapshot as without. Reconciling the baseline with the cluster costs ~2.5ms at that size, and runs once per refresh tick rather than on the ~20x/sec redraw.

### Changed
- The indices table's metric columns are fixed-width instead of percentages, matching the nodes table. Each has to fit its header plus a sort arrow and its value plus a trend arrow, and at 80 columns the old percentages truncated `Size` values such as `1023.9 KiB` even before the arrows were added.

## [0.4.1] - 2026-09-09

### Changed
- The indices and nodes tables now build only the rows that are actually on screen. Both previously constructed a `Row` for every visible entry — each cloning two strings and formatting three numbers — and handed the lot to ratatui, which then drew about forty of them. On a 50,000-index cluster that cost ~93ms per frame, and since the event loop redraws unconditionally roughly twenty times a second, navigation was permanently queued behind it. Rendering the same table now takes ~0.26ms, and arrow keys are immediate.
  - Scroll position is computed by `ui::scroll_offset()` rather than by ratatui, which can no longer do it now that it only receives the visible window. Behaviour is unchanged: the list holds still until the cursor would leave the viewport.
- Filter patterns containing no regex metacharacters are matched with a substring search instead of the regex engine. Typed filters are overwhelmingly plain substrings, and on a 50,000-index cluster filtering dropped from ~15.6ms per frame to ~0.22ms. Patterns that do use metacharacters still compile as regexes, so anchors and character classes behave exactly as before.

### Added
- A performance harness for the indices panel at cluster scale, run with `cargo test --release perf -- --ignored --nocapture`. It is `#[ignore]`d, so it never runs in CI.

## [0.4.0] - 2026-09-09

### Changed
- The cluster indexing rate history is now a smooth filled area graph rather than a bar chart. The old one drew six-column-wide bars with a numeric label under each, which fit about nine samples across a typical panel and read as a row of blocks rather than a trend. The new graph plots one sample per terminal column and uses partial block glyphs (`▁▂▃▄▅▆▇█`) for eight vertical steps per row, so a six-row panel resolves 48 levels. Columns are shaded bottom-to-top through the active colormap, so `c`/`C` restyles the graph along with the tables, and history scrolls in from the right edge. A sample of zero draws a dim floor tick so an idle cluster is distinguishable from no data at all.
- The cluster rate history keeps 300 samples instead of 60. One sample is drawn per terminal column, and 60 could not fill the graph panel on anything but a narrow terminal.

## [0.3.0] - 2026-09-09

### Added
- A nodes view, toggled with `n`, that replaces the indices table in the main panel. Sourced from `GET /_nodes/stats`, it shows one row per node with process CPU %, JVM heap used %, failed indexing operations, average bulk request size, and parent circuit-breaker trips. Columns are sortable (`←`/`→`, `r` to reverse) and rows selectable (`j`/`k`) just like the indices table, and rows are shaded with the same colormap gradient, keyed on whichever column is sorted.
  - `_nodes/stats` is requested only while the nodes panel is on screen, so the view costs nothing when closed. The indices and cluster-health requests continue regardless, since the header rate and the cluster graph depend on them whichever panel is showing.
  - The two panels keep independent sort columns, cursors and scroll offsets, so switching between them preserves your place in each. Exclusions (`x`) remain index-only.
  - Nodes on clusters older than ES 7.13 report `0 B` for bulk average size; that section of the API doesn't exist there. Any missing section degrades to zero rather than failing the panel.

### Changed
- The filter box now applies to the nodes panel as well as the indices panel, matching node names in regex mode and the node's fields in jq mode. Previously pressing `/` on the nodes panel entered filter mode with nothing shown and no effect, so keystrokes vanished.
- The main panel's title is now an `Indices │ Nodes` tab strip with the active view highlighted, rather than naming only the table being shown — the nodes view is otherwise undiscoverable from the main screen. The refresh spinner, fetch duration and row count moved to the right-hand end of the same title bar, with the spinner inside the duration's parentheses (`(⠹ 0.4s)`), where they hold a fixed position instead of being pushed around by the filter text growing to their left.
- The indices table now scrolls only when the cursor would leave the viewport, instead of re-centering the selected row on every frame. Previously the cursor was effectively pinned to the middle of the table and the whole list slid underneath it on each `j`/`k`; now the cursor moves within a stable page, which is what every other list-shaped TUI does.
- The `_stats` and `_cluster/health` requests are now issued concurrently on each refresh tick rather than one after the other, roughly halving the latency of a refresh against a slow or distant cluster.

### Fixed
- The table selection no longer drifts onto a different index. It was tracked as a row position, but the list is re-sorted on every refresh (by rate, by default) — so on a busy cluster the highlighted row would silently end up on an index other than the one you selected, and `Enter`/`x` would act on that other index. Selection is now tracked by index name and follows the index across re-sorts, filter changes, and system-index toggling; it is dropped only when the index itself disappears from the cluster.
- Indexing rates below 1/s rendered with an SI sub-unit suffix — `0.8251` came out as `825.1m` (milli-units) in a column headed `Rate (/s)`, which reads as minutes. Values under 1 are now shown as plain decimals (`0.8`).
- `--colormap` had three disagreeing defaults: the CLI defaulted to `warm`, `Colormap::default()` was `turbo`, and the README documented `inferno`. All three are now `warm`, with the CLI default derived from `Colormap::default()` so they cannot drift apart again. The README also documented `--rate-samples` as defaulting to 3 in one place; the actual default is 10.

## [0.2.2] - 2026-07-25

### Changed
- Updated dependencies, including `reqwest` 0.12 → 0.13. Its `rustls-tls` feature was renamed/restructured upstream; we now use `rustls` + `webpki-roots`, which pulls in `aws-lc-rs` as the TLS crypto backend. Building from source now requires a C compiler and CMake (both already present in most dev environments and GitHub Actions' `ubuntu-latest`).
- `jaq-core`/`jaq-std`/`jaq-json` remain pinned to their 2.x/1.x lines for now — 3.x is available but is a breaking rewrite of the API our jq filter mode depends on; upgrading needs a dedicated, carefully-verified pass rather than a routine bump.

## [0.2.1] - 2026-07-25

### Added
- The header now shows the running version (e.g. `EstiCLI v0.2.1`), read from `Cargo.toml` at compile time so it can never drift from what's actually running.

### Changed
- The filter box cursor now blinks using its own timer instead of the terminal's SGR blink attribute, which most modern terminal emulators ignore and simply render as a solid, non-blinking caret.

### Fixed
- Regex-mode filtering was extremely slow on clusters with thousands of indices. It compiled down to jq's `select(.name | test(...))`, and jq's `test()` recompiles its regex argument from scratch on every call — meaning the same regex was being recompiled once per index on every redraw (~20x/second). Regex mode now compiles the pattern once per keystroke and matches directly against the index name, measured ~28x faster on 5,000 indices.
- Moving the cursor in the filter box (arrow keys, Home/End, word-jumps) recompiled the entire filter on every keypress even though the text hadn't changed, making cursor movement feel sluggish — especially in jq mode, where recompiling means re-parsing jq's standard library. Cursor-only movement no longer triggers a recompile.

## [0.2.0] - 2026-07-25

### Added
- Filter mode now defaults to a plain regex matched against the index name — no jq knowledge required. Press `/` again while the filter box is empty ("//") to switch to jq mode, where the typed boolean expression is automatically wrapped in `select(...)`.
- The indices table title now indicates which filter syntax is active ("Filter:" for regex, "jq:" for jq mode).

### Changed
- The index details popup now refreshes automatically in step with the main list's refresh interval instead of staying frozen at whatever it showed when opened. Refreshes update the existing view in place (a small "⟳ refreshing" indicator appears in the title) rather than flashing back to a loading screen, and a transient refresh failure keeps showing the last known-good data instead of replacing it with an error.

### Fixed
- Filters that fail at runtime (e.g. an invalid regex) are now caught immediately when typed and surfaced as an error, instead of silently matching every index.

## [0.1.0] - 2025-12-26

### Added
- Initial release of EstiCLI.
- Real-time monitoring of Elasticsearch index ingestion rates.
- Cluster health and sparkline chart visualizations.
- Regex filtering and smart sorting of indices.
- Detailed index information popup.
- Flexible authentication (Basic, API Key, Custom CA).
- Keyboard-driven Navigation (Vim-style).
- Support for multiple colormaps for data visualization.

### Changed
- Refined error handling to use custom `EstiCliError` enum.
- Improved resource management with automatic history pruning for deleted indices.
- Enhanced code documentation and added some unit tests.
