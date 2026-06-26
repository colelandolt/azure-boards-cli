use crate::cli::items::RelationCmd;
use crate::client::{enc, Clients};
use crate::context::{prompt, Ctx};
use crate::domain::{hydrate, patch};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use azure_devops_rust_api::wit::models::JsonPatchOperation;
use serde_json::{json, Value};

pub async fn run(
    cmd: &RelationCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    match cmd {
        RelationCmd::List { id } => list(clients, ctx.project()?, *id).await,
        RelationCmd::Add {
            id,
            r#type,
            target,
            comment,
            dry_run,
        } => {
            add(
                clients,
                ctx,
                *id,
                r#type,
                target,
                comment.as_deref(),
                *dry_run,
            )
            .await
        }
        RelationCmd::Remove {
            id,
            relation_id,
            dry_run,
        } => remove(clients, ctx, *id, *relation_id, *dry_run).await,
        RelationCmd::Tree { id, depth } => tree(clients, ctx.project()?, *id, *depth).await,
        RelationCmd::ListType => list_types(clients).await,
    }
}

/// GET a work item with $expand=relations, as raw JSON.
pub async fn fetch_with_relations(
    clients: &Clients,
    project: &str,
    id: i32,
) -> Result<Value, CliError> {
    let url = format!(
        "{}/{}/_apis/wit/workitems/{id}?$expand=relations&api-version=7.1",
        clients.org_url(),
        enc(project)
    );
    clients.raw().get_json(&url).await
}

/// Friendly classification matching the legacy bash script's rel_type().
pub fn classify(rel: &str, url: &str, name: &str) -> String {
    match rel {
        "System.LinkTypes.Hierarchy-Reverse" => "Parent".into(),
        "System.LinkTypes.Hierarchy-Forward" => "Child".into(),
        "System.LinkTypes.Related" => "Related".into(),
        "System.LinkTypes.Dependency-Forward" => "Successor".into(),
        "System.LinkTypes.Dependency-Reverse" => "Predecessor".into(),
        "System.LinkTypes.Duplicate-Forward" => "Duplicate".into(),
        "System.LinkTypes.Duplicate-Reverse" => "Duplicate Of".into(),
        "AttachedFile" => "Attachment".into(),
        "Hyperlink" => "Hyperlink".into(),
        "ArtifactLink" => {
            if url.contains("/GitHub/PullRequest/") {
                "GitHub PR".into()
            } else if url.contains("/GitHub/Commit/") {
                "GitHub Commit".into()
            } else if url.contains("/GitHub/Branch/") {
                "GitHub Branch".into()
            } else if url.contains("/GitHub/Issue/") {
                "GitHub Issue".into()
            } else {
                "Artifact".into()
            }
        }
        other => {
            if name.is_empty() {
                other.into()
            } else {
                name.into()
            }
        }
    }
}

pub fn work_item_id_from_url(url: &str) -> Option<i64> {
    let idx = url.rfind("/workItems/")?;
    url[idx + "/workItems/".len()..].parse().ok()
}

