use crate::cli::planning::{IterationCmd, IterationProjectCmd, IterationTeamCmd};
use crate::client::{enc, Clients};
use crate::context::{prompt, Ctx};
use crate::domain::classification::{self, Group};
use crate::domain::{hydrate, macros};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(
    cmd: &IterationCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    match cmd {
        IterationCmd::Project { cmd } => project(cmd, ctx, clients).await,
        IterationCmd::Team { cmd } => team(cmd, ctx, clients).await,
        IterationCmd::Current => current(ctx, clients).await,
        IterationCmd::Import { file, dry_run } => import(ctx, clients, file, *dry_run).await,
    }
}

fn iteration_columns() -> Vec<Column> {
    vec![
        Column::new("Name", "name"),
        Column::new("Path", "path"),
        Column::new("Start", "startDate"),
        Column::new("Finish", "finishDate"),
        Column::new("Current", "current"),
        Column::new("Ends Soon", "endsSoon"),
    ]
}

async fn project(
    cmd: &IterationProjectCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    match cmd {
        IterationProjectCmd::List { depth, under } => {
            let tree = classification::get_tree(clients, proj, Group::Iterations, *depth).await?;
            let mut nodes = classification::flatten(&tree);
            // The root node is the project itself; drop it from listings.
            if !nodes.is_empty() {
                nodes.remove(0);
            }
            if let Some(prefix) = under {
                nodes.retain(|n| {
                    n.path
                        .to_ascii_lowercase()
                        .contains(&prefix.to_ascii_lowercase())
                });
            }
            let today = time::OffsetDateTime::now_utc().date();
            Ok(CommandOutput::List {
                value: classification::enrich_iterations(&nodes, today),
                columns: iteration_columns(),
            })
        }
        IterationProjectCmd::Show { path } => {
            let node = classification::get_node(clients, proj, Group::Iterations, path, 1).await?;
            Ok(CommandOutput::Item {
                value: node,
                columns: vec![],
            })
        }
        IterationProjectCmd::Create {
            name,
            path,
            start_date,
            finish_date,
            dry_run,
        } => {
            validate_dates(start_date.as_deref(), finish_date.as_deref())?;
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "iteration.create".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"name": name, "parent": path, "startDate": start_date, "finishDate": finish_date}),
                }));
            }
            let created = classification::create_node(
                clients,
                proj,
                Group::Iterations,
                path.as_deref(),
                name,
                start_date.as_deref(),
                finish_date.as_deref(),
            )
            .await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "iteration.create".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result: created,
            }))
        }
        IterationProjectCmd::Update {
            path,
            name,
            start_date,
            finish_date,
            dry_run,
        } => {
            validate_dates(start_date.as_deref(), finish_date.as_deref())?;
            let mut body = serde_json::Map::new();
            if let Some(n) = name {
                body.insert("name".into(), Value::String(n.clone()));
            }
            if let (Some(s), Some(f)) = (start_date, finish_date) {
                body.insert(
                    "attributes".into(),
                    json!({
                        "startDate": format!("{s}T00:00:00Z"),
                        "finishDate": format!("{f}T00:00:00Z"),
                    }),
                );
            }
            if body.is_empty() {
                return Err(CliError::Validation(
                    "nothing to update: pass --name or both --start-date and --finish-date".into(),
                ));
            }
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "iteration.update".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: Value::Object(body),
                }));
            }
            let updated = classification::update_node(
                clients,
                proj,
                Group::Iterations,
                path,
                Value::Object(body),
            )
            .await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "iteration.update".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result: updated,
            }))
        }
        IterationProjectCmd::Delete { path, dry_run } => {
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "iteration.delete".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"path": path}),
                }));
            }
            prompt::confirm(
                ctx.assume_yes,
                prompt::Danger::Destructive,
                &format!("Delete iteration '{path}'?"),
            )?;
            let result =
                classification::delete_node(clients, proj, Group::Iterations, path, None).await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "iteration.delete".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result,
            }))
        }
    }
}

