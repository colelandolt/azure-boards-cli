use crate::cli::read_arg_or_file;
use crate::cli::work_item::WorkItemCmd;
use crate::client::Clients;
use crate::context::{prompt, Ctx};
use crate::domain::{html, hydrate, patch};
use crate::error::CliError;
use crate::output::{CommandOutput, MutationEnvelope};
use azure_devops_rust_api::wit::models::{JsonPatchOperation, WorkItemExpand};
use serde_json::{json, Value};

pub async fn run(
    cmd: &WorkItemCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        WorkItemCmd::Show {
            id,
            fields,
            expand,
            as_of,
            web,
        } => {
            show(
                clients,
                ctx,
                project,
                *id,
                fields.as_deref(),
                expand.as_deref(),
                as_of.as_deref(),
                *web,
            )
            .await
        }
        WorkItemCmd::BatchGet { ids, fields } => {
            batch_get(clients, project, ids, fields.as_deref()).await
        }
        WorkItemCmd::Create {
            r#type,
            title,
            description,
            acceptance_criteria,
            parent,
            area,
            iteration,
            assigned_to,
            fields,
            from_json,
            markdown,
            dry_run,
            web,
        } => {
            create(
                clients,
                ctx,
                project,
                r#type.as_deref(),
                title.as_deref(),
                description.as_deref(),
                acceptance_criteria.as_deref(),
                *parent,
                area.as_deref(),
                iteration.as_deref(),
                assigned_to.as_deref(),
                fields,
                from_json.as_deref(),
                *markdown,
                *dry_run,
                *web,
            )
            .await
        }
        WorkItemCmd::Update {
            id,
            title,
            state,
            reason,
            assigned_to,
            area,
            iteration,
            description,
            acceptance_criteria,
            fields,
            markdown,
            expected_rev,
            dry_run,
        } => {
            update(
                clients,
                ctx,
                project,
                *id,
                title.as_deref(),
                state.as_deref(),
                reason.as_deref(),
                assigned_to.as_deref(),
                area.as_deref(),
                iteration.as_deref(),
                description.as_deref(),
                acceptance_criteria.as_deref(),
                fields,
                *markdown,
                *expected_rev,
                *dry_run,
            )
            .await
        }
        WorkItemCmd::Patch {
            id,
            op,
            path,
            value,
            dry_run,
        } => {
            apply_patch(
                clients,
                ctx,
                project,
                *id,
                op,
                path,
                value.as_deref(),
                *dry_run,
            )
            .await
        }
        WorkItemCmd::Delete {
            id,
            destroy,
            confirm_id,
            dry_run,
        } => delete(clients, ctx, project, *id, *destroy, *confirm_id, *dry_run).await,
        WorkItemCmd::Restore { id } => restore(clients, ctx, project, *id).await,
        WorkItemCmd::Open { id } => open_in_browser(clients, project, *id),
    }
}

/// Convert resolved description / acceptance-criteria from Markdown to HTML
/// when `--markdown` is set; otherwise pass the content through unchanged.
fn maybe_markdown(
    markdown: bool,
    description: Option<String>,
    acceptance_criteria: Option<String>,
) -> (Option<String>, Option<String>) {
    if markdown {
        (
            description.map(|d| crate::domain::markdown::to_html(&d)),
            acceptance_criteria.map(|a| crate::domain::markdown::to_html(&a)),
        )
    } else {
        (description, acceptance_criteria)
    }
}

fn parse_expand(expand: Option<&str>) -> Option<WorkItemExpand> {
    match expand? {
        "relations" => Some(WorkItemExpand::Relations),
        "all" => Some(WorkItemExpand::All),
        "links" => Some(WorkItemExpand::Links),
        "fields" => Some(WorkItemExpand::Fields),
        _ => None,
    }
}

