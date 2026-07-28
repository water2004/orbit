use super::{
    AccountLaunchIdentity, AccountMetadata, AccountProvider, AccountSecret, now_unix_seconds,
    parse_profile_id, persist_authenticated_account,
};
use crate::config::{GlobalConfig, YggdrasilProviderConfig};
use crate::error::LauncherError;
use crate::runtime::RuntimePaths;
use crate::secret_store::SecretStore;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

const MAX_AUTH_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct ExternalYggdrasilLoginRequest<'a> {
    pub provider_id: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub profile_selector: Option<&'a str>,
}

pub async fn login_external_yggdrasil(
    paths: &RuntimePaths,
    config: &GlobalConfig,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
    request: ExternalYggdrasilLoginRequest<'_>,
) -> Result<AccountMetadata, LauncherError> {
    let provider = super::find_yggdrasil_provider(config, request.provider_id)?;
    validate_login_name(request.username)?;
    if request.password.is_empty() {
        return Err(LauncherError::Authentication(
            "Yggdrasil password must not be empty".to_string(),
        ));
    }
    let client_token = uuid::Uuid::new_v4().simple().to_string();
    let response: AuthenticationResponse = post_json(
        client,
        provider,
        AuthserverOperation::Authenticate,
        &AuthenticateRequest {
            agent: Agent {
                name: "Minecraft",
                version: 1,
            },
            username: request.username,
            password: request.password,
            client_token: &client_token,
            request_user: true,
        },
    )
    .await?;
    ensure_client_token(&client_token, &response.client_token)?;

    let session = if let Some(profile) = response.selected_profile {
        if let Some(selector) = request.profile_selector
            && !profile.matches(selector)
        {
            return Err(LauncherError::Authentication(format!(
                "Yggdrasil selected profile '{}' but --profile requested '{selector}'",
                profile.name
            )));
        }
        AuthenticatedSession {
            access_token: response.access_token,
            client_token: response.client_token,
            selected_profile: profile,
        }
    } else {
        let profiles = response.available_profiles.unwrap_or_default();
        let selected = select_profile(&profiles, request.profile_selector)?;
        refresh_session(
            client,
            provider,
            &response.access_token,
            &response.client_token,
            Some(selected),
        )
        .await?
    };
    let profile_id = parse_profile_id(&session.selected_profile.id, "Yggdrasil")?;
    let skin_url = fetch_profile_skin(client, provider, profile_id).await?;
    let now = now_unix_seconds()?;
    let secret = AccountSecret::ExternalYggdrasil {
        access_token: session.access_token,
        client_token: session.client_token,
    };
    let account = persist_authenticated_account(
        paths,
        secrets,
        AccountMetadata {
            id: uuid::Uuid::new_v4(),
            provider: AccountProvider::ExternalYggdrasil {
                provider_id: provider.id.clone(),
            },
            profile_id,
            profile_name: session.selected_profile.name,
            authentication_state: super::AccountAuthenticationState::Active,
            skin_url,
            login_name: Some(request.username.to_string()),
            created_at_unix_seconds: now,
            last_authenticated_at_unix_seconds: Some(now),
        },
        &secret,
    )
    .await?;
    let _ = super::ensure_account_avatar(paths, client, &account).await;
    Ok(account)
}

pub(super) async fn resolve_yggdrasil_identity(
    paths: &RuntimePaths,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
    account: AccountMetadata,
    provider: &YggdrasilProviderConfig,
) -> Result<AccountLaunchIdentity, LauncherError> {
    let secret = super::load_account_secret(secrets, account.id).await?;
    let AccountSecret::ExternalYggdrasil {
        access_token,
        client_token,
    } = &secret
    else {
        return Err(LauncherError::Authentication(format!(
            "stored session kind does not match External Yggdrasil account '{}'",
            account.id
        )));
    };
    let prefetched_metadata = crate::yggdrasil::fetch_provider_metadata(client, provider).await?;
    let api_root = crate::yggdrasil::provider_api_root(provider)?.to_string();
    if validate_session(client, provider, access_token, client_token).await? {
        return Ok(identity(
            &account,
            access_token.clone(),
            &provider.id,
            api_root,
            prefetched_metadata,
        ));
    }
    let refreshed = refresh_session(client, provider, access_token, client_token, None)
        .await
        .map_err(|error| match error {
            LauncherError::InteractionRequired(detail) => LauncherError::ReauthenticationRequired {
                account_id: account.id,
                detail,
            },
            other => other,
        })?;
    let profile_id = parse_profile_id(&refreshed.selected_profile.id, "Yggdrasil")?;
    if profile_id != account.profile_id {
        return Err(LauncherError::Authentication(format!(
            "Yggdrasil refresh changed profile from '{}' to '{}'",
            account.profile_id, profile_id
        )));
    }
    let skin_url = fetch_profile_skin(client, provider, profile_id).await?;
    let mut updated = account;
    updated.profile_name = refreshed.selected_profile.name;
    updated.skin_url = skin_url;
    updated.last_authenticated_at_unix_seconds = Some(now_unix_seconds()?);
    let access_token = refreshed.access_token.clone();
    let secret = AccountSecret::ExternalYggdrasil {
        access_token: refreshed.access_token,
        client_token: refreshed.client_token,
    };
    let updated = persist_authenticated_account(paths, secrets, updated, &secret).await?;
    let _ = super::ensure_account_avatar(paths, client, &updated).await;
    Ok(identity(
        &updated,
        access_token,
        &provider.id,
        api_root,
        prefetched_metadata,
    ))
}

