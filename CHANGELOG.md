# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
