//! ghit visual theme: ayu mirage.
//!
//! Desaturated blue-ink background, warm accent, quiet gray-blue muted tone.
//! Selection is a marker plus bold type, never a full-width color block.
use ratatui::style::{Color, Style};
use ratatui::text::Text;
use tui_markdown::{Options, StyleSheet, from_str_with_options};

pub const INK: Color = Color::Rgb(0x1F, 0x24, 0x30);
pub const CODE_BG: Color = Color::Rgb(0x1C, 0x21, 0x2B);
pub const PAPER: Color = Color::Rgb(0xCC, 0xCA, 0xC2);
pub const AMBER: Color = Color::Rgb(0xFF, 0xCC, 0x66);
pub const TEAL: Color = Color::Rgb(0x95, 0xE6, 0xCB);
pub const MUTED: Color = Color::Rgb(0x6C, 0x7A, 0x8B);
pub const RED: Color = Color::Rgb(0xFF, 0x66, 0x66);
pub const GREEN: Color = Color::Rgb(0x87, 0xD9, 0x6C);
pub const VIOLET: Color = Color::Rgb(0xDF, 0xBF, 0xFF);

pub fn base() -> Style {
    Style::new().bg(INK).fg(PAPER)
}

pub fn state_color(state: &str) -> Color {
    match state {
        "OPEN" => GREEN,
        "CLOSED" => RED,
        "MERGED" => VIOLET,
        _ => MUTED,
    }
}

/// Amber-led markdown: headings glow, code sits on a warm block,
/// links read teal, quotes stay quiet.
#[derive(Debug, Clone, Copy, Default)]
pub struct GhitSheet;

impl StyleSheet for GhitSheet {
    fn heading(&self, level: u8) -> Style {
        let s = Style::new().fg(AMBER).bold();
        match level {
            1 => s.underlined(),
            2 => s,
            _ => Style::new().fg(AMBER).italic(),
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
        Style::new().fg(PAPER).bg(CODE_BG)
    }

    fn code_block_fence(&self) -> &str {
        "───"
    }

    fn link(&self) -> Style {
        Style::new().fg(TEAL).underlined()
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(MUTED).italic()
    }

    fn list_marker(&self) -> Style {
        Style::new().fg(AMBER).bold()
    }

    fn table_header(&self) -> Style {
        Style::new().fg(AMBER).bold()
    }

    fn table_border(&self) -> Style {
        Style::new().fg(MUTED)
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
        let sheet = GhitSheet;
        assert_eq!(sheet.heading(1).fg, Some(AMBER));
        assert!(sheet.heading(1).add_modifier.contains(Modifier::BOLD));
        assert_eq!(sheet.link().fg, Some(TEAL));
        assert_eq!(sheet.code().bg, Some(CODE_BG));
    }

    #[test]
    fn states_have_colors() {
        assert_eq!(state_color("OPEN"), GREEN);
        assert_eq!(state_color("CLOSED"), RED);
        assert_eq!(state_color("MERGED"), VIOLET);
        assert_eq!(state_color("WHATEVER"), MUTED);
    }

    #[test]
    fn themed_markdown_has_lines() {
        let text = render_markdown("# hi\n\n- a\n- b\n");
        assert!(text.height() > 3);
    }
}