fn validate_dates(start: Option<&str>, finish: Option<&str>) -> Result<(), CliError> {
    match (start, finish) {
        (None, None) => Ok(()),
        (Some(s), Some(f)) => {
            let sd = classification::parse_date(s).ok_or_else(|| {
                CliError::Validation(format!("invalid start date '{s}' (YYYY-MM-DD)"))
            })?;
            let fd = classification::parse_date(f).ok_or_else(|| {
                CliError::Validation(format!("invalid finish date '{f}' (YYYY-MM-DD)"))
            })?;
            if sd > fd {
                return Err(CliError::Validation(format!(
                    "start date {s} is after finish date {f}"
                )));
            }
            Ok(())
        }
        _ => Err(CliError::Validation(
            "start and finish dates must be provided together".into(),
        )),
    }
}

fn team_iterations_url(clients: &Clients, project: &str, team: &str) -> String {
    format!(
        "{}/{}/{}/_apis/work/teamsettings/iterations",
        clients.org_url(),
        enc(project),
        enc(team)
    )
}

async fn team(
    cmd: &IterationTeamCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    let team_name = ctx.team()?;
    let raw = clients.raw();
    match cmd {
        IterationTeamCmd::List { timeframe } => {
            let mut url = format!(
                "{}?api-version=7.1",
                team_iterations_url(clients, proj, team_name)
            );
            if let Some(tf) = timeframe {
                url.push_str(&format!("&$timeframe={tf}"));
            }
            let value = raw.get_json(&url).await?;
            Ok(CommandOutput::List {
                value: value
                    .get("value")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                columns: vec![
                    Column::new("Name", "name"),
                    Column::new("Path", "path"),
                    Column::new("Start", "attributes.startDate"),
                    Column::new("Finish", "attributes.finishDate"),
                    Column::new("Timeframe", "attributes.timeFrame"),
                ],
            })
        }
        IterationTeamCmd::Add { path, dry_run } => {
            // The team-settings API takes the node's identifier GUID.
            let node = classification::get_node(clients, proj, Group::Iterations, path, 0).await?;
            let guid = node
                .get("identifier")
                .and_then(|s| s.as_str())
                .ok_or_else(|| CliError::NotFound(format!("iteration '{path}'")))?;
            let body = json!({"id": guid});
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "iteration.team.add".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"path": path, "identifier": guid}),
                }));
            }
            let url = format!(
                "{}?api-version=7.1",
                team_iterations_url(clients, proj, team_name)
            );
            let result = raw.post_json(&url, body).await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "iteration.team.add".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result,
            }))
        }
        IterationTeamCmd::ListWorkItems { iteration } => {
            let it = macros::resolve(clients, proj, team_name, iteration).await?;
            let id = it.id.ok_or_else(|| {
                CliError::NotFound(format!(
                    "iteration '{}' is not in the team's sprint list",
                    it.path
                ))
            })?;
            let url = format!(
                "{}/{id}/workitems?api-version=7.1",
                team_iterations_url(clients, proj, team_name)
            );
            let value = raw.get_json(&url).await?;
            let ids: Vec<i32> = value
                .get("workItemRelations")
                .and_then(|r| r.as_array())
                .map(|rels| {
                    rels.iter()
                        .filter_map(|r| {
                            r.get("target")
                                .and_then(|t| t.get("id"))
                                .and_then(|i| i.as_i64())
                                .map(|i| i as i32)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let fields = hydrate::default_fields();
            let hydrated = hydrate::fetch(clients, proj, &ids, Some(&fields)).await?;
            Ok(CommandOutput::List {
                value: hydrated.items,
                columns: hydrate::work_item_columns(),
            })
        }
        IterationTeamCmd::Remove { iteration, dry_run } => {
            let it = macros::resolve(clients, proj, team_name, iteration).await?;
            let id = it.id.ok_or_else(|| {
                CliError::NotFound(format!(
                    "iteration '{}' is not in the team's sprint list",
                    it.path
                ))
            })?;
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "iteration.team.remove".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"path": it.path, "identifier": id}),
                }));
            }
            prompt::confirm(
                ctx.assume_yes,
                prompt::Danger::Destructive,
                &format!("Remove iteration '{}' from team {team_name}?", it.path),
            )?;
            let url = format!(
                "{}/{id}?api-version=7.1",
                team_iterations_url(clients, proj, team_name)
            );
            let result = raw.delete(&url).await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "iteration.team.remove".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result,
            }))
        }
        IterationTeamCmd::SetBacklogIteration { path, dry_run } => {
            set_team_setting(
                clients,
                ctx,
                proj,
                team_name,
                "backlogIteration",
                path,
                *dry_run,
            )
            .await
        }
        IterationTeamCmd::ShowBacklogIteration => {
            let settings = team_settings(clients, proj, team_name).await?;
            Ok(CommandOutput::Item {
                value: settings
                    .get("backlogIteration")
                    .cloned()
                    .unwrap_or(Value::Null),
                columns: vec![],
            })
        }
        IterationTeamCmd::SetDefaultIteration { path, dry_run } => {
            if path.eq_ignore_ascii_case("@currentiteration") {
                let body = json!({"defaultIterationMacro": "@CurrentIteration"});
                return patch_team_settings(
                    clients,
                    ctx,
                    proj,
                    team_name,
                    body,
                    "iteration.team.set-default",
                    *dry_run,
                )
                .await;
            }
            set_team_setting(
                clients,
                ctx,
                proj,
                team_name,
                "defaultIteration",
                path,
                *dry_run,
            )
            .await
        }
        IterationTeamCmd::ShowDefaultIteration => {
            let settings = team_settings(clients, proj, team_name).await?;
            let value = json!({
                "defaultIteration": settings.get("defaultIteration").cloned().unwrap_or(Value::Null),
                "defaultIterationMacro": settings.get("defaultIterationMacro").cloned().unwrap_or(Value::Null),
            });
            Ok(CommandOutput::Item {
                value,
                columns: vec![],
            })
        }
    }
}

