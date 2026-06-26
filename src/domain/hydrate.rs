//! Batch hydration of work item IDs via workitemsbatch, 200 per call,
//! preserving caller order, with errorPolicy=omit and omission reporting.

use crate::client::Clients;
use crate::error::CliError;
use azure_devops_rust_api::wit::models::{
    work_item_batch_get_request::ErrorPolicy, WorkItemBatchGetRequest,
};
use serde_json::Value;

pub const CHUNK: usize = 200;

pub struct Hydrated {
    /// Work items as JSON values, in the same order as the requested IDs.
    pub items: Vec<Value>,
    /// IDs the service omitted (deleted/inaccessible).
    pub omitted: Vec<i32>,
}

pub async fn fetch(
    clients: &Clients,
    project: &str,
    ids: &[i32],
    fields: Option<&[String]>,
) -> Result<Hydrated, CliError> {
    if ids.is_empty() {
        return Ok(Hydrated {
            items: vec![],
            omitted: vec![],
        });
    }
    let client = clients.wit().work_items_client();
    let mut by_id = std::collections::HashMap::new();
    for chunk in ids.chunks(CHUNK) {
        let body = WorkItemBatchGetRequest {
            ids: chunk.to_vec(),
            fields: fields.map(|f| f.to_vec()).unwrap_or_default(),
            error_policy: Some(ErrorPolicy::Omit),
            ..Default::default()
        };
        let resp = client
            .get_work_items_batch(&clients.org, body, project)
            .send()
            .await?
            .into_body()?;
        for item in resp.value {
            let v = serde_json::to_value(&item)?;
            if let Some(id) = v.get("id").and_then(|i| i.as_i64()) {
                by_id.insert(id as i32, v);
            }
        }
    }
    let mut items = Vec::with_capacity(ids.len());
    let mut omitted = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue; // dedupe repeated ids, keep first position
        }
        match by_id.remove(id) {
            Some(v) => items.push(v),
            None => omitted.push(*id),
        }
    }
    Ok(Hydrated { items, omitted })
}

/// Standard columns for work item list rendering.
pub fn work_item_columns() -> Vec<crate::output::Column> {
    use crate::output::Column;
    vec![
        Column::new("ID", "id"),
        Column::new("Type", "fields.System.WorkItemType"),
        Column::new("Title", "fields.System.Title"),
        Column::new("State", "fields.System.State"),
        Column::new("Assigned To", "fields.System.AssignedTo"),
        Column::new("Iteration", "fields.System.IterationPath"),
    ]
}

/// Default fields when the caller didn't specify any (matches the bash script).
pub fn default_fields() -> Vec<String> {
    [
        "System.Id",
        "System.Title",
        "System.State",
        "System.AssignedTo",
        "System.WorkItemType",
        "System.IterationPath",
        "System.AreaPath",
        "System.Tags",
        "System.ChangedDate",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
