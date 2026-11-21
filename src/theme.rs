//! ghit themes: system (terminal ANSI), ayu mirage, ayu dark, GitHub light.
//!
//! One accent color per theme. Selection is a marker
//! plus bold type plus a subtle background shade, and only on the
//! focused pane; blurred panes go fully quiet. The active palette is
//! global so every widget and the markdown stylesheet stay in sync; the
//! Settings dialog switches it and the Nerd Font icon mode at runtime. The
//! choices persist in the data dir so restarts keep the last theme and
//! icon mode.
#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};
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
    /// Link colors by kind: issue refs and PR refs. URLs use `teal`.
    pub issue_ref: Color,
    pub pr_ref: Color,
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
    issue_ref: Color::Rgb(0x73, 0xD0, 0xFF),
    pr_ref: Color::Rgb(0xFF, 0xA7, 0x59),
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
    issue_ref: Color::Rgb(0x59, 0xC2, 0xFF),
    pr_ref: Color::Rgb(0xFF, 0x8F, 0x40),
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
    issue_ref: Color::Rgb(0x05, 0x50, 0xAE),
    pr_ref: Color::Rgb(0xBC, 0x4C, 0x00),
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
    issue_ref: Color::Blue,
    pr_ref: Color::LightMagenta,
};

pub const THEMES: &[(&str, Palette)] =
    &[("system", SYSTEM), ("mirage", MIRAGE), ("ayu dark", DARK), ("github light", LIGHT)];

// Global in the app; per thread in tests so a test that switches the
// theme never races color assertions in parallel tests.
#[cfg(not(test))]
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
thread_local!(static ACTIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

fn store_active(idx: usize) {
    #[cfg(not(test))]
    ACTIVE.store(idx, Ordering::Relaxed);
    #[cfg(test)]
    ACTIVE.with(|a| a.set(idx));
}

fn load_active() -> usize {
    #[cfg(not(test))]
    return ACTIVE.load(Ordering::Relaxed);
    #[cfg(test)]
    return ACTIVE.with(|a| a.get());
}

fn active_idx() -> usize {
    load_active() % THEMES.len()
}

/// Select a theme by name, persisting it. False when the name is unknown.
pub fn set(name: &str) -> bool {
    let Some(idx) = THEMES.iter().position(|(n, _)| *n == name) else { return false };
    store_active(idx);
    crate::cache::save_theme(name);
    true
}

/// Restore the persisted theme and icon mode. Runs once at startup.
pub fn init() {
    if let Some(saved) = crate::cache::load_theme() {
        set(&saved);
    }
    set_nerd(crate::cache::load_nerd_font());
}

// Nerd Font icons, on by default; Settings switches them. Tests keep the flag
// per thread so a test that turns icons off never races the snapshots.
#[cfg(not(test))]
static NERD: AtomicBool = AtomicBool::new(true);
#[cfg(test)]
thread_local!(static NERD: std::cell::Cell<bool> = const { std::cell::Cell::new(true) });

/// True when the UI draws Nerd Font glyphs.
pub fn nerd() -> bool {
    #[cfg(not(test))]
    return NERD.load(Ordering::Relaxed);
    #[cfg(test)]
    return NERD.with(|n| n.get());
}

pub fn set_nerd(on: bool) {
    #[cfg(not(test))]
    NERD.store(on, Ordering::Relaxed);
    #[cfg(test)]
    NERD.with(|n| n.set(on));
}

/// Switch Nerd Font icons and persist the choice.
pub fn choose_nerd(on: bool) {
    set_nerd(on);
    crate::cache::save_nerd_font(on);
}

/// `nerd` when Nerd Font icons are on, else the `plain` fallback.
/// Plain fallbacks keep to common Unicode, or are empty where a label
/// already says the same thing.
pub fn nf(nerd: &'static str, plain: &'static str) -> &'static str {
    if self::nerd() { nerd } else { plain }
}

/// Name of the active theme.
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
/// Foreground of non-RGB cells while the background dims.
pub fn dim_fg() -> Color {
    Color::Rgb(70, 76, 88)
}

pub fn paper() -> Color {
    pal().paper
}
pub fn amber() -> Color {
    pal().amber
}
/// Faint active-row tint: amber mixed 18% into the background, so the
/// row reads as selected while its own text colors stay legible.
pub fn line_hl() -> Color {
    match (pal().ink, pal().amber) {
        (Color::Rgb(ir, ig, ib), Color::Rgb(ar, ag, ab)) => {
            let mix = |i: u8, a: u8| (i as f32 + (a as f32 - i as f32) * 0.18) as u8;
            Color::Rgb(mix(ir, ar), mix(ig, ag), mix(ib, ab))
        }
        // System theme: the terminal's own background is unknown, so use
        // ANSI bright black, which stays faint on dark and light schemes.
        _ => Color::DarkGray,
    }
}
/// Background of a content-search match: amber mixed 40% into the
/// background, so the matched text stays legible.
pub fn search_match() -> Style {
    let bg = match (pal().ink, pal().amber) {
        (Color::Rgb(ir, ig, ib), Color::Rgb(ar, ag, ab)) => {
            let mix = |i: u8, a: u8| (i as f32 + (a as f32 - i as f32) * 0.4) as u8;
            Color::Rgb(mix(ir, ar), mix(ig, ag), mix(ib, ab))
        }
        _ => Color::DarkGray,
    };
    Style::new().bg(bg)
}

