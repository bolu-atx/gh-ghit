//! CLI surface: argument parsing only. No state, no fetching, no widgets.

use clap::Parser;

/// Terminal viewer for GitHub issues and PRs.
#[derive(Parser, Debug)]
#[command(name = "ghit", version)]
pub struct Args {
    /// Issue or PR numbers to open on startup.
    #[arg(value_name = "NUMBER")]
    pub numbers: Vec<String>,

    /// Repository in owner/repo form (default: `gh repo view` resolution).
    #[arg(short = 'R', long = "repo", value_name = "OWNER/REPO")]
    pub repo: Option<String>,

    /// Headless drive mode: serve the UI over this Unix socket path
    /// instead of taking over the terminal. No TTY required.
    #[arg(long = "drive", value_name = "PATH")]
    pub drive: Option<std::path::PathBuf>,

    /// Drive-mode geometry in WxH cells. Only used with `--drive`.
    #[arg(long = "size", value_name = "WxH", default_value = "200x50")]
    pub size: String,
}

impl Args {
    /// Numeric startup arguments; non-numeric input is ignored.
    pub fn issue_numbers(&self) -> Vec<u64> {
        self.numbers.iter().filter_map(|s| s.parse().ok()).collect()
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
        let args = Args { numbers: vec!["12".into(), "abc".into(), "34".into()], repo: None, drive: None, size: "200x50".into() };
        assert_eq!(args.issue_numbers(), vec![12, 34]);
    }

    fn args_with_size(size: &str) -> Args {
        Args { numbers: vec![], repo: None, drive: None, size: size.into() }
    }

    #[test]
    fn size_defaults_to_200x50() {
        assert_eq!(args_with_size("200x50").size_parsed(), Ok((200, 50)));
    }

    #[test]
    fn size_parses_custom_geometry() {
        assert_eq!(args_with_size("120x40").size_parsed(), Ok((120, 40)));
    }

    #[test]
    fn size_rejects_garbage() {
        for bad in ["", "200", "200x", "x50", "0x50", "200x0", "axb", "200*50"] {
            assert!(args_with_size(bad).size_parsed().is_err(), "size {bad:?}");
        }
    }
}