fn parse_as_of(s: &str) -> Result<time::OffsetDateTime, CliError> {
    // Accept YYYY-MM-DD or RFC3339.
    if let Ok(date) = time::Date::parse(
        s,
        &time::macros::format_description!("[year]-[month]-[day]"),
    ) {
        return Ok(date.midnight().assume_utc());
    }
    time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .map_err(|e| CliError::Validation(format!("invalid --as-of '{s}': {e}")))
}

/// Add normalized text for rich-text fields alongside the raw HTML.
fn add_normalized_text(value: &mut Value) {
    let fields = match value.get("fields") {
        Some(f) => f.clone(),
        None => return,
    };
    let mut normalized = serde_json::Map::new();
    for (key, label) in [
        ("System.Description", "description"),
        (
            "Microsoft.VSTS.Common.AcceptanceCriteria",
            "acceptanceCriteria",
        ),
    ] {
        if let Some(html_text) = fields.get(key).and_then(|v| v.as_str()) {
            normalized.insert(label.to_string(), Value::String(html::to_text(html_text)));
        }
    }
    if !normalized.is_empty() {
        value["normalizedText"] = Value::Object(normalized);
    }
}

#[allow(clippy::too_many_arguments)]
async fn show(
    clients: &Clients,
    _ctx: &Ctx,
    project: &str,
    id: i32,
    fields: Option<&[String]>,
    expand: Option<&str>,
    as_of: Option<&str>,
    web: bool,
) -> Result<CommandOutput, CliError> {
    if web {
        return open_in_browser(clients, project, id);
    }
    let client = clients.wit().work_items_client();
    let mut req = client.get_work_item(&clients.org, id, project);
    // fields and $expand are mutually exclusive on this API: with --expand we
    // fetch everything and project client-side.
    let expand_parsed = parse_expand(expand);
    let project_fields: Option<Vec<String>> = match (&expand_parsed, fields) {
        (Some(e), _) => {
            req = req.expand(e.clone());
            fields.map(|f| f.to_vec())
        }
        (None, Some(f)) => {
            req = req.fields(f.join(","));
            None
        }
        (None, None) => None,
    };
    if let Some(s) = as_of {
        req = req.as_of(parse_as_of(s)?);
    }
    let item = req.send().await?.into_body()?;
    let mut value = serde_json::to_value(&item)?;
    if let (Some(keep), Some(field_map)) = (project_fields, value.get_mut("fields")) {
        if let Some(obj) = field_map.as_object_mut() {
            obj.retain(|k, _| keep.iter().any(|f| f == k));
        }
    }
    add_normalized_text(&mut value);
    Ok(CommandOutput::Item {
        value,
        columns: detail_columns(),
    })
}

fn detail_columns() -> Vec<crate::output::Column> {
    use crate::output::Column;
    vec![
        Column::new("ID", "id"),
        Column::new("Type", "fields.System.WorkItemType"),
        Column::new("Title", "fields.System.Title"),
        Column::new("State", "fields.System.State"),
        Column::new("Assigned To", "fields.System.AssignedTo"),
        Column::new("Area", "fields.System.AreaPath"),
        Column::new("Iteration", "fields.System.IterationPath"),
        Column::new("Tags", "fields.System.Tags"),
        Column::new("Rev", "rev"),
    ]
}

