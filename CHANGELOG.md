# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
