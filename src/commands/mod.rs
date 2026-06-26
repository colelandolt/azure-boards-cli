pub mod area;
pub mod attachment;
pub mod auth_cmd;
pub mod backlog;
pub mod board;
pub mod comment;
pub mod configure;
pub mod context_cmd;
pub mod github;
pub mod image;
pub mod iteration;
pub mod metrics;
pub mod query;
pub mod relation;
pub mod sprint;
pub mod tag_cmd;
pub mod wiql_cmd;
pub mod work_item;

use crate::auth;
use crate::auth::store::SecretStore;
use crate::client::Clients;
use crate::context::Ctx;
use crate::error::CliError;
use std::sync::Arc;

/// Build authenticated SDK/raw clients for a command that talks to the API.
/// Interactive credential acquisition is allowed only on a TTY.
pub async fn make_clients(ctx: &Ctx) -> Result<Clients, CliError> {
    let org = ctx.org_name()?.to_string();
    let store = Arc::new(SecretStore::open());
    let resolved = auth::resolve(&store, true).await?;
    tracing::info!("credential source: {}", resolved.source.describe());
    Ok(Clients::new(resolved.credential, org))
}
