# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- The indices table now scrolls only when the cursor would leave the viewport, instead of re-centering the selected row on every frame. Previously the cursor was effectively pinned to the middle of the table and the whole list slid underneath it on each `j`/`k`; now the cursor moves within a stable page, which is what every other list-shaped TUI does.
- The cluster's `_stats` and `_cluster/health` requests are now issued concurrently on each refresh tick rather than one after the other, roughly halving the latency of a refresh against a slow or distant cluster.

### Fixed
- The table selection no longer drifts onto a different index. It was tracked as a row position, but the list is re-sorted on every refresh (by rate, by default) — so on a busy cluster the highlighted row would silently end up on an index other than the one you selected, and `Enter`/`x` would act on that other index. Selection is now tracked by index name and follows the index across re-sorts, filter changes, and system-index toggling; it is dropped only when the index itself disappears from the cluster.
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
