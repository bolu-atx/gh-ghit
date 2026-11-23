//! Jira Cloud config, client and the jtui startup check.

#[cfg(test)]
#[path = "jira_tests.rs"]
mod tests;

pub mod auth;
mod detail;
mod search;
mod sync;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

/// One repo entry in the jtui config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraRepoConfig {
    pub site: String,
    pub project: String,
    pub email: String,
    pub token_command: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    repos: HashMap<String, RepoRaw>,
}

#[derive(Debug, Deserialize)]
struct RepoRaw {
    site: Option<String>,
    project: Option<String>,
    email: Option<String>,
    token_command: Option<String>,
}

/// A startup failure. `message` is the one clear line jtui prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupError {
    NoConfig { path: PathBuf },
    NoRepo { path: PathBuf, repo: String },
    MissingField { repo: String, field: &'static str },
    BadToml { path: PathBuf, detail: String },
    MissingToken,
    TokenCommand { command: String },
    KeychainUnavailable,
    BadAuth,
    UnknownProject { project: String },
    Http { status: u16, url: String },
    Network { detail: String },
}

impl StartupError {
    pub fn message(&self) -> String {
        match self {
            Self::NoConfig { path } => format!(
                "no config file at {}: create it with an entry like [repos.\"owner/repo\"] site = \"example.atlassian.net\", project = \"PROJ\", email = \"me@example.com\" and set JIRA_API_TOKEN",
                path.display()
            ),
            Self::NoRepo { path, repo } => format!(
                "no Jira entry for {repo} in {}: add a [repos.\"{repo}\"] entry with site, project and email",
                path.display()
            ),
            Self::MissingField { repo, field } => {
                format!("Jira config for {repo} is missing the {field} field")
            }
            Self::BadToml { path, detail } => {
                format!("bad Jira config at {}: {detail}", path.display())
            }
            Self::MissingToken => {
                "no Jira token: run jtui auth login, or set JIRA_API_TOKEN".to_string()
            }
            Self::TokenCommand { command } => {
                format!("token_command failed or returned empty output: {command}")
            }
            Self::KeychainUnavailable => "keychain is unavailable".to_string(),
            Self::BadAuth => {
                concat!(
                    "Jira rejected the email or JIRA_API_TOKEN (401/403): ",
                    "check the config email and token (scoped tokens are not supported)",
                ).to_string()
            }
            Self::UnknownProject { project } => {
                format!("unknown Jira project {project}: check the project key in the config")
            }
            Self::Http { status, url } => format!("Jira request failed: {status} for {url}"),
            Self::Network { detail } => format!("could not reach Jira: {detail}"),
        }
    }
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for StartupError {}

/// Map an HTTP status to a startup failure. `None` means success.
/// `project` names the project key when the failing call is the project lookup.
pub fn startup_error_for_status(status: u16, url: &str, project: Option<&str>) -> Option<StartupError> {
    if (200..300).contains(&status) {
        return None;
    }
    Some(match status {
        401 | 403 => StartupError::BadAuth,
        404 => match project {
            Some(key) => StartupError::UnknownProject { project: key.to_string() },
            None => StartupError::Http { status, url: url.to_string() },
        },
        _ => StartupError::Http { status, url: url.to_string() },
    })
}

/// The config path for given config home and home values. Pure, so tests pin it.
pub fn config_path_from(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    if let Some(dir) = xdg_config_home.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir).join("jtui").join("config.toml");
    }
    if let Some(home) = home.map(str::trim).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(home).join(".config").join("jtui").join("config.toml");
    }
    PathBuf::from(".config").join("jtui").join("config.toml")
}

/// The config path jtui reads: `$XDG_CONFIG_HOME/jtui/config.toml`, else `~/.config/jtui/config.toml`.
pub fn config_path() -> PathBuf {
    config_path_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn parse_config(text: &str, path: &std::path::Path) -> Result<FileConfig, StartupError> {
    toml::from_str(text).map_err(|e| StartupError::BadToml {
        path: path.to_path_buf(),
        detail: e.to_string().lines().next().unwrap_or("invalid TOML").to_string(),
    })
}

fn repo_config(parsed: &FileConfig, path: &std::path::Path, repo: &str) -> Result<JiraRepoConfig, StartupError> {
    let raw = parsed
        .repos
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(repo))
        .map(|(_, raw)| raw)
        .ok_or_else(|| StartupError::NoRepo { path: path.to_path_buf(), repo: repo.to_string() })?;
    let field = |value: &Option<String>, name: &'static str| {
        value
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or(StartupError::MissingField { repo: repo.to_string(), field: name })
    };
    Ok(JiraRepoConfig {
        site: field(&raw.site, "site")?,
        project: field(&raw.project, "project")?,
        email: field(&raw.email, "email")?,
        token_command: raw.token_command.clone(),
    })
}

