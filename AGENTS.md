# Repository Guidelines

## Language

Use English by default for:

- Documentation and Markdown files
- Code comments and docstrings
- API, CLI, configuration, schema, and field descriptions
- User-facing technical descriptions and examples

Use another language only when the task explicitly requests localization or the text is intentionally user-facing in that language. Keep identifiers and example names in English where practical.

## Before pushing

All three have to pass. CI runs the same three, so a failure here is a failure
there:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features
cargo test --locked --workspace --all-features
```

CI builds with `RUSTFLAGS=-D warnings`, so anything clippy has to say fails the
build: fix it rather than pushing past it. It also builds on the current stable
toolchain, so a lint that CI reports and the local build does not means the
local toolchain is behind — `rustup update stable` reproduces it.

`pedro-pdf`'s tests open real documents through pdfium and fail rather than
skip without it, so run `./scripts/fetch-pdfium.sh` once. The embedding model
is optional; no test needs it.
