use crate::cli::read_arg_or_file;
use crate::cli::work_item::WiqlCmd;
use crate::client::{enc, Clients};
use crate::context::Ctx;
use crate::domain::hydrate;
use crate::error::CliError;
use crate::output::CommandOutput;
use serde_json::{json, Value};

pub async fn run(cmd: &WiqlCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        WiqlCmd::Run { wiql, top } => {
            let text = read_arg_or_file(wiql)?;
            let result = execute(clients, project, &text, *top).await?;
            Ok(CommandOutput::Item {
                value: result,
                columns: vec![],
            })
        }
        WiqlCmd::Fetch { wiql, fields, top } => {
            let text = read_arg_or_file(wiql)?;
            let result = execute(clients, project, &text, *top).await?;
            let ids = collect_ids(&result);
            let default = hydrate::default_fields();
            let fields = fields.as_deref().map(|f| f.to_vec()).unwrap_or(default);
            let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;
            if !hydrated.omitted.is_empty() {
                tracing::warn!("omitted (deleted or inaccessible): {:?}", hydrated.omitted);
            }
            Ok(CommandOutput::List {
                value: hydrated.items,
                columns: hydrate::work_item_columns(),
            })
        }
    }
}

/// POST the WIQL. Raw client: the SDK route requires a team segment, and
/// project-scoped WIQL (no team) is the common case.
pub async fn execute(
    clients: &Clients,
    project: &str,
    wiql: &str,
    top: Option<i32>,
) -> Result<Value, CliError> {
    let mut url = format!(
        "{}/{}/_apis/wit/wiql?api-version=7.1",
        clients.org_url(),
        enc(project)
    );
    if let Some(t) = top {
        url.push_str(&format!("&$top={t}"));
    }
    clients
        .raw()
        .post_json(&url, json!({"query": wiql}))
        .await
        .map_err(|e| {
            // The 20k cap deserves an actionable message.
            if e.to_string().contains("VS402337") {
                CliError::Validation(
                    "query exceeds the 20000-result limit (VS402337): narrow the WHERE clause or pass --top"
                        .into(),
                )
            } else {
                e
            }
        })
}

/// IDs from either a flat query (workItems) or a tree/one-hop query
/// (workItemRelations: distinct source+target ids, document order).
pub fn collect_ids(result: &Value) -> Vec<i32> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(items) = result.get("workItems").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(id) = item.get("id").and_then(|i| i.as_i64()) {
                if seen.insert(id) {
                    ids.push(id as i32);
                }
            }
        }
    }
    if let Some(relations) = result.get("workItemRelations").and_then(|v| v.as_array()) {
        for rel in relations {
            for side in ["source", "target"] {
                if let Some(id) = rel
                    .get(side)
                    .and_then(|s| s.get("id"))
                    .and_then(|i| i.as_i64())
                {
                    if seen.insert(id) {
                        ids.push(id as i32);
                    }
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ids_preserve_order() {
        let v = json!({"queryType": "flat", "workItems": [{"id": 3}, {"id": 1}, {"id": 2}]});
        assert_eq!(collect_ids(&v), vec![3, 1, 2]);
    }

    #[test]
    fn tree_ids_from_relations_deduped() {
        let v = json!({"queryType": "tree", "workItemRelations": [
            {"target": {"id": 10}},
            {"source": {"id": 10}, "target": {"id": 11}},
            {"source": {"id": 10}, "target": {"id": 12}}
        ]});
        assert_eq!(collect_ids(&v), vec![10, 11, 12]);
    }
}
