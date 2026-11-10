//! ghit themes: system (terminal ANSI), ayu mirage, ayu dark, GitHub light.
//!
//! One accent color per theme. Selection is a marker
//! plus bold type plus a subtle background shade, and only on the
//! focused pane; blurred panes go fully quiet. The active palette is
//! global so every widget and the markdown stylesheet stay in sync; `t`
//! cycles it at runtime. The choice persists in the data dir so restarts
//! keep the last theme.
use std::sync::atomic::{AtomicUsize, Ordering};

use ratatui::style::{Color, Style};
use ratatui::text::Text;
use tui_markdown::{Options, StyleSheet, from_str_with_options};

#[derive(Clone, Copy)]
pub struct Palette {
    pub ink: Color,
    pub code_bg: Color,
    pub paper: Color,
    pub amber: Color,
    pub teal: Color,
    pub muted: Color,
    pub red: Color,
    pub green: Color,
    pub violet: Color,
}

pub const MIRAGE: Palette = Palette {
    ink: Color::Rgb(0x1F, 0x24, 0x30),
    code_bg: Color::Rgb(0x1C, 0x21, 0x2B),
    paper: Color::Rgb(0xCC, 0xCA, 0xC2),
    amber: Color::Rgb(0xFF, 0xCC, 0x66),
    teal: Color::Rgb(0x95, 0xE6, 0xCB),
    muted: Color::Rgb(0x6C, 0x7A, 0x8B),
    red: Color::Rgb(0xFF, 0x66, 0x66),
    green: Color::Rgb(0x87, 0xD9, 0x6C),
    violet: Color::Rgb(0xDF, 0xBF, 0xFF),
};

pub const DARK: Palette = Palette {
    ink: Color::Rgb(0x0B, 0x0E, 0x14),
    code_bg: Color::Rgb(0x0F, 0x13, 0x1A),
    paper: Color::Rgb(0xBF, 0xBD, 0xB6),
    amber: Color::Rgb(0xE6, 0xB4, 0x50),
    teal: Color::Rgb(0x95, 0xE6, 0xCB),
    muted: Color::Rgb(0x63, 0x6A, 0x72),
    red: Color::Rgb(0xD9, 0x57, 0x57),
    green: Color::Rgb(0x7F, 0xD9, 0x62),
    violet: Color::Rgb(0xD2, 0xA6, 0xFF),
};

pub const LIGHT: Palette = Palette {
    ink: Color::Rgb(0xFF, 0xFF, 0xFF),
    code_bg: Color::Rgb(0xF6, 0xF8, 0xFA),
    paper: Color::Rgb(0x1F, 0x23, 0x28),
    amber: Color::Rgb(0x09, 0x69, 0xDA),
    teal: Color::Rgb(0x09, 0x69, 0xDA),
    muted: Color::Rgb(0x59, 0x63, 0x6E),
    red: Color::Rgb(0xD1, 0x24, 0x2C),
    green: Color::Rgb(0x1A, 0x7F, 0x37),
    violet: Color::Rgb(0x82, 0x50, 0xDF),
};

/// System theme: no hardcoded colors. Backgrounds and body text stay on
/// the terminal default (`Reset`); accents use the six ANSI colors so any
/// terminal scheme (dark or light) keeps working.
pub const SYSTEM: Palette = Palette {
    ink: Color::Reset,
    code_bg: Color::Reset,
    paper: Color::Reset,
    amber: Color::Yellow,
    teal: Color::Cyan,
    muted: Color::DarkGray,
    red: Color::Red,
    green: Color::Green,
    violet: Color::Magenta,
};

pub const THEMES: &[(&str, Palette)] =
    &[("system", SYSTEM), ("mirage", MIRAGE), ("ayu dark", DARK), ("github light", LIGHT)];

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn active_idx() -> usize {
    ACTIVE.load(Ordering::Relaxed) % THEMES.len()
}

/// Select a theme by name, persisting it. False when the name is unknown.
pub fn set(name: &str) -> bool {
    let Some(idx) = THEMES.iter().position(|(n, _)| *n == name) else { return false };
    ACTIVE.store(idx, Ordering::Relaxed);
    crate::cache::save_theme(name);
    true
}

