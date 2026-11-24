#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;

use std::process::{Command, Stdio};

use crate::jira::{JiraClient, JiraRepoConfig, StartupError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Command,
    Keychain,
}

pub struct ResolvedToken {
    pub token: Option<(String, TokenSource)>,
    pub keychain_unavailable: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct KeychainUnavailable;

pub trait TokenStore {
    fn load(&self, account: &str) -> Result<Option<String>, KeychainUnavailable>;
    fn save(&self, account: &str, token: &str) -> Result<(), KeychainUnavailable>;
    fn delete(&self, account: &str) -> Result<bool, KeychainUnavailable>;
}

impl TokenSource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Command => "command",
            Self::Keychain => "keychain",
        }
    }

    pub fn settings_text(self) -> &'static str {
        match self {
            Self::Env => "from JIRA_API_TOKEN",
            Self::Command => "from token_command",
            Self::Keychain => "from keychain",
        }
    }
}

pub fn account(config: &JiraRepoConfig) -> String {
    let site = super::base_url_with_override(&config.site, None);
    let host = reqwest::Url::parse(&site).ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| config.site.trim().trim_end_matches('/').to_string());
    format!("{}@{host}", config.email)
}

pub fn resolve_token(
    config: &JiraRepoConfig, store: Option<&dyn TokenStore>,
) -> Result<ResolvedToken, StartupError> {
    resolve_with(config, std::env::var("JIRA_API_TOKEN").ok().as_deref(), store)
}

fn resolve_with(
    config: &JiraRepoConfig, env: Option<&str>, store: Option<&dyn TokenStore>,
) -> Result<ResolvedToken, StartupError> {
    let found = |token, source| ResolvedToken {
        token: Some((token, source)), keychain_unavailable: false,
    };
    if let Some(token) = env.map(str::trim).filter(|token| !token.is_empty()) {
        return Ok(found(token.to_string(), TokenSource::Env));
    }
    if let Some(command) = &config.token_command {
        let error = || StartupError::TokenCommand { command: command.clone() };
        let mut parts = command.split_whitespace();
        let program = parts.next().ok_or_else(error)?;
        let output = Command::new(program).args(parts).stdin(Stdio::null())
            .stderr(Stdio::null()).output().map_err(|_| error())?;
        if !output.status.success() {
            return Err(error());
        }
        let token = String::from_utf8(output.stdout).map_err(|_| error())?;
        let token = token.trim();
        if token.is_empty() {
            return Err(error());
        }
        return Ok(found(token.to_string(), TokenSource::Command));
    }
    let Some(store) = store else {
        return Ok(ResolvedToken { token: None, keychain_unavailable: false });
    };
    match store.load(&account(config)) {
        Ok(Some(token)) if !token.trim().is_empty() => {
            Ok(found(token.trim().to_string(), TokenSource::Keychain))
        }
        result => Ok(ResolvedToken { token: None, keychain_unavailable: result.is_err() }),
    }
}

pub(super) fn identity_status(status: u16, url: &str) -> Result<(), StartupError> {
    if status == 200 {
        return Ok(());
    }
    Err(super::startup_error_for_status(status, url, None)
        .unwrap_or_else(|| StartupError::Http { status, url: url.to_string() }))
}

pub async fn login(
    config: JiraRepoConfig, token: String, store: &impl TokenStore,
) -> Result<(), StartupError> {
    let account = account(&config);
    let client = JiraClient::from_config(config, token.clone(), TokenSource::Keychain);
    save_after_check(&account, &token, store, client.check_identity()).await
}

async fn save_after_check(
    account: &str, token: &str, store: &impl TokenStore,
    check: impl std::future::Future<Output = Result<(), StartupError>>,
) -> Result<(), StartupError> {
    check.await?;
    store.save(account, token).map_err(|_| StartupError::KeychainUnavailable)
}
