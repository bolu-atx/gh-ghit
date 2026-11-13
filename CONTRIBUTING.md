# Contributing

Thanks for helping. Bug reports, feature requests and fixes are all
welcome. This page is for people and for the coding agents they use.
If you are an agent, also read [AGENTS.md](AGENTS.md).

## Issue first

1. Search open and closed issues first.
2. File a bug or feature request with the issue form. Blank issues
   are off. New issues get the `needs-triage` label.
3. Wait for the maintainer to add `accepted` (or `help wanted`).
   That label means a PR is welcome.

A PR that does not close an `accepted` or `help wanted` issue is
closed automatically. This keeps review time for agreed work.
Small typo and doc fixes can still go through an issue. Keep that
issue to one line.

## Pull requests

- One issue per PR. Start the description with `Closes #N`.
- Branch: `feat/…`, `fix/…`, `docs/…`, `refactor/…` or `chore/…`.
  Title: `feat: …`, `fix: …` and so on. The title states the
  outcome for the user.
- Keep the diff small and on topic. Do not reformat, rename or
  refactor code that the issue does not need.
- Before you push, run both gates. CI runs the same commands:

  ```bash
  cargo clippy --all-targets -- -D warnings
  cargo test
  ```

- UI changes need a frame test in `render_smoke` (see `src/ui.rs`).
  Do not change existing frame snapshots unless the issue asks for it.
- Fill in the PR template. Paste real command output under
  Verification. Do not claim checks you did not run.

These areas need a maintainer. Do not change them in a contributor
PR unless the issue says so:

- `.github/` (workflows, templates, CODEOWNERS)
- `scripts/release.sh`, `rust-toolchain.toml`
- new dependencies in `Cargo.toml`
- `AGENTS.md`, `CLAUDE.md`, `docs/maintaining.md`

## AI-assisted work

AI-assisted contributions are welcome. You are responsible for every
line, as if you wrote it yourself.

- Say in the PR which tools you used and what you checked by hand.
- Read the full diff before you open the PR. Remove generated noise.
- Answer review comments yourself. Do not paste them back into an
  agent and push the output unread.
- A PR that is clearly unreviewed agent output is closed.

## CI on your PR

CI for outside contributors runs after a maintainer approves the run.
This is a GitHub setting. It is not a judgement of your PR.

## Security

Do not report vulnerabilities in public issues. See
[SECURITY.md](SECURITY.md).

## License

By contributing you agree that your work is licensed under the
project's [MIT license](LICENSE).

## Conduct

Be kind and specific. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