/// Decorate raw relations with index/type/workItemId while preserving the
/// raw rel/url/attributes.
pub fn decorate(relations: &[Value]) -> Vec<Value> {
    relations
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let rel = r.get("rel").and_then(|v| v.as_str()).unwrap_or("");
            let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let name = r
                .get("attributes")
                .and_then(|a| a.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            json!({
                "index": i,
                "type": classify(rel, url, name),
                "rel": rel,
                "url": url,
                "workItemId": work_item_id_from_url(url),
                "attributes": r.get("attributes").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

async fn list(clients: &Clients, project: &str, id: i32) -> Result<CommandOutput, CliError> {
    let item = fetch_with_relations(clients, project, id).await?;
    let relations = item
        .get("relations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(CommandOutput::List {
        value: decorate(&relations),
        columns: vec![
            Column::new("Index", "index"),
            Column::new("Type", "type"),
            Column::new("Work Item", "workItemId"),
            Column::new("URL", "url"),
        ],
    })
}

/// Map friendly alias -> reference name. Full reference names pass through.
pub fn resolve_rel_type(alias: &str) -> String {
    match alias.to_ascii_lowercase().as_str() {
        "parent" => "System.LinkTypes.Hierarchy-Reverse".into(),
        "child" => "System.LinkTypes.Hierarchy-Forward".into(),
        "related" => "System.LinkTypes.Related".into(),
        "predecessor" => "System.LinkTypes.Dependency-Reverse".into(),
        "successor" => "System.LinkTypes.Dependency-Forward".into(),
        "duplicate" => "System.LinkTypes.Duplicate-Forward".into(),
        "duplicate-of" => "System.LinkTypes.Duplicate-Reverse".into(),
        "hyperlink" => "Hyperlink".into(),
        "artifact" => "ArtifactLink".into(),
        _ => alias.to_string(),
    }
}

async fn add(
    clients: &Clients,
    ctx: &Ctx,
    id: i32,
    rel_type: &str,
    target: &str,
    comment: Option<&str>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let rel = resolve_rel_type(rel_type);
    let url = if target.starts_with("http") || target.starts_with("vstfs:") {
        target.to_string()
    } else {
        let target_id: i64 = target.parse().map_err(|_| {
            CliError::Validation(format!(
                "--target must be a work item ID or a URL, got '{target}'"
            ))
        })?;
        format!("{}/_apis/wit/workItems/{target_id}", clients.org_url())
    };
    let attributes = match comment {
        Some(c) => json!({"comment": c}),
        None => json!({}),
    };
    let ops = vec![patch::add_relation(&rel, &url, attributes)];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "relation.add".into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: serde_json::to_value(&ops)?,
        }));
    }
    let updated = clients
        .wit()
        .work_items_client()
        .update(&clients.org, ops, id, project)
        .send()
        .await?
        .into_body()?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "relation.add".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: serde_json::to_value(&updated)?,
    }))
}

/// Remove by index with a revision test: if anything changed the work item
/// between read and write the patch fails -> exit 6 (re-run `relation list`).
pub async fn remove_relation_checked(
    clients: &Clients,
    ctx: &Ctx,
    id: i32,
    index: usize,
    operation: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let item = fetch_with_relations(clients, project, id).await?;
    let relations = item
        .get("relations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    if index >= relations.len() {
        return Err(CliError::NotFound(format!(
            "relation index {index} (work item {id} has {} relations; run `ab relation list {id}`)",
            relations.len()
        )));
    }
    let rev = item
        .get("rev")
        .and_then(|r| r.as_i64())
        .ok_or_else(|| CliError::General("work item has no rev".into()))? as i32;
    let removed = decorate(&relations)[index].clone();
    let ops: Vec<JsonPatchOperation> = vec![patch::test_rev(rev), patch::remove_relation(index)];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"removing": removed, "patch": serde_json::to_value(&ops)?}),
        }));
    }
    prompt::confirm(
        ctx.assume_yes,
        prompt::Danger::Destructive,
        &format!(
            "Remove {} relation (index {index}) from work item {id}?",
            removed.get("type").and_then(|t| t.as_str()).unwrap_or("?")
        ),
    )?;
    let updated = clients
        .wit()
        .work_items_client()
        .update(&clients.org, ops, id, project)
        .send()
        .await?
        .into_body()?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: json!({"removed": removed, "rev": updated.rev}),
    }))
}

async fn remove(
    clients: &Clients,
    ctx: &Ctx,
    id: i32,
    index: usize,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    remove_relation_checked(clients, ctx, id, index, "relation.remove", dry_run).await
}

async fn tree(
    clients: &Clients,
    project: &str,
    root: i32,
    depth: u32,
) -> Result<CommandOutput, CliError> {
    let depth = depth.min(10);
    // BFS by level; each level fetched in one batch with relations expanded.
    let mut nodes: std::collections::HashMap<i32, Value> = std::collections::HashMap::new();
    let mut children_of: std::collections::HashMap<i32, Vec<i32>> =
        std::collections::HashMap::new();
    let mut visited = std::collections::HashSet::new();
    let mut level = vec![root];
    visited.insert(root);
    for _ in 0..=depth {
        if level.is_empty() {
            break;
        }
        let batch = batch_with_relations(clients, project, &level).await?;
        let mut next = Vec::new();
        for item in batch {
            let id = item.get("id").and_then(|i| i.as_i64()).unwrap_or(0) as i32;
            let mut kids = Vec::new();
            if let Some(rels) = item.get("relations").and_then(|r| r.as_array()) {
                for r in rels {
                    if r.get("rel").and_then(|v| v.as_str())
                        == Some("System.LinkTypes.Hierarchy-Forward")
                    {
                        if let Some(child) = r
                            .get("url")
                            .and_then(|u| u.as_str())
                            .and_then(work_item_id_from_url)
                        {
                            let child = child as i32;
                            kids.push(child);
                            if visited.insert(child) {
                                next.push(child);
                            }
                        }
                    }
                }
            }
            children_of.insert(id, kids);
            nodes.insert(id, item);
        }
        level = next;
    }
    let tree = build_node(root, &nodes, &children_of);
    Ok(CommandOutput::Item {
        value: tree,
        columns: vec![],
    })
}

