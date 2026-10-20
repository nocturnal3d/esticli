# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.2] - 2026-10-20

### Fixed
- The footer named regex as the filter mode even while typing a jq expression. It now says which mode the box is in.
- The details popup counted shards as "1 primary, 1 replicas" and "3 primary". It now reads "1 primary, 1 replica", or "3 primaries, 1 replica each" when there is more than one primary, since the replica count is per primary.

### Changed
- The README shows an animated demo instead of a screenshot.

## [0.6.1] - 2026-10-11

### Changed
- An active filter no longer slows down every frame and every keypress. The result of filtering is now kept until something it depends on changes — a refresh tick, an edit to the filter, an exclusion, the system-index toggle, switching panels — instead of being recomputed over every index about twenty times a second and again on each cursor movement. On a 50,000-index cluster, a jq filter dropped from ~35ms per frame and per keypress to ~0.27ms per frame and ~14µs per keypress, and a regex filter from ~15ms to ~0.17ms. Previously a jq filter kept most of a CPU core busy while the app sat idle, and arrow keys lagged by ~70ms. The filter still runs once per refresh tick, since the data has changed by then.
  - Moving the cursor within the filter box doesn't count as a change, so it stays instant however expensive the filter is.
- Frames without a filter are about twice as fast at that scale (~0.39ms → ~0.21ms), mostly because formatting the numbers in each row no longer rebuilds the formatter's tables on every call.
- Each refresh tick does less on the main thread when no index was deleted since the last one (~7.2ms → ~5.5ms at 50,000 indices), and reconciling an active snapshot baseline costs ~0.6ms instead of ~2.1ms.
- The details popup no longer lays out its content twice per frame. For an index with 1000 shard copies a frame took ~2.2ms with the popup open, and now takes ~1.3ms.

### Added
- The performance harness also measures a whole frame, the refresh tick, the rate graph, each filter mode, and the details popup at large shard counts.

## [0.6.0] - 2026-10-10

### Fixed
- Elasticsearch 7.x (and 8.0) clusters work again. `_stats` only reports a per-index `health` from 8.1, and one entry without it failed the whole response, so every refresh errored with ``missing field `health` ``. Health now shows `-` where the cluster doesn't report it. Entries whose primaries didn't answer, and nodes reporting CPU as `-1` (unavailable), no longer fail the whole response either.
- Typing any non-ASCII character in the filter box (`é`, `日`, an emoji) no longer crashes the app. The caret position counts characters, and it was being used as a byte offset.
- Data stream backing indices (`.ds-<stream>-…`) are no longer hidden as system indices. On clusters fed by Elastic Agent, Logstash or OpenTelemetry they hold most of the ingestion, so the header's Cluster Rate and the graph were reporting a fraction of real traffic by default. Dot-prefixed indices that really are internal are still hidden until `.`.
- A base URL with a path prefix (`-u https://gateway/elasticsearch`) is honoured; previously the prefix was silently dropped from every request. Index names containing `%`, `&`, `#` or non-ASCII characters are now percent-encoded in request paths, so their details popup loads.
- The details popup shows the ILM phase. The request went to the wrong endpoint and the response field was misnamed, and both failures were silently swallowed.
- A filter typed on the nodes panel no longer also filters the indices, which zeroed the header rate and drew zeros into the graph while it was active.
- `Enter` and `x` do nothing on the nodes panel, instead of opening details for, or excluding, an index selected off screen.
- A failing `_cluster/health` or `_nodes/stats` request (a role without the cluster `monitor` privilege, one unresponsive node) no longer discards the `_stats` reading that arrived alongside it. The table, header rate and graph keep updating, and the header names the part that failed.
- Indexing rates are no longer under-reported for the first `--rate-samples` refreshes after startup and after every rollover. Each index's first reading has nothing to diff against, and it had been averaged in as a rate of 0.
- Rows with equal values (every idle index, under the default rate sort) keep a stable name order instead of reshuffling on every refresh. Sorting by Health orders by severity, so red can be brought to the top.
- `Ctrl+C` quits from anywhere. Modified keys no longer trigger the bare key's binding: `Ctrl+C` used to cycle the colormap, `Ctrl+S` re-took the snapshot, `Alt+Q` quit.
- `Ctrl+U` clears the filter but keeps you in the filter box, instead of turning your next keystrokes (`q`…) into commands. Pasting into the filter box inserts the text rather than replaying it as keystrokes.
- An invalid filter now says why next to the input. A long filter scrolls to keep the caret in view instead of being cut off behind the status.
- jq filters that only make sense on the nodes panel (`.heap_used_percent * 2 > 100`) are accepted. jq's `halt` and `halt_error` no longer exit the app with the terminal left in raw mode, and `stderr` no longer writes over the screen.
- Taking a snapshot away from the nodes panel no longer freezes stale node readings as the baseline. A node that drops out for a tick while restarting keeps its baseline, so the restart shows as `⟲`. A smoothed rate whose displayed value hasn't changed no longer carries an arrow.
- The details popup is no longer held up behind a slow polling request, and a result for a previously opened index can't land in a newly opened popup. Its content can be scrolled to the end when lines wrap, and scrolling past the end of either popup no longer banks dead keypresses.
- The cluster graph scales to the samples it's drawing, rather than to a peak that has scrolled off screen, and it graphs sub-1/s ingestion instead of truncating it to 0. The health panel shows a placeholder until the first response, instead of a red `0%`.
- A full mount of a searchable snapshot is no longer labelled partial, and a legacy `_freeze`d index is no longer described as a searchable snapshot. Templates matching wildcards anywhere in their pattern (`logs-*-*`) are found, and a backing index shows its data stream's template.
- Credentials embedded in the URL (`https://user:pass@host`) are no longer shown in the header. Connection errors include their cause (connection refused, certificate not trusted, timeout), and error responses are cut down to Elasticsearch's `reason` instead of a whole HTML page.

