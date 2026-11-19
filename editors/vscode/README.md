# ghit for VS Code

Runs [ghit](https://github.com/bolu-atx/gh-ghit), a terminal UI for GitHub
issues and PRs, in a VS Code terminal.

Needs the [GitHub CLI](https://cli.github.com) (`gh`) to be installed and logged in.

## Use

- Run **ghit: Open**, click `ghit` in the status bar, or press `Ctrl+Alt+G` (`Cmd+Alt+G` on macOS).
- Run **ghit: Install / Update Binary** to download the latest release.

ghit is found in this order: `ghit.path`, your PATH, the downloaded copy.
If none exists, the extension offers to download it.

## Settings

- `ghit.location`: `panel` (default) or `editor` for an editor tab.
- `ghit.path`: absolute path to a ghit binary.
