# CLAUDE.md — ghit

Rust TUI for browsing GitHub issues/PRs via the `gh` CLI.
Crate and binary: `ghit`. Public repo: `bolu-atx/gh-ghit`.

Start with [AGENTS.md](AGENTS.md) for commands, architecture, and gates.
Style rules live in [docs/code-style.md](docs/code-style.md);
task routing and the live-verification procedure in [docs/routers.md](docs/routers.md).

Standing rules: `cargo build` after every change (it refreshes the debug
target). `cargo test` green with zero warnings before every push.
Never regress the frame-snapshot tests in `render_smoke`.
