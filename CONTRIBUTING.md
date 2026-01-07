# Contributing to EstiCLI

First off, thanks for taking the time to contribute! :tada:

Contributions are welcome! Here's how you can help:

## Getting Started

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run linting:
   ```bash
   cargo clippy
   cargo fmt --check
   ```
5. Commit your changes (`git commit -m 'Add amazing feature'`)
6. Push to the branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

## Development Setup

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/esticli.git
cd esticli

# Run in development mode
cargo run -- -u http://localhost:9200

# Run with live reload (requires cargo-watch)
cargo install cargo-watch
cargo watch -x run
```

## Code Style

- Follow Rust conventions and idioms
- Run `cargo fmt` before committing
- Ensure `cargo clippy` passes without warnings
- Update documentation as needed

## Areas for Contribution

- [ ] Additional Elasticsearch metrics (search rate, query latency)
- [ ] Configuration file support
- [ ] Export functionality (CSV, JSON)
- [ ] Docker image
- [ ] Homebrew formula

## Reporting Issues

- Use the GitHub issue tracker
- Include Elasticsearch version and OS
- Provide steps to reproduce
- Include relevant error messages
