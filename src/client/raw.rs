//! Thin raw REST client for endpoints the SDK does not cover (Analytics
//! OData, githubconnections, binary downloads) and for token endpoints.

use super::retry;
use crate::error::CliError;
use azure_devops_rust_api::Credential;
use base64::Engine;
use serde_json::Value;

const ALLOWED_HOSTS: &[&str] = &[
    "dev.azure.com",
    "analytics.dev.azure.com",
    "login.microsoftonline.com",
];

pub struct RawClient {
    http: reqwest::Client,
    credential: Credential,
}

impl RawClient {
    pub fn new(credential: Credential) -> Self {
        Self {
            http: reqwest::Client::new(),
            credential,
        }
    }

    async fn auth_header(&self) -> Result<Option<String>, CliError> {
        match &self.credential {
            Credential::Unauthenticated => Ok(None),
            Credential::Pat(pat) => Ok(Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"))
            ))),
            Credential::TokenCredential(cred) => {
                let token = cred
                    .get_token(&[azure_devops_rust_api::ADO_SCOPE], None)
                    .await
                    .map_err(|e| CliError::Auth(e.to_string()))?;
                Ok(Some(format!("Bearer {}", token.token.secret())))
            }
        }
    }

    fn check_host(url: &str) -> Result<(), CliError> {
        // Testing hook: an explicit AZURE_BOARDS_API_BASE is trusted.
        if let Some(base) = super::api_base_override() {
            if url.starts_with(&base) {
                return Ok(());
            }
        }
        let host = url
            .strip_prefix("https://")
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("");
        if ALLOWED_HOSTS.contains(&host) {
            Ok(())
        } else {
            Err(CliError::General(format!(
                "refusing to send credentials to unexpected host: {host}"
            )))
        }
    }

    /// Send with retry on 408/429/5xx (GET always; writes only on 429),
    /// honoring Retry-After. Returns (status, body-text).
    pub async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Value>,
        content_type: &str,
    ) -> Result<(u16, String), CliError> {
        Self::check_host(url)?;
        let auth = self.auth_header().await?;
        let is_get = method == reqwest::Method::GET;
        let mut attempt = 0u32;
        loop {
            let mut req = self
                .http
                .request(method.clone(), url)
                .header("Accept", "application/json");
            if let Some(a) = &auth {
                req = req.header("Authorization", a);
            }
            if let Some(b) = &body {
                req = req
                    .header("Content-Type", content_type)
                    .body(serde_json::to_string(b)?);
            }
            let resp = req.send().await;
            match resp {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let retry_after = retry::parse_retry_after(resp.headers());
                    let text = resp.text().await.unwrap_or_default();
                    let retryable = status == 408 || status == 429 || (500..=599).contains(&status);
                    let allowed = is_get || status == 429;
                    if retryable && allowed && attempt < retry::MAX_ATTEMPTS {
                        let delay = retry::delay(attempt, retry_after);
                        tracing::debug!("HTTP {status} from {url}; retrying in {delay:?}");
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue;
                    }
                    return Ok((status, text));
                }
                Err(e)
                    if (e.is_connect() || e.is_timeout())
                        && is_get
                        && attempt < retry::MAX_ATTEMPTS =>
                {
                    let delay = retry::delay(attempt, None);
                    tracing::debug!("network error ({e}); retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub async fn get_json(&self, url: &str) -> Result<Value, CliError> {
        let (status, body) = self.send(reqwest::Method::GET, url, None, "").await?;
        Self::expect_json(status, &body)
    }

    pub async fn post_json(&self, url: &str, body: Value) -> Result<Value, CliError> {
        let (status, text) = self
            .send(reqwest::Method::POST, url, Some(body), "application/json")
            .await?;
        Self::expect_json(status, &text)
    }

    pub async fn patch_json(
        &self,
        url: &str,
        body: Value,
        content_type: &str,
    ) -> Result<Value, CliError> {
        let (status, text) = self
            .send(reqwest::Method::PATCH, url, Some(body), content_type)
            .await?;
        Self::expect_json(status, &text)
    }

    pub async fn delete(&self, url: &str) -> Result<Value, CliError> {
        let (status, text) = self.send(reqwest::Method::DELETE, url, None, "").await?;
        if (200..300).contains(&status) && text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Self::expect_json(status, &text)
    }

    fn expect_json(status: u16, body: &str) -> Result<Value, CliError> {
        if !(200..300).contains(&status) {
            return Err(CliError::from_response(status, body));
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        // A 200 with an HTML body is the classic expired-PAT sign-in page.
        if body.trim_start().starts_with('<') {
            return Err(CliError::from_response(203, body));
        }
        serde_json::from_str(body).map_err(|e| CliError::General(format!("response parse: {e}")))
    }

    /// Upload raw bytes (attachment upload). No retry: not idempotent.
    pub async fn post_bytes(&self, url: &str, bytes: Vec<u8>) -> Result<Value, CliError> {
        Self::check_host(url)?;
        let mut req = self
            .http
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/octet-stream")
            .body(bytes);
        if let Some(a) = self.auth_header().await? {
            req = req.header("Authorization", a);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Self::expect_json(status, &text)
    }

    /// Download raw bytes. Sends the Authorization header only to
    /// dev.azure.com — inline images can reference third-party hosts and the
    /// credential must never leak there.
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, CliError> {
        let is_ado = url.starts_with("https://dev.azure.com/");
        let mut req = self.http.get(url);
        if is_ado {
            if let Some(a) = self.auth_header().await? {
                req = req.header("Authorization", a);
            }
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::from_response(status, &body));
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowlist_blocks_third_parties() {
        assert!(RawClient::check_host("https://dev.azure.com/org/_apis/x").is_ok());
        assert!(RawClient::check_host("https://analytics.dev.azure.com/org/_odata").is_ok());
        assert!(RawClient::check_host("https://evil.example.com/x").is_err());
        assert!(RawClient::check_host("http://dev.azure.com/insecure").is_err());
    }

    #[test]
    fn non_2xx_maps_via_from_response() {
        let e = RawClient::expect_json(404, r#"{"message":"nope"}"#).unwrap_err();
        assert_eq!(e.exit_code(), 5);
    }

    #[test]
    fn html_200_treated_as_auth_failure() {
        let e = RawClient::expect_json(200, "<html>Sign in</html>").unwrap_err();
        assert_eq!(e.exit_code(), 3);
    }
}