/// The current content-search match: solid amber with ink text.
pub fn search_current() -> Style {
    Style::new().bg(pal().amber).fg(pal().ink).bold()
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
pub fn issue_ref() -> Color {
    pal().issue_ref
}
pub fn pr_ref() -> Color {
    pal().pr_ref
}

pub fn base() -> Style {
    let p = pal();
    Style::new().bg(p.ink).fg(p.paper)
}

/// How one list row relates to the pane's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// The selected item of its pane.
    Active,
    /// Wheel or drag preview. Never reads as selection.
    Hover,
    Plain,
}

/// Block for a pane: a double amber frame while focused, a plain muted
/// one otherwise. Every pane frames itself through this one function.
pub fn pane_block(focused: bool) -> Block<'static> {
    let (kind, fg) = if focused { (BorderType::Double, amber()) } else { (BorderType::Plain, muted()) };
    Block::default()
        .borders(Borders::ALL)
        .border_type(kind)
        .border_style(Style::new().bg(ink()).fg(fg))
        .style(base())
}

/// Marker glyph and style for a row. The active row of the focused pane
/// is amber and bold; in an unfocused pane it goes muted. A hover row
/// gets its own glyph so it never matches a selection.
pub fn row_marker(state: RowState, focused: bool, active_glyph: &'static str) -> (&'static str, Style) {
    match state {
        RowState::Active if focused => (active_glyph, Style::new().fg(amber()).bold()),
        RowState::Active => (active_glyph, Style::new().fg(muted())),
        RowState::Hover => ("\u{203a} ", Style::new().fg(paper())),
        RowState::Plain => ("  ", Style::new().fg(muted())),
    }
}

/// Row background: only the active row of the focused pane is shaded.
pub fn row_bg(state: RowState, focused: bool) -> Color {
    if state == RowState::Active && focused { code_bg() } else { ink() }
}

/// List highlight style for the focused pane's active row.
pub fn row_highlight(focused: bool) -> Style {
    if focused { Style::new().bold().bg(code_bg()) } else { Style::new() }
}

pub fn state_color(state: &str) -> Color {
    match state {
        "OPEN" => green(),
        "CLOSED" => red(),
        "MERGED" => violet(),
        _ => muted(),
    }
}

/// Compact CI rollup color for a PR head commit: pass reads green, fail
/// reads red, pending reads amber. Unknown states stay quiet.
pub fn check_color(state: &crate::model::CheckState) -> Color {
    use crate::model::CheckState;
    match state {
        CheckState::Pass => green(),
        CheckState::Fail => red(),
        CheckState::Pending => amber(),
        CheckState::Unknown => muted(),
    }
}

/// Compact CI rollup glyph, in the same Nerd Font vocabulary as the
/// column icons (plain Unicode when icons are off). Empty for unknown states, which render nothing.
pub fn check_glyph(state: &crate::model::CheckState) -> &'static str {
    use crate::model::CheckState;
    match state {
        CheckState::Pass => nf("\u{f00c}", "✓"),
        CheckState::Fail => nf("\u{f00d}", "✗"),
        CheckState::Pending => nf("\u{f017}", "•"),
        CheckState::Unknown => "",
    }
}

/// Main pane toolbar glyphs, Font Awesome codepoints like the rest.
pub const TOOL_CLOSE: &str = "\u{f057}";
pub const TOOL_REOPEN: &str = "\u{f01e}";
pub const TOOL_COMMENT: &str = "\u{f075}";
pub const TOOL_SPLIT: &str = "\u{f0db}";
pub const TOOL_EXPAND: &str = "\u{f0b2}";
pub const TOOL_FOLD: &str = "\u{f066}";
pub const TOOL_SCOPE: &str = "\u{f0e8}";
pub const TOOL_FIT: &str = "\u{f065}";
pub const TOOL_CONTEXT: &str = "\u{f0c9}";
pub const TOOL_PHYSICS: &str = "\u{f085}";
pub const TOOL_RANK: &str = "\u{f0cb}";
pub const TOOL_FILTER: &str = "\u{f0b0}";

