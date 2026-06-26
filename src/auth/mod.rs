pub mod credential;
pub mod device_code;
pub mod pat;
pub mod store;

use crate::error::CliError;
use azure_devops_rust_api::Credential;
use credential::{CachedEntraCredential, StaticBearerCredential};
use device_code::TokenCache;
use std::sync::Arc;
use store::{SecretStore, ENTRA_KEY, PAT_KEY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    EnvBearer,
    EnvPat,
    EnvPatAzCompat,
    StoredEntra,
    AzureCli,
    StoredPat,
    DeviceCodePrompt,
}

impl CredentialSource {
    pub fn describe(&self) -> &'static str {
        match self {
            CredentialSource::EnvBearer => "ADO_TOKEN environment variable (Entra bearer)",
            CredentialSource::EnvPat => "ADO_PAT environment variable",
            CredentialSource::EnvPatAzCompat => "AZURE_DEVOPS_EXT_PAT environment variable",
            CredentialSource::StoredEntra => "stored Entra sign-in (device code)",
            CredentialSource::AzureCli => "Azure CLI (az login)",
            CredentialSource::StoredPat => "stored Personal Access Token",
            CredentialSource::DeviceCodePrompt => "interactive device-code sign-in",
        }
    }
}

pub struct ResolvedCredential {
    pub credential: Credential,
    pub source: CredentialSource,
    /// Identity hint for `auth status` (never a secret).
    pub username: Option<String>,
}

/// Stored PAT entry (JSON under the "pat" key).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StoredPat {
    pub version: u32,
    pub pat: String,
    pub organization: Option<String>,
    pub created_at: i64,
}

/// Resolve a credential by precedence:
/// env ADO_TOKEN > env ADO_PAT > env AZURE_DEVOPS_EXT_PAT > stored Entra >
/// Azure CLI > stored PAT > interactive device-code (TTY only, when allowed).
pub async fn resolve(
    store: &Arc<SecretStore>,
    allow_interactive: bool,
) -> Result<ResolvedCredential, CliError> {
    if let Some(token) = env_var("ADO_TOKEN") {
        return Ok(ResolvedCredential {
            credential: Credential::from_token_credential(Arc::new(StaticBearerCredential::new(
                token,
            ))),
            source: CredentialSource::EnvBearer,
            username: None,
        });
    }
    if let Some(pat) = env_var("ADO_PAT") {
        return Ok(ResolvedCredential {
            credential: Credential::from_pat(pat),
            source: CredentialSource::EnvPat,
            username: None,
        });
    }
    if let Some(pat) = env_var("AZURE_DEVOPS_EXT_PAT") {
        return Ok(ResolvedCredential {
            credential: Credential::from_pat(pat),
            source: CredentialSource::EnvPatAzCompat,
            username: None,
        });
    }
    if let Some(cached) = load_entra_cache(store)? {
        let cred = CachedEntraCredential::new(cached, store.clone());
        let username = cred.username();
        return Ok(ResolvedCredential {
            credential: Credential::from_token_credential(Arc::new(cred)),
            source: CredentialSource::StoredEntra,
            username,
        });
    }
    if az_cli_available() {
        if let Ok(cred) = azure_identity::AzureCliCredential::new(None) {
            return Ok(ResolvedCredential {
                credential: Credential::from_token_credential(cred),
                source: CredentialSource::AzureCli,
                username: None,
            });
        }
    }
    if let Some(stored) = load_stored_pat(store)? {
        return Ok(ResolvedCredential {
            credential: Credential::from_pat(stored.pat),
            source: CredentialSource::StoredPat,
            username: None,
        });
    }
    if allow_interactive && stderr_stdin_tty() {
        let cache = interactive_device_code_login(store).await?;
        let cred = CachedEntraCredential::new(cache, store.clone());
        let username = cred.username();
        return Ok(ResolvedCredential {
            credential: Credential::from_token_credential(Arc::new(cred)),
            source: CredentialSource::DeviceCodePrompt,
            username,
        });
    }
    Err(CliError::Auth(
        "no credential found. Tried: ADO_TOKEN, ADO_PAT, AZURE_DEVOPS_EXT_PAT, stored Entra sign-in, \
         Azure CLI, stored PAT. Run `ab auth login` (interactive) or set ADO_PAT/ADO_TOKEN (agents/CI)."
            .into(),
    ))
}

/// Run the device-code flow, persist the resulting token cache, return it.
/// Prompts go to stderr; stdout stays clean.
pub async fn interactive_device_code_login(
    store: &Arc<SecretStore>,
) -> Result<TokenCache, CliError> {
    let http = reqwest::Client::new();
    let start = device_code::start(&http).await?;
    if start.message.is_empty() {
        eprintln!(
            "To sign in, open {} and enter the code {}",
            start.verification_uri, start.user_code
        );
    } else {
        eprintln!("{}", start.message);
    }
    let cache = device_code::poll(&http, &start).await?;
    store.save(ENTRA_KEY, &serde_json::to_string(&cache)?)?;
    Ok(cache)
}

pub fn load_entra_cache(store: &SecretStore) -> Result<Option<TokenCache>, CliError> {
    match store.load(ENTRA_KEY)? {
        Some(text) => Ok(Some(serde_json::from_str(&text).map_err(|e| {
            CliError::Auth(format!(
                "corrupt stored Entra credential ({e}); run `ab auth login`"
            ))
        })?)),
        None => Ok(None),
    }
}

pub fn load_stored_pat(store: &SecretStore) -> Result<Option<StoredPat>, CliError> {
    match store.load(PAT_KEY)? {
        Some(text) => Ok(Some(serde_json::from_str(&text).map_err(|e| {
            CliError::Auth(format!(
                "corrupt stored PAT ({e}); run `ab auth login --pat`"
            ))
        })?)),
        None => Ok(None),
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn az_cli_available() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| {
        dir.join("az").is_file() || dir.join("az.cmd").is_file() || dir.join("az.exe").is_file()
    })
}

fn stderr_stdin_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_pat_roundtrip() {
        let p = StoredPat {
            version: 1,
            pat: "secret".into(),
            organization: Some("nrgmr".into()),
            created_at: 0,
        };
        let text = serde_json::to_string(&p).unwrap();
        let back: StoredPat = serde_json::from_str(&text).unwrap();
        assert_eq!(back.pat, "secret");
    }
}
