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
}

impl Args {
    /// Numeric startup arguments; non-numeric input is ignored.
    pub fn issue_numbers(&self) -> Vec<u64> {
        self.numbers.iter().filter_map(|s| s.parse().ok()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_numeric_numbers_are_ignored() {
        let args = Args { numbers: vec!["12".into(), "abc".into(), "34".into()], repo: None };
        assert_eq!(args.issue_numbers(), vec![12, 34]);
    }
}
