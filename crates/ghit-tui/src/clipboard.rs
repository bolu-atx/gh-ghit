//! Clipboard writer: OS tool via stdin, no new crates.
//!
//! Tries `pbcopy` (macOS), then Wayland/X11 tools. Returns true on the
//! first successful write so callers can keep the toast truthful.

use std::io::Write;
use std::process::{Command, Stdio};

fn pipe_to(tool: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let ok = child
        .stdin
        .as_mut()
        .map(|stdin| stdin.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    if !ok {
        return false;
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Copy `text` to the system clipboard. False when no tool worked.
pub fn copy(text: &str) -> bool {
    if pipe_to("pbcopy", &[], text) {
        return true;
    }
    if pipe_to("wl-copy", &[], text) {
        return true;
    }
    if pipe_to("xclip", &["-selection", "clipboard"], text) {
        return true;
    }
    pipe_to("xsel", &["--clipboard", "--input"], text)
}