/// Restore the persisted theme, if any. Runs once at startup.
pub fn init() {
    if let Some(saved) = crate::cache::load_theme() {
        set(&saved);
    }
}

/// Switch to the next theme, persisting it; returns its name for the status line.
pub fn cycle() -> &'static str {
    let next = (active_idx() + 1) % THEMES.len();
    ACTIVE.store(next, Ordering::Relaxed);
    crate::cache::save_theme(THEMES[next].0);
    THEMES[next].0
}

pub fn name() -> &'static str {
    THEMES[active_idx()].0
}

fn pal() -> Palette {
    THEMES[active_idx()].1
}

pub fn ink() -> Color {
    pal().ink
}
pub fn code_bg() -> Color {
    pal().code_bg
}
pub fn paper() -> Color {
    pal().paper
}
pub fn amber() -> Color {
    pal().amber
}
pub fn teal() -> Color {
    pal().teal
}
pub fn muted() -> Color {
    pal().muted
}
pub fn red() -> Color {
    pal().red
}
pub fn green() -> Color {
    pal().green
}
pub fn violet() -> Color {
    pal().violet
}

pub fn base() -> Style {
    let p = pal();
    Style::new().bg(p.ink).fg(p.paper)
}

pub fn state_color(state: &str) -> Color {
    match state {
        "OPEN" => green(),
        "CLOSED" => red(),
        "MERGED" => violet(),
        _ => muted(),
    }
}

/// Swimlane header color, matched by keyword so custom board
/// vocabularies ("In progress", "In Progress", …) still resolve.
pub fn column_color(name: &str) -> Color {
    let n = name.to_lowercase();
    if n.contains("block") {
        red()
    } else if n.contains("progress") {
        amber()
    } else if n.contains("review") {
        teal()
    } else if n.contains("done") {
        green()
    } else {
        muted()
    }
}

/// Swimlane glyph, matched by the same keywords in the same order as
/// `column_color` so icon and color never disagree. Empty for unknown
/// lanes, which stay quiet. Nerd Font codepoints are long-stable
/// Font Awesome glyphs, like the rest of the UI vocabulary.
pub fn column_icon(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("block") {
        "\u{f05e}"
    } else if n.contains("progress") {
        "\u{f017}"
    } else if n.contains("review") {
        "\u{f06e}"
    } else if n.contains("done") {
        "\u{f00c}"
    } else if n.contains("todo") {
        "\u{f10c}"
    } else {
        ""
    }
}

/// Amber-led markdown: headings glow, code sits on a warm block,
/// links read teal, quotes stay quiet.
#[derive(Debug, Clone, Copy, Default)]
pub struct GhitSheet;

impl StyleSheet for GhitSheet {
    fn heading(&self, level: u8) -> Style {
        let s = Style::new().fg(amber()).bold();
        match level {
            1 => s.underlined(),
            2 => s,
            _ => Style::new().fg(amber()).italic(),
        }
    }

    fn heading_marker(&self, level: u8) -> &str {
        match level {
            1 => "■",
            2 => "▪",
            _ => "–",
        }
    }

    fn code(&self) -> Style {
        Style::new().fg(paper()).bg(code_bg())
    }

    fn code_block_fence(&self) -> &str {
        "───"
    }

    fn link(&self) -> Style {
        Style::new().fg(teal()).underlined()
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(muted()).italic()
    }

    fn list_marker(&self) -> Style {
        Style::new().fg(amber()).bold()
    }

    fn table_header(&self) -> Style {
        Style::new().fg(amber()).bold()
    }

    fn table_border(&self) -> Style {
        Style::new().fg(muted())
    }
}