### Changed
- Each refresh asks Elasticsearch only for the fields it displays (`filter_path`), about 8x less for the cluster to build, send and parse. That matters at tens of thousands of indices, and on HTTPS clusters, where Elasticsearch doesn't compress responses by default.
- `--help` describes every option, and `--version` exists. `ESTICLI_URL`, `ESTICLI_USERNAME`, `ESTICLI_PASSWORD`, `ESTICLI_API_KEY` and `ESTICLI_CA_CERT` can replace their arguments, which keeps secrets out of `ps`. `--username` and `--password` must be given together, and `--refresh` (1–60) and `--rate-samples` are range-checked. A URL without `http://`/`https://`, or a CA file with no certificate in it, is rejected at startup with a clear message.
- The minimum supported Rust version is 1.88, which is what the dependencies already required; 1.70 was never buildable. CI now lints test code too (`clippy --all-targets`).

## [0.5.0] - 2026-09-10

### Added
- Snapshots: `s` freezes every metric in both the indices and nodes panels at that moment, and every subsequent refresh is compared against that frozen baseline. Metrics that have moved carry an arrow — `↑` higher than the snapshot, `↓` lower, nothing at all when the value is exactly where it was. `s` again re-baselines from now, `S` drops the baseline. Previously the tables showed only the current reading, so answering "how much has this index actually grown while I've been watching?" meant writing the numbers down first.
  - The baseline is fixed, not rolling: this compares against the snapshot, not against the previous tick, so a value that spikes and then partially falls back still reads as `↑` until it drops below where it started.
  - Marked columns are the numeric ones — `Docs Count`, `Rate (/s)` and `Size` on the indices panel, and all five metrics on the nodes panel. The index name and health are left alone: neither is a magnitude, so "increased" isn't something either can do.
  - The arrows are `↑`/`↓` rather than the `▲`/`▼` a column header shows for sort direction, since the two mean different things and appear a row apart. Green for up and red for down is direction, not judgement — a node whose `IdxFailed` count has climbed gets a green arrow like anything else that went up.
  - One press captures both panels, so you can snapshot on the indices view, press `n`, and see node movement from the same moment. When a snapshot is taken before the nodes panel has ever been opened there are no node readings to capture — `_nodes/stats` isn't requested until that panel is on screen — so the node half of the baseline is filled in by the first fetch that does happen.
  - Rows that appear after the snapshot adopt their first observed reading as their baseline and so start unmarked; rows that disappear are dropped from it. There is no honest way to compare an index against a moment it didn't exist in.
  - Metrics are classified as gauges or cumulative counters, because the two mean opposite things when they fall. `IdxFailed` and `BrkTripd` are counters: they only climb, and go backwards solely when the node restarts, so they never show a down arrow. A fall there is drawn as `⟲` and the row is re-baselined to the restart value, so failures logged after a restart are visible immediately rather than staying hidden until the count climbed back past its pre-restart total. Without this a node restart rendered as `0 ↓`, which reads as failures going away when the truth is that their history was lost.
  - Everything else is a gauge, compared in both directions — including `Docs Count` and `Size`, which trend upwards but genuinely shrink through deletes, segment merges and ILM. A real decrease there is worth seeing rather than being explained away as a restart.
  - A metric with no arrow is exactly where it was, *including* one that moved and came back. Because the comparison is against the snapshot rather than the previous refresh, an indexing rate that bursts and settles back to zero — or a heap percentage returning to the number it started on — drops its arrow again.
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
