//! Work item state -> state category mapping, discovered per type from the
//! process (never hardcoded). Categories: Proposed, InProgress, Resolved,
//! Completed, Removed.

use crate::client::Clients;
use crate::error::CliError;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct StateMap {
    /// type -> state -> category
    pub by_type: HashMap<String, HashMap<String, String>>,
}

impl StateMap {
    pub fn category(&self, work_item_type: &str, state: &str) -> Option<&str> {
        self.by_type
            .get(work_item_type)
            .and_then(|m| m.get(state))
            .map(|s| s.as_str())
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.by_type).unwrap_or(serde_json::Value::Null)
    }
}

/// Fetch state categories for the given types (or all types when empty).
pub async fn discover(
    clients: &Clients,
    project: &str,
    types: &[String],
) -> Result<StateMap, CliError> {
    let raw = clients.raw();
    let type_names: Vec<String> = if types.is_empty() {
        let url = format!(
            "{}/{}/_apis/wit/workitemtypes?api-version=7.1",
            clients.org_url(),
            crate::client::enc(project)
        );
        let v = raw.get_json(&url).await?;
        v.get("value")
            .and_then(|a| a.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        types.to_vec()
    };
    let mut map = StateMap::default();
    for name in type_names {
        let url = format!(
            "{}/{}/_apis/wit/workitemtypes/{}/states?api-version=7.1",
            clients.org_url(),
            crate::client::enc(project),
            crate::client::enc(&name)
        );
        let v = raw.get_json(&url).await?;
        let states: HashMap<String, String> = v
            .get("value")
            .and_then(|a| a.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|s| {
                        let state = s.get("name").and_then(|n| n.as_str())?;
                        let cat = s.get("stateCategory").and_then(|c| c.as_str())?;
                        Some((state.to_string(), cat.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        map.by_type.insert(name, states);
    }
    Ok(map)
}
