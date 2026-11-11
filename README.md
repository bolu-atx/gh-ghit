# ghit

Terminal viewer for GitHub issues and PRs, backed by the [`gh`](https://cli.github.com) CLI.
Punch in an issue number, read the ticket. `o` opens it in GitHub.

![ghit demo](demo.gif)

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

Keybindings (full list in the app: `?`):

| Key | Does |
| --- | ---- |
| `1` / `2` | Details / Visual tab (Visual shows the diff for PRs, the graph for issues) |
| `?` | Shortcuts modal |
| `b` | Project board modal (`h`/`l` columns, `j`/`k` cards, `Enter` opens) |
| `j` / `k`, arrows, PgUp/PgDn | Select and scroll |
| `Ctrl+O` / `Ctrl+I` | Back / forward in issue history |
| `Tab` | Cycle focus between panes |
| `:` or `/` | Command bar (`/` starts a find; `n` / `N` jumps) |
| `f` | Filter (merged window, open/closed toggles, query) |
| `c` | Comment / reply (type, `Ctrl+S` sends, `Esc` discards) |
| `o` | Open the current issue in GitHub |
| `r` / `p` / `y` | Refresh / pin / yank to clipboard |
| `e` | Toggle the refs panel |
| `a` / `A` | Sort the sidebar / reverse direction |
| `x` | Evict the row from the sidebar |
| `Space` | Expand the row, or fold the file in a diff |
| `s` | Diff: toggle split view |
| `t` / `w` | Cycle theme / merged-PR window |
| `q` | Quit |

The command bar takes space-separated issue lists (`12 34 56`), `o [n]` to open,
and `q` to quit.

## Local state

- Pins: `~/.local/share/ghit/pinned-<owner>__<repo>.txt` (or `$XDG_DATA_HOME`)
- Read cache: `~/.local/share/ghit/cache/`, 30-minute TTL

## License

MIT. See [LICENSE](LICENSE).
