use ghit_core::jira::{self, auth, JiraClient, StartupError};
use ghit_core::jira::auth::{KeychainUnavailable, TokenStore};

use crate::cli::AuthAction;

pub struct OsKeychain;

impl TokenStore for OsKeychain {
    fn load(&self, account: &str) -> Result<Option<String>, KeychainUnavailable> {
        let entry = entry(account).map_err(|_| KeychainUnavailable)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(KeychainUnavailable),
        }
    }

    fn save(&self, account: &str, token: &str) -> Result<(), KeychainUnavailable> {
        entry(account)
            .and_then(|entry| entry.set_password(token))
            .map_err(|_| KeychainUnavailable)
    }

    fn delete(&self, account: &str) -> Result<bool, KeychainUnavailable> {
        let entry = entry(account).map_err(|_| KeychainUnavailable)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(KeychainUnavailable),
        }
    }
}

pub fn run(action: &AuthAction, repo: &str) -> Result<(), String> {
    let config = jira::load_repo_config(repo).map_err(|error| error.message())?;
    match action {
        AuthAction::Login => {
            let token = rpassword::prompt_password("Jira API token: ")
                .map_err(|_| "could not read Jira API token".to_string())?;
            if token.trim().is_empty() {
                return Err("Jira API token is empty".to_string());
            }
            ghit_core::runtime::rt().block_on(auth::login(
                config, token.trim().to_string(), &OsKeychain,
            )).map_err(|error| error.message())?;
            println!("token saved to keychain");
        }
        AuthAction::Status => {
            println!("site: {}", config.site);
            println!("email: {}", config.email);
            println!("project: {}", config.project);
            let resolved = match auth::resolve_token(&config, Some(&OsKeychain)) {
                Ok(resolved) => resolved,
                Err(error) => {
                    println!("token source: none");
                    return Err(error.message());
                }
            };
            if resolved.keychain_unavailable {
                println!("keychain is unavailable");
            }
            let Some((token, source)) = resolved.token else {
                println!("token source: none");
                return Err(StartupError::MissingToken.message());
            };
            println!("token source: {}", source.name());
            let client = JiraClient::from_config(config, token, source);
            ghit_core::runtime::rt().block_on(client.check_identity())
                .map_err(|error| error.message())?;
            println!("ok");
        }
        AuthAction::Logout => {
            let existed = OsKeychain.delete(&auth::account(&config))
                .map_err(|_| StartupError::KeychainUnavailable.message())?;
            println!("{}", if existed { "keychain token deleted" } else { "no keychain token" });
        }
    }
    Ok(())
}


fn entry(account: &str) -> keyring::Result<keyring::Entry> {
    #[cfg(not(test))]
    {
        keyring::Entry::new("jtui", account)
    }
    #[cfg(test)]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
        });
        keyring_core::Entry::new("jtui", account).map(|inner| keyring::Entry { inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_store_handles_missing_saved_deleted_and_unavailable_credentials() {
        let account = "mock@example.com@fixture.atlassian.net";
        let store = OsKeychain;
        assert_eq!(store.load(account).unwrap(), None);
        assert!(!store.delete(account).unwrap());
        store.save(account, "secret").unwrap();
        assert_eq!(store.load(account).unwrap().as_deref(), Some("secret"));
        let entry = entry(account).unwrap();
        let credential = entry.inner.as_any().downcast_ref::<keyring_core::mock::Cred>().unwrap();
        for action in ["load", "save", "delete"] {
            credential.set_error(keyring::Error::NoDefaultStore);
            let unavailable = match action {
                "load" => store.load(account).is_err(),
                "save" => store.save(account, "replacement").is_err(),
                _ => store.delete(account).is_err(),
            };
            assert!(unavailable, "{action}");
        }
        assert_eq!(store.load(account).unwrap().as_deref(), Some("secret"));
        assert!(store.delete(account).unwrap());
        assert_eq!(store.load(account).unwrap(), None);
    }
}
