// account persistence, active-account switching, and offline uuids.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub uuid: String,
    pub username: String,
    pub account_type: AccountType,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_mc_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_mc_token_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AccountType {
    Microsoft,
    Offline,
}

#[derive(Debug)]
pub enum AuthResult {
    Success(Account),
    Error(String),
}

pub struct AccountStore {
    pub accounts: Vec<Account>,
    path: PathBuf,
}

impl AccountStore {
    pub fn load() -> Self {
        let path = account_store_path();
        let accounts = match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(accounts) => accounts,
                Err(e) => {
                    tracing::warn!("Failed to parse accounts file {}: {}", path.display(), e);
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("No accounts file at {}", path.display());
                Vec::new()
            }
            Err(e) => {
                tracing::warn!("Failed to read accounts file {}: {}", path.display(), e);
                Vec::new()
            }
        };
        tracing::debug!(
            "Loaded {} account(s) from {}",
            accounts.len(),
            path.display()
        );
        Self { accounts, path }
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::error!("Failed to create accounts directory: {}", e);
            return;
        }
        match serde_json::to_string_pretty(&self.accounts) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    tracing::error!(
                        "Failed to write accounts file {}: {}",
                        self.path.display(),
                        e
                    );
                } else {
                    // holds long-lived refresh tokens, so lock it to the owner
                    // regardless of the umask.
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Err(e) =
                            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                        {
                            tracing::warn!(
                                "Failed to restrict permissions on accounts file {}: {}",
                                self.path.display(),
                                e
                            );
                        }
                    }
                    tracing::debug!(
                        "Saved {} account(s) to {}",
                        self.accounts.len(),
                        self.path.display()
                    );
                }
            }
            Err(e) => tracing::error!("Failed to serialize accounts: {}", e),
        }
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.active)
    }

    pub fn has_microsoft_account(&self) -> bool {
        self.accounts
            .iter()
            .any(|account| account.account_type == AccountType::Microsoft)
    }

    // any account at all. gates offline creation: the first account must be
    // microsoft (prove ownership once); after that, anything goes.
    pub fn has_any_account(&self) -> bool {
        !self.accounts.is_empty()
    }

    pub fn set_active(&mut self, index: usize) {
        let username = self
            .accounts
            .get(index)
            .map(|account| account.username.clone());
        for (i, acc) in self.accounts.iter_mut().enumerate() {
            acc.active = i == index;
        }
        if let Some(username) = username {
            tracing::info!("Selected account '{}'", username);
        } else {
            tracing::warn!("Tried to select missing account index {}", index);
        }
        self.save();
    }

    // same uuid replaces the old account; the first one added auto-becomes
    // active so there's always a selection.
    pub fn add(&mut self, account: Account) {
        let uuid = &account.uuid;
        let replaced = self.accounts.iter().any(|a| a.uuid == *uuid);
        let account_type = account.account_type.clone();
        let username = account.username.clone();
        self.accounts.retain(|a| a.uuid != *uuid);
        let mut account = account;
        if self.accounts.is_empty() {
            account.active = true;
        }
        self.accounts.push(account);
        tracing::info!(
            "{} {:?} account '{}'",
            if replaced { "Updated" } else { "Added" },
            account_type,
            username
        );
        self.save();
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.accounts.len() {
            tracing::warn!("Tried to remove missing account index {}", index);
            return;
        }
        let account = self.accounts.remove(index);
        tracing::info!(
            "Removed {:?} account '{}'",
            account.account_type,
            account.username
        );
        if account.active && !self.accounts.is_empty() {
            self.accounts[0].active = true;
            tracing::debug!("Activated fallback account '{}'", self.accounts[0].username);
        }
        self.save();
    }
}

pub fn account_store_path() -> PathBuf {
    crate::config::get_config_path().join("accounts.json")
}