/// Review-comment row marker. `model::diff` finds comment rows in the
/// rendered text by this same string, so both modes must stay distinct.
pub fn comment_marker() -> &'static str {
    nf("\u{f075}", "¶")
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
        nf("\u{f05e}", "⊘")
    } else if n.contains("progress") {
        nf("\u{f017}", "◐")
    } else if n.contains("review") {
        nf("\u{f06e}", "◉")
    } else if n.contains("done") {
        nf("\u{f00c}", "✓")
    } else if n.contains("todo") {
        nf("\u{f10c}", "○")
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

    /// Point XDG at a temp dir so `set` persistence never touches
    /// real data. Returns the guard plus saved var to restore.
    fn temp_data() -> (std::sync::MutexGuard<'static, ()>, Option<String>, tempfile::TempDir) {
        let guard = crate::cache::test_env_lock();
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
        let _guard = crate::cache::test_env_lock();
        store_active(1);
        let sheet = GhitSheet;
        assert_eq!(sheet.heading(1).fg, Some(amber()));
        assert!(sheet.heading(1).add_modifier.contains(Modifier::BOLD));
        assert_eq!(sheet.link().fg, Some(teal()));
        assert_eq!(sheet.code().bg, Some(code_bg()));
        store_active(0);
    }

    #[test]
    fn columns_have_colors() {
        let _guard = crate::cache::test_env_lock();
        store_active(1);
        assert_eq!(column_color("Todo"), muted());
        assert_eq!(column_color("In Progress"), amber());
        assert_eq!(column_color("Blocked"), red());
        assert_eq!(column_color("In Review"), teal());
        assert_eq!(column_color("Done"), green());
        assert_eq!(column_color("Something Else"), muted());
        store_active(0);
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
    fn checks_have_colors_and_glyphs() {
        let _guard = crate::cache::test_env_lock();
        store_active(1);
        use crate::model::CheckState;
        assert_eq!(check_color(&CheckState::Pass), green());
        assert_eq!(check_color(&CheckState::Fail), red());
        assert_eq!(check_color(&CheckState::Pending), amber());
        assert_eq!(check_glyph(&CheckState::Pass), "\u{f00c}");
        assert_eq!(check_glyph(&CheckState::Fail), "\u{f00d}");
        assert_eq!(check_glyph(&CheckState::Pending), "\u{f017}");
        assert_eq!(check_glyph(&CheckState::Unknown), "");
        store_active(0);
    }

    #[test]
    fn plain_mode_uses_unicode_fallbacks() {
        use crate::model::CheckState;
        set_nerd(false);
        assert_eq!(nf("\u{f00c}", "x"), "x");
        assert_eq!(check_glyph(&CheckState::Pass), "✓");
        assert_eq!(check_glyph(&CheckState::Fail), "✗");
        assert_eq!(check_glyph(&CheckState::Pending), "•");
        assert_eq!(column_icon("Todo"), "○");
        assert_eq!(column_icon("Blocked"), "⊘");
        assert_eq!(column_icon("—"), "");
        assert_eq!(comment_marker(), "¶");
        set_nerd(true);
        assert_eq!(nf("\u{f00c}", "x"), "\u{f00c}");
    }

    #[test]
    fn states_have_colors() {
        let _guard = crate::cache::test_env_lock();
        store_active(1);
        assert_eq!(state_color("OPEN"), green());
        assert_eq!(state_color("CLOSED"), red());
        assert_eq!(state_color("MERGED"), violet());
        assert_eq!(state_color("WHATEVER"), muted());
        store_active(0);
    }

    #[test]
    fn set_selects_a_theme_by_name() {
        let (guard, saved, _dir) = temp_data();
        store_active(0);
        assert_eq!(name(), "system");
        for (theme, _) in THEMES {
            assert!(set(theme));
            assert_eq!(name(), *theme);
        }
        assert!(!set("nope"), "an unknown name changes nothing");
        assert_eq!(name(), "github light");
        store_active(0);
        restore(saved, guard);
    }

    #[test]
    fn system_theme_uses_terminal_colors() {
        let _guard = crate::cache::test_env_lock();
        store_active(0);
        assert_eq!(ink(), Color::Reset);
        assert_eq!(paper(), Color::Reset);
        assert_eq!(code_bg(), Color::Reset);
        assert_eq!(amber(), Color::Yellow);
        assert_eq!(teal(), Color::Cyan);
        assert_eq!(red(), Color::Red);
        assert_eq!(green(), Color::Green);
        assert_eq!(violet(), Color::Magenta);
        store_active(0);
    }

    #[test]
    fn set_persists_and_init_restores() {
        let (guard, saved, _dir) = temp_data();
        assert!(set("ayu dark"));
        assert_eq!(name(), "ayu dark");
        // Simulate a restart: reset in-memory, then init from disk.
        store_active(0);
        init();
        assert_eq!(name(), "ayu dark");
        assert!(!set("nope"));
        assert_eq!(name(), "ayu dark");
        store_active(0);
        restore(saved, guard);
    }

    #[test]
    fn themed_markdown_has_lines() {
        let _guard = crate::cache::test_env_lock();
        store_active(0);
        let text = render_markdown("# hi\n\n- a\n- b\n");
        assert!(text.height() > 3);
        store_active(0);
    }
}