async fn batch_get(
    clients: &Clients,
    project: &str,
    ids: &[i32],
    fields: Option<&[String]>,
) -> Result<CommandOutput, CliError> {
    let default = hydrate::default_fields();
    let fields = fields.map(|f| f.to_vec()).unwrap_or(default);
    let result = hydrate::fetch(clients, project, ids, Some(&fields)).await?;
    if !result.omitted.is_empty() {
        tracing::warn!("omitted (deleted or inaccessible): {:?}", result.omitted);
    }
    Ok(CommandOutput::List {
        value: result.items,
        columns: hydrate::work_item_columns(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn create(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    type_: Option<&str>,
    title: Option<&str>,
    description: Option<&str>,
    acceptance_criteria: Option<&str>,
    parent: Option<i32>,
    area: Option<&str>,
    iteration: Option<&str>,
    assigned_to: Option<&str>,
    extra_fields: &[(String, String)],
    from_json: Option<&str>,
    markdown: bool,
    dry_run: bool,
    web: bool,
) -> Result<CommandOutput, CliError> {
    // Two construction modes: structured --from-json, or the field flags.
    let (resolved_type, ops) = if let Some(src) = from_json {
        if title.is_some()
            || description.is_some()
            || acceptance_criteria.is_some()
            || parent.is_some()
            || area.is_some()
            || iteration.is_some()
            || assigned_to.is_some()
            || !extra_fields.is_empty()
        {
            return Err(CliError::Validation(
                "--from-json cannot be combined with --title/--description/--field/etc.".into(),
            ));
        }
        let content = read_arg_or_file(src)?;
        let doc: Value = serde_json::from_str(&content)
            .map_err(|e| CliError::Validation(format!("--from-json is not valid JSON: {e}")))?;
        // --type flag wins over a "type" key in the document.
        let ty = type_
            .map(String::from)
            .or_else(|| doc.get("type").and_then(|t| t.as_str()).map(String::from))
            .ok_or_else(|| {
                CliError::Validation(
                    "work item type required: pass --type or a \"type\" key in --from-json".into(),
                )
            })?;
        let ops = patch::ops_from_json(&doc, &clients.org_url()).map_err(CliError::Validation)?;
        (ty, ops)
    } else {
        let type_ = type_.ok_or_else(|| CliError::Validation("--type is required".into()))?;
        let title = title.ok_or_else(|| CliError::Validation("--title is required".into()))?;
        // Resolve @file / @- on free-text fields, then optionally Markdown->HTML.
        let description = description.map(read_arg_or_file).transpose()?;
        let acceptance_criteria = acceptance_criteria.map(read_arg_or_file).transpose()?;
        let (description, acceptance_criteria) =
            maybe_markdown(markdown, description, acceptance_criteria);
        let mut pairs = crate::cli::work_item::sugar_fields(
            Some(title),
            None,
            None,
            assigned_to,
            area,
            iteration,
            description.as_deref(),
            acceptance_criteria.as_deref(),
        );
        for (k, v) in extra_fields {
            pairs.push((k.clone(), read_arg_or_file(v)?));
        }
        let mut ops = patch::field_ops(&pairs);
        if let Some(parent_id) = parent {
            ops.push(patch::add_relation(
                "System.LinkTypes.Hierarchy-Reverse",
                &format!("{}/_apis/wit/workItems/{parent_id}", clients.org_url()),
                json!({"comment": ""}),
            ));
        }
        (type_.to_string(), ops)
    };

    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "work-item.create".into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: json!({"type": resolved_type, "patch": serde_json::to_value(&ops)?}),
        }));
    }
    let created = clients
        .wit()
        .work_items_client()
        .create(&clients.org, ops, project, resolved_type.as_str())
        .send()
        .await?
        .into_body()?;
    let value = serde_json::to_value(&created)?;
    let id = value.get("id").and_then(|i| i.as_i64());
    if web {
        if let Some(id) = id {
            let _ = open::that(clients.work_item_web_url(project, id as i32));
        }
    }
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "work-item.create".into(),
        target: ctx.target(id)?,
        dry_run: false,
        result: value,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn update(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    title: Option<&str>,
    state: Option<&str>,
    reason: Option<&str>,
    assigned_to: Option<&str>,
    area: Option<&str>,
    iteration: Option<&str>,
    description: Option<&str>,
    acceptance_criteria: Option<&str>,
    extra_fields: &[(String, String)],
    markdown: bool,
    expected_rev: Option<i32>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    // Resolve @file / @- on free-text fields, then optionally Markdown->HTML.
    let description = description.map(read_arg_or_file).transpose()?;
    let acceptance_criteria = acceptance_criteria.map(read_arg_or_file).transpose()?;
    let (description, acceptance_criteria) =
        maybe_markdown(markdown, description, acceptance_criteria);
    let mut pairs = crate::cli::work_item::sugar_fields(
        title,
        state,
        reason,
        assigned_to,
        area,
        iteration,
        description.as_deref(),
        acceptance_criteria.as_deref(),
    );
    // Resolve @file / @- on -f values (e.g. -f System.Description=@desc.md).
    for (k, v) in extra_fields {
        pairs.push((k.clone(), read_arg_or_file(v)?));
    }
    if pairs.is_empty() {
        return Err(CliError::Validation(
            "nothing to update: pass --title/--state/... or --field REF=VALUE".into(),
        ));
    }
    let mut ops: Vec<JsonPatchOperation> = Vec::new();
    if let Some(rev) = expected_rev {
        ops.push(patch::test_rev(rev));
    }
    ops.extend(patch::field_ops(&pairs));
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "work-item.update".into(),
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
        operation: "work-item.update".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: serde_json::to_value(&updated)?,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn apply_patch(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    op: &str,
    path: &str,
    value: Option<&str>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let parsed_op = patch::parse_op(op).ok_or_else(|| {
        CliError::Validation(format!(
            "unknown patch op '{op}' (add, replace, remove, test, copy, move)"
        ))
    })?;
    // Resolve @file / @- first, then JSON-vs-string detection on the content.
    let resolved = value.map(read_arg_or_file).transpose()?;
    let operation = JsonPatchOperation {
        op: Some(parsed_op),
        path: Some(path.to_string()),
        value: resolved.as_deref().map(patch::parse_value),
        from: None,
    };
    let ops = vec![operation];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "work-item.patch".into(),
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
        operation: "work-item.patch".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: serde_json::to_value(&updated)?,
    }))
}