fn identity(
    account: &AccountMetadata,
    access_token: String,
    provider_id: &str,
    api_root: String,
    prefetched_metadata: String,
) -> AccountLaunchIdentity {
    AccountLaunchIdentity {
        account_id: account.id,
        profile_id: account.profile_id,
        profile_name: account.profile_name.clone(),
        user_type: "mojang".to_string(),
        user_properties: "{}".to_string(),
        access_token,
        yggdrasil_provider: Some(provider_id.to_string()),
        yggdrasil_api_root: Some(api_root),
        yggdrasil_prefetched_metadata: Some(prefetched_metadata),
    }
}

pub(super) async fn fetch_profile_skin(
    client: &reqwest::Client,
    provider: &YggdrasilProviderConfig,
    profile_id: uuid::Uuid,
) -> Result<Option<String>, LauncherError> {
    let endpoint = crate::yggdrasil::provider_api_root(provider)?
        .join(&format!(
            "sessionserver/session/minecraft/profile/{}?unsigned=true",
            profile_id.simple()
        ))
        .map_err(|error| {
            LauncherError::Authentication(format!(
                "cannot build Yggdrasil profile endpoint: {error}"
            ))
        })?;
    let response = client.get(endpoint).send().await?;
    if matches!(response.status().as_u16(), 204 | 404) {
        return Ok(None);
    }
    let status = response.status();
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_AUTH_RESPONSE_BYTES {
        return Err(LauncherError::Authentication(
            "Yggdrasil profile response is too large".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(LauncherError::Authentication(format!(
            "Yggdrasil profile lookup failed with HTTP {}",
            status.as_u16()
        )));
    }
    let profile: SessionProfile = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Authentication(format!(
            "Yggdrasil profile response is invalid JSON: {error}"
        ))
    })?;
    let Some(encoded) = profile
        .properties
        .iter()
        .find(|property| property.name == "textures")
        .map(|property| property.value.as_str())
    else {
        return Ok(None);
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| {
            LauncherError::Authentication(format!(
                "Yggdrasil textures property is not valid Base64: {error}"
            ))
        })?;
    let payload: TexturePayload = serde_json::from_slice(&decoded).map_err(|error| {
        LauncherError::Authentication(format!(
            "Yggdrasil textures property is invalid JSON: {error}"
        ))
    })?;
    let Some(url) = payload.textures.skin.map(|skin| skin.url) else {
        return Ok(None);
    };
    Ok(super::normalize_skin_url(&url))
}

async fn validate_session(
    client: &reqwest::Client,
    provider: &YggdrasilProviderConfig,
    access_token: &str,
    client_token: &str,
) -> Result<bool, LauncherError> {
    let response = client
        .post(authserver_endpoint(
            provider,
            AuthserverOperation::Validate,
        )?)
        .json(&CredentialRequest {
            access_token,
            client_token,
        })
        .send()
        .await?;
    match response.status().as_u16() {
        204 => Ok(true),
        400 | 401 | 403 => Ok(false),
        _ => Err(authentication_http_error("Yggdrasil validate", response).await),
    }
}

