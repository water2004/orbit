use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;

pub const CONFIG_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    pub schema: u32,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub installer: InstallerConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub minecraft: MinecraftGlobalConfig,
    #[serde(default)]
    pub yggdrasil: YggdrasilConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            schema: CONFIG_SCHEMA,
            network: NetworkConfig::default(),
            installer: InstallerConfig::default(),
            cache: CacheConfig::default(),
            minecraft: MinecraftGlobalConfig::default(),
            yggdrasil: YggdrasilConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl GlobalConfig {
    pub fn http_client(&self) -> Result<reqwest::Client, LauncherError> {
        self.validate()?;
        Ok(reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(self.network.connect_timeout_seconds))
            .timeout(Duration::from_secs(self.network.request_timeout_seconds))
            .user_agent(concat!("orbit-launcher-core/", env!("CARGO_PKG_VERSION")))
            .build()?)
    }

    pub fn load(path: &Path) -> Result<Self, LauncherError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content).map_err(LauncherError::ConfigParse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), LauncherError> {
        self.validate()?;
        let content = toml::to_string_pretty(self)?;
        write_atomic(path, content.as_bytes())
    }

    pub fn validate(&self) -> Result<(), LauncherError> {
        if self.schema != CONFIG_SCHEMA {
            return Err(LauncherError::InvalidConfig(format!(
                "unsupported schema {}; expected {CONFIG_SCHEMA}",
                self.schema
            )));
        }
        if self.network.concurrency == 0 {
            return Err(LauncherError::InvalidConfig(
                "network.concurrency must be greater than zero".to_string(),
            ));
        }
        if self.network.connect_timeout_seconds == 0 || self.network.request_timeout_seconds == 0 {
            return Err(LauncherError::InvalidConfig(
                "network timeouts must be greater than zero".to_string(),
            ));
        }
        if self.installer.timeout_seconds == 0 {
            return Err(LauncherError::InvalidConfig(
                "installer.timeout-seconds must be greater than zero".to_string(),
            ));
        }
        self.cache.max_size_bytes()?;
        if let Some(directory) = &self.minecraft.directory
            && !directory.is_absolute()
        {
            return Err(LauncherError::InvalidConfig(format!(
                "minecraft.directory '{}' must be absolute",
                directory.display()
            )));
        }
        let mut ids = HashSet::new();
        for provider in &self.yggdrasil.providers {
            validate_identifier(&provider.id, "Yggdrasil provider")?;
            if !ids.insert(provider.id.as_str()) {
                return Err(LauncherError::InvalidConfig(format!(
                    "duplicate Yggdrasil provider ID '{}'",
                    provider.id
                )));
            }
            let url = url::Url::parse(&provider.api_root).map_err(|error| {
                LauncherError::InvalidConfig(format!(
                    "invalid Yggdrasil API root for '{}': {error}",
                    provider.id
                ))
            })?;
            if url.scheme() != "https" && !provider.allow_insecure_http {
                return Err(LauncherError::InvalidConfig(format!(
                    "Yggdrasil provider '{}' must use HTTPS unless allow_insecure_http is explicitly enabled",
                    provider.id
                )));
            }
            if url.host_str().is_none() {
                return Err(LauncherError::InvalidConfig(format!(
                    "Yggdrasil provider '{}' API root must contain a host",
                    provider.id
                )));
            }
            if !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(LauncherError::InvalidConfig(format!(
                    "Yggdrasil provider '{}' API root cannot contain credentials, query, or fragment",
                    provider.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigKey {
    NetworkConcurrency,
    NetworkConnectTimeoutSeconds,
    NetworkRequestTimeoutSeconds,
    InstallerTimeoutSeconds,
    CacheMaxSize,
    UiProgressBar,
    UiColor,
}

impl ConfigKey {
    pub const ALL: [Self; 7] = [
        Self::NetworkConcurrency,
        Self::NetworkConnectTimeoutSeconds,
        Self::NetworkRequestTimeoutSeconds,
        Self::InstallerTimeoutSeconds,
        Self::CacheMaxSize,
        Self::UiProgressBar,
        Self::UiColor,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NetworkConcurrency => "network.concurrency",
            Self::NetworkConnectTimeoutSeconds => "network.connect-timeout-seconds",
            Self::NetworkRequestTimeoutSeconds => "network.request-timeout-seconds",
            Self::InstallerTimeoutSeconds => "installer.timeout-seconds",
            Self::CacheMaxSize => "cache.max-size",
            Self::UiProgressBar => "ui.progress-bar",
            Self::UiColor => "ui.color",
        }
    }

    const fn toml_path(self) -> (&'static str, &'static str) {
        match self {
            Self::NetworkConcurrency => ("network", "concurrency"),
            Self::NetworkConnectTimeoutSeconds => ("network", "connect_timeout_seconds"),
            Self::NetworkRequestTimeoutSeconds => ("network", "request_timeout_seconds"),
            Self::InstallerTimeoutSeconds => ("installer", "timeout_seconds"),
            Self::CacheMaxSize => ("cache", "max_size"),
            Self::UiProgressBar => ("ui", "progress_bar"),
            Self::UiColor => ("ui", "color"),
        }
    }

    fn read(self, config: &GlobalConfig) -> Option<String> {
        match self {
            Self::NetworkConcurrency => Some(config.network.concurrency.to_string()),
            Self::NetworkConnectTimeoutSeconds => {
                Some(config.network.connect_timeout_seconds.to_string())
            }
            Self::NetworkRequestTimeoutSeconds => {
                Some(config.network.request_timeout_seconds.to_string())
            }
            Self::InstallerTimeoutSeconds => Some(config.installer.timeout_seconds.to_string()),
            Self::CacheMaxSize => Some(config.cache.max_size.clone()),
            Self::UiProgressBar => Some(config.ui.progress_bar.as_str().to_string()),
            Self::UiColor => Some(config.ui.color.as_str().to_string()),
        }
    }

    fn parse_item(self, raw: &str) -> Result<Item, LauncherError> {
        let invalid = |expected: &str| {
            LauncherError::InvalidConfig(format!(
                "{} value '{raw}' is invalid; expected {expected}",
                self.as_str()
            ))
        };
        Ok(match self {
            Self::NetworkConcurrency => value(
                raw.parse::<u16>()
                    .map_err(|_| invalid("an integer from 1 to 65535"))? as i64,
            ),
            Self::NetworkConnectTimeoutSeconds
            | Self::NetworkRequestTimeoutSeconds
            | Self::InstallerTimeoutSeconds => value(
                i64::try_from(
                    raw.parse::<u64>()
                        .map_err(|_| invalid("a positive integer"))?,
                )
                .map_err(|_| invalid("a positive integer no greater than 9223372036854775807"))?,
            ),
            Self::CacheMaxSize => value(raw),
            Self::UiProgressBar | Self::UiColor => value(
                UiPreference::from_str(raw)
                    .map_err(|()| invalid("auto, always, or never"))?
                    .as_str(),
            ),
        })
    }
}

impl FromStr for ConfigKey {
    type Err = LauncherError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|key| key.as_str() == value)
            .ok_or_else(|| {
                LauncherError::InvalidConfig(format!(
                    "unknown configuration key '{value}'; run 'orbit-launcher config list' to list supported keys"
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigEntry {
    pub key: ConfigKey,
    pub value: Option<String>,
    pub explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigMutation {
    pub key: ConfigKey,
    pub previous: Option<String>,
    pub current: Option<String>,
    pub explicit: bool,
}

pub fn list_config(path: &Path) -> Result<Vec<ConfigEntry>, LauncherError> {
    let config = GlobalConfig::load(path)?;
    let document = load_document(path)?;
    Ok(ConfigKey::ALL
        .into_iter()
        .map(|key| ConfigEntry {
            key,
            value: key.read(&config),
            explicit: is_explicit(document.as_ref(), key),
        })
        .collect())
}

pub fn get_config(path: &Path, key: ConfigKey) -> Result<ConfigEntry, LauncherError> {
    let config = GlobalConfig::load(path)?;
    let document = load_document(path)?;
    Ok(ConfigEntry {
        key,
        value: key.read(&config),
        explicit: is_explicit(document.as_ref(), key),
    })
}

pub fn set_config(
    path: &Path,
    key: ConfigKey,
    raw_value: &str,
) -> Result<ConfigMutation, LauncherError> {
    let previous = get_config(path, key)?;
    let mut document = editable_document(path)?;
    let (section, field) = key.toml_path();
    if !document.as_table().contains_key(section) {
        document.insert(section, Item::Table(Table::new()));
    }
    document[section][field] = key.parse_item(raw_value)?;
    validate_and_write(path, &document)?;
    let current = get_config(path, key)?;
    Ok(ConfigMutation {
        key,
        previous: previous.value,
        current: current.value,
        explicit: current.explicit,
    })
}

pub fn unset_config(path: &Path, key: ConfigKey) -> Result<ConfigMutation, LauncherError> {
    let previous = get_config(path, key)?;
    if let Some(mut document) = load_document(path)? {
        let (section, field) = key.toml_path();
        if let Some(table) = document.get_mut(section).and_then(Item::as_table_mut) {
            table.remove(field);
        }
        validate_and_write(path, &document)?;
    }
    let current = get_config(path, key)?;
    Ok(ConfigMutation {
        key,
        previous: previous.value,
        current: current.value,
        explicit: current.explicit,
    })
}

pub fn set_minecraft_directory(path: &Path, directory: &Path) -> Result<(), LauncherError> {
    if !directory.is_absolute() {
        return Err(LauncherError::InvalidConfig(format!(
            "Minecraft directory '{}' must be absolute",
            directory.display()
        )));
    }
    let mut document = editable_document(path)?;
    if !document.as_table().contains_key("minecraft") {
        document.insert("minecraft", Item::Table(Table::new()));
    }
    document["minecraft"]["directory"] = value(directory.display().to_string());
    validate_and_write(path, &document)
}

pub fn add_yggdrasil_provider(
    path: &Path,
    provider: YggdrasilProviderConfig,
) -> Result<YggdrasilProviderConfig, LauncherError> {
    let mut config = GlobalConfig::load(path)?;
    if config
        .yggdrasil
        .providers
        .iter()
        .any(|existing| existing.id == provider.id)
    {
        return Err(LauncherError::InvalidConfig(format!(
            "Yggdrasil provider '{}' already exists",
            provider.id
        )));
    }
    config.yggdrasil.providers.push(provider.clone());
    config
        .yggdrasil
        .providers
        .sort_by_key(|provider| provider.id.clone());
    config.validate()?;
    write_yggdrasil_providers(path, &config.yggdrasil.providers)?;
    Ok(provider)
}

pub fn remove_yggdrasil_provider(
    path: &Path,
    provider_id: &str,
) -> Result<YggdrasilProviderConfig, LauncherError> {
    let mut config = GlobalConfig::load(path)?;
    let index = config
        .yggdrasil
        .providers
        .iter()
        .position(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            LauncherError::InvalidConfig(format!(
                "Yggdrasil provider '{provider_id}' does not exist"
            ))
        })?;
    let removed = config.yggdrasil.providers.remove(index);
    write_yggdrasil_providers(path, &config.yggdrasil.providers)?;
    Ok(removed)
}

fn write_yggdrasil_providers(
    path: &Path,
    providers: &[YggdrasilProviderConfig],
) -> Result<(), LauncherError> {
    let mut document = editable_document(path)?;
    if !document.as_table().contains_key("yggdrasil") {
        document.insert("yggdrasil", Item::Table(Table::new()));
    }
    let mut array = ArrayOfTables::new();
    for provider in providers {
        let mut table = Table::new();
        table.insert("id", value(&provider.id));
        table.insert("api_root", value(&provider.api_root));
        if provider.allow_insecure_http {
            table.insert("allow_insecure_http", value(true));
        }
        array.push(table);
    }
    document["yggdrasil"]["providers"] = Item::ArrayOfTables(array);
    validate_and_write(path, &document)
}

fn load_document(path: &Path) -> Result<Option<DocumentMut>, LauncherError> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(path)?
        .parse::<DocumentMut>()
        .map(Some)
        .map_err(LauncherError::ConfigDocumentParse)
}

fn editable_document(path: &Path) -> Result<DocumentMut, LauncherError> {
    if let Some(document) = load_document(path)? {
        return Ok(document);
    }
    format!("schema = {CONFIG_SCHEMA}\n")
        .parse::<DocumentMut>()
        .map_err(LauncherError::ConfigDocumentParse)
}

fn is_explicit(document: Option<&DocumentMut>, key: ConfigKey) -> bool {
    let (section, field) = key.toml_path();
    document
        .and_then(|document| document.get(section))
        .and_then(|item| item.get(field))
        .is_some()
}

fn validate_and_write(path: &Path, document: &DocumentMut) -> Result<(), LauncherError> {
    let content = document.to_string();
    let config: GlobalConfig = toml::from_str(&content).map_err(LauncherError::ConfigParse)?;
    config.validate()?;
    write_atomic(path, content.as_bytes())
}

fn validate_identifier(value: &str, subject: &str) -> Result<(), LauncherError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().any(char::is_control)
        || value.len() > 64
    {
        return Err(LauncherError::InvalidConfig(format!(
            "{subject} ID '{value}' is invalid"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    pub concurrency: u16,
    pub connect_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            concurrency: 8,
            connect_timeout_seconds: 15,
            request_timeout_seconds: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstallerConfig {
    pub timeout_seconds: u64,
}

impl Default for InstallerConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 1_800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    pub max_size: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MinecraftGlobalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
}

impl CacheConfig {
    pub fn max_size_bytes(&self) -> Result<u64, LauncherError> {
        parse_byte_size(&self.max_size)
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: "20 GiB".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct YggdrasilConfig {
    pub providers: Vec<YggdrasilProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YggdrasilProviderConfig {
    pub id: String,
    pub api_root: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_insecure_http: bool,
}

const fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub progress_bar: UiPreference,
    pub color: UiPreference,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            progress_bar: UiPreference::Auto,
            color: UiPreference::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiPreference {
    Auto,
    Always,
    Never,
}

impl UiPreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

impl FromStr for UiPreference {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            _ => Err(()),
        }
    }
}

fn parse_byte_size(value: &str) -> Result<u64, LauncherError> {
    let value = value.trim();
    let number_end = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(number_end);
    let number = number.parse::<u64>().map_err(|_| {
        LauncherError::InvalidConfig(format!(
            "cache.max_size '{value}' must start with a positive integer"
        ))
    })?;
    if number == 0 {
        return Err(LauncherError::InvalidConfig(
            "cache.max_size must be greater than zero".to_string(),
        ));
    }
    let multiplier = match unit.trim() {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024_u64.pow(2),
        "GiB" => 1024_u64.pow(3),
        "TiB" => 1024_u64.pow(4),
        _ => {
            return Err(LauncherError::InvalidConfig(format!(
                "cache.max_size '{value}' must use B, KiB, MiB, GiB, or TiB"
            )));
        }
    };
    number.checked_mul(multiplier).ok_or_else(|| {
        LauncherError::InvalidConfig(format!("cache.max_size '{value}' is too large"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_valid_defaults_without_creating_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let config = GlobalConfig::load(&path).unwrap();
        assert_eq!(config.schema, CONFIG_SCHEMA);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_unknown_fields_and_insecure_yggdrasil_by_default() {
        let unknown = "schema = 2\nunknown = true\n";
        assert!(toml::from_str::<GlobalConfig>(unknown).is_err());

        let config: GlobalConfig = toml::from_str(
            r#"
schema = 2
[[yggdrasil.providers]]
id = "private"
api_root = "http://auth.example.com/api/yggdrasil"
"#,
        )
        .unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn config_roundtrip_is_atomic_and_strict() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut config = GlobalConfig::default();
        config.yggdrasil.providers.push(YggdrasilProviderConfig {
            id: "private".to_string(),
            api_root: "https://auth.example.com/api/yggdrasil".to_string(),
            allow_insecure_http: false,
        });
        config.save(&path).unwrap();
        config.network.concurrency = 4;
        config.save(&path).unwrap();

        let loaded = GlobalConfig::load(&path).unwrap();
        assert_eq!(loaded.network.concurrency, 4);
        assert_eq!(loaded.yggdrasil.providers[0].id, "private");
    }

    #[test]
    fn yggdrasil_provider_commands_preserve_unrelated_comments() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "schema = 2\n# keep me\n[network]\nconcurrency = 4\n").unwrap();
        add_yggdrasil_provider(
            &path,
            YggdrasilProviderConfig {
                id: "private".to_string(),
                api_root: "https://auth.example/api/yggdrasil".to_string(),
                allow_insecure_http: false,
            },
        )
        .unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# keep me")
        );
        let removed = remove_yggdrasil_provider(&path, "private").unwrap();
        assert_eq!(removed.id, "private");
    }

    #[test]
    fn cache_size_is_strict_and_overflow_checked() {
        assert_eq!(parse_byte_size("20 GiB").unwrap(), 20 * 1024_u64.pow(3));
        assert!(parse_byte_size("20 GB").is_err());
        assert!(parse_byte_size("0 GiB").is_err());
        assert!(parse_byte_size("18446744073709551615 TiB").is_err());
    }

    #[test]
    fn config_mutations_are_typed_and_preserve_comments() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "schema = 2\n\n[network]\n# Keep this explanation.\nconcurrency = 6\n",
        )
        .unwrap();

        let mutation = set_config(&path, ConfigKey::NetworkConcurrency, "12").unwrap();
        assert_eq!(mutation.previous.as_deref(), Some("6"));
        assert_eq!(mutation.current.as_deref(), Some("12"));
        assert!(mutation.explicit);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# Keep this explanation.")
        );

        let mutation = unset_config(&path, ConfigKey::NetworkConcurrency).unwrap();
        assert_eq!(mutation.previous.as_deref(), Some("12"));
        assert_eq!(mutation.current.as_deref(), Some("8"));
        assert!(!mutation.explicit);
    }

    #[test]
    fn first_config_mutation_only_marks_the_selected_key_explicit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        set_config(&path, ConfigKey::UiColor, "never").unwrap();

        let entries = list_config(&path).unwrap();
        assert!(
            entries
                .iter()
                .find(|entry| entry.key == ConfigKey::UiColor)
                .unwrap()
                .explicit
        );
        assert!(
            !entries
                .iter()
                .find(|entry| entry.key == ConfigKey::NetworkConcurrency)
                .unwrap()
                .explicit
        );
        assert!(set_config(&path, ConfigKey::NetworkConcurrency, "0").is_err());
    }

    #[test]
    fn unsupported_java_providers_are_not_exposed_as_configuration() {
        assert!(ConfigKey::from_str("java.default-provider").is_err());
        assert!(
            toml::from_str::<GlobalConfig>("schema = 2\n[java]\ndefault_provider = \"temurin\"\n")
                .is_err()
        );
    }
}
