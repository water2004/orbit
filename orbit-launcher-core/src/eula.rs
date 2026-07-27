use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;

pub const MINECRAFT_EULA_URL: &str = "https://www.minecraft.net/en-us/eula";
const STATE_DIRECTORY: &str = ".orbit-launcher";
const SHOWN_EULA_FILE: &str = "eula-shown.json";
const ACCEPTED_EULA_FILE: &str = "eula-acceptance.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EulaDocument {
    pub url: String,
    pub digest_sha256: String,
    pub fetched_at_unix_seconds: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EulaAcceptanceMethod {
    InteractivePrompt,
    DigestCommand,
}

impl EulaAcceptanceMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InteractivePrompt => "interactive-prompt",
            Self::DigestCommand => "digest-command",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EulaAcceptance {
    pub url: String,
    pub digest_sha256: String,
    pub accepted_at_unix_seconds: u64,
    pub method: EulaAcceptanceMethod,
}

pub async fn show_current_eula(
    instance_root: &Path,
    client: &reqwest::Client,
) -> Result<EulaDocument, LauncherError> {
    let response = client.get(MINECRAFT_EULA_URL).send().await?;
    let response = response.error_for_status()?;
    let final_url = response.url().clone();
    if final_url.scheme() != "https" || final_url.host_str() != Some("www.minecraft.net") {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Minecraft EULA redirected to untrusted URL '{final_url}'"
        )));
    }
    let html = response.text().await?;
    let text = extract_eula_text(&html)?;
    let document = EulaDocument::new(final_url.to_string(), text)?;
    write_json(&shown_path(instance_root), &document)?;
    Ok(document)
}

pub fn accept_shown_eula(
    instance_root: &Path,
    digest_sha256: &str,
    method: EulaAcceptanceMethod,
) -> Result<EulaAcceptance, LauncherError> {
    validate_sha256(digest_sha256)?;
    let shown: EulaDocument = read_json(&shown_path(instance_root)).map_err(|error| {
        LauncherError::EulaRequired(format!(
            "show the current EULA before accepting digest '{digest_sha256}': {error}"
        ))
    })?;
    shown.validate()?;
    if shown.digest_sha256 != digest_sha256 {
        return Err(LauncherError::EulaRequired(format!(
            "digest '{digest_sha256}' is not the EULA most recently shown for this instance"
        )));
    }
    let acceptance = EulaAcceptance {
        url: shown.url,
        digest_sha256: shown.digest_sha256,
        accepted_at_unix_seconds: unix_seconds()?,
        method,
    };
    write_json(&acceptance_path(instance_root), &acceptance)?;
    Ok(acceptance)
}

pub fn require_current_acceptance(
    instance_root: &Path,
    current: &EulaDocument,
) -> Result<EulaAcceptance, LauncherError> {
    current.validate()?;
    let acceptance: EulaAcceptance = read_json(&acceptance_path(instance_root)).map_err(|_| {
        LauncherError::EulaRequired(format!(
            "the current Minecraft EULA ({}) has not been accepted",
            current.digest_sha256
        ))
    })?;
    if acceptance.url != current.url || acceptance.digest_sha256 != current.digest_sha256 {
        return Err(LauncherError::EulaRequired(format!(
            "the Minecraft EULA has changed; show and accept digest '{}'",
            current.digest_sha256
        )));
    }
    Ok(acceptance)
}

impl EulaDocument {
    fn new(url: String, mut text: String) -> Result<Self, LauncherError> {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let document = Self {
            url,
            digest_sha256: hex::encode(Sha256::digest(text.as_bytes())),
            fetched_at_unix_seconds: unix_seconds()?,
            text,
        };
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> Result<(), LauncherError> {
        validate_sha256(&self.digest_sha256)?;
        let actual = hex::encode(Sha256::digest(self.text.as_bytes()));
        if actual != self.digest_sha256 {
            return Err(LauncherError::InvalidRemoteData(
                "stored Minecraft EULA text does not match its SHA-256 digest".to_string(),
            ));
        }
        if self.url != MINECRAFT_EULA_URL
            || self.text.len() < 4_000
            || !self
                .text
                .contains("Minecraft End(er)-User License Agreement")
            || !self.text.contains("COMPANY INFORMATION")
        {
            return Err(LauncherError::InvalidRemoteData(
                "official Minecraft EULA page did not contain the complete expected document"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

fn extract_eula_text(html: &str) -> Result<String, LauncherError> {
    let page = Html::parse_document(html);
    let selector = Selector::parse("#main-content .MC_Link_Style_RichText").map_err(|error| {
        LauncherError::InvalidRemoteData(format!("invalid internal EULA selector: {error}"))
    })?;
    let mut matches = page.select(&selector);
    let content = matches.next().ok_or_else(|| {
        LauncherError::InvalidRemoteData(
            "official Minecraft EULA content container was not found".to_string(),
        )
    })?;
    if matches.next().is_some() {
        return Err(LauncherError::InvalidRemoteData(
            "official Minecraft EULA page contained multiple legal documents".to_string(),
        ));
    }
    let text = html2text::from_read(content.inner_html().as_bytes(), 100).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "failed to render the official Minecraft EULA: {error}"
        ))
    })?;
    Ok(text.trim().to_string())
}

fn shown_path(instance_root: &Path) -> PathBuf {
    instance_root.join(STATE_DIRECTORY).join(SHOWN_EULA_FILE)
}

fn acceptance_path(instance_root: &Path) -> PathBuf {
    instance_root.join(STATE_DIRECTORY).join(ACCEPTED_EULA_FILE)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), LauncherError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("failed to serialize EULA state: {error}"))
    })?;
    write_atomic(path, &bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, LauncherError> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::InvalidRemoteData(format!("failed to parse EULA state: {error}"))
    })
}

fn validate_sha256(value: &str) -> Result<(), LauncherError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::EulaRequired(format!(
            "'{value}' is not a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

fn unix_seconds() -> Result<u64, LauncherError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            LauncherError::InvalidConfig(format!("system clock is before the Unix epoch: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_text() -> String {
        format!(
            "Minecraft End(er)-User License Agreement (“EULA”)\n\n{}\nCOMPANY INFORMATION\nMojang AB\n",
            "All terms apply. ".repeat(300)
        )
    }

    #[test]
    fn acceptance_only_applies_to_the_most_recently_shown_digest() {
        let directory = tempfile::tempdir().unwrap();
        let document = EulaDocument::new(MINECRAFT_EULA_URL.to_string(), complete_text()).unwrap();
        write_json(&shown_path(directory.path()), &document).unwrap();

        assert!(
            accept_shown_eula(
                directory.path(),
                &"0".repeat(64),
                EulaAcceptanceMethod::DigestCommand
            )
            .is_err()
        );
        let acceptance = accept_shown_eula(
            directory.path(),
            &document.digest_sha256,
            EulaAcceptanceMethod::DigestCommand,
        )
        .unwrap();
        assert_eq!(
            require_current_acceptance(directory.path(), &document).unwrap(),
            acceptance
        );
    }

    #[test]
    fn html_extraction_rejects_partial_legal_pages() {
        let partial = r#"<main id="main-content"><div class="MC_Link_Style_RichText"><h1>Minecraft End(er)-User License Agreement</h1></div></main>"#;
        let text = extract_eula_text(partial).unwrap();
        assert!(EulaDocument::new(MINECRAFT_EULA_URL.to_string(), text).is_err());
    }
}
