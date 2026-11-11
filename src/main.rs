//! ghit — terminal viewer for GitHub issues and PRs, backed by the `gh` CLI.
//!
//! Punch in an issue number, read the ticket. `o` opens it in GitHub.
//! Usage: `ghit [numbers...] [-R owner/repo]`
//!
//! Data-first layout: `model` is the pure domain core and `core` the pure
//! business rules. `github` fronts all data access (`api`/`gh`/`cache`).
//! `app` owns state and the event loop, `input` maps keys/mouse to state
//! calls, and `ui`/`theme` own the look-and-feel. `cli` parses args.

mod api;
mod app;
mod cache;
mod cli;
mod clipboard;
mod core;
#[cfg(unix)]
mod drive;
mod gh;
mod github;
mod input;
mod model;
mod theme;
mod ui;

use clap::Parser;

fn main() {
    let args = cli::Args::parse();
    theme::init();
    let numbers = args.issue_numbers();
    let mut app = app::App::new(args.repo.clone());
    if let Some(sock) = args.drive.clone() {
        #[cfg(unix)]
        {
            let (width, height) = args.size_parsed().unwrap_or_else(|e| {
                eprintln!("ghit: {e}");
                std::process::exit(1);
            });
            if let Err(e) = drive::run_drive(&mut app, &numbers, &sock, width, height) {
                eprintln!("ghit: {e}");
                std::process::exit(1);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = &sock;
            eprintln!("ghit: --drive needs a Unix socket and is unavailable on this platform");
            std::process::exit(1);
        }
    } else if let Err(e) = app::run(&mut app, &numbers) {
        eprintln!("ghit: {e}");
        std::process::exit(1);
    }
}
