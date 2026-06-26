pub mod raw;
pub mod retry;

use azure_devops_rust_api::Credential;

/// Testing hook: AZURE_BOARDS_API_BASE redirects all API traffic (used by the
/// mock-server integration tests). Unset in normal operation.
pub fn api_base_override() -> Option<String> {
    std::env::var("AZURE_BOARDS_API_BASE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
}

pub fn api_base() -> String {
    api_base_override().unwrap_or_else(|| "https://dev.azure.com".to_string())
}

/// Percent-encode a URL path segment (project/team names with spaces, etc.).
pub fn enc(segment: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const SEGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'`')
        .add(b'{')
        .add(b'}')
        .add(b'/')
        .add(b'%')
        .add(b'&')
        .add(b'+');
    utf8_percent_encode(segment, SEGMENT).to_string()
}

/// Per-invocation client factory. SDK client builders are cheap (no network
/// setup), so clients are constructed lazily per command.
pub struct Clients {
    pub credential: Credential,
    /// Org short name (e.g. "nrgmr").
    pub org: String,
}

impl Clients {
    pub fn new(credential: Credential, org: String) -> Self {
        Self { credential, org }
    }

    pub fn org_url(&self) -> String {
        format!("{}/{}", api_base(), self.org)
    }

    pub fn wit(&self) -> azure_devops_rust_api::wit::Client {
        let mut builder = azure_devops_rust_api::wit::ClientBuilder::new(self.credential.clone());
        if let Some(base) = api_base_override() {
            builder = builder.endpoint(azure_core::http::Url::parse(&base).expect("valid base"));
        }
        builder.build()
    }

    pub fn work(&self) -> azure_devops_rust_api::work::Client {
        let mut builder = azure_devops_rust_api::work::ClientBuilder::new(self.credential.clone());
        if let Some(base) = api_base_override() {
            builder = builder.endpoint(azure_core::http::Url::parse(&base).expect("valid base"));
        }
        builder.build()
    }

    pub fn core(&self) -> azure_devops_rust_api::core::Client {
        let mut builder = azure_devops_rust_api::core::ClientBuilder::new(self.credential.clone());
        if let Some(base) = api_base_override() {
            builder = builder.endpoint(azure_core::http::Url::parse(&base).expect("valid base"));
        }
        builder.build()
    }

    pub fn raw(&self) -> raw::RawClient {
        raw::RawClient::new(self.credential.clone())
    }

    /// Browser URL for a work item.
    pub fn work_item_web_url(&self, project: &str, id: i32) -> String {
        use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
        format!(
            "https://dev.azure.com/{}/{}/_workitems/edit/{id}",
            self.org,
            utf8_percent_encode(project, NON_ALPHANUMERIC)
        )
    }
}
