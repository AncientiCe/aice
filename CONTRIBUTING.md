# Contributing

Thanks for contributing to `aice`.

## Development Setup

1. Follow local setup: [docs/setup/local-dev.md](docs/setup/local-dev.md)
2. Copy config template:

```bash
cp config.example.json config.json
```

3. Run required quality gates:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo audit --deny warnings
cargo test --all
```

## Pull Requests

- Keep changes focused and explain behavioral impact.
- Add or update tests for behavior changes (TDD expected in this repository).
- Update docs when user-facing behavior changes.
- Ensure CI passes before requesting review.

## Code Style and Quality

- `cargo fmt` formatting is required.
- Clippy warnings are treated as errors.
- Avoid dead code and unused variables.
- Do not add placeholder implementations (`todo!`, `unimplemented!`).

## Security and Secrets

- Never commit `config.json`, `.env*`, memory databases/files, or key material.
- Use example templates for configuration and keep local secrets out of git history.
