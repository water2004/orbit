mod microsoft;
mod yggdrasil;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::atomic_io::write_atomic;
use crate::config::{GlobalConfig, YggdrasilProviderConfig};
use crate::error::LauncherError;
use crate::runtime::RuntimePaths;
use crate::secret_store::SecretStore;

pub use microsoft::{
    MicrosoftDeviceSession, MicrosoftLoginProgressEvent, begin_microsoft_device_login,
    complete_microsoft_device_login,
};
pub use yggdrasil::{ExternalYggdrasilLoginRequest, login_external_yggdrasil};

pub const ACCOUNTS_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AccountProvider {
    Microsoft,
    Offline,
    ExternalYggdrasil { provider_id: String },
}

impl AccountProvider {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Microsoft => "microsoft",
            Self::Offline => "offline",
            Self::ExternalYggdrasil { .. } => "external-yggdrasil",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountMetadata {
    pub id: Uuid,
    pub provider: AccountProvider,
    pub profile_id: Uuid,
    pub profile_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_name: Option<String>,
    pub created_at_unix_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_authenticated_at_unix_seconds: Option<u64>,
}

impl AccountMetadata {
    fn validate(&self) -> Result<(), LauncherError> {
        if self.id.is_nil() || self.profile_id.is_nil() {
            return Err(LauncherError::Authentication(
                "account and profile IDs must not be nil".to_string(),
            ));
        }
        validate_profile_name(&self.profile_name)?;
        if self.created_at_unix_seconds == 0 {
            return Err(LauncherError::Authentication(
                "account creation time is invalid".to_string(),
            ));
        }
        match &self.provider {
            AccountProvider::Microsoft | AccountProvider::Offline if self.login_name.is_some() => {
                Err(LauncherError::Authentication(
                    "only External Yggdrasil accounts may store a login name".to_string(),
                ))
            }
            AccountProvider::ExternalYggdrasil { provider_id }
                if !valid_identifier(provider_id)
                    || self.login_name.as_ref().is_none_or(|name| {
                        name.is_empty()
                            || name.trim() != name
                            || name.len() > 320
                            || name.chars().any(char::is_control)
                    }) =>
            {
                Err(LauncherError::Authentication(
                    "External Yggdrasil account metadata is invalid".to_string(),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountsDocument {
    schema: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_account: Option<Uuid>,
    accounts: Vec<AccountMetadata>,
}

impl Default for AccountsDocument {
    fn default() -> Self {
        Self {
            schema: ACCOUNTS_SCHEMA,
            default_account: None,
            accounts: Vec::new(),
        }
    }
}

impl AccountsDocument {
    fn validate(&self) -> Result<(), LauncherError> {
        if self.schema != ACCOUNTS_SCHEMA {
            return Err(LauncherError::Authentication(format!(
                "unsupported accounts.json schema {}; expected {ACCOUNTS_SCHEMA}",
                self.schema
            )));
        }
        let mut ids = HashSet::new();
        let mut profiles = HashSet::new();
        for account in &self.accounts {
            account.validate()?;
            if !ids.insert(account.id) {
                return Err(LauncherError::Authentication(format!(
                    "duplicate account ID '{}'",
                    account.id
                )));
            }
            let provider_identity = match &account.provider {
                AccountProvider::Microsoft => "microsoft".to_string(),
                AccountProvider::Offline => "offline".to_string(),
                AccountProvider::ExternalYggdrasil { provider_id } => {
                    format!("external-yggdrasil:{provider_id}")
                }
            };
            if !profiles.insert((provider_identity, account.profile_id)) {
                return Err(LauncherError::Authentication(format!(
                    "profile '{}' is registered more than once for the same provider",
                    account.profile_id
                )));
            }
        }
        if self
            .default_account
            .is_some_and(|default| !ids.contains(&default))
        {
            return Err(LauncherError::Authentication(
                "default account does not exist".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AccountRepository {
    path: PathBuf,
    document: AccountsDocument,
}

impl AccountRepository {
    pub fn load(paths: &RuntimePaths) -> Result<Self, LauncherError> {
        Self::open(&paths.accounts_file())
    }

    fn open(path: &Path) -> Result<Self, LauncherError> {
        let document = if path.exists() {
            serde_json::from_slice::<AccountsDocument>(&std::fs::read(path)?).map_err(|error| {
                LauncherError::Authentication(format!("failed to parse accounts.json: {error}"))
            })?
        } else {
            AccountsDocument::default()
        };
        document.validate()?;
        Ok(Self {
            path: path.to_path_buf(),
            document,
        })
    }

    pub fn accounts(&self) -> &[AccountMetadata] {
        &self.document.accounts
    }

    pub fn default_account(&self) -> Option<Uuid> {
        self.document.default_account
    }

    pub fn get(&self, selector: &str) -> Result<&AccountMetadata, LauncherError> {
        let id = Uuid::parse_str(selector).ok();
        let mut matches = self.document.accounts.iter().filter(|account| {
            id == Some(account.id)
                || account.profile_name == selector
                || account.profile_id.to_string() == selector
        });
        let account = matches.next().ok_or_else(|| {
            LauncherError::Authentication(format!("account '{selector}' does not exist"))
        })?;
        if matches.next().is_some() {
            return Err(LauncherError::Authentication(format!(
                "account selector '{selector}' is ambiguous; use the account ID"
            )));
        }
        Ok(account)
    }

    pub fn selected(&self, explicit: Option<Uuid>) -> Result<&AccountMetadata, LauncherError> {
        let id = explicit.or(self.document.default_account).ok_or_else(|| {
            LauncherError::InteractionRequired(
                "a client account must be selected in [launch].account or with 'account select'"
                    .to_string(),
            )
        })?;
        self.document
            .accounts
            .iter()
            .find(|account| account.id == id)
            .ok_or_else(|| {
                LauncherError::Authentication(format!("selected account '{id}' does not exist"))
            })
    }

    pub fn set_default(&mut self, account: Option<Uuid>) -> Result<(), LauncherError> {
        if account.is_some_and(|id| !self.document.accounts.iter().any(|item| item.id == id)) {
            return Err(LauncherError::Authentication(
                "cannot select an account that does not exist".to_string(),
            ));
        }
        self.document.default_account = account;
        self.save()
    }

    fn identity_match(left: &AccountMetadata, right: &AccountMetadata) -> bool {
        left.provider == right.provider && left.profile_id == right.profile_id
    }

    fn existing_id(&self, candidate: &AccountMetadata) -> Option<(Uuid, u64)> {
        self.document
            .accounts
            .iter()
            .find(|account| Self::identity_match(account, candidate))
            .map(|account| (account.id, account.created_at_unix_seconds))
    }

    fn upsert(&mut self, mut account: AccountMetadata) -> Result<AccountMetadata, LauncherError> {
        if let Some((id, created)) = self.existing_id(&account) {
            account.id = id;
            account.created_at_unix_seconds = created;
        }
        account.validate()?;
        if let Some(existing) = self
            .document
            .accounts
            .iter_mut()
            .find(|existing| existing.id == account.id)
        {
            *existing = account.clone();
        } else {
            self.document.accounts.push(account.clone());
        }
        if self.document.default_account.is_none() {
            self.document.default_account = Some(account.id);
        }
        self.document.accounts.sort_by_key(|account| account.id);
        self.save()?;
        Ok(account)
    }

    pub async fn remove(
        &mut self,
        account_id: Uuid,
        secrets: &dyn SecretStore,
    ) -> Result<AccountMetadata, LauncherError> {
        let index = self
            .document
            .accounts
            .iter()
            .position(|account| account.id == account_id)
            .ok_or_else(|| {
                LauncherError::Authentication(format!("account '{account_id}' does not exist"))
            })?;
        let removed = self.document.accounts.remove(index);
        secrets.delete(&account_secret_key(account_id)).await?;
        if self.document.default_account == Some(account_id) {
            self.document.default_account = self.document.accounts.first().map(|item| item.id);
        }
        self.save()?;
        Ok(removed)
    }

    fn save(&self) -> Result<(), LauncherError> {
        self.document.validate()?;
        let mut bytes = serde_json::to_vec_pretty(&self.document).map_err(|error| {
            LauncherError::Authentication(format!("failed to serialize accounts.json: {error}"))
        })?;
        bytes.push(b'\n');
        write_atomic(&self.path, &bytes)
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum AccountSecret {
    Microsoft {
        refresh_token: String,
        minecraft_access_token: String,
        expires_at_unix_seconds: u64,
        token_type: String,
    },
    ExternalYggdrasil {
        access_token: String,
        client_token: String,
    },
}

pub struct AccountLaunchIdentity {
    pub account_id: Uuid,
    pub profile_id: Uuid,
    pub profile_name: String,
    pub user_type: String,
    pub user_properties: String,
    pub access_token: String,
    pub yggdrasil_provider: Option<String>,
    pub yggdrasil_api_root: Option<String>,
    pub yggdrasil_prefetched_metadata: Option<String>,
}

impl Drop for AccountLaunchIdentity {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.user_properties.zeroize();
        if let Some(metadata) = &mut self.yggdrasil_prefetched_metadata {
            metadata.zeroize();
        }
    }
}

pub fn create_offline_account(
    paths: &RuntimePaths,
    profile_name: &str,
) -> Result<AccountMetadata, LauncherError> {
    validate_offline_name(profile_name)?;
    let mut digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{profile_name}")).into();
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    let now = now_unix_seconds()?;
    AccountRepository::load(paths)?.upsert(AccountMetadata {
        id: Uuid::new_v4(),
        provider: AccountProvider::Offline,
        profile_id: Uuid::from_bytes(digest),
        profile_name: profile_name.to_string(),
        login_name: None,
        created_at_unix_seconds: now,
        last_authenticated_at_unix_seconds: None,
    })
}

pub async fn resolve_launch_identity(
    paths: &RuntimePaths,
    config: &GlobalConfig,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
    explicit_account: Option<Uuid>,
) -> Result<AccountLaunchIdentity, LauncherError> {
    let repository = AccountRepository::load(paths)?;
    let account = repository.selected(explicit_account)?.clone();
    match &account.provider {
        AccountProvider::Offline => Ok(AccountLaunchIdentity {
            account_id: account.id,
            profile_id: account.profile_id,
            profile_name: account.profile_name,
            user_type: "legacy".to_string(),
            user_properties: "{}".to_string(),
            access_token: "0".to_string(),
            yggdrasil_provider: None,
            yggdrasil_api_root: None,
            yggdrasil_prefetched_metadata: None,
        }),
        AccountProvider::Microsoft => {
            microsoft::resolve_microsoft_identity(paths, config, client, secrets, account).await
        }
        AccountProvider::ExternalYggdrasil { provider_id } => {
            let provider = find_yggdrasil_provider(config, provider_id)?;
            yggdrasil::resolve_yggdrasil_identity(paths, client, secrets, account, provider).await
        }
    }
}

async fn persist_authenticated_account(
    paths: &RuntimePaths,
    secrets: &dyn SecretStore,
    mut metadata: AccountMetadata,
    secret: &AccountSecret,
) -> Result<AccountMetadata, LauncherError> {
    let mut repository = AccountRepository::load(paths)?;
    if let Some((id, created)) = repository.existing_id(&metadata) {
        metadata.id = id;
        metadata.created_at_unix_seconds = created;
    }
    let serialized = Zeroizing::new(serde_json::to_vec(secret).map_err(|error| {
        LauncherError::Authentication(format!(
            "failed to serialize private account session: {error}"
        ))
    })?);
    secrets
        .replace(&account_secret_key(metadata.id), serialized.as_slice())
        .await?;
    match repository.upsert(metadata.clone()) {
        Ok(account) => Ok(account),
        Err(error) => {
            let _ = secrets.delete(&account_secret_key(metadata.id)).await;
            Err(error)
        }
    }
}

async fn load_account_secret(
    secrets: &dyn SecretStore,
    account_id: Uuid,
) -> Result<AccountSecret, LauncherError> {
    let mut bytes = Zeroizing::new(
        secrets
            .load(&account_secret_key(account_id))
            .await?
            .ok_or_else(|| {
                LauncherError::Authentication(format!(
                    "account '{account_id}' has no stored session; log in again"
                ))
            })?,
    );
    serde_json::from_slice(&bytes).map_err(|error| {
        bytes.zeroize();
        LauncherError::Authentication(format!(
            "stored session for account '{account_id}' is invalid: {error}"
        ))
    })
}

fn account_secret_key(account_id: Uuid) -> String {
    format!("account:{account_id}")
}

fn find_yggdrasil_provider<'a>(
    config: &'a GlobalConfig,
    provider_id: &str,
) -> Result<&'a YggdrasilProviderConfig, LauncherError> {
    config
        .yggdrasil
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            LauncherError::Authentication(format!(
                "Yggdrasil provider '{provider_id}' is not configured"
            ))
        })
}

fn now_unix_seconds() -> Result<u64, LauncherError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| LauncherError::Authentication(format!("system clock is invalid: {error}")))
}

fn validate_profile_name(name: &str) -> Result<(), LauncherError> {
    if name.is_empty()
        || name.len() > 64
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(LauncherError::Authentication(
            "Minecraft profile name is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_offline_name(name: &str) -> Result<(), LauncherError> {
    if !(1..=16).contains(&name.len())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(LauncherError::Authentication(
            "offline profile name must be 1-16 ASCII letters, digits, or underscores".to_string(),
        ));
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn parse_profile_id(value: &str, subject: &str) -> Result<Uuid, LauncherError> {
    Uuid::parse_str(value).map_err(|_| {
        LauncherError::Authentication(format!("{subject} returned invalid profile ID '{value}'"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(directory: &Path) -> RuntimePaths {
        RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_dir: Some(directory.join("config")),
            data_dir: Some(directory.join("data")),
            cache_dir: Some(directory.join("cache")),
        })
        .unwrap()
    }

    #[test]
    fn offline_uuid_matches_java_name_uuid_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let account = create_offline_account(&paths(directory.path()), "Notch").unwrap();
        assert_eq!(
            account.profile_id.to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
        assert_eq!(
            AccountRepository::load(&paths(directory.path()))
                .unwrap()
                .default_account(),
            Some(account.id)
        );
    }

    #[test]
    fn account_file_contains_no_secret_fields() {
        let directory = tempfile::tempdir().unwrap();
        create_offline_account(&paths(directory.path()), "Player_1").unwrap();
        let text = std::fs::read_to_string(paths(directory.path()).accounts_file()).unwrap();
        assert!(!text.contains("access_token"));
        assert!(!text.contains("refresh_token"));
    }

    #[tokio::test]
    async fn private_sessions_are_kept_out_of_account_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let store = crate::secret_store::test_support::MemorySecretStore::default();
        let now = now_unix_seconds().unwrap();
        let secret = AccountSecret::Microsoft {
            refresh_token: "refresh-secret".to_string(),
            minecraft_access_token: "access-secret".to_string(),
            expires_at_unix_seconds: now + 3600,
            token_type: "Bearer".to_string(),
        };
        let account = persist_authenticated_account(
            &paths,
            &store,
            AccountMetadata {
                id: Uuid::new_v4(),
                provider: AccountProvider::Microsoft,
                profile_id: Uuid::new_v4(),
                profile_name: "Player".to_string(),
                login_name: None,
                created_at_unix_seconds: now,
                last_authenticated_at_unix_seconds: Some(now),
            },
            &secret,
        )
        .await
        .unwrap();
        let stored = load_account_secret(&store, account.id).await.unwrap();
        assert!(matches!(stored, AccountSecret::Microsoft { .. }));
        let metadata = std::fs::read_to_string(paths.accounts_file()).unwrap();
        assert!(!metadata.contains("refresh-secret"));
        assert!(!metadata.contains("access-secret"));
    }
}
