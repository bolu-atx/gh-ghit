# ghit

Terminal viewer for GitHub issues and PRs, backed by the [`gh`](https://cli.github.com) CLI.
Punch in an issue number, read the ticket. `o` opens it in GitHub.

![ghit demo](demo.gif)

Rust port of the `ghit` TUI that lived in my dotfiles (`scripts/gh-issue-tui.py`).
Strictly the TUI — the old throwaway `gh-issue.bash` viewer did not make the trip.

## Why

- Agent swarms and orchestration produce plans faster than any human can read them.
- Triaging that volume in browser tabs takes a toll and does not scale.
- `ghit` keeps issues and plans in one terminal view, so a human can keep up with the agents.

## Requirements

- A GitHub token: `GH_TOKEN` (or `GITHUB_TOKEN`), or [`gh`](https://cli.github.com) authenticated (`gh auth login`)
- A terminal

## Install

As a standalone binary:

```bash
cargo install --git https://github.com/bolu-atx/gh-ghit
ghit 241
```

As a `gh` extension (runs `gh ghit`):

```bash
gh extension install bolu-atx/gh-ghit
gh ghit 241
```

The repo is named `gh-ghit` because `gh` requires extension repositories to carry
the `gh-` prefix. The crate and the binary are still called `ghit`.

## Usage

```bash
ghit [numbers...] [-R owner/repo]
gh ghit [numbers...] [-R owner/repo]
```

Without `-R`, the repo resolves from the current directory via `gh repo view`.
Without numbers, pinned issues load and the command bar takes focus.

Keybindings:

| Key | Does |
| --- | ---- |
| `1` / `2` | Body / Comments tab |
| `b` | Project board modal (`h`/`l` columns, `j`/`k` cards, `Enter` opens) |
| `j` / `k` | Scroll (with no issue loaded: move selection) |
| `:` or `/` | Focus the command bar |
| `o` | Open the current issue in GitHub |
| `r` | Refresh (busts the cache for this issue and board) |
| `p` | Pin to the top of the sidebar |
| `q` | Quit |

The command bar takes space-separated issue lists (`12 34 56`), `o [n]` to open,
and `q` to quit.

## Local state

- Pins: `~/.local/share/ghit/pinned-<owner>__<repo>.txt` (or `$XDG_DATA_HOME`)
- Read cache: `~/.local/share/ghit/cache/`, 30-minute TTL

## Differences from the Python version

- Bodies and comments render with real markdown (`tui-markdown`).
- Issue references are not clickable; type the number in the command bar instead.
- Fetching is synchronous with a status line, not background workers.

## License

MIT. See [LICENSE](LICENSE).