// deterministic offline uuid from a username, matching vanilla's scheme: a
// version-3 (MD5) uuid over "OfflinePlayer:<username>" with the
// version/variant bits patched in. keeps offline uuids consistent across
// launchers (save data, whitelists, etc. key off this).
pub fn offline_uuid(username: &str) -> String {
    use md5::{Digest, Md5};

    let mut hasher = Md5::new();
    hasher.update(format!("OfflinePlayer:{username}"));
    let mut bytes: [u8; 16] = hasher.finalize().into();

    bytes[6] = (bytes[6] & 0x0F) | 0x30;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

pub fn create_offline_account(username: &str) -> Account {
    Account {
        uuid: offline_uuid(username),
        username: username.to_owned(),
        account_type: AccountType::Offline,
        active: false,
        refresh_token: None,
        cached_mc_token: None,
        cached_mc_token_expires_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_is_valid_format() {
        let uuid = offline_uuid("Steve");
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5, "UUID must have 5 dash-separated parts");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }

    #[test]
    fn offline_uuid_version_3_marker() {
        let uuid = offline_uuid("Steve");
        assert!(uuid.split('-').nth(2).unwrap().starts_with('3'));
    }

    #[test]
    fn offline_uuid_variant_bit_set() {
        let uuid = offline_uuid("Steve");
        let part3 = uuid.split('-').nth(3).unwrap();
        let first_nibble = u8::from_str_radix(&part3[..1], 16).unwrap();
        assert!((0x8..=0xb).contains(&first_nibble));
    }

    #[test]
    fn offline_uuid_deterministic() {
        assert_eq!(offline_uuid("Steve"), offline_uuid("Steve"));
        assert_eq!(offline_uuid("Alex"), offline_uuid("Alex"));
    }

    #[test]
    fn offline_uuid_different_for_different_names() {
        assert_ne!(offline_uuid("Steve"), offline_uuid("Alex"));
    }

    // pins offline_uuid to vanilla's algorithm with a known-good value, so a
    // regression fails here instead of as a mismatch against other launchers.
    #[test]
    fn offline_uuid_matches_vanilla_algorithm() {
        assert_eq!(offline_uuid("Steve"), "5627dd98-e6be-3c21-b8a8-e92344183641");
    }

    #[test]
    fn create_offline_account_fields() {
        let acc = create_offline_account("TestPlayer");
        assert_eq!(acc.username, "TestPlayer");
        assert_eq!(acc.account_type, AccountType::Offline);
        assert!(!acc.active);
        assert!(acc.refresh_token.is_none());
        // pin to offline_uuid output so a derivation regression fails here,
        // not just a non-empty-string check.
        assert_eq!(acc.uuid, offline_uuid("TestPlayer"));
    }

    fn make_store(dir: &std::path::Path) -> AccountStore {
        AccountStore {
            accounts: Vec::new(),
            path: dir.join("accounts.json"),
        }
    }

    fn microsoft_account(name: &str) -> Account {
        Account {
            uuid: format!("00000000-0000-0000-0000-{:012}", name.len()),
            username: name.to_owned(),
            account_type: AccountType::Microsoft,
            active: false,
            refresh_token: Some("refresh".to_owned()),
            cached_mc_token: None,
            cached_mc_token_expires_at: None,
        }
    }

    #[test]
    fn store_add_first_becomes_active() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        assert_eq!(store.accounts.len(), 1);
        assert!(store.accounts[0].active);
    }

    #[test]
    fn store_add_second_stays_inactive() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.add(create_offline_account("Bob"));
        assert_eq!(store.accounts.len(), 2);
        assert!(store.accounts[0].active);
        assert!(!store.accounts[1].active);
    }

    #[test]
    fn store_add_duplicate_uuid_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        let mut dup = create_offline_account("Alice");
        dup.username = "AliceRenamed".to_owned();
        dup.uuid = store.accounts[0].uuid.clone();
        store.add(dup);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts[0].username, "AliceRenamed");
    }

    #[test]
    fn store_active_account_none_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = make_store(tmp.path());
        assert!(store.active_account().is_none());
    }

    #[test]
    fn store_has_microsoft_account_when_one_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Offline"));
        assert!(!store.has_microsoft_account());

        store.add(microsoft_account("Owner"));
        assert!(store.has_microsoft_account());
    }

    #[test]
    fn store_has_any_account_true_for_offline_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        assert!(!store.has_any_account());

        store.add(create_offline_account("Offline"));
        assert!(store.has_any_account());
    }

    #[test]
    fn store_active_account_returns_active() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.add(create_offline_account("Bob"));
        let active = store.active_account().unwrap();
        assert_eq!(active.username, "Alice");
    }

    #[test]
    fn store_set_active_changes_active() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.add(create_offline_account("Bob"));
        store.set_active(1);
        assert!(!store.accounts[0].active);
        assert!(store.accounts[1].active);
    }

    #[test]
    fn store_remove_activates_first_remaining() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.add(create_offline_account("Bob"));
        store.remove(0);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.accounts[0].username, "Bob");
        assert!(store.accounts[0].active);
    }

    #[test]
    fn store_remove_out_of_bounds_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.remove(5);
        assert_eq!(store.accounts.len(), 1);
    }

    #[test]
    fn store_save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = make_store(tmp.path());
        store.add(create_offline_account("Alice"));
        store.add(create_offline_account("Bob"));
        store.save();

        let reloaded = AccountStore {
            accounts: serde_json::from_str(
                &std::fs::read_to_string(tmp.path().join("accounts.json")).unwrap(),
            )
            .unwrap(),
            path: tmp.path().join("accounts.json"),
        };
        assert_eq!(reloaded.accounts.len(), 2);
        assert_eq!(reloaded.accounts[0].username, "Alice");
        assert!(reloaded.accounts[0].active);
    }
}