/// Load the config entry for `repo` from the jtui config file.
pub fn load_repo_config(repo: &str) -> Result<JiraRepoConfig, StartupError> {
    let path = config_path();
    load_repo_config_at(repo, &path)
}

fn load_repo_config_at(repo: &str, path: &std::path::Path) -> Result<JiraRepoConfig, StartupError> {
    if !path.exists() {
        return Err(StartupError::NoConfig { path: path.to_path_buf() });
    }
    let text = std::fs::read_to_string(path).map_err(|e| StartupError::BadToml {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    let parsed = parse_config(&text, path)?;
    repo_config(&parsed, path, repo)
}

fn base_url(site: &str) -> String {
    base_url_with_override(site, std::env::var("JIRA_API_BASE").ok().as_deref())
}

fn base_url_with_override(site: &str, api_base: Option<&str>) -> String {
    if let Some(base) = api_base.map(str::trim).filter(|base| !base.is_empty()) {
        return base.trim_end_matches('/').to_string();
    }
    let site = site.trim().trim_end_matches('/');
    if site.starts_with("https://") || site.starts_with("http://") {
        site.to_string()
    } else {
        format!("https://{site}")
    }
}

/// Jira Cloud client for one project. Auth is HTTP Basic with email plus token.
#[derive(Clone)]
pub struct JiraClient {
    base: String,
    email: String,
    token: String,
    source: auth::TokenSource,
    project: String,
    config_path: PathBuf,
}

/// Never prints the token, so logs and panics cannot leak it.
impl std::fmt::Debug for JiraClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JiraClient")
            .field("base", &self.base)
            .field("email", &self.email)
            .field("token", &"<redacted>")
            .field("project", &self.project)
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl JiraClient {
    pub fn new(site: &str, email: String, token: String, project: String) -> Self {
        Self {
            base: base_url(site), email, token, project, config_path: config_path(),
            source: auth::TokenSource::Env,
        }
    }

    pub fn from_config(
        config: JiraRepoConfig, token: String, source: auth::TokenSource,
    ) -> Self {
        let mut client = Self::new(&config.site, config.email, token, config.project);
        client.source = source;
        client
    }

    /// Load the config entry and token for `repo` and build the client.
    pub fn for_repo(
        repo: &str, store: Option<&dyn auth::TokenStore>,
    ) -> Result<Self, StartupError> {
        let path = config_path();
        let config = load_repo_config_at(repo, &path)?;
        let resolved = auth::resolve_token(&config, store)?;
        let (token, source) = resolved.token.ok_or(StartupError::MissingToken)?;
        let mut client = Self::from_config(config, token, source);
        client.config_path = path;
        Ok(client)
    }

    pub fn token_source(&self) -> auth::TokenSource {
        self.source
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The site URL the client talks to.
    pub fn site(&self) -> &str {
        &self.base
    }

    /// The config file the client was loaded from.
    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    pub fn myself_url(&self) -> String {
        format!("{}/rest/api/3/myself", self.base)
    }

    pub fn project_url(&self) -> String {
        format!("{}/rest/api/3/project/{}", self.base, self.project)
    }

    /// Startup check: the token must answer for this account and the project must exist.
    pub async fn check_startup(&self) -> Result<(), StartupError> {
        self.check_identity().await?;
        self.get_status(&self.project_url(), Some(&self.project)).await
    }

    pub async fn check_identity(&self) -> Result<(), StartupError> {
        let url = self.myself_url();
        let status = self.request_status(&url).await?;
        auth::identity_status(status, &url)
    }

    async fn get_status(&self, url: &str, project: Option<&str>) -> Result<(), StartupError> {
        let status = self.request_status(url).await?;
        match startup_error_for_status(status, url, project) {
            None => Ok(()),
            Some(error) => Err(error),
        }
    }

    async fn request_status(&self, url: &str) -> Result<u16, StartupError> {
        let res = client()
            .get(url)
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| StartupError::Network { detail: e.to_string() })?;
        Ok(res.status().as_u16())
    }
}

fn client() -> reqwest::Client {
    #[cfg(any(test, feature = "test-support"))]
    no_network_in_tests("the Jira HTTP client");
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("jtui")
                .build()
                .expect("reqwest client")
        })
        .clone()
}

#[cfg(any(test, feature = "test-support"))]
fn no_network_in_tests(what: &str) {
    panic!("unit test reached {what}: unit tests must not touch the network");
}
