//! Work item comments. The comments API is preview-only at 7.1
//! (7.1-preview.4 current); version strings are pinned here and only here,
//! with a one-shot fallback to preview.3 on a version-rejection error.

use crate::cli::items::CommentCmd;
use crate::cli::read_arg_or_file;
use crate::client::{enc, Clients};
use crate::context::{prompt, Ctx};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

const VERSION: &str = "7.1-preview.4";
const FALLBACK_VERSION: &str = "7.1-preview.3";

pub async fn run(
    cmd: &CommentCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        CommentCmd::List { work_item_id } => list(clients, project, *work_item_id).await,
        CommentCmd::Add { work_item_id, text } => {
            let text = read_arg_or_file(text)?;
            add(clients, ctx, project, *work_item_id, &text).await
        }
        CommentCmd::Update {
            work_item_id,
            comment_id,
            text,
        } => {
            let text = read_arg_or_file(text)?;
            update(clients, ctx, project, *work_item_id, *comment_id, &text).await
        }
        CommentCmd::Delete {
            work_item_id,
            comment_id,
            dry_run,
        } => delete(clients, ctx, project, *work_item_id, *comment_id, *dry_run).await,
    }
}

fn base_url(clients: &Clients, project: &str, work_item_id: i32) -> String {
    format!(
        "{}/{}/_apis/wit/workItems/{work_item_id}/comments",
        clients.org_url(),
        enc(project)
    )
}

fn version_rejected(e: &CliError) -> bool {
    let msg = e.to_string();
    msg.contains("api-version") || msg.contains("ApiVersion")
}

async fn list(
    clients: &Clients,
    project: &str,
    work_item_id: i32,
) -> Result<CommandOutput, CliError> {
    let raw = clients.raw();
    let base = base_url(clients, project, work_item_id);
    let mut version = VERSION;
    let mut comments: Vec<Value> = Vec::new();
    let mut continuation: Option<String> = None;
    loop {
        let mut url = format!("{base}?api-version={version}&$top=200&order=asc");
        if let Some(token) = &continuation {
            url.push_str(&format!("&continuationToken={token}"));
        }
        let page = match raw.get_json(&url).await {
            Ok(p) => p,
            Err(e) if version == VERSION && version_rejected(&e) => {
                version = FALLBACK_VERSION;
                continue;
            }
            Err(e) => return Err(e),
        };
        if let Some(items) = page.get("comments").and_then(|c| c.as_array()) {
            comments.extend(items.iter().cloned());
        }
        continuation = page
            .get("continuationToken")
            .and_then(|t| t.as_str())
            .map(String::from);
        if continuation.is_none() {
            break;
        }
    }
    Ok(CommandOutput::List {
        value: comments,
        columns: vec![
            Column::new("ID", "id"),
            Column::new("Author", "createdBy.displayName"),
            Column::new("Created", "createdDate"),
            Column::new("Text", "text"),
        ],
    })
}

async fn post_with_fallback(
    clients: &Clients,
    url_for: impl Fn(&str) -> String,
    body: Value,
    method: reqwest::Method,
) -> Result<Value, CliError> {
    let raw = clients.raw();
    let first = url_for(VERSION);
    let result = match method {
        reqwest::Method::POST => raw.post_json(&first, body.clone()).await,
        reqwest::Method::PATCH => {
            raw.patch_json(&first, body.clone(), "application/json")
                .await
        }
        _ => unreachable!(),
    };
    match result {
        Err(e) if version_rejected(&e) => {
            let second = url_for(FALLBACK_VERSION);
            match method {
                reqwest::Method::POST => raw.post_json(&second, body).await,
                reqwest::Method::PATCH => raw.patch_json(&second, body, "application/json").await,
                _ => unreachable!(),
            }
        }
        other => other,
    }
}

async fn add(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    work_item_id: i32,
    text: &str,
) -> Result<CommandOutput, CliError> {
    let base = base_url(clients, project, work_item_id);
    let created = post_with_fallback(
        clients,
        |v| format!("{base}?api-version={v}"),
        json!({"text": text}),
        reqwest::Method::POST,
    )
    .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "comment.add".into(),
        target: ctx.target(Some(work_item_id as i64))?,
        dry_run: false,
        result: created,
    }))
}

async fn update(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    work_item_id: i32,
    comment_id: i32,
    text: &str,
) -> Result<CommandOutput, CliError> {
    let base = base_url(clients, project, work_item_id);
    let updated = post_with_fallback(
        clients,
        |v| format!("{base}/{comment_id}?api-version={v}"),
        json!({"text": text}),
        reqwest::Method::PATCH,
    )
    .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "comment.update".into(),
        target: ctx.target(Some(work_item_id as i64))?,
        dry_run: false,
        result: updated,
    }))
}

async fn delete(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    work_item_id: i32,
    comment_id: i32,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "comment.delete".into(),
            target: ctx.target(Some(work_item_id as i64))?,
            dry_run: true,
            result: json!({"commentId": comment_id}),
        }));
    }
    prompt::confirm(
        ctx.assume_yes,
        prompt::Danger::Destructive,
        &format!("Delete comment {comment_id} on work item {work_item_id}?"),
    )?;
    let base = base_url(clients, project, work_item_id);
    let raw = clients.raw();
    let url = format!("{base}/{comment_id}?api-version={VERSION}");
    let result = match raw.delete(&url).await {
        Err(e) if version_rejected(&e) => {
            raw.delete(&format!(
                "{base}/{comment_id}?api-version={FALLBACK_VERSION}"
            ))
            .await?
        }
        other => other?,
    };
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "comment.delete".into(),
        target: ctx.target(Some(work_item_id as i64))?,
        dry_run: false,
        result,
    }))
}