async fn team_settings(clients: &Clients, project: &str, team: &str) -> Result<Value, CliError> {
    let url = format!(
        "{}/{}/{}/_apis/work/teamsettings?api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(team)
    );
    clients.raw().get_json(&url).await
}

async fn patch_team_settings(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    team: &str,
    body: Value,
    operation: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: body,
        }));
    }
    let url = format!(
        "{}/{}/{}/_apis/work/teamsettings?api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(team)
    );
    let result = clients
        .raw()
        .patch_json(&url, body, "application/json")
        .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(None)?,
        dry_run: false,
        result,
    }))
}

async fn set_team_setting(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    team: &str,
    key: &str,
    path: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let node = classification::get_node(clients, project, Group::Iterations, path, 0).await?;
    let guid = node
        .get("identifier")
        .and_then(|s| s.as_str())
        .ok_or_else(|| CliError::NotFound(format!("iteration '{path}'")))?;
    patch_team_settings(
        clients,
        ctx,
        project,
        team,
        json!({key: guid}),
        &format!("iteration.team.set-{key}"),
        dry_run,
    )
    .await
}

async fn current(ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    // Team current iteration when a team is configured; otherwise fall back
    // to a date-window scan of the project tree (bash-script behavior).
    if let Ok(team_name) = ctx.team() {
        let it = macros::resolve(clients, proj, team_name, "@current").await?;
        return Ok(CommandOutput::Item {
            value: json!({
                "name": it.name,
                "path": it.path,
                "startDate": it.start_date,
                "finishDate": it.finish_date,
                "identifier": it.id,
                "source": "team",
            }),
            columns: vec![],
        });
    }
    let tree = classification::get_tree(clients, proj, Group::Iterations, 10).await?;
    let nodes = classification::flatten(&tree);
    let today = time::OffsetDateTime::now_utc().date();
    let enriched = classification::enrich_iterations(&nodes, today);
    let current = enriched
        .into_iter()
        .filter(|n| {
            n["current"] == true
                && n["path"]
                    .as_str()
                    .map(|p| p.contains('\\'))
                    .unwrap_or(false)
        })
        .max_by_key(|n| {
            n["path"]
                .as_str()
                .map(|p| p.matches('\\').count())
                .unwrap_or(0)
        });
    match current {
        Some(mut node) => {
            node["source"] = Value::String("projectDateScan".into());
            Ok(CommandOutput::Item {
                value: node,
                columns: vec![],
            })
        }
        None => Err(CliError::NotFound(
            "no iteration covers today's date (set --team for team-based resolution)".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Import: generic YAML/JSON sprint plan -> classification node diff.
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub struct ImportPlan {
    /// Optional path prefix under the iteration root (e.g. "2027").
    #[serde(default)]
    pub root: String,
    /// PATCH dates on nodes that already exist.
    #[serde(default, rename = "update-existing")]
    pub update_existing: bool,
    pub nodes: Vec<PlanNode>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PlanNode {
    pub name: String,
    #[serde(default, deserialize_with = "de_date")]
    pub start: Option<String>,
    #[serde(default, deserialize_with = "de_date")]
    pub finish: Option<String>,
    #[serde(default)]
    pub children: Vec<PlanNode>,
}

/// YAML can parse bare dates as dates or strings depending on the parser;
/// accept either and normalize to YYYY-MM-DD strings.
fn de_date<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v: Option<serde_norway::Value> = serde::Deserialize::deserialize(d)?;
    Ok(v.and_then(|v| match v {
        serde_norway::Value::String(s) => Some(s),
        other => serde_norway::to_string(&other)
            .ok()
            .map(|s| s.trim().to_string()),
    }))
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub enum Action {
    Create,
    UpdateDates,
    Skip,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanStep {
    pub action: Action,
    /// Parent path (under project root), "" = root.
    pub parent: String,
    pub name: String,
    pub start: Option<String>,
    pub finish: Option<String>,
}

pub fn validate_plan(plan: &ImportPlan) -> Result<Vec<String>, CliError> {
    let mut warnings = Vec::new();
    fn check(
        nodes: &[PlanNode],
        parent: &str,
        parent_range: Option<(&str, &str)>,
        warnings: &mut Vec<String>,
    ) -> Result<(), CliError> {
        let mut seen = std::collections::HashSet::new();
        for node in nodes {
            if node.name.trim().is_empty() || node.name.contains('\\') || node.name.contains('/') {
                return Err(CliError::Validation(format!(
                    "invalid node name '{}' under '{parent}'",
                    node.name
                )));
            }
            if !seen.insert(node.name.to_ascii_lowercase()) {
                return Err(CliError::Validation(format!(
                    "duplicate sibling name '{}' under '{parent}'",
                    node.name
                )));
            }
            match (&node.start, &node.finish) {
                (Some(s), Some(f)) => {
                    let sd = classification::parse_date(s).ok_or_else(|| {
                        CliError::Validation(format!("invalid date '{s}' on '{}'", node.name))
                    })?;
                    let fd = classification::parse_date(f).ok_or_else(|| {
                        CliError::Validation(format!("invalid date '{f}' on '{}'", node.name))
                    })?;
                    if sd > fd {
                        return Err(CliError::Validation(format!(
                            "'{}': start {s} after finish {f}",
                            node.name
                        )));
                    }
                    if let Some((ps, pf)) = parent_range {
                        if s.as_str() < ps || f.as_str() > pf {
                            warnings.push(format!(
                                "'{}' ({s}..{f}) extends outside parent range ({ps}..{pf})",
                                node.name
                            ));
                        }
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(CliError::Validation(format!(
                        "'{}': start and finish must be provided together",
                        node.name
                    )))
                }
            }
            let range = node.start.as_deref().zip(node.finish.as_deref());
            check(&node.children, &node.name, range, warnings)?;
        }
        Ok(())
    }
    check(&plan.nodes, "(root)", None, &mut warnings)?;
    Ok(warnings)
}

/// Diff the plan against the existing tree. `existing` maps normalized
/// path-under-project (e.g. "2026 Q2\\Lost") to (startDate, finishDate).
pub fn diff_plan(
    plan: &ImportPlan,
    existing: &std::collections::HashMap<String, (Option<String>, Option<String>)>,
) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    fn walk(
        nodes: &[PlanNode],
        parent: &str,
        existing: &std::collections::HashMap<String, (Option<String>, Option<String>)>,
        update_existing: bool,
        steps: &mut Vec<PlanStep>,
    ) {
        for node in nodes {
            let path = if parent.is_empty() {
                node.name.clone()
            } else {
                format!("{parent}\\{}", node.name)
            };
            let action = match existing.get(&path) {
                None => Action::Create,
                Some((s, f)) => {
                    let dates_differ =
                        node.start.is_some() && (s != &node.start || f != &node.finish);
                    if dates_differ && update_existing {
                        Action::UpdateDates
                    } else {
                        Action::Skip
                    }
                }
            };
            steps.push(PlanStep {
                action,
                parent: parent.to_string(),
                name: node.name.clone(),
                start: node.start.clone(),
                finish: node.finish.clone(),
            });
            walk(&node.children, &path, existing, update_existing, steps);
        }
    }
    let root = plan.root.trim_matches('\\').to_string();
    walk(
        &plan.nodes,
        &root,
        existing,
        plan.update_existing,
        &mut steps,
    );
    // The root prefix itself must exist first if specified and absent.
    if !root.is_empty() && !existing.contains_key(&root) {
        steps.insert(
            0,
            PlanStep {
                action: Action::Create,
                parent: String::new(),
                name: root,
                start: None,
                finish: None,
            },
        );
    }
    steps
}

async fn import(
    ctx: &Ctx,
    clients: &Clients,
    file: &std::path::Path,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    let text = std::fs::read_to_string(file)
        .map_err(|e| CliError::Validation(format!("cannot read {}: {e}", file.display())))?;
    let plan: ImportPlan = if file.extension().and_then(|e| e.to_str()) == Some("json") {
        serde_json::from_str(&text)
            .map_err(|e| CliError::Validation(format!("invalid plan JSON: {e}")))?
    } else {
        serde_norway::from_str(&text)
            .map_err(|e| CliError::Validation(format!("invalid plan YAML: {e}")))?
    };
    let warnings = validate_plan(&plan)?;
    for w in &warnings {
        tracing::warn!("{w}");
    }

    // Existing tree, keyed by path under the project root.
    let tree = classification::get_tree(clients, proj, Group::Iterations, 10).await?;
    let nodes = classification::flatten(&tree);
    let mut existing = std::collections::HashMap::new();
    for n in &nodes {
        let under_project = n
            .path
            .split_once('\\')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_default();
        if !under_project.is_empty() {
            existing.insert(under_project, (n.start_date.clone(), n.finish_date.clone()));
        }
    }
    let steps = diff_plan(&plan, &existing);

    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "iteration.import".into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: json!({
                "file": file.display().to_string(),
                "warnings": warnings,
                "plan": serde_json::to_value(&steps)?,
                "summary": summarize(&steps),
            }),
        }));
    }

    let mut results = Vec::new();
    for step in &steps {
        match step.action {
            Action::Skip => results.push(json!({"name": step.name, "status": "skipped"})),
            Action::Create => {
                let parent = if step.parent.is_empty() {
                    None
                } else {
                    Some(step.parent.as_str())
                };
                match classification::create_node(
                    clients,
                    proj,
                    Group::Iterations,
                    parent,
                    &step.name,
                    step.start.as_deref(),
                    step.finish.as_deref(),
                )
                .await
                {
                    Ok(v) => results.push(json!({
                        "name": step.name,
                        "status": "created",
                        "id": v.get("id").cloned().unwrap_or(Value::Null),
                    })),
                    Err(e) => results.push(json!({
                        "name": step.name,
                        "status": "error",
                        "error": e.to_string(),
                    })),
                }
            }
            Action::UpdateDates => {
                let path = if step.parent.is_empty() {
                    step.name.clone()
                } else {
                    format!("{}\\{}", step.parent, step.name)
                };
                let body = json!({"attributes": {
                    "startDate": step.start.as_deref().map(|s| format!("{s}T00:00:00Z")),
                    "finishDate": step.finish.as_deref().map(|f| format!("{f}T00:00:00Z")),
                }});
                match classification::update_node(clients, proj, Group::Iterations, &path, body)
                    .await
                {
                    Ok(_) => results.push(json!({"name": step.name, "status": "datesUpdated"})),
                    Err(e) => results.push(json!({
                        "name": step.name,
                        "status": "error",
                        "error": e.to_string(),
                    })),
                }
            }
        }
    }
    let failed = results.iter().filter(|r| r["status"] == "error").count();
    let succeeded = results.len() - failed;
    if failed > 0 && succeeded > 0 {
        return Err(CliError::Partial {
            ok: succeeded,
            failed,
            report: json!({"results": results}),
        });
    }
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "iteration.import".into(),
        target: ctx.target(None)?,
        dry_run: false,
        result: json!({"warnings": warnings, "results": results}),
    }))
}