pub fn render_markdown(md: &str) -> Text<'_> {
    let options = Options::new(GhitSheet);
    from_str_with_options(md, &options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    /// Point XDG at a temp dir so `set`/`cycle` persistence never touches
    /// real data. Returns the guard plus saved var to restore.
    fn temp_data() -> (std::sync::MutexGuard<'static, ()>, Option<String>, tempfile::TempDir) {
        let guard = crate::cache::test_env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        (guard, saved, dir)
    }

    fn restore(saved: Option<String>, _guard: std::sync::MutexGuard<'static, ()>) {
        match saved {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn headings_glow_amber() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        ACTIVE.store(1, Ordering::Relaxed);
        let sheet = GhitSheet;
        assert_eq!(sheet.heading(1).fg, Some(amber()));
        assert!(sheet.heading(1).add_modifier.contains(Modifier::BOLD));
        assert_eq!(sheet.link().fg, Some(teal()));
        assert_eq!(sheet.code().bg, Some(code_bg()));
        ACTIVE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn columns_have_colors() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        ACTIVE.store(1, Ordering::Relaxed);
        assert_eq!(column_color("Todo"), muted());
        assert_eq!(column_color("In Progress"), amber());
        assert_eq!(column_color("Blocked"), red());
        assert_eq!(column_color("In Review"), teal());
        assert_eq!(column_color("Done"), green());
        assert_eq!(column_color("Something Else"), muted());
        ACTIVE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn columns_have_icons_matching_colors() {
        assert_eq!(column_icon("Todo"), "\u{f10c}");
        assert_eq!(column_icon("In progress"), "\u{f017}");
        assert_eq!(column_icon("In Progress"), "\u{f017}");
        assert_eq!(column_icon("Blocked"), "\u{f05e}");
        assert_eq!(column_icon("In Review"), "\u{f06e}");
        assert_eq!(column_icon("Done"), "\u{f00c}");
        // Same keyword precedence as the colors: progress wins the tie.
        assert_eq!(column_icon("Todo · In Progress"), "\u{f017}");
        assert_eq!(column_icon("Something Else"), "");
        assert_eq!(column_icon("—"), "");
    }

    #[test]
    fn states_have_colors() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        ACTIVE.store(1, Ordering::Relaxed);
        assert_eq!(state_color("OPEN"), green());
        assert_eq!(state_color("CLOSED"), red());
        assert_eq!(state_color("MERGED"), violet());
        assert_eq!(state_color("WHATEVER"), muted());
        ACTIVE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn toggle_cycles_all_themes() {
        let (guard, saved, _dir) = temp_data();
        ACTIVE.store(0, Ordering::Relaxed);
        assert_eq!(name(), "system");
        assert_eq!(cycle(), "mirage");
        assert_eq!(cycle(), "ayu dark");
        assert_eq!(cycle(), "github light");
        assert_eq!(name(), "github light");
        assert_eq!(cycle(), "system");
        ACTIVE.store(0, Ordering::Relaxed);
        restore(saved, guard);
    }

    #[test]
    fn system_theme_uses_terminal_colors() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        ACTIVE.store(0, Ordering::Relaxed);
        assert_eq!(ink(), Color::Reset);
        assert_eq!(paper(), Color::Reset);
        assert_eq!(code_bg(), Color::Reset);
        assert_eq!(amber(), Color::Yellow);
        assert_eq!(teal(), Color::Cyan);
        assert_eq!(red(), Color::Red);
        assert_eq!(green(), Color::Green);
        assert_eq!(violet(), Color::Magenta);
        ACTIVE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn set_persists_and_init_restores() {
        let (guard, saved, _dir) = temp_data();
        assert!(set("ayu dark"));
        assert_eq!(name(), "ayu dark");
        // Simulate a restart: reset in-memory, then init from disk.
        ACTIVE.store(0, Ordering::Relaxed);
        init();
        assert_eq!(name(), "ayu dark");
        assert!(!set("nope"));
        assert_eq!(name(), "ayu dark");
        ACTIVE.store(0, Ordering::Relaxed);
        restore(saved, guard);
    }

    #[test]
    fn themed_markdown_has_lines() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        ACTIVE.store(0, Ordering::Relaxed);
        let text = render_markdown("# hi\n\n- a\n- b\n");
        assert!(text.height() > 3);
        ACTIVE.store(0, Ordering::Relaxed);
    }
}