async fn refresh_session(
    client: &reqwest::Client,
    provider: &YggdrasilProviderConfig,
    access_token: &str,
    client_token: &str,
    selected_profile: Option<&GameProfile>,
) -> Result<AuthenticatedSession, LauncherError> {
    let response: AuthenticationResponse = post_json(
        client,
        provider,
        AuthserverOperation::Refresh,
        &RefreshRequest {
            access_token,
            client_token,
            request_user: true,
            selected_profile,
        },
    )
    .await
    .map_err(|error| match error {
        LauncherError::InteractionRequired(message) => LauncherError::InteractionRequired(format!(
            "External Yggdrasil session cannot be refreshed ({message}); log in again with the account password"
        )),
        other => other,
    })?;
    ensure_client_token(client_token, &response.client_token)?;
    let profile = response.selected_profile.ok_or_else(|| {
        LauncherError::Authentication(
            "Yggdrasil refresh response did not select a profile".to_string(),
        )
    })?;
    if let Some(expected) = selected_profile
        && profile.id != expected.id
    {
        return Err(LauncherError::Authentication(
            "Yggdrasil refresh selected a different profile".to_string(),
        ));
    }
    Ok(AuthenticatedSession {
        access_token: response.access_token,
        client_token: response.client_token,
        selected_profile: profile,
    })
}

async fn post_json<T, R>(
    client: &reqwest::Client,
    provider: &YggdrasilProviderConfig,
    operation: AuthserverOperation,
    request: &T,
) -> Result<R, LauncherError>
where
    T: Serialize + ?Sized,
    R: for<'de> Deserialize<'de>,
{
    let response = client
        .post(authserver_endpoint(provider, operation)?)
        .json(request)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_AUTH_RESPONSE_BYTES {
        return Err(LauncherError::Authentication(format!(
            "Yggdrasil {} response exceeds {MAX_AUTH_RESPONSE_BYTES} bytes",
            operation.name()
        )));
    }
    if !status.is_success() {
        let error = parse_remote_error(operation.name(), status.as_u16(), &bytes);
        if operation == AuthserverOperation::Refresh && matches!(status.as_u16(), 400 | 401 | 403) {
            return Err(LauncherError::InteractionRequired(error.to_string()));
        }
        return Err(error);
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Authentication(format!(
            "Yggdrasil {} returned invalid JSON: {error}",
            operation.name()
        ))
    })
}

fn authserver_endpoint(
    provider: &YggdrasilProviderConfig,
    operation: AuthserverOperation,
) -> Result<url::Url, LauncherError> {
    crate::yggdrasil::provider_api_root(provider)?
        .join(operation.relative_path())
        .map_err(|error| {
            LauncherError::Authentication(format!(
                "Yggdrasil provider '{}' has an invalid API root: {error}",
                provider.id
            ))
        })
}

fn select_profile<'a>(
    profiles: &'a [GameProfile],
    selector: Option<&str>,
) -> Result<&'a GameProfile, LauncherError> {
    if profiles.is_empty() {
        return Err(LauncherError::Authentication(
            "Yggdrasil account has no Minecraft profiles".to_string(),
        ));
    }
    if let Some(selector) = selector {
        return profiles
            .iter()
            .find(|profile| profile.matches(selector))
            .ok_or_else(|| {
                LauncherError::Authentication(format!(
                    "Yggdrasil account has no profile matching '{selector}'"
                ))
            });
    }
    if profiles.len() == 1 {
        return Ok(&profiles[0]);
    }
    let choices = profiles
        .iter()
        .map(|profile| format!("{} ({})", profile.name, profile.id))
        .collect::<Vec<_>>()
        .join(", ");
    Err(LauncherError::InteractionRequired(format!(
        "Yggdrasil account has multiple profiles; retry with --profile <name-or-uuid>: {choices}"
    )))
}

fn ensure_client_token(expected: &str, actual: &str) -> Result<(), LauncherError> {
    if actual != expected {
        return Err(LauncherError::Authentication(
            "Yggdrasil changed the client token unexpectedly".to_string(),
        ));
    }
    Ok(())
}

fn validate_login_name(value: &str) -> Result<(), LauncherError> {
    if value.is_empty()
        || value.len() > 320
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(LauncherError::Authentication(
            "Yggdrasil login name is invalid".to_string(),
        ));
    }
    Ok(())
}

fn parse_remote_error(path: &str, status: u16, bytes: &[u8]) -> LauncherError {
    let parsed = serde_json::from_slice::<RemoteError>(bytes).ok();
    let remote = parsed
        .as_ref()
        .and_then(|error| error.error.as_deref())
        .unwrap_or("remote authentication error");
    let message = parsed
        .as_ref()
        .and_then(|error| error.error_message.as_deref())
        .unwrap_or("no error description was returned");
    LauncherError::Authentication(format!(
        "Yggdrasil {path} failed with HTTP {status}: {remote}: {message}"
    ))
}

