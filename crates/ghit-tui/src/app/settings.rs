//! The settings dialog's actions: app-wide prefs that apply and save at
//! once. Rows: theme, icons, store backfill age limit.

use crate::app::{App, Entry};
use crate::theme;

pub(crate) const ROWS: usize = 3;
pub(crate) const THEME_ROW: usize = 0;
pub(crate) const ICONS_ROW: usize = 1;
pub(crate) const BACKFILL_ROW: usize = 2;

/// Backfill age limits the stepper walks, in months. 0 is all history.
const MONTHS: [u32; 6] = [1, 3, 6, 12, 24, 0];

/// Order of an age limit: 0 (all history) is the largest.
fn span(months: u32) -> u32 {
    if months == 0 { u32::MAX } else { months }
}

/// Label of an age limit for the stepper.
pub(crate) fn months_label(months: u32) -> String {
    match months {
        0 => "all".to_string(),
        1 => "1 month".to_string(),
        n => format!("{n} months"),
    }
}

impl App {
    pub(crate) fn open_settings(&mut self) {
        self.open(Entry::Settings(0));
    }

    /// Move the focus `delta` rows, wrapping at the ends.
    pub(crate) fn settings_step(&mut self, delta: isize) {
        let Some(focus) = self.settings_focus() else { return };
        self.set_settings_focus((focus as isize + delta).rem_euclid(ROWS as isize) as usize);
    }

    /// Move the focused row's value one step: the previous or next
    /// option of a choice row (wrapping), a lower or higher age limit.
    pub(crate) fn settings_nudge(&mut self, delta: isize) {
        match self.settings_focus() {
            Some(THEME_ROW) => {
                let at = theme::THEMES.iter().position(|(name, _)| *name == theme::name()).unwrap_or(0);
                let next = (at as isize + delta).rem_euclid(theme::THEMES.len() as isize) as usize;
                self.settings_select(THEME_ROW, next);
            }
            Some(ICONS_ROW) => self.settings_select(ICONS_ROW, usize::from(theme::nerd())),
            Some(BACKFILL_ROW) => self.settings_select(BACKFILL_ROW, if delta < 0 { 0 } else { 2 }),
            _ => {}
        }
    }

    /// Pick option `option` of `row`, as a click does. The stepper's
    /// options are 0 lower and 2 raise.
    pub(crate) fn settings_select(&mut self, row: usize, option: usize) {
        match row {
            THEME_ROW => {
                if let Some((name, _)) = theme::THEMES.get(option) {
                    theme::set(name);
                }
            }
            ICONS_ROW => theme::choose_nerd(option == 0),
            BACKFILL_ROW => {
                let now = self.session.backfill_months();
                let next = match option {
                    0 => MONTHS.iter().rev().find(|m| span(**m) < span(now)),
                    2 => MONTHS.iter().find(|m| span(**m) > span(now)),
                    _ => None,
                };
                if let Some(months) = next {
                    self.session.set_backfill_months(*months);
                    self.persist_prefs();
                }
            }
            _ => {}
        }
    }
}