fn summarize(steps: &[PlanStep]) -> Value {
    let count = |a: Action| steps.iter().filter(|s| s.action == a).count();
    json!({
        "create": count(Action::Create),
        "updateDates": count(Action::UpdateDates),
        "skip": count(Action::Skip),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> ImportPlan {
        serde_norway::from_str(
            r#"
root: ""
update-existing: true
nodes:
  - name: "2027 Q1"
    start: 2027-01-01
    finish: 2027-03-31
    children:
      - name: "Sprint 1"
        start: 2027-01-04
        finish: 2027-01-15
      - name: "Sprint 2"
        start: 2027-01-18
        finish: 2027-01-29
"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_yaml_with_bare_dates() {
        let plan = sample_plan();
        assert_eq!(plan.nodes[0].name, "2027 Q1");
        assert_eq!(plan.nodes[0].start.as_deref(), Some("2027-01-01"));
        assert_eq!(plan.nodes[0].children.len(), 2);
    }

    #[test]
    fn validates_ok_and_rejects_bad_plans() {
        assert!(validate_plan(&sample_plan()).unwrap().is_empty());

        let dup: ImportPlan = serde_norway::from_str("nodes:\n  - name: A\n  - name: a\n").unwrap();
        assert_eq!(validate_plan(&dup).unwrap_err().exit_code(), 7);

        let inverted: ImportPlan = serde_norway::from_str(
            "nodes:\n  - name: A\n    start: 2027-02-01\n    finish: 2027-01-01\n",
        )
        .unwrap();
        assert_eq!(validate_plan(&inverted).unwrap_err().exit_code(), 7);
    }

    #[test]
    fn warns_when_child_outside_parent_range() {
        let plan: ImportPlan = serde_norway::from_str(
            r#"
nodes:
  - name: Q1
    start: 2027-01-01
    finish: 2027-03-31
    children:
      - name: Overflow
        start: 2027-03-25
        finish: 2027-04-10
"#,
        )
        .unwrap();
        let warnings = validate_plan(&plan).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Overflow"));
    }

    #[test]
    fn diff_marks_create_skip_update() {
        let plan = sample_plan();
        let mut existing = std::collections::HashMap::new();
        existing.insert(
            "2027 Q1".to_string(),
            (
                Some("2027-01-01".to_string()),
                Some("2027-03-31".to_string()),
            ),
        );
        existing.insert(
            "2027 Q1\\Sprint 1".to_string(),
            (
                Some("2027-01-05".to_string()),
                Some("2027-01-15".to_string()),
            ),
        );
        let steps = diff_plan(&plan, &existing);
        let by_name: std::collections::HashMap<&str, &PlanStep> =
            steps.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(by_name["2027 Q1"].action, Action::Skip);
        assert_eq!(by_name["Sprint 1"].action, Action::UpdateDates); // dates differ
        assert_eq!(by_name["Sprint 2"].action, Action::Create);
    }

    #[test]
    fn diff_creates_missing_root_first() {
        let mut plan = sample_plan();
        plan.root = "2027".into();
        let steps = diff_plan(&plan, &std::collections::HashMap::new());
        assert_eq!(steps[0].name, "2027");
        assert_eq!(steps[0].action, Action::Create);
        assert_eq!(steps[1].parent, "2027");
    }
}