async fn authentication_http_error(subject: &str, response: reqwest::Response) -> LauncherError {
    let status = response.status().as_u16();
    let bytes = response.bytes().await.unwrap_or_default();
    let bytes = &bytes[..bytes.len().min(MAX_AUTH_RESPONSE_BYTES)];
    let remote = serde_json::from_slice::<RemoteError>(bytes).ok();
    let code = remote
        .as_ref()
        .and_then(|error| error.error.as_deref())
        .unwrap_or("remote_error");
    let message = remote
        .as_ref()
        .and_then(|error| error.error_message.as_deref())
        .unwrap_or("no error description was returned");
    LauncherError::Authentication(format!(
        "{subject} failed with HTTP {status}: {code}: {message}"
    ))
}

struct AuthenticatedSession {
    access_token: String,
    client_token: String,
    selected_profile: GameProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthserverOperation {
    Authenticate,
    Refresh,
    Validate,
}

impl AuthserverOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Authenticate => "authenticate",
            Self::Refresh => "refresh",
            Self::Validate => "validate",
        }
    }

    const fn relative_path(self) -> &'static str {
        match self {
            Self::Authenticate => "authserver/authenticate",
            Self::Refresh => "authserver/refresh",
            Self::Validate => "authserver/validate",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticateRequest<'a> {
    agent: Agent<'a>,
    username: &'a str,
    password: &'a str,
    client_token: &'a str,
    request_user: bool,
}

#[derive(Serialize)]
struct Agent<'a> {
    name: &'a str,
    version: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialRequest<'a> {
    access_token: &'a str,
    client_token: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshRequest<'a> {
    access_token: &'a str,
    client_token: &'a str,
    request_user: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_profile: Option<&'a GameProfile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticationResponse {
    access_token: String,
    client_token: String,
    selected_profile: Option<GameProfile>,
    available_profiles: Option<Vec<GameProfile>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GameProfile {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct SessionProfile {
    #[serde(default)]
    properties: Vec<ProfileProperty>,
}

#[derive(Deserialize)]
struct ProfileProperty {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct TexturePayload {
    #[serde(default)]
    textures: ProfileTextures,
}

#[derive(Default, Deserialize)]
struct ProfileTextures {
    #[serde(rename = "SKIN")]
    skin: Option<ProfileSkin>,
}

#[derive(Deserialize)]
struct ProfileSkin {
    url: String,
}

impl GameProfile {
    fn matches(&self, selector: &str) -> bool {
        self.name == selector
            || self.id == selector
            || parse_profile_id(&self.id, "Yggdrasil").is_ok_and(|id| id.to_string() == selector)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteError {
    error: Option<String>,
    error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_selection_requires_an_explicit_choice_when_ambiguous() {
        let profiles = vec![
            GameProfile {
                id: "069a79f444e94726a5befca90e38aaf5".to_string(),
                name: "Notch".to_string(),
            },
            GameProfile {
                id: "853c80ef3c3749fdaa49938b674adae6".to_string(),
                name: "jeb_".to_string(),
            },
        ];
        let error = select_profile(&profiles, None).unwrap_err();
        assert_eq!(error.code(), "interaction_required");
        assert_eq!(
            select_profile(&profiles, Some("Notch")).unwrap().name,
            "Notch"
        );
    }

    #[test]
    fn authserver_endpoints_preserve_the_configured_yggdrasil_root_path() {
        let provider = YggdrasilProviderConfig {
            id: "private".to_string(),
            api_root: "https://example.com/api/yggdrasil".to_string(),
            allow_insecure_http: false,
        };
        assert_eq!(
            crate::yggdrasil::provider_api_root(&provider)
                .unwrap()
                .as_str(),
            "https://example.com/api/yggdrasil/"
        );
        for (operation, expected) in [
            (
                AuthserverOperation::Authenticate,
                "https://example.com/api/yggdrasil/authserver/authenticate",
            ),
            (
                AuthserverOperation::Refresh,
                "https://example.com/api/yggdrasil/authserver/refresh",
            ),
            (
                AuthserverOperation::Validate,
                "https://example.com/api/yggdrasil/authserver/validate",
            ),
        ] {
            assert_eq!(
                authserver_endpoint(&provider, operation).unwrap().as_str(),
                expected
            );
        }
    }
}
