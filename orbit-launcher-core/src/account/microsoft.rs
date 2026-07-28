use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::{
    AccountLaunchIdentity, AccountMetadata, AccountProvider, AccountSecret, now_unix_seconds,
    parse_profile_id, persist_authenticated_account,
};
use crate::atomic_io::write_atomic;
use crate::config::GlobalConfig;
use crate::error::LauncherError;
use crate::runtime::RuntimePaths;
use crate::secret_store::SecretStore;

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX_USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MINECRAFT_ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const MINECRAFT_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const MICROSOFT_SCOPE: &str = "XboxLive.signin offline_access";
const AUTH_SESSION_SCHEMA: u32 = 1;
const MAX_AUTH_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MicrosoftDeviceSession {
    pub id: Uuid,
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at_unix_seconds: u64,
    pub polling_interval_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrosoftLoginProgressEvent {
    Polling {
        attempt: u64,
        elapsed_seconds: u64,
        expires_at_unix_seconds: u64,
    },
    AuthorizationReceived,
    XboxAuthenticated,
    MinecraftAuthenticated,
    SessionStored {
        account_id: Uuid,
    },
}

pub async fn begin_microsoft_device_login(
    paths: &RuntimePaths,
    config: &GlobalConfig,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
) -> Result<MicrosoftDeviceSession, LauncherError> {
    let client_id = microsoft_client_id(config)?;
    let response = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", MICROSOFT_SCOPE)])
        .send()
        .await?;
    let response: DeviceCodeResponse =
        decode_success("Microsoft device authorization", response).await?;
    if response.device_code.is_empty()
        || response.user_code.is_empty()
        || response.expires_in == 0
        || response.interval == 0
    {
        return Err(LauncherError::Authentication(
            "Microsoft device authorization returned incomplete data".to_string(),
        ));
    }
    let verification = url::Url::parse(&response.verification_uri).map_err(|error| {
        LauncherError::Authentication(format!(
            "Microsoft returned an invalid verification URI: {error}"
        ))
    })?;
    if verification.scheme() != "https" || verification.host_str().is_none() {
        return Err(LauncherError::Authentication(
            "Microsoft verification URI must use HTTPS".to_string(),
        ));
    }
    let now = now_unix_seconds()?;
    let expires_at = now.checked_add(response.expires_in).ok_or_else(|| {
        LauncherError::Authentication("Microsoft device session expiry overflowed".to_string())
    })?;
    let public = MicrosoftDeviceSession {
        id: Uuid::new_v4(),
        verification_uri: response.verification_uri,
        user_code: response.user_code,
        expires_at_unix_seconds: expires_at,
        polling_interval_seconds: response.interval,
        message: response.message,
    };
    let private = DeviceCodeSecret {
        client_id: client_id.to_string(),
        device_code: response.device_code,
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&private).map_err(|error| {
        LauncherError::Authentication(format!(
            "failed to serialize Microsoft login session: {error}"
        ))
    })?);
    secrets
        .replace(&device_secret_key(public.id), bytes.as_slice())
        .await?;
    if let Err(error) = save_device_session(paths, &public) {
        let _ = secrets.delete(&device_secret_key(public.id)).await;
        return Err(error);
    }
    Ok(public)
}

