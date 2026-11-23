//! The settings dialog's actions: app-wide prefs that apply and save at
//! once. Rows: theme, icons, store backfill age limit, and for Jira an
//! "open config file" action. The Jira connection rows are read-only and
//! have no index, so focus never lands on them.

use std::io::Stdout;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{App, Entry};
use crate::theme;

pub(crate) const ROWS: usize = 3;
pub(crate) const CONFIG_ROW: usize = 3;
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

    /// Rows that take focus: the prefs, plus the config action under Jira.
    pub(crate) fn settings_rows(&self) -> usize {
        ROWS + usize::from(self.session.jira_info().is_some())
    }

    /// Move the focus `delta` rows, wrapping at the ends.
    pub(crate) fn settings_step(&mut self, delta: isize) {
        let Some(focus) = self.settings_focus() else { return };
        let rows = self.settings_rows() as isize;
        self.set_settings_focus((focus as isize + delta).rem_euclid(rows) as usize);
    }

    /// Arm the config file edit. The loop owns the terminal, so it runs
    /// the editor. GitHub has no config file, so it arms nothing.
    pub(crate) fn request_config_edit(&mut self) {
        if self.session.jira_info().is_some() {
            self.config_edit_requested = true;
        }
    }

    /// The editor program and its arguments: `$VISUAL`, else `$EDITOR`, else `vi`.
    pub(crate) fn config_editor() -> Vec<String> {
        let var = |name| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let editor = var("VISUAL").or_else(|| var("EDITOR")).unwrap_or_else(|| "vi".to_string());
        editor.split_whitespace().map(str::to_string).collect()
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
            Some(CONFIG_ROW) if delta > 0 => self.request_config_edit(),
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

/// Suspend the TUI, open the jtui config file in the editor, and resume.
/// The loop owns the terminal, so input only arms `config_edit_requested`.
pub(crate) fn run_config_edit(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use crossterm::ExecutableCommand;
    app.config_edit_requested = false;
    let Some(info) = app.session.jira_info() else { return };
    let mut words = App::config_editor().into_iter();
    let program = words.next().unwrap_or_else(|| "vi".to_string());
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let status = std::process::Command::new(&program).args(words).arg(&info.config_path).status();
    let _ = enable_raw_mode();
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = terminal.clear();
    match status {
        Ok(_) => app.notify("config saved: restart jtui to apply the changes".to_string()),
        Err(e) => app.notify(format!("{program} failed to start ({e})")),
    }
}

/// Drive-mode landing for an armed config edit: no terminal exists to
/// suspend, so it names the command instead. True when it fired.
pub(crate) fn land_config_edit_drive(app: &mut App) -> bool {
    if !app.config_edit_requested {
        return false;
    }
    app.config_edit_requested = false;
    if let Some(info) = app.session.jira_info() {
        let editor = App::config_editor().join(" ");
        app.notify(format!("edit by hand: {editor} {}", info.config_path.display()));
    }
    true
}
