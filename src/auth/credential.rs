use super::device_code::{self, TokenCache};
use super::store::{SecretStore, ENTRA_KEY};
use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use std::sync::{Arc, Mutex};

/// TokenCredential backed by the persisted device-code cache. Serves the
/// cached access token while fresh; refreshes (and persists the rotated
/// refresh token) when it is not.
pub struct CachedEntraCredential {
    cache: Mutex<TokenCache>,
    store: Arc<SecretStore>,
    http: reqwest::Client,
}

impl std::fmt::Debug for CachedEntraCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedEntraCredential")
            .finish_non_exhaustive()
    }
}

impl CachedEntraCredential {
    pub fn new(cache: TokenCache, store: Arc<SecretStore>) -> Self {
        Self {
            cache: Mutex::new(cache),
            store,
            http: reqwest::Client::new(),
        }
    }

    pub fn username(&self) -> Option<String> {
        self.cache.lock().unwrap().username.clone()
    }

    pub fn expires_in_secs(&self) -> i64 {
        self.cache.lock().unwrap().expires_in_secs()
    }

    async fn current_token(&self) -> Result<(String, i64), crate::error::CliError> {
        let snapshot = self.cache.lock().unwrap().clone();
        if snapshot.access_token_fresh() {
            return Ok((snapshot.access_token, snapshot.access_token_expires_at));
        }
        let renewed = device_code::refresh(&self.http, &snapshot).await?;
        let serialized = serde_json::to_string(&renewed)?;
        self.store.save(ENTRA_KEY, &serialized)?;
        let result = (
            renewed.access_token.clone(),
            renewed.access_token_expires_at,
        );
        *self.cache.lock().unwrap() = renewed;
        Ok(result)
    }
}

#[async_trait::async_trait]
impl TokenCredential for CachedEntraCredential {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        let (token, expires_at) = self.current_token().await.map_err(|e| {
            azure_core::Error::with_message(azure_core::error::ErrorKind::Credential, e.to_string())
        })?;
        let expires_on = time::OffsetDateTime::from_unix_timestamp(expires_at)
            .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        Ok(AccessToken::new(token, expires_on))
    }
}

/// Fixed bearer token from the environment (ADO_TOKEN). No refresh; if it
/// expires the API returns 401 -> exit 3.
#[derive(Debug)]
pub struct StaticBearerCredential(String);

impl StaticBearerCredential {
    pub fn new(token: String) -> Self {
        Self(token)
    }
}

#[async_trait::async_trait]
impl TokenCredential for StaticBearerCredential {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        Ok(AccessToken::new(
            self.0.clone(),
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        ))
    }
}