pub async fn complete_microsoft_device_login<F>(
    paths: &RuntimePaths,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
    session_id: Uuid,
    mut progress: F,
) -> Result<AccountMetadata, LauncherError>
where
    F: FnMut(MicrosoftLoginProgressEvent),
{
    let public = load_device_session(paths, session_id)?;
    let private_bytes = Zeroizing::new(
        secrets
            .load(&device_secret_key(session_id))
            .await?
            .ok_or_else(|| {
                LauncherError::Authentication(format!(
                    "Microsoft login session '{session_id}' has no private device code"
                ))
            })?,
    );
    let private: DeviceCodeSecret = serde_json::from_slice(&private_bytes).map_err(|error| {
        LauncherError::Authentication(format!(
            "Microsoft login session '{session_id}' is invalid: {error}"
        ))
    })?;
    let started = now_unix_seconds()?;
    if started >= public.expires_at_unix_seconds {
        cleanup_device_session(paths, secrets, session_id).await?;
        return Err(LauncherError::Authentication(
            "Microsoft device authorization has expired; begin a new login".to_string(),
        ));
    }
    let mut interval = public.polling_interval_seconds;
    let mut attempt = 0;
    let oauth = loop {
        attempt += 1;
        progress(MicrosoftLoginProgressEvent::Polling {
            attempt,
            elapsed_seconds: now_unix_seconds()?.saturating_sub(started),
            expires_at_unix_seconds: public.expires_at_unix_seconds,
        });
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if now_unix_seconds()? >= public.expires_at_unix_seconds {
            cleanup_device_session(paths, secrets, session_id).await?;
            return Err(LauncherError::Authentication(
                "Microsoft device authorization expired before it was approved".to_string(),
            ));
        }
        let response = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", private.client_id.as_str()),
                ("device_code", private.device_code.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        let token: OAuthTokenResponse = decode_json("Microsoft device token", response).await?;
        if status.is_success() {
            break token.require_tokens()?;
        }
        match token.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = interval.saturating_add(5);
            }
            Some("authorization_declined") => {
                cleanup_device_session(paths, secrets, session_id).await?;
                return Err(LauncherError::Authentication(
                    "Microsoft device authorization was declined".to_string(),
                ));
            }
            Some("expired_token") => {
                cleanup_device_session(paths, secrets, session_id).await?;
                return Err(LauncherError::Authentication(
                    "Microsoft device authorization expired".to_string(),
                ));
            }
            Some(error) => {
                return Err(LauncherError::Authentication(format!(
                    "Microsoft device authorization failed: {error}: {}",
                    token
                        .error_description
                        .as_deref()
                        .unwrap_or("no error description was returned")
                )));
            }
            None => {
                return Err(LauncherError::Authentication(format!(
                    "Microsoft device token endpoint returned HTTP {} without an OAuth error",
                    status.as_u16()
                )));
            }
        }
    };
    progress(MicrosoftLoginProgressEvent::AuthorizationReceived);
    let session =
        exchange_microsoft_token(client, oauth.access_token.as_str(), &mut progress).await?;
    let profile_id = parse_profile_id(&session.profile.id, "Microsoft")?;
    let now = now_unix_seconds()?;
    let secret = AccountSecret::Microsoft {
        refresh_token: oauth.refresh_token,
        minecraft_access_token: session.access_token.to_string(),
        expires_at_unix_seconds: session.expires_at_unix_seconds,
        token_type: session.token_type,
    };
    let skin_url = session.profile.skin_url();
    let account = persist_authenticated_account(
        paths,
        secrets,
        AccountMetadata {
            id: Uuid::new_v4(),
            provider: AccountProvider::Microsoft,
            profile_id,
            profile_name: session.profile.name,
            authentication_state: super::AccountAuthenticationState::Active,
            skin_url,
            login_name: None,
            created_at_unix_seconds: now,
            last_authenticated_at_unix_seconds: Some(now),
        },
        &secret,
    )
    .await?;
    cleanup_device_session(paths, secrets, session_id).await?;
    progress(MicrosoftLoginProgressEvent::SessionStored {
        account_id: account.id,
    });
    Ok(account)
}

