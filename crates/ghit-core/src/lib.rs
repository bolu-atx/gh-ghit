use std::sync::OnceLock;

static APP_NAME: OnceLock<&'static str> = OnceLock::new();

/// Process-wide app name. It names the data directory and the brand text.
pub fn app_name() -> &'static str {
    APP_NAME.get().copied().unwrap_or("ghit")
}

/// Set the app name. The first call wins, so a bin sets it before any other work.
pub fn set_app_name(name: &'static str) {
    let _ = APP_NAME.set(name);
}

pub mod api;
pub mod backend;
pub mod cache;
pub mod core;
pub mod fetch_queue;
pub mod github;
pub mod jira;
pub mod model;
pub mod runtime;
pub mod session;
pub mod store;
