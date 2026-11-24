//! jtui — terminal viewer for Jira issues and GitHub PRs.
//!
//! Usage: `jtui [numbers...] [-R owner/repo]`

mod auth;
mod cli;

use std::sync::Arc;

use clap::Parser;
use ghit_core::backend::IssueBackend;
use ghit_core::jira::JiraClient;
use ghit_tui::{app, theme};
#[cfg(unix)]
use ghit_tui::drive;

fn main() {
    ghit_core::set_app_name("jtui");
    let args = cli::Args::parse();
    let repo = match ghit_core::github::resolve_repo(args.repo.as_deref()) {
        Some(repo) => repo,
        None => {
            eprintln!("jtui: repository unknown: pass -R owner/repo or run inside a checkout");
            std::process::exit(1);
        }
    };
    if let Some(cli::Command::Auth { action }) = &args.command {
        if let Err(error) = auth::run(action, &repo) {
            eprintln!("jtui: {error}");
            std::process::exit(1);
        }
        return;
    }
    let client = match JiraClient::for_repo(&repo, Some(&auth::OsKeychain)) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("jtui: {}", error.message());
            std::process::exit(1);
        }
    };
    ghit_core::model::set_jira_key(client.project());
    if let Err(error) = ghit_core::runtime::rt().block_on(client.check_startup()) {
        eprintln!("jtui: {}", error.message());
        std::process::exit(1);
    }
    theme::init();
    let numbers = args.issue_numbers();
    let backend = IssueBackend::Jira(Arc::new(client));
    let mut app = app::App::with_backend(args.repo.clone(), backend);
    if let Some(sock) = args.drive.clone() {
        #[cfg(unix)]
        {
            let (width, height) = args.size_parsed().unwrap_or_else(|e| {
                eprintln!("jtui: {e}");
                std::process::exit(1);
            });
            if let Err(e) = drive::run_drive(&mut app, &numbers, &sock, width, height) {
                eprintln!("jtui: {e}");
                std::process::exit(1);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = &sock;
            eprintln!("jtui: --drive needs a Unix socket and is unavailable on this platform");
            std::process::exit(1);
        }
    } else if let Err(e) = app::run(&mut app, &numbers) {
        eprintln!("jtui: {e}");
        std::process::exit(1);
    }
}