pub(super) async fn resolve_microsoft_identity(
    paths: &RuntimePaths,
    config: &GlobalConfig,
    client: &reqwest::Client,
    secrets: &dyn SecretStore,
    account: AccountMetadata,
) -> Result<AccountLaunchIdentity, LauncherError> {
    let secret = super::load_account_secret(secrets, account.id).await?;
    let AccountSecret::Microsoft {
        refresh_token,
        minecraft_access_token,
        expires_at_unix_seconds,
        token_type,
    } = &secret
    else {
        return Err(LauncherError::Authentication(format!(
            "stored session kind does not match Microsoft account '{}'",
            account.id
        )));
    };
    let now = now_unix_seconds()?;
    if *expires_at_unix_seconds > now.saturating_add(60) {
        match get_minecraft_profile(client, token_type, minecraft_access_token).await? {
            ProfileLookup::Found(profile) => {
                let profile_id = parse_profile_id(&profile.id, "Microsoft")?;
                if profile_id != account.profile_id {
                    return Err(LauncherError::Authentication(
                        "stored Microsoft session resolves to a different Minecraft profile"
                            .to_string(),
                    ));
                }
                return Ok(identity(&account, minecraft_access_token.clone()));
            }
            ProfileLookup::Unauthorized => {}
        }
    }

    let client_id = microsoft_client_id(config)?;
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", MICROSOFT_SCOPE),
        ])
        .send()
        .await?;
    let status = response.status();
    let oauth: OAuthTokenResponse = decode_json("Microsoft token refresh", response).await?;
    if !status.is_success() {
        let detail = oauth
            .error_description
            .as_deref()
            .or(oauth.error.as_deref())
            .unwrap_or("the token endpoint rejected the request");
        if matches!(status.as_u16(), 400 | 401)
            && matches!(
                oauth.error.as_deref(),
                Some("invalid_grant" | "interaction_required" | "invalid_request")
            )
        {
            return Err(LauncherError::ReauthenticationRequired {
                account_id: account.id,
                detail: detail.to_string(),
            });
        }
        return Err(LauncherError::Authentication(format!(
            "Microsoft token refresh failed with HTTP {}: {detail}",
            status.as_u16()
        )));
    }
    let oauth = oauth.require_tokens_with_fallback(refresh_token.clone())?;
    let mut no_progress = |_| {};
    let session =
        exchange_microsoft_token(client, oauth.access_token.as_str(), &mut no_progress).await?;
    let profile_id = parse_profile_id(&session.profile.id, "Microsoft")?;
    if profile_id != account.profile_id {
        return Err(LauncherError::Authentication(format!(
            "Microsoft refresh changed profile from '{}' to '{}'",
            account.profile_id, profile_id
        )));
    }
    let access_token = session.access_token.to_string();
    let skin_url = session.profile.skin_url();
    let mut updated = account;
    updated.profile_name = session.profile.name;
    updated.skin_url = skin_url;
    updated.last_authenticated_at_unix_seconds = Some(now_unix_seconds()?);
    let secret = AccountSecret::Microsoft {
        refresh_token: oauth.refresh_token,
        minecraft_access_token: session.access_token.to_string(),
        expires_at_unix_seconds: session.expires_at_unix_seconds,
        token_type: session.token_type,
    };
    let updated = persist_authenticated_account(paths, secrets, updated, &secret).await?;
    Ok(identity(&updated, access_token))
}

fn identity(account: &AccountMetadata, access_token: String) -> AccountLaunchIdentity {
    AccountLaunchIdentity {
        account_id: account.id,
        profile_id: account.profile_id,
        profile_name: account.profile_name.clone(),
        user_type: "msa".to_string(),
        user_properties: "{}".to_string(),
        access_token,
        yggdrasil_provider: None,
        yggdrasil_api_root: None,
        yggdrasil_prefetched_metadata: None,
    }
}

