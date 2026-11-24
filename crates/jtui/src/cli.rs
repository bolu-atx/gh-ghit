//! CLI surface: argument parsing only. No state, no fetching, no widgets.

use clap::{Parser, Subcommand};
use ghit_core::model::ItemRef;

/// Terminal viewer for Jira issues and GitHub PRs.
#[derive(Parser, Debug)]
#[command(name = "jtui", version)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Issue or PR numbers to open on startup.
    #[arg(value_name = "NUMBER")]
    pub numbers: Vec<String>,

    /// Repository in owner/repo form (default: `gh repo view` resolution).
    #[arg(short = 'R', long = "repo", value_name = "OWNER/REPO", global = true)]
    pub repo: Option<String>,

    /// Headless drive mode: serve the UI over this Unix socket path
    /// instead of taking over the terminal. No TTY required.
    #[arg(long = "drive", value_name = "PATH")]
    pub drive: Option<std::path::PathBuf>,

    /// Drive-mode geometry in WxH cells. Only used with `--drive`.
    #[arg(long = "size", value_name = "WxH", default_value = "200x50")]
    pub size: String,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage the Jira API token.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthAction {
    /// Validate and store a token in the OS keychain.
    Login,
    /// Show the token source and check the account.
    Status,
    /// Remove the stored keychain token.
    Logout,
}

impl Args {
    /// Numeric startup arguments; non-numeric input is ignored.
    pub fn issue_numbers(&self) -> Vec<ItemRef> {
        self.numbers
            .iter()
            .filter_map(|s| s.parse().ok().map(ItemRef::github))
            .collect()
    }

    /// Drive-mode geometry as (width, height) cells.
    pub fn size_parsed(&self) -> Result<(u16, u16), String> {
        let err = || format!("bad --size {:?}: want WxH like 200x50", self.size);
        let (w, h) = self.size.split_once(['x', 'X']).ok_or_else(&err)?;
        let parse = |s: &str| s.parse::<u16>().map_err(|_| err());
        let (w, h) = (parse(w)?, parse(h)?);
        if w == 0 || h == 0 {
            return Err(err());
        }
        Ok((w, h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_numeric_numbers_are_ignored() {
        let args = Args {
            command: None, numbers: vec!["12".into(), "abc".into(), "34".into()],
            repo: None, drive: None, size: "200x50".into(),
        };
        assert_eq!(args.issue_numbers(), vec![ItemRef::github(12), ItemRef::github(34)]);
    }

    #[test]
    fn auth_subcommands_accept_repo_before_or_after_action() {
        for action in ["login", "status", "logout"] {
            for argv in [
                vec!["jtui", "auth", action, "-R", "owner/repo"],
                vec!["jtui", "-R", "owner/repo", "auth", action],
            ] {
                let args = Args::try_parse_from(argv).unwrap();
                assert!(matches!(args.command, Some(Command::Auth { .. })));
                assert_eq!(args.repo.as_deref(), Some("owner/repo"));
            }
        }
        let args = Args::try_parse_from(["jtui", "123", "456", "-R", "owner/repo"]).unwrap();
        assert!(args.command.is_none());
        assert_eq!(args.issue_numbers(), vec![ItemRef::github(123), ItemRef::github(456)]);
    }

    #[test]
    fn size_parses_and_rejects_garbage() {
        let args = |size: &str| Args {
            command: None, numbers: vec![], repo: None, drive: None, size: size.into(),
        };
        assert_eq!(args("120x40").size_parsed(), Ok((120, 40)));
        assert!(args("200").size_parsed().is_err());
        assert!(args("0x50").size_parsed().is_err());
    }
}
