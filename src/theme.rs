//! ghit themes: ayu mirage, ayu dark, and GitHub light.
//!
//! One accent color per theme. Selection is a marker
//! plus bold type, never a full-width color block. The active palette is
//! global so every widget and the markdown stylesheet stay in sync; `t`
//! cycles it at runtime.
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

pub const THEMES: &[(&str, Palette)] = &[("mirage", MIRAGE), ("ayu dark", DARK), ("github light", LIGHT)];

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn active_idx() -> usize {
    ACTIVE.load(Ordering::Relaxed) % THEMES.len()
}

/// Switch to the next theme, returning its name for the status line.
pub fn cycle() -> &'static str {
    let next = (active_idx() + 1) % THEMES.len();
    ACTIVE.store(next, Ordering::Relaxed);
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

    #[test]
    fn headings_glow_amber() {
        ACTIVE.store(0, Ordering::Relaxed);
        let sheet = GhitSheet;
        assert_eq!(sheet.heading(1).fg, Some(amber()));
        assert!(sheet.heading(1).add_modifier.contains(Modifier::BOLD));
        assert_eq!(sheet.link().fg, Some(teal()));
        assert_eq!(sheet.code().bg, Some(code_bg()));
    }

    #[test]
    fn states_have_colors() {
        ACTIVE.store(0, Ordering::Relaxed);
        assert_eq!(state_color("OPEN"), green());
        assert_eq!(state_color("CLOSED"), red());
        assert_eq!(state_color("MERGED"), violet());
        assert_eq!(state_color("WHATEVER"), muted());
    }

    #[test]
    fn toggle_cycles_all_themes() {
        ACTIVE.store(0, Ordering::Relaxed);
        assert_eq!(name(), "mirage");
        assert_eq!(cycle(), "ayu dark");
        assert_eq!(cycle(), "github light");
        assert_eq!(name(), "github light");
        assert_eq!(cycle(), "mirage");
        ACTIVE.store(0, Ordering::Relaxed);
    }

    #[test]
    fn themed_markdown_has_lines() {
        let text = render_markdown("# hi\n\n- a\n- b\n");
        assert!(text.height() > 3);
    }
}
