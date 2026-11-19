# Renders demo.gif for the README.
# Run from the repo root:
#   cargo build --release && vhs scripts/demo.tape && gifsicle -O3 --lossy=60 -b demo.gif
Output demo.gif

Require gh

Set Shell zsh
Set FontFamily "GeistMono Nerd Font"
Set FontSize 14
Set Width 1400
Set Height 800
Set Padding 12
Set Framerate 20
Set TypingSpeed 80ms
Set Theme "Catppuccin Mocha"

# Launch off-camera on a PR, so the demo opens on a loaded screen.
# Start in ayu dark; the saved theme is restored on exit.
Hide
Type "T=~/.local/share/ghit/theme; cp $T $T.bak 2>/dev/null; echo 'ayu dark' > $T"
Enter
Type "clear && ./target/release/ghit 252 -R bolu-atx/gh-ghit"
Enter
Sleep 5s
Show

# PR body, then scroll it.
Sleep 2s
Tab
Tab
Type@300ms "jjjjj"
Sleep 1s

# Visual tab: the PR diff.
Type "2"
Sleep 2s
Type@250ms "jjjjjjjj"
Sleep 1s
Type "s"
Sleep 2s
Type "s"
Sleep 500ms
Type "1"
Sleep 1s

# Second half in github light.
Type "t"
Sleep 2s

# Sidebar: step through issues and PRs.
Tab
Type@400ms "jjj"
Enter
Sleep 2s

# Fuzzy finder over the local store.
Ctrl+P
Sleep 500ms
Type "store"
Sleep 1500ms
Down
Sleep 500ms
Enter
Sleep 2s

# Project board.
Type "b"
Sleep 3s
Type@400ms "lljj"
Sleep 1500ms
Escape
Sleep 1s

# Shortcuts help.
Type "?"
Sleep 2500ms
Escape
Sleep 1s

Hide
Type "q"
Sleep 500ms
Type "mv $T.bak $T 2>/dev/null || rm -f $T"
Enter
Sleep 500ms
