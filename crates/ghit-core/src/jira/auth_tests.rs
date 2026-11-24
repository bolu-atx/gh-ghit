use std::cell::RefCell;

use super::*;

#[derive(Default)]
struct MemoryStore {
    token: RefCell<Option<String>>,
    unavailable: bool,
    reads: RefCell<usize>,
}

impl TokenStore for MemoryStore {
    fn load(&self, _: &str) -> Result<Option<String>, KeychainUnavailable> {
        *self.reads.borrow_mut() += 1;
        if self.unavailable {
            return Err(KeychainUnavailable);
        }
        Ok(self.token.borrow().clone())
    }

    fn save(&self, _: &str, token: &str) -> Result<(), KeychainUnavailable> {
        *self.token.borrow_mut() = Some(token.to_string());
        Ok(())
    }

    fn delete(&self, _: &str) -> Result<bool, KeychainUnavailable> {
        Ok(self.token.borrow_mut().take().is_some())
    }
}

fn config(command: Option<&str>) -> JiraRepoConfig {
    JiraRepoConfig {
        site: "https://example.atlassian.net/".into(),
        email: "me@example.com".into(),
        project: "PROJ".into(),
        token_command: command.map(str::to_string),
    }
}

#[test]
fn env_beats_command_and_keychain() {
    let store = MemoryStore::default();
    let resolved = resolve_with(&config(Some("nonexistent-command")), Some(" env "), Some(&store))
        .unwrap();
    assert_eq!(resolved.token, Some(("env".into(), TokenSource::Env)));
    assert_eq!(*store.reads.borrow(), 0);
}

#[test]
fn command_beats_keychain_and_trims_stdout() {
    let store = MemoryStore::default();
    store.save("", "stored").unwrap();
    let resolved = resolve_with(&config(Some("rustc --version")), None, Some(&store)).unwrap();
    let (token, source) = resolved.token.unwrap();
    assert_eq!(source, TokenSource::Command);
    assert!(token.starts_with("rustc "));
    assert_eq!(token, token.trim());
    assert_eq!(*store.reads.borrow(), 0);
}

#[test]
fn command_errors_do_not_fall_back_or_expose_output() {
    let store = MemoryStore::default();
    for command in ["false", "echo", "", "nonexistent-jtui-command"] {
        let result = resolve_with(&config(Some(command)), None, Some(&store));
        let Err(error) = result else { panic!("command should fail") };
        assert_eq!(error, StartupError::TokenCommand { command: command.into() });
        assert!(error.message().contains(command));
    }
    assert_eq!(*store.reads.borrow(), 0);
}

#[test]
fn keychain_is_used_only_without_env_or_command() {
    let store = MemoryStore::default();
    store.save("", "stored").unwrap();
    let resolved = resolve_with(&config(None), Some(" "), Some(&store)).unwrap();
    assert_eq!(resolved.token, Some(("stored".into(), TokenSource::Keychain)));
    assert!(!resolved.keychain_unavailable);
    assert!(store.delete("").unwrap());
    assert!(!store.delete("").unwrap());
}

#[test]
fn absent_and_unavailable_keychains_leave_no_token() {
    for unavailable in [false, true] {
        let store = MemoryStore { unavailable, ..MemoryStore::default() };
        let resolved = resolve_with(&config(None), None, Some(&store)).unwrap();
        assert!(resolved.token.is_none());
        assert_eq!(resolved.keychain_unavailable, unavailable);
    }
    assert!(StartupError::MissingToken.message().contains("jtui auth login"));
}

#[test]
fn account_uses_email_and_site_host() {
    assert_eq!(account(&config(None)), "me@example.com@example.atlassian.net");
}

#[test]
fn login_saves_only_after_200() {
    for status in [200, 201, 204, 401, 403, 500] {
        let store = MemoryStore::default();
        store.save("", "previous").unwrap();
        let check = std::future::ready(identity_status(status, "https://x/myself"));
        let result = crate::runtime::rt().block_on(
            save_after_check("", "new-token", &store, check),
        );
        assert_eq!(result.is_ok(), status == 200);
        let expected = if status == 200 { "new-token" } else { "previous" };
        assert_eq!(store.load("").unwrap().as_deref(), Some(expected));
    }
}

#[test]
fn login_does_not_save_on_network_failure() {
    let store = MemoryStore::default();
    let check = std::future::ready(Err(StartupError::Network { detail: "offline".into() }));
    assert!(crate::runtime::rt().block_on(
        save_after_check("", "new-token", &store, check),
    ).is_err());
    assert!(store.token.borrow().is_none());
}

#[cfg(unix)]
#[test]
fn failed_command_output_is_never_in_the_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("token-command");
    std::fs::write(&script, "#!/bin/sh\necho secret-output\necho secret-error >&2\nexit 1\n")
        .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let command = script.to_str().unwrap();
    let result = resolve_with(&config(Some(command)), None, Some(&MemoryStore::default()));
    let Err(error) = result else { panic!("command should fail") };
    let shown = format!("{error:?} {error}");
    assert!(shown.contains(command));
    assert!(!shown.contains("secret-output"));
    assert!(!shown.contains("secret-error"));
}


#[test]
fn token_store_is_optional() {
    let resolved = resolve_with(&config(None), None, None).unwrap();
    assert!(resolved.token.is_none());
    assert!(!resolved.keychain_unavailable);
    let resolved = resolve_with(&config(None), Some("env"), None).unwrap();
    assert_eq!(resolved.token, Some(("env".into(), TokenSource::Env)));
    let resolved = resolve_with(&config(Some("rustc --version")), None, None).unwrap();
    assert_eq!(resolved.token.unwrap().1, TokenSource::Command);
}
