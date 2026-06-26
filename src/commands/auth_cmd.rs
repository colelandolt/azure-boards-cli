use crate::auth::{
    self, device_code, pat as pat_mod, store::SecretStore, store::ENTRA_KEY, store::PAT_KEY,
};
use crate::cli::AuthCmd;
use crate::context::Ctx;
use crate::error::CliError;
use crate::output::CommandOutput;
use serde_json::json;
use std::io::IsTerminal;
use std::sync::Arc;

pub async fn run(cmd: &AuthCmd, ctx: &Ctx) -> Result<CommandOutput, CliError> {
    let store = Arc::new(SecretStore::open());
    match cmd {
        AuthCmd::Login { pat: false } => login_device_code(&store).await,
        AuthCmd::Login { pat: true } => login_pat(&store, ctx).await,
        AuthCmd::Status => status(&store, ctx).await,
        AuthCmd::Logout => logout(&store),
    }
}

async fn login_device_code(store: &Arc<SecretStore>) -> Result<CommandOutput, CliError> {
    if !std::io::stderr().is_terminal() {
        return Err(CliError::UnsafeBlocked(
            "auth login is interactive; in CI/agents set ADO_TOKEN or ADO_PAT instead".into(),
        ));
    }
    let cache = auth::interactive_device_code_login(store).await?;
    eprintln!(
        "Signed in{} (tokens stored in {}).",
        cache
            .username
            .as_deref()
            .map(|u| format!(" as {u}"))
            .unwrap_or_default(),
        store.backend_name()
    );
    Ok(CommandOutput::Item {
        value: json!({
            "status": "signedIn",
            "method": "deviceCode",
            "username": cache.username,
            "storage": store.backend_name(),
        }),
        columns: vec![],
    })
}

async fn login_pat(store: &Arc<SecretStore>, ctx: &Ctx) -> Result<CommandOutput, CliError> {
    let org = ctx.org_name()?.to_string();
    let pat = read_pat_from_stdin()?;
    if pat.is_empty() {
        return Err(CliError::Validation("no PAT provided on stdin".into()));
    }
    // Validate before storing.
    let cred = azure_devops_rust_api::Credential::from_pat(pat.clone());
    let identity = pat_mod::connection_data(&cred, &org)
        .await
        .map_err(|e| CliError::Auth(format!("PAT validation against {org} failed: {e}")))?;
    let entry = auth::StoredPat {
        version: 1,
        pat,
        organization: Some(org.clone()),
        created_at: device_code::now_unix(),
    };
    store.save(PAT_KEY, &serde_json::to_string(&entry)?)?;
    eprintln!(
        "PAT validated for {} and stored in {}.",
        org,
        store.backend_name()
    );
    Ok(CommandOutput::Item {
        value: json!({
            "status": "signedIn",
            "method": "pat",
            "organization": org,
            "identity": identity.display_name,
            "storage": store.backend_name(),
        }),
        columns: vec![],
    })
}

/// PATs are accepted on stdin only — never as argv (visible in `ps`) and
/// never echoed back.
fn read_pat_from_stdin() -> Result<String, CliError> {
    use std::io::BufRead;
    if std::io::stdin().is_terminal() {
        eprint!("Paste PAT (input is read from stdin; it will not be stored until validated): ");
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

async fn status(store: &Arc<SecretStore>, ctx: &Ctx) -> Result<CommandOutput, CliError> {
    // Never trigger interactive sign-in from `status`.
    let resolved = auth::resolve(store, false).await?;
    let org = ctx.org_name().ok().map(String::from);
    let identity = if let Some(org) = &org {
        pat_mod::connection_data(&resolved.credential, org)
            .await
            .ok()
    } else {
        None
    };
    let expires_in = auth::load_entra_cache(store)
        .ok()
        .flatten()
        .map(|c| c.expires_in_secs());
    Ok(CommandOutput::Item {
        value: json!({
            "credentialSource": resolved.source.describe(),
            "username": resolved.username,
            "identity": identity.as_ref().and_then(|i| i.display_name.clone()),
            "account": identity.as_ref().and_then(|i| i.unique_name.clone()),
            "organization": org,
            "storage": store.backend_name(),
            "accessTokenExpiresInSeconds": expires_in,
        }),
        columns: vec![],
    })
}

fn logout(store: &Arc<SecretStore>) -> Result<CommandOutput, CliError> {
    let entra = store.delete(ENTRA_KEY)?;
    let pat = store.delete(PAT_KEY)?;
    Ok(CommandOutput::Item {
        value: json!({
            "removedEntra": entra,
            "removedPat": pat,
            "storage": store.backend_name(),
        }),
        columns: vec![],
    })
}