async fn exchange_microsoft_token<F>(
    client: &reqwest::Client,
    microsoft_access_token: &str,
    progress: &mut F,
) -> Result<MinecraftSession, LauncherError>
where
    F: FnMut(MicrosoftLoginProgressEvent),
{
    let xbox: XboxAuthenticationResponse = post_json(
        client,
        "Xbox Live authentication",
        XBOX_USER_AUTH_URL,
        &XboxUserAuthenticationRequest {
            properties: XboxUserProperties {
                auth_method: "RPS",
                site_name: "user.auth.xboxlive.com",
                rps_ticket: format!("d={microsoft_access_token}"),
            },
            relying_party: "http://auth.xboxlive.com",
            token_type: "JWT",
        },
    )
    .await?;
    let uhs = xbox.uhs()?;
    progress(MicrosoftLoginProgressEvent::XboxAuthenticated);
    let xsts: XboxAuthenticationResponse = post_json(
        client,
        "Xbox XSTS authorization",
        XSTS_AUTH_URL,
        &XstsRequest {
            properties: XstsProperties {
                sandbox_id: "RETAIL",
                user_tokens: vec![xbox.token],
            },
            relying_party: "rp://api.minecraftservices.com/",
            token_type: "JWT",
        },
    )
    .await?;
    if xsts.uhs()? != uhs {
        return Err(LauncherError::Authentication(
            "Xbox XSTS response changed the user hash".to_string(),
        ));
    }
    let login: MinecraftLoginResponse = post_json(
        client,
        "Minecraft Services authentication",
        MINECRAFT_LOGIN_URL,
        &MinecraftLoginRequest {
            identity_token: format!("XBL3.0 x={uhs};{}", xsts.token),
        },
    )
    .await?;
    if login.access_token.is_empty() || login.expires_in == 0 {
        return Err(LauncherError::Authentication(
            "Minecraft Services returned an incomplete session".to_string(),
        ));
    }
    let entitlements_response = client
        .get(MINECRAFT_ENTITLEMENTS_URL)
        .bearer_auth(&login.access_token)
        .send()
        .await?;
    let entitlements: EntitlementsResponse =
        decode_success("Minecraft ownership check", entitlements_response).await?;
    if entitlements.items.is_empty() {
        return Err(LauncherError::Authentication(
            "Microsoft account does not own Minecraft: Java Edition".to_string(),
        ));
    }
    let profile =
        match get_minecraft_profile(client, &login.token_type, &login.access_token).await? {
            ProfileLookup::Found(profile) => profile,
            ProfileLookup::Unauthorized => {
                return Err(LauncherError::Authentication(
                    "new Minecraft access token was rejected by the profile service".to_string(),
                ));
            }
        };
    progress(MicrosoftLoginProgressEvent::MinecraftAuthenticated);
    Ok(MinecraftSession {
        token_type: login.token_type,
        access_token: Zeroizing::new(login.access_token),
        expires_at_unix_seconds: now_unix_seconds()?
            .checked_add(login.expires_in)
            .ok_or_else(|| {
                LauncherError::Authentication("Minecraft session expiry overflowed".to_string())
            })?,
        profile,
    })
}

async fn get_minecraft_profile(
    client: &reqwest::Client,
    token_type: &str,
    access_token: &str,
) -> Result<ProfileLookup, LauncherError> {
    let response = client
        .get(MINECRAFT_PROFILE_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("{token_type} {access_token}"),
        )
        .send()
        .await?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Ok(ProfileLookup::Unauthorized);
    }
    decode_success("Minecraft profile", response)
        .await
        .map(ProfileLookup::Found)
}

async fn post_json<T, R>(
    client: &reqwest::Client,
    subject: &str,
    url: &str,
    request: &T,
) -> Result<R, LauncherError>
where
    T: Serialize + ?Sized,
    R: for<'de> Deserialize<'de>,
{
    let response = client.post(url).json(request).send().await?;
    decode_success(subject, response).await
}

async fn decode_success<R>(subject: &str, response: reqwest::Response) -> Result<R, LauncherError>
where
    R: for<'de> Deserialize<'de>,
{
    let status = response.status();
    let bytes = read_bounded_response(subject, response).await?;
    if !status.is_success() {
        let remote = serde_json::from_slice::<RemoteServiceError>(&bytes).ok();
        let code = remote
            .as_ref()
            .and_then(|error| error.error.as_deref())
            .map(str::to_string)
            .or_else(|| {
                remote
                    .as_ref()
                    .and_then(|error| error.xerr)
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| "remote_error".to_string());
        let description = remote
            .as_ref()
            .and_then(|error| {
                error
                    .error_description
                    .as_deref()
                    .or(error.message.as_deref())
            })
            .unwrap_or("no error description was returned");
        return Err(LauncherError::Authentication(format!(
            "{subject} failed with HTTP {}: {code}: {description}",
            status.as_u16(),
        )));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Authentication(format!("{subject} returned invalid JSON: {error}"))
    })
}

async fn decode_json<R>(subject: &str, response: reqwest::Response) -> Result<R, LauncherError>
where
    R: for<'de> Deserialize<'de>,
{
    let bytes = read_bounded_response(subject, response).await?;
    serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Authentication(format!("{subject} returned invalid JSON: {error}"))
    })
}

