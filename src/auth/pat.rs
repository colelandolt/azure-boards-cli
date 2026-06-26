use crate::client::raw::RawClient;
use crate::error::CliError;
use azure_devops_rust_api::Credential;
use serde_json::Value;

/// Identity info from GET {org}/_apis/connectionData — used both to validate
/// PATs before storing them and by `auth status`.
pub struct ConnectionIdentity {
    pub display_name: Option<String>,
    pub unique_name: Option<String>,
}

pub async fn connection_data(
    credential: &Credential,
    org: &str,
) -> Result<ConnectionIdentity, CliError> {
    let raw = RawClient::new(credential.clone());
    let url = format!("https://dev.azure.com/{org}/_apis/connectionData");
    let value: Value = raw.get_json(&url).await?;
    let user = value
        .get("authenticatedUser")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(ConnectionIdentity {
        display_name: user
            .get("providerDisplayName")
            .and_then(|v| v.as_str())
            .map(String::from),
        unique_name: user
            .get("properties")
            .and_then(|p| p.get("Account"))
            .and_then(|a| a.get("$value"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}
