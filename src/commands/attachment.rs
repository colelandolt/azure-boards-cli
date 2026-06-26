use crate::cli::items::AttachmentCmd;
use crate::client::{enc, Clients};
use crate::context::Ctx;
use crate::domain::patch;
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};
use std::path::Path;

pub async fn run(
    cmd: &AttachmentCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        AttachmentCmd::List { id } => list(clients, project, *id).await,
        AttachmentCmd::Download {
            id,
            name,
            output_dir,
        } => download(clients, project, *id, name.as_deref(), output_dir).await,
        AttachmentCmd::Add {
            id,
            file,
            comment,
            dry_run,
        } => {
            add(
                clients,
                ctx,
                project,
                *id,
                file,
                comment.as_deref(),
                *dry_run,
            )
            .await
        }
        AttachmentCmd::Remove { id, name, dry_run } => {
            remove(clients, ctx, project, *id, name, *dry_run).await
        }
    }
}

/// AttachedFile relations as {index, name, size, url, comment}.
async fn attachments_of(clients: &Clients, project: &str, id: i32) -> Result<Vec<Value>, CliError> {
    let item = super::relation::fetch_with_relations(clients, project, id).await?;
    let relations = item
        .get("relations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(relations
        .iter()
        .enumerate()
        .filter(|(_, r)| r.get("rel").and_then(|v| v.as_str()) == Some("AttachedFile"))
        .map(|(i, r)| {
            let attrs = r.get("attributes").cloned().unwrap_or(Value::Null);
            json!({
                "index": i,
                "name": attrs.get("name").cloned().unwrap_or(Value::Null),
                "size": attrs.get("resourceSize").cloned().unwrap_or(Value::Null),
                "comment": attrs.get("comment").cloned().unwrap_or(Value::Null),
                "url": r.get("url").cloned().unwrap_or(Value::Null),
            })
        })
        .collect())
}

async fn list(clients: &Clients, project: &str, id: i32) -> Result<CommandOutput, CliError> {
    Ok(CommandOutput::List {
        value: attachments_of(clients, project, id).await?,
        columns: vec![
            Column::new("Name", "name"),
            Column::new("Size", "size"),
            Column::new("Comment", "comment"),
            Column::new("URL", "url"),
        ],
    })
}

async fn download(
    clients: &Clients,
    project: &str,
    id: i32,
    only_name: Option<&str>,
    output_dir: &Path,
) -> Result<CommandOutput, CliError> {
    let attachments = attachments_of(clients, project, id).await?;
    let selected: Vec<&Value> = attachments
        .iter()
        .filter(|a| match only_name {
            Some(n) => a.get("name").and_then(|v| v.as_str()) == Some(n),
            None => true,
        })
        .collect();
    if selected.is_empty() {
        return Err(CliError::NotFound(match only_name {
            Some(n) => format!("attachment '{n}' on work item {id}"),
            None => format!("attachments on work item {id}"),
        }));
    }
    std::fs::create_dir_all(output_dir)?;
    let raw = clients.raw();
    let mut results = Vec::new();
    for a in selected {
        let url = a.get("url").and_then(|u| u.as_str()).unwrap_or_default();
        let name = a
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("attachment.bin");
        let sep = if url.contains('?') { '&' } else { '?' };
        let bytes = raw
            .download(&format!("{url}{sep}download=true&api-version=7.1"))
            .await?;
        let path = output_dir.join(sanitize_file_name(name));
        std::fs::write(&path, &bytes)?;
        results.push(json!({
            "name": name,
            "file": path.display().to_string(),
            "bytes": bytes.len(),
        }));
    }
    Ok(CommandOutput::List {
        value: results,
        columns: vec![
            Column::new("Name", "name"),
            Column::new("File", "file"),
            Column::new("Bytes", "bytes"),
        ],
    })
}

/// Strip path separators from server-supplied names before writing locally.
fn sanitize_file_name(name: &str) -> String {
    name.replace(['/', '\\'], "_")
}

const SIMPLE_UPLOAD_LIMIT: u64 = 130 * 1024 * 1024;

async fn add(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    file: &Path,
    comment: Option<&str>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CliError::Validation(format!("bad file path {}", file.display())))?
        .to_string();
    let meta = std::fs::metadata(file)
        .map_err(|e| CliError::Validation(format!("cannot read {}: {e}", file.display())))?;
    if meta.len() > SIMPLE_UPLOAD_LIMIT {
        return Err(CliError::Validation(format!(
            "{} is {}MB; simple upload supports up to 130MB",
            file.display(),
            meta.len() / (1024 * 1024)
        )));
    }
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "attachment.add".into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"file": file.display().to_string(), "name": name, "bytes": meta.len()}),
        }));
    }
    let bytes = std::fs::read(file)?;
    let upload_url = format!(
        "{}/{}/_apis/wit/attachments?fileName={}&api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(&name)
    );
    let uploaded = clients.raw().post_bytes(&upload_url, bytes).await?;
    let attachment_url = uploaded
        .get("url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| CliError::General("attachment upload returned no url".into()))?;
    let mut attributes = json!({"name": name});
    if let Some(c) = comment {
        attributes["comment"] = Value::String(c.to_string());
    }
    let ops = vec![patch::add_relation(
        "AttachedFile",
        attachment_url,
        attributes,
    )];
    let updated = clients
        .wit()
        .work_items_client()
        .update(&clients.org, ops, id, project)
        .send()
        .await?
        .into_body()?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "attachment.add".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: json!({
            "attachment": uploaded,
            "rev": updated.rev,
        }),
    }))
}

async fn remove(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    name: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let attachments = attachments_of(clients, project, id).await?;
    let matching: Vec<&Value> = attachments
        .iter()
        .filter(|a| a.get("name").and_then(|n| n.as_str()) == Some(name))
        .collect();
    let index = match matching.as_slice() {
        [] => {
            return Err(CliError::NotFound(format!(
                "attachment '{name}' on work item {id}"
            )))
        }
        [one] => one.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize,
        many => {
            return Err(CliError::Validation(format!(
                "{} attachments named '{name}'; remove by relation index instead: {:?}",
                many.len(),
                many.iter()
                    .filter_map(|a| a.get("index").and_then(|i| i.as_u64()))
                    .collect::<Vec<_>>()
            )))
        }
    };
    super::relation::remove_relation_checked(clients, ctx, id, index, "attachment.remove", dry_run)
        .await
}
