//! Manual OAuth 2.0 device-code flow against Microsoft Entra. azure_identity
//! 1.x has no device-code credential and no token persistence, so this is
//! implemented directly (~150 lines of plain reqwest).

use crate::error::CliError;
use serde::{Deserialize, Serialize};

/// The Azure DevOps resource (NOT a public client app).
pub const ADO_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";
/// Default public client used for the device-code flow: the well-known Azure
/// CLI app, pre-consented for Azure DevOps user_impersonation in most tenants.
/// Override with AZURE_BOARDS_CLIENT_ID for tenants whose Conditional Access
/// policies block it.
pub const DEFAULT_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";

const TENANT: &str = "organizations";

pub fn client_id() -> String {
    std::env::var("AZURE_BOARDS_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string())
}

pub fn scope() -> String {
    format!("{ADO_RESOURCE}/.default offline_access openid profile")
}

fn token_url() -> String {
    format!("https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/token")
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodeStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    pub expires_in: u64,
    #[serde(default)]
    pub message: String,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Persisted token cache (stored in the secret store under the "entra" key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCache {
    pub version: u32,
    pub tenant: String,
    pub client_id: String,
    pub scope: String,
    pub refresh_token: String,
    pub access_token: String,
    /// Unix seconds.
    pub access_token_expires_at: i64,
    pub username: Option<String>,
    pub acquired_at: i64,
}

impl TokenCache {
    pub fn access_token_fresh(&self) -> bool {
        // 5-minute skew.
        self.access_token_expires_at - 300 > now_unix()
    }

    pub fn expires_in_secs(&self) -> i64 {
        self.access_token_expires_at - now_unix()
    }
}

pub fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

pub async fn start(http: &reqwest::Client) -> Result<DeviceCodeStart, CliError> {
    let resp = http
        .post(format!(
            "https://login.microsoftonline.com/{TENANT}/oauth2/v2.0/devicecode"
        ))
        .form(&[("client_id", client_id()), ("scope", scope())])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if status != 200 {
        return Err(CliError::Auth(format!(
            "device code request failed (HTTP {status}): {body}"
        )));
    }
    serde_json::from_str(&body).map_err(|e| CliError::Auth(format!("device code response: {e}")))
}

/// Poll the token endpoint until the user completes sign-in, the code
/// expires, or the user declines.
pub async fn poll(http: &reqwest::Client, start: &DeviceCodeStart) -> Result<TokenCache, CliError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(start.expires_in);
    let mut interval = start.interval.max(1);
    loop {
        if std::time::Instant::now() > deadline {
            return Err(CliError::Auth(
                "device code expired before sign-in completed".into(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        let resp = http
            .post(token_url())
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", &client_id()),
                ("device_code", &start.device_code),
            ])
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if status == 200 {
            return cache_from_response(&body);
        }
        let err: TokenError = serde_json::from_str(&body)
            .map_err(|e| CliError::Auth(format!("token response: {e}")))?;
        match err.error.as_str() {
            "authorization_pending" => continue,
            "slow_down" => interval += 5,
            "expired_token" => {
                return Err(CliError::Auth(
                    "device code expired before sign-in completed".into(),
                ))
            }
            "authorization_declined" => return Err(CliError::Auth("sign-in was declined".into())),
            other => {
                return Err(CliError::Auth(format!(
                    "device code sign-in failed: {other}: {}",
                    err.error_description
                )))
            }
        }
    }
}

/// Exchange a refresh token for a fresh access token (rotates the refresh
/// token; callers must persist the returned cache).
pub async fn refresh(http: &reqwest::Client, cache: &TokenCache) -> Result<TokenCache, CliError> {
    let resp = http
        .post(token_url())
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &cache.client_id),
            ("refresh_token", &cache.refresh_token),
            ("scope", &cache.scope),
        ])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if status == 200 {
        let mut new = cache_from_response(&body)?;
        // Some responses omit the rotated refresh token; keep the old one.
        if new.refresh_token.is_empty() {
            new.refresh_token = cache.refresh_token.clone();
        }
        if new.username.is_none() {
            new.username = cache.username.clone();
        }
        Ok(new)
    } else {
        let detail = serde_json::from_str::<TokenError>(&body)
            .map(|e| format!("{}: {}", e.error, e.error_description))
            .unwrap_or(body);
        Err(CliError::Auth(format!(
            "token refresh failed; run `ab auth login` again ({detail})"
        )))
    }
}

fn cache_from_response(body: &str) -> Result<TokenCache, CliError> {
    let tok: TokenResponse =
        serde_json::from_str(body).map_err(|e| CliError::Auth(format!("token response: {e}")))?;
    let username = tok
        .id_token
        .as_deref()
        .and_then(extract_username)
        .or_else(|| extract_username(&tok.access_token));
    Ok(TokenCache {
        version: 1,
        tenant: TENANT.to_string(),
        client_id: client_id(),
        scope: scope(),
        refresh_token: tok.refresh_token.unwrap_or_default(),
        access_token: tok.access_token,
        access_token_expires_at: now_unix() + tok.expires_in,
        username,
        acquired_at: now_unix(),
    })
}

/// Pull a human-readable identity out of a JWT without verifying it (display
/// only; the token is never trusted locally).
pub fn extract_username(jwt: &str) -> Option<String> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    for key in ["preferred_username", "upn", "unique_name", "email"] {
        if let Some(v) = claims.get(key).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_extracted_from_jwt_payload() {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"upn":"cole.landolt@nrgmr.com"}"#);
        let jwt = format!("eyJhbGciOiJub25lIn0.{payload}.sig");
        assert_eq!(
            extract_username(&jwt).as_deref(),
            Some("cole.landolt@nrgmr.com")
        );
        assert_eq!(extract_username("not-a-jwt"), None);
    }

    #[test]
    fn freshness_uses_5_minute_skew() {
        let mut cache = TokenCache {
            version: 1,
            tenant: TENANT.into(),
            client_id: "c".into(),
            scope: "s".into(),
            refresh_token: "r".into(),
            access_token: "a".into(),
            access_token_expires_at: now_unix() + 600,
            username: None,
            acquired_at: now_unix(),
        };
        assert!(cache.access_token_fresh());
        cache.access_token_expires_at = now_unix() + 100;
        assert!(!cache.access_token_fresh());
    }

    #[test]
    fn default_client_id_overridable_by_env() {
        // Not setting the env var here (process-global); just pin the default.
        assert_eq!(DEFAULT_CLIENT_ID, "04b07795-8ddb-461a-bbee-02f9e1bf7b46");
        assert!(scope().contains(ADO_RESOURCE));
        assert!(scope().contains("offline_access"));
    }
}