async fn delete(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
    destroy: bool,
    confirm_id: Option<i64>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let operation = if destroy {
        "work-item.destroy"
    } else {
        "work-item.delete"
    };
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"id": id, "destroy": destroy, "recoverable": !destroy}),
        }));
    }
    if destroy {
        // --yes alone is never sufficient for permanent destruction.
        prompt::require_confirm_id(id as i64, confirm_id)?;
        prompt::confirm(
            ctx_yes(ctx),
            prompt::Danger::Permanent,
            &format!("Permanently destroy work item {id}?"),
        )?;
    } else {
        prompt::confirm(
            ctx_yes(ctx),
            prompt::Danger::Destructive,
            &format!("Move work item {id} to the recycle bin?"),
        )?;
    }
    let mut req = clients
        .wit()
        .work_items_client()
        .delete(&clients.org, id, project);
    if destroy {
        req = req.destroy(true);
    }
    let deleted = req.send().await?.into_body()?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: serde_json::to_value(&deleted)?,
    }))
}

async fn restore(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    id: i32,
) -> Result<CommandOutput, CliError> {
    let body = azure_devops_rust_api::wit::models::WorkItemDeleteUpdate {
        is_deleted: Some(false),
    };
    let restored = clients
        .wit()
        .recyclebin_client()
        .restore_work_item(&clients.org, body, id, project)
        .send()
        .await?
        .into_body()?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "work-item.restore".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: serde_json::to_value(&restored)?,
    }))
}

fn open_in_browser(clients: &Clients, project: &str, id: i32) -> Result<CommandOutput, CliError> {
    let url = clients.work_item_web_url(project, id);
    open::that(&url).map_err(|e| CliError::General(format!("failed to open browser: {e}")))?;
    Ok(CommandOutput::Item {
        value: json!({"opened": url}),
        columns: vec![],
    })
}

/// The global --yes flag travels on Ctx for handlers that confirm.
fn ctx_yes(ctx: &Ctx) -> bool {
    ctx.assume_yes
}
