use crate::cli::items::TagCmd;
use crate::client::Clients;
use crate::context::Ctx;
use crate::domain::patch;
use crate::error::CliError;
use crate::output::{CommandOutput, MutationEnvelope};
use serde_json::json;

pub async fn run(cmd: &TagCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        TagCmd::List { id } => {
            let tags = current_tags(clients, project, *id).await?;
            Ok(CommandOutput::List {
                value: tags.into_iter().map(serde_json::Value::String).collect(),
                columns: vec![],
            })
        }
        TagCmd::Add { id, tag } => {
            let current = current_tags(clients, project, *id).await?;
            let updated = add_tag(&current, tag);
            write_tags(clients, ctx, project, *id, &updated, "tag.add", false).await
        }
        TagCmd::Remove { id, tag } => {
            let current = current_tags(clients, project, *id).await?;
            let updated = remove_tag(&current, tag);
            if updated.len() == current.len() {
                return Err(CliError::NotFound(format!(
                    "tag '{tag}' not present on work item {id}"
                )));
            }
            write_tags(clients, ctx, project, *id, &updated, "tag.remove", false).await
        }
        TagCmd::Set { id, tags, dry_run } => {
            let parsed = parse_tags(tags);
            write_tags(clients, ctx, project, *id, &parsed, "tag.set", *dry_run).await
        }
    }
}

/// Split on ';' (and tolerate ',' like the bash script), trim, drop empties.
pub fn parse_tags(raw: &str) -> Vec<String> {
    raw.replace(',', ";")
        .split(';')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Case-insensitive append preserving existing casing and order.
pub fn add_tag(current: &[String], tag: &str) -> Vec<String> {
    let mut out = current.to_vec();
    if !current.iter().any(|t| t.eq_ignore_ascii_case(tag.trim())) {
        out.push(tag.trim().to_string());
    }
    out
}

pub fn remove_tag(current: &[String], tag: &str) -> Vec<String> {
    current
        .iter()
        .filter(|t| !t.eq_ignore_ascii_case(tag.trim()))
        .cloned()
        .collect()
}

async fn current_tags(clients: &Clients, project: &str, id: i32) -> Result<Vec<String>, CliError> {
    let item = clients
        .wit()
        .work_items_client()
        .get_work_item(&clients.org, id, project)
        .fields("System.Tags")
        .send()
        .await?
        .into_body()?;
    let value = serde_json::to_value(&item)?;
    Ok(value
        .get("fields")
        .and_then(|f| f.get("System.Tags"))
        .and_then(|t| t.as_str())
        .map(parse_tags)
        .unwrap_or_default())
}

async fn write_tags(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    tags: &[String],
    operation: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let joined = tags.join("; ");
    let ops = vec![patch::add_field("System.Tags", joined.clone())];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"tags": tags, "patch": serde_json::to_value(&ops)?}),
        }));
    }
    let updated = clients
        .wit()
        .work_items_client()
        .update(&clients.org, ops, id, project)
        .send()
        .await?
        .into_body()?;
    let value = serde_json::to_value(&updated)?;
    let result_tags = value
        .get("fields")
        .and_then(|f| f.get("System.Tags"))
        .cloned()
        .unwrap_or(serde_json::Value::String(joined));
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: json!({"tags": result_tags}),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handles_semicolons_and_commas() {
        assert_eq!(parse_tags("a; b,c ;; "), vec!["a", "b", "c"]);
    }

    #[test]
    fn add_is_case_insensitive_and_preserves_existing() {
        let current = vec!["Manual-Step".to_string(), "qa".to_string()];
        assert_eq!(add_tag(&current, "manual-step"), current);
        assert_eq!(add_tag(&current, "new"), vec!["Manual-Step", "qa", "new"]);
    }

    #[test]
    fn remove_is_case_insensitive() {
        let current = vec!["Manual-Step".to_string(), "qa".to_string()];
        assert_eq!(remove_tag(&current, "MANUAL-STEP"), vec!["qa"]);
    }
}