async fn batch_with_relations(
    clients: &Clients,
    project: &str,
    ids: &[i32],
) -> Result<Vec<Value>, CliError> {
    use azure_devops_rust_api::wit::models::{
        work_item_batch_get_request::ErrorPolicy, work_item_batch_get_request::Expand,
        WorkItemBatchGetRequest,
    };
    let mut out = Vec::new();
    for chunk in ids.chunks(hydrate::CHUNK) {
        let body = WorkItemBatchGetRequest {
            ids: chunk.to_vec(),
            // fields and $expand are mutually exclusive; relations need expand.
            expand: Some(Expand::Relations),
            error_policy: Some(ErrorPolicy::Omit),
            ..Default::default()
        };
        let resp = clients
            .wit()
            .work_items_client()
            .get_work_items_batch(&clients.org, body, project)
            .send()
            .await?
            .into_body()?;
        for item in resp.value {
            out.push(serde_json::to_value(&item)?);
        }
    }
    Ok(out)
}

fn build_node(
    id: i32,
    nodes: &std::collections::HashMap<i32, Value>,
    children_of: &std::collections::HashMap<i32, Vec<i32>>,
) -> Value {
    let item = nodes.get(&id);
    let fields = item
        .and_then(|i| i.get("fields"))
        .cloned()
        .unwrap_or(Value::Null);
    let children: Vec<Value> = children_of
        .get(&id)
        .map(|kids| {
            kids.iter()
                .map(|k| build_node(*k, nodes, children_of))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": id,
        "title": fields.get("System.Title").cloned().unwrap_or(Value::Null),
        "type": fields.get("System.WorkItemType").cloned().unwrap_or(Value::Null),
        "state": fields.get("System.State").cloned().unwrap_or(Value::Null),
        "children": children,
    })
}

async fn list_types(clients: &Clients) -> Result<CommandOutput, CliError> {
    let url = format!(
        "{}/_apis/wit/workitemrelationtypes?api-version=7.1",
        clients.org_url()
    );
    let value = clients.raw().get_json(&url).await?;
    let items = value
        .get("value")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(CommandOutput::List {
        value: items,
        columns: vec![
            Column::new("Name", "name"),
            Column::new("Reference Name", "referenceName"),
            Column::new("Usage", "attributes.usage"),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_like_bash_script() {
        assert_eq!(
            classify("System.LinkTypes.Hierarchy-Reverse", "", ""),
            "Parent"
        );
        assert_eq!(
            classify("System.LinkTypes.Hierarchy-Forward", "", ""),
            "Child"
        );
        assert_eq!(
            classify(
                "ArtifactLink",
                "vstfs:///GitHub/PullRequest/abc%2F45",
                "GitHub Pull Request"
            ),
            "GitHub PR"
        );
        assert_eq!(
            classify("ArtifactLink", "vstfs:///GitHub/Commit/abc%2Fdeadbeef", ""),
            "GitHub Commit"
        );
        assert_eq!(
            classify("ArtifactLink", "vstfs:///Git/Commit/x", "Fixed in Commit"),
            "Artifact"
        );
        assert_eq!(classify("Unknown.Rel", "", "Custom Link"), "Custom Link");
    }

    #[test]
    fn extracts_work_item_id_from_url() {
        assert_eq!(
            work_item_id_from_url("https://dev.azure.com/o/_apis/wit/workItems/123"),
            Some(123)
        );
        assert_eq!(work_item_id_from_url("vstfs:///GitHub/PullRequest/x"), None);
    }

    #[test]
    fn alias_mapping() {
        assert_eq!(
            resolve_rel_type("parent"),
            "System.LinkTypes.Hierarchy-Reverse"
        );
        assert_eq!(
            resolve_rel_type("Child"),
            "System.LinkTypes.Hierarchy-Forward"
        );
        assert_eq!(
            resolve_rel_type("System.LinkTypes.Custom"),
            "System.LinkTypes.Custom"
        );
    }
}
