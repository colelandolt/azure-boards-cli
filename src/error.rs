use serde_json::{json, Value};

/// Stable CLI error taxonomy. Exit codes are part of the public contract:
/// 0 success, 1 general, 2 usage (clap), 3 auth, 4 forbidden, 5 not found,
/// 6 conflict, 7 validation, 8 service unavailable, 9 unsafe blocked, 10 partial.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    General(String),
    #[error("{0}")]
    Usage(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    Validation(String),
    #[error("service unavailable: {0}")]
    Service(String),
    #[error("unsafe operation blocked: {0}")]
    UnsafeBlocked(String),
    #[error("partial success: {ok} succeeded, {failed} failed")]
    Partial {
        ok: usize,
        failed: usize,
        report: Value,
    },
}

impl CliError {
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::General(_) => 1,
            CliError::Usage(_) => 2,
            CliError::Auth(_) => 3,
            CliError::Forbidden(_) => 4,
            CliError::NotFound(_) => 5,
            CliError::Conflict(_) => 6,
            CliError::Validation(_) => 7,
            CliError::Service(_) => 8,
            CliError::UnsafeBlocked(_) => 9,
            CliError::Partial { .. } => 10,
        }
    }

    pub fn code_str(&self) -> &'static str {
        match self {
            CliError::General(_) => "general",
            CliError::Usage(_) => "usage",
            CliError::Auth(_) => "auth",
            CliError::Forbidden(_) => "forbidden",
            CliError::NotFound(_) => "notFound",
            CliError::Conflict(_) => "conflict",
            CliError::Validation(_) => "validation",
            CliError::Service(_) => "serviceUnavailable",
            CliError::UnsafeBlocked(_) => "unsafeBlocked",
            CliError::Partial { .. } => "partialSuccess",
        }
    }

    /// Single choke point mapping HTTP responses from Azure DevOps to errors.
    /// ADO error bodies look like {"message": "...", "typeKey": "..."}; a 203
    /// with an HTML body is the classic bad-PAT sign-in redirect.
    pub fn from_response(status: u16, body: &str) -> Self {
        let message = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
            .unwrap_or_else(|| {
                if body.trim_start().starts_with('<') {
                    "received a sign-in page instead of an API response (credential rejected)"
                        .to_string()
                } else {
                    format!("HTTP {status}")
                }
            });
        match status {
            203 | 401 => CliError::Auth(message),
            403 => CliError::Forbidden(format!(
                "{message} (check PAT scopes: vso.work_write, vso.analytics, vso.githubconnections)"
            )),
            404 => CliError::NotFound(message),
            409 | 412 => CliError::Conflict(message),
            400 | 422 => {
                // A failed JSON-patch `test` op (optimistic concurrency) surfaces as 400/409
                // with a test-operation message; treat it as a conflict.
                if message.contains("test operation") || message.contains("Test Operation") {
                    CliError::Conflict(message)
                } else {
                    CliError::Validation(message)
                }
            }
            429 => CliError::Service(format!("rate limited after retries: {message}")),
            500..=599 => CliError::Service(message),
            _ => CliError::General(message),
        }
    }

    /// Stable machine-readable error object, emitted on stderr in JSON output modes.
    pub fn to_json(&self) -> Value {
        let mut obj = json!({
            "code": self.code_str(),
            "message": self.to_string(),
            "exitCode": self.exit_code(),
        });
        if let CliError::Partial { report, .. } = self {
            obj["details"] = report.clone();
        }
        obj
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::General(e.to_string())
    }
}

impl From<reqwest::Error> for CliError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_connect() || e.is_timeout() {
            CliError::Service(e.to_string())
        } else {
            CliError::General(e.to_string())
        }
    }
}

impl From<azure_core::Error> for CliError {
    fn from(e: azure_core::Error) -> Self {
        use azure_core::error::ErrorKind;
        match e.kind() {
            ErrorKind::HttpResponse {
                status,
                raw_response,
                ..
            } => {
                let code: u16 = (*status).into();
                // The error carries the raw response; surface the service's
                // own message (e.g. "TF401232: ...") instead of a bare status.
                let body = raw_response
                    .as_ref()
                    .map(|raw| {
                        let bytes: &[u8] = raw.body();
                        String::from_utf8_lossy(bytes).to_string()
                    })
                    .unwrap_or_else(|| e.to_string());
                CliError::from_response(code, &body)
            }
            ErrorKind::Io => CliError::Service(e.to_string()),
            ErrorKind::Credential => CliError::Auth(e.to_string()),
            _ => CliError::General(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::General(String::new()).exit_code(), 1);
        assert_eq!(CliError::Usage(String::new()).exit_code(), 2);
        assert_eq!(CliError::Auth(String::new()).exit_code(), 3);
        assert_eq!(CliError::Forbidden(String::new()).exit_code(), 4);
        assert_eq!(CliError::NotFound(String::new()).exit_code(), 5);
        assert_eq!(CliError::Conflict(String::new()).exit_code(), 6);
        assert_eq!(CliError::Validation(String::new()).exit_code(), 7);
        assert_eq!(CliError::Service(String::new()).exit_code(), 8);
        assert_eq!(CliError::UnsafeBlocked(String::new()).exit_code(), 9);
        assert_eq!(
            CliError::Partial {
                ok: 1,
                failed: 1,
                report: Value::Null
            }
            .exit_code(),
            10
        );
    }

    #[test]
    fn maps_http_status_to_error_kind() {
        let cases = [
            (401, 3u8),
            (203, 3),
            (403, 4),
            (404, 5),
            (409, 6),
            (412, 6),
            (400, 7),
            (422, 7),
            (429, 8),
            (500, 8),
            (503, 8),
        ];
        for (status, code) in cases {
            let e = CliError::from_response(status, "{}");
            assert_eq!(e.exit_code(), code, "status {status}");
        }
    }

    #[test]
    fn parses_ado_error_body_message() {
        let e = CliError::from_response(
            404,
            r#"{"message": "TF401232: Work item 999 does not exist", "typeKey": "WorkItemNotFound"}"#,
        );
        assert!(e.to_string().contains("TF401232"));
    }

    #[test]
    fn rev_test_failure_is_conflict() {
        let e = CliError::from_response(
            400,
            r#"{"message": "the test operation for path /rev failed"}"#,
        );
        assert_eq!(e.exit_code(), 6);
    }

    #[test]
    fn html_body_is_flagged_as_credential_problem() {
        let e = CliError::from_response(203, "<html><body>Sign in</body></html>");
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("sign-in page"));
    }

    #[test]
    fn error_json_shape() {
        let e = CliError::NotFound("work item 1".into());
        let v = e.to_json();
        assert_eq!(v["code"], "notFound");
        assert_eq!(v["exitCode"], 5);
        assert!(v["message"].as_str().unwrap().contains("work item 1"));
    }
}