async fn read_bounded_response(
    subject: &str,
    response: reqwest::Response,
) -> Result<Vec<u8>, LauncherError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_AUTH_RESPONSE_BYTES as u64)
    {
        return Err(LauncherError::Authentication(format!(
            "{subject} response exceeds {MAX_AUTH_RESPONSE_BYTES} bytes"
        )));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_AUTH_RESPONSE_BYTES {
        return Err(LauncherError::Authentication(format!(
            "{subject} response exceeds {MAX_AUTH_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

fn microsoft_client_id(config: &GlobalConfig) -> Result<&str, LauncherError> {
    config
        .microsoft
        .client_id
        .as_deref()
        .or(option_env!("ORBIT_MICROSOFT_CLIENT_ID"))
        .ok_or_else(|| {
            LauncherError::Authentication(
                "Microsoft login requires microsoft.client-id or a release build with ORBIT_MICROSOFT_CLIENT_ID"
                    .to_string(),
            )
        })
}

fn device_session_path(paths: &RuntimePaths, id: Uuid) -> PathBuf {
    paths.auth_sessions_dir().join(format!("{id}.json"))
}

fn save_device_session(
    paths: &RuntimePaths,
    session: &MicrosoftDeviceSession,
) -> Result<(), LauncherError> {
    let document = StoredDeviceSession {
        schema: AUTH_SESSION_SCHEMA,
        session: session.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&document).map_err(|error| {
        LauncherError::Authentication(format!(
            "failed to serialize Microsoft login metadata: {error}"
        ))
    })?;
    bytes.push(b'\n');
    write_atomic(&device_session_path(paths, session.id), &bytes)
}

fn load_device_session(
    paths: &RuntimePaths,
    id: Uuid,
) -> Result<MicrosoftDeviceSession, LauncherError> {
    let path = device_session_path(paths, id);
    let document: StoredDeviceSession =
        serde_json::from_slice(&std::fs::read(&path)?).map_err(|error| {
            LauncherError::Authentication(format!(
                "failed to parse Microsoft login session '{}': {error}",
                path.display()
            ))
        })?;
    if document.schema != AUTH_SESSION_SCHEMA || document.session.id != id {
        return Err(LauncherError::Authentication(format!(
            "Microsoft login session '{id}' has invalid metadata"
        )));
    }
    Ok(document.session)
}

async fn cleanup_device_session(
    paths: &RuntimePaths,
    secrets: &dyn SecretStore,
    id: Uuid,
) -> Result<(), LauncherError> {
    secrets.delete(&device_secret_key(id)).await?;
    remove_if_exists(&device_session_path(paths, id))
}

fn remove_if_exists(path: &Path) -> Result<(), LauncherError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn device_secret_key(id: Uuid) -> String {
    format!("microsoft-device:{id}")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDeviceSession {
    schema: u32,
    session: MicrosoftDeviceSession,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct DeviceCodeSecret {
    client_id: String,
    device_code: String,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    message: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
struct OAuthTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

struct OAuthTokens {
    access_token: Zeroizing<String>,
    refresh_token: String,
}

impl OAuthTokenResponse {
    fn require_tokens(mut self) -> Result<OAuthTokens, LauncherError> {
        let access_token = self.access_token.take().filter(|token| !token.is_empty());
        let refresh_token = self.refresh_token.take().filter(|token| !token.is_empty());
        match (access_token, refresh_token) {
            (Some(access_token), Some(refresh_token)) => Ok(OAuthTokens {
                access_token: Zeroizing::new(access_token),
                refresh_token,
            }),
            _ => Err(LauncherError::Authentication(
                "Microsoft token response did not contain access and refresh tokens".to_string(),
            )),
        }
    }

    fn require_tokens_with_fallback(
        mut self,
        refresh_fallback: String,
    ) -> Result<OAuthTokens, LauncherError> {
        let access_token = self
            .access_token
            .take()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                LauncherError::Authentication(
                    "Microsoft refresh response did not contain an access token".to_string(),
                )
            })?;
        Ok(OAuthTokens {
            access_token: Zeroizing::new(access_token),
            refresh_token: self
                .refresh_token
                .take()
                .filter(|token| !token.is_empty())
                .unwrap_or(refresh_fallback),
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XboxUserAuthenticationRequest {
    properties: XboxUserProperties,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XboxUserProperties {
    auth_method: &'static str,
    site_name: &'static str,
    rps_ticket: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsRequest {
    properties: XstsProperties,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsProperties {
    sandbox_id: &'static str,
    user_tokens: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxAuthenticationResponse {
    token: String,
    display_claims: XboxDisplayClaims,
}

impl XboxAuthenticationResponse {
    fn uhs(&self) -> Result<String, LauncherError> {
        self.display_claims
            .xui
            .first()
            .map(|claim| claim.uhs.clone())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                LauncherError::Authentication(
                    "Xbox response did not contain a user hash".to_string(),
                )
            })
    }
}

#[derive(Deserialize)]
struct XboxDisplayClaims {
    xui: Vec<XboxUserClaim>,
}

#[derive(Deserialize)]
struct XboxUserClaim {
    uhs: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MinecraftLoginRequest {
    identity_token: String,
}

#[derive(Deserialize)]
struct MinecraftLoginResponse {
    #[serde(default = "bearer")]
    token_type: String,
    access_token: String,
    expires_in: u64,
}

fn bearer() -> String {
    "Bearer".to_string()
}

#[derive(Deserialize)]
struct EntitlementsResponse {
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct RemoteServiceError {
    error: Option<String>,
    error_description: Option<String>,
    #[serde(rename = "Message")]
    message: Option<String>,
    #[serde(rename = "XErr")]
    xerr: Option<u64>,
}

#[derive(Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<MinecraftSkin>,
}

impl MinecraftProfile {
    fn skin_url(&self) -> Option<String> {
        self.skins
            .iter()
            .find(|skin| skin.state.as_deref().is_none_or(|state| state == "ACTIVE"))
            .and_then(|skin| super::normalize_skin_url(&skin.url))
    }
}

#[derive(Deserialize)]
struct MinecraftSkin {
    url: String,
    state: Option<String>,
}

enum ProfileLookup {
    Found(MinecraftProfile),
    Unauthorized,
}

struct MinecraftSession {
    token_type: String,
    access_token: Zeroizing<String>,
    expires_at_unix_seconds: u64,
    profile: MinecraftProfile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_client_id_is_never_invented() {
        let config = GlobalConfig::default();
        if option_env!("ORBIT_MICROSOFT_CLIENT_ID").is_none() {
            assert!(microsoft_client_id(&config).is_err());
        }
    }

    #[tokio::test]
    async fn missing_client_id_does_not_mutate_existing_accounts() {
        if option_env!("ORBIT_MICROSOFT_CLIENT_ID").is_some() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_dir: Some(directory.path().join("config")),
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
        })
        .unwrap();
        super::super::create_offline_account(&paths, "ExistingPlayer").unwrap();
        let before = std::fs::read(paths.accounts_file()).unwrap();
        let store = crate::secret_store::test_support::MemorySecretStore::default();

        let error = begin_microsoft_device_login(
            &paths,
            &GlobalConfig::default(),
            &reqwest::Client::new(),
            &store,
        )
        .await
        .unwrap_err();

        assert_eq!(error.code(), "authentication");
        assert_eq!(std::fs::read(paths.accounts_file()).unwrap(), before);
        assert_eq!(
            super::super::AccountRepository::load(&paths)
                .unwrap()
                .accounts()
                .len(),
            1
        );
    }

    #[test]
    fn device_session_path_is_derived_only_from_a_uuid() {
        let paths = RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_dir: Some(PathBuf::from("config")),
            data_dir: Some(PathBuf::from("data")),
            cache_dir: Some(PathBuf::from("cache")),
        })
        .unwrap();
        let id = Uuid::nil();
        assert_eq!(
            device_session_path(&paths, id),
            PathBuf::from("data/auth-sessions/00000000-0000-0000-0000-000000000000.json")
        );
    }
}
