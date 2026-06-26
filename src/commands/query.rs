//! Saved queries (_apis/wit/queries): list, show, run, create, update, delete.

use crate::cli::read_arg_or_file;
use crate::cli::work_item::QueryCmd;
use crate::client::{enc, Clients};
use crate::context::{prompt, Ctx};
use crate::domain::hydrate;
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(cmd: &QueryCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        QueryCmd::List { folder } => list(clients, project, folder.as_deref()).await,
        QueryCmd::Show { query } => show(clients, project, query).await,
        QueryCmd::Run { query, fields } => {
            run_query(clients, project, query, fields.as_deref()).await
        }
        QueryCmd::Create {
            name,
            folder,
            wiql,
            dry_run,
        } => create(clients, ctx, project, name, folder, wiql, *dry_run).await,
        QueryCmd::Update {
            query,
            wiql,
            name,
            dry_run,
        } => {
            update(
                clients,
                ctx,
                project,
                query,
                wiql.as_deref(),
                name.as_deref(),
                *dry_run,
            )
            .await
        }
        QueryCmd::Delete { query, dry_run } => delete(clients, ctx, project, query, *dry_run).await,
    }
}

fn queries_url(clients: &Clients, project: &str) -> String {
    format!("{}/{}/_apis/wit/queries", clients.org_url(), enc(project))
}

/// Encode a query path's segments, preserving '/' separators.
fn enc_path(path: &str) -> String {
    path.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn flatten_queries(node: &Value, out: &mut Vec<Value>) {
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            out.push(json!({
                "id": child.get("id").cloned().unwrap_or(Value::Null),
                "name": child.get("name").cloned().unwrap_or(Value::Null),
                "path": child.get("path").cloned().unwrap_or(Value::Null),
                "isFolder": child.get("isFolder").cloned().unwrap_or(Value::Bool(false)),
                "isPublic": child.get("isPublic").cloned().unwrap_or(Value::Null),
            }));
            flatten_queries(child, out);
        }
    }
}

async fn list(
    clients: &Clients,
    project: &str,
    folder: Option<&str>,
) -> Result<CommandOutput, CliError> {
    let raw = clients.raw();
    let mut out = Vec::new();
    match folder {
        Some(f) => {
            let url = format!(
                "{}/{}?$depth=2&api-version=7.1",
                queries_url(clients, project),
                enc_path(f)
            );
            let node = raw.get_json(&url).await?;
            flatten_queries(&node, &mut out);
        }
        None => {
            let url = format!("{}?$depth=2&api-version=7.1", queries_url(clients, project));
            let roots = raw.get_json(&url).await?;
            if let Some(values) = roots.get("value").and_then(|v| v.as_array()) {
                for root in values {
                    out.push(json!({
                        "id": root.get("id").cloned().unwrap_or(Value::Null),
                        "name": root.get("name").cloned().unwrap_or(Value::Null),
                        "path": root.get("path").cloned().unwrap_or(Value::Null),
                        "isFolder": root.get("isFolder").cloned().unwrap_or(Value::Bool(true)),
                        "isPublic": root.get("isPublic").cloned().unwrap_or(Value::Null),
                    }));
                    flatten_queries(root, &mut out);
                }
            }
        }
    }
    Ok(CommandOutput::List {
        value: out,
        columns: vec![
            Column::new("Path", "path"),
            Column::new("Folder", "isFolder"),
            Column::new("ID", "id"),
        ],
    })
}

async fn fetch_query(clients: &Clients, project: &str, query: &str) -> Result<Value, CliError> {
    let url = format!(
        "{}/{}?$expand=wiql&api-version=7.1",
        queries_url(clients, project),
        enc_path(query)
    );
    clients.raw().get_json(&url).await
}

async fn show(clients: &Clients, project: &str, query: &str) -> Result<CommandOutput, CliError> {
    Ok(CommandOutput::Item {
        value: fetch_query(clients, project, query).await?,
        columns: vec![],
    })
}

async fn run_query(
    clients: &Clients,
    project: &str,
    query: &str,
    fields: Option<&[String]>,
) -> Result<CommandOutput, CliError> {
    let q = fetch_query(clients, project, query).await?;
    let id = q
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| CliError::NotFound(format!("saved query '{query}'")))?;
    if q.get("isFolder").and_then(|f| f.as_bool()) == Some(true) {
        return Err(CliError::Validation(format!("'{query}' is a folder")));
    }
    let url = format!(
        "{}/{}/_apis/wit/wiql/{id}?api-version=7.1",
        clients.org_url(),
        enc(project)
    );
    let result = clients.raw().get_json(&url).await?;
    let ids = super::wiql_cmd::collect_ids(&result);
    let default = hydrate::default_fields();
    let field_list = fields.map(|f| f.to_vec()).unwrap_or(default);
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&field_list)).await?;
    Ok(CommandOutput::List {
        value: hydrated.items,
        columns: hydrate::work_item_columns(),
    })
}

async fn create(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    name: &str,
    folder: &str,
    wiql: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let wiql_text = read_arg_or_file(wiql)?;
    let body = json!({"name": name, "wiql": wiql_text});
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "query.create".into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: json!({"folder": folder, "body": body}),
        }));
    }
    let url = format!(
        "{}/{}?api-version=7.1",
        queries_url(clients, project),
        enc_path(folder)
    );
    let created = clients.raw().post_json(&url, body).await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "query.create".into(),
        target: ctx.target(None)?,
        dry_run: false,
        result: created,
    }))
}

async fn update(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    query: &str,
    wiql: Option<&str>,
    name: Option<&str>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let mut body = serde_json::Map::new();
    if let Some(w) = wiql {
        body.insert("wiql".into(), Value::String(read_arg_or_file(w)?));
    }
    if let Some(n) = name {
        body.insert("name".into(), Value::String(n.to_string()));
    }
    if body.is_empty() {
        return Err(CliError::Validation("pass --wiql and/or --name".into()));
    }
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "query.update".into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: Value::Object(body),
        }));
    }
    let url = format!(
        "{}/{}?api-version=7.1",
        queries_url(clients, project),
        enc_path(query)
    );
    let updated = clients
        .raw()
        .patch_json(&url, Value::Object(body), "application/json")
        .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "query.update".into(),
        target: ctx.target(None)?,
        dry_run: false,
        result: updated,
    }))
}

async fn delete(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    query: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "query.delete".into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: json!({"query": query}),
        }));
    }
    prompt::confirm(
        ctx.assume_yes,
        prompt::Danger::Destructive,
        &format!("Delete saved query '{query}' (folders delete their contents)?"),
    )?;
    let url = format!(
        "{}/{}?api-version=7.1",
        queries_url(clients, project),
        enc_path(query)
    );
    let result = clients.raw().delete(&url).await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "query.delete".into(),
        target: ctx.target(None)?,
        dry_run: false,
        result,
    }))
}
