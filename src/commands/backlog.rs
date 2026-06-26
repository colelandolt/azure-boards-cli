use crate::cli::boards::BacklogCmd;
use crate::client::{enc, Clients};
use crate::context::Ctx;
use crate::domain::{hydrate, macros, patch};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(
    cmd: &BacklogCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let team = ctx.team()?;
    match cmd {
        BacklogCmd::Levels => levels(clients, project, team).await,
        BacklogCmd::List { level, fields } => {
            list(clients, project, team, level.as_deref(), fields.as_deref()).await
        }
        BacklogCmd::Prioritize {
            id,
            before,
            after,
            dry_run,
        } => prioritize(clients, ctx, project, team, *id, *before, *after, *dry_run).await,
        BacklogCmd::Move {
            id,
            to_iteration,
            dry_run,
        } => move_item(clients, ctx, project, team, *id, to_iteration, *dry_run).await,
        BacklogCmd::Forecast {
            velocity,
            field,
            level,
        } => {
            forecast(
                clients,
                project,
                team,
                *velocity,
                field.as_deref(),
                level.as_deref(),
            )
            .await
        }
    }
}

fn team_url(clients: &Clients, project: &str, team: &str) -> String {
    format!("{}/{}/{}", clients.org_url(), enc(project), enc(team))
}

async fn backlog_levels(
    clients: &Clients,
    project: &str,
    team: &str,
) -> Result<Vec<Value>, CliError> {
    let value = clients
        .raw()
        .get_json(&format!(
            "{}/_apis/work/backlogs?api-version=7.1",
            team_url(clients, project, team)
        ))
        .await?;
    Ok(value
        .get("value")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

async fn levels(clients: &Clients, project: &str, team: &str) -> Result<CommandOutput, CliError> {
    Ok(CommandOutput::List {
        value: backlog_levels(clients, project, team).await?,
        columns: vec![
            Column::new("ID", "id"),
            Column::new("Name", "name"),
            Column::new("Rank", "rank"),
            Column::new("Type", "type"),
        ],
    })
}

/// Resolve a level by id or name; default = the requirement-level backlog
/// (the one whose id is Microsoft.RequirementCategory when present, else the
/// highest-rank "backlog"-type level).
async fn resolve_level(
    clients: &Clients,
    project: &str,
    team: &str,
    level: Option<&str>,
) -> Result<Value, CliError> {
    let all = backlog_levels(clients, project, team).await?;
    if let Some(wanted) = level {
        return all
            .iter()
            .find(|l| {
                l.get("id").and_then(|v| v.as_str()) == Some(wanted)
                    || l.get("name")
                        .and_then(|v| v.as_str())
                        .map(|n| n.eq_ignore_ascii_case(wanted))
                        .unwrap_or(false)
            })
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = all
                    .iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .collect();
                CliError::NotFound(format!(
                    "backlog level '{wanted}' (available: {})",
                    names.join(", ")
                ))
            });
    }
    all.iter()
        .find(|l| l.get("id").and_then(|v| v.as_str()) == Some("Microsoft.RequirementCategory"))
        .or_else(|| all.first())
        .cloned()
        .ok_or_else(|| CliError::NotFound("no backlog levels for this team".into()))
}

/// Ordered work item IDs for a backlog level (server returns priority order).
async fn backlog_ids(
    clients: &Clients,
    project: &str,
    team: &str,
    level_id: &str,
) -> Result<Vec<i32>, CliError> {
    let value = clients
        .raw()
        .get_json(&format!(
            "{}/_apis/work/backlogs/{}/workItems?api-version=7.1",
            team_url(clients, project, team),
            enc(level_id)
        ))
        .await?;
    Ok(value
        .get("workItems")
        .and_then(|w| w.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|r| {
                    r.get("target")
                        .and_then(|t| t.get("id"))
                        .and_then(|i| i.as_i64())
                        .map(|i| i as i32)
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn list(
    clients: &Clients,
    project: &str,
    team: &str,
    level: Option<&str>,
    fields: Option<&[String]>,
) -> Result<CommandOutput, CliError> {
    let level_value = resolve_level(clients, project, team, level).await?;
    let level_id = level_value
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| CliError::General("backlog level has no id".into()))?;
    let ids = backlog_ids(clients, project, team, level_id).await?;
    let field_list = fields.map(|f| f.to_vec()).unwrap_or_else(|| {
        let mut f = hydrate::default_fields();
        f.push("Microsoft.VSTS.Common.StackRank".into());
        f.push("Microsoft.VSTS.Scheduling.StoryPoints".into());
        f
    });
    // Hydration preserves the backlog priority order.
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&field_list)).await?;
    Ok(CommandOutput::List {
        value: hydrated.items,
        columns: hydrate::work_item_columns(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn prioritize(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    team: &str,
    id: i32,
    before: Option<i32>,
    after: Option<i32>,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let mut body = json!({
        "ids": [id],
        "parentId": 0,
        "iterationPath": null,
    });
    match (before, after) {
        (Some(b), None) => body["nextId"] = json!(b),
        (None, Some(a)) => body["previousId"] = json!(a),
        _ => {
            return Err(CliError::Validation(
                "pass exactly one of --before <id> or --after <id>".into(),
            ))
        }
    }
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "backlog.prioritize".into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: body,
        }));
    }
    let result = clients
        .raw()
        .patch_json(
            &format!(
                "{}/_apis/work/workitemsorder?api-version=7.1",
                team_url(clients, project, team)
            ),
            body,
            "application/json",
        )
        .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: "backlog.prioritize".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result,
    }))
}

async fn move_item(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    team: &str,
    id: i32,
    to_iteration: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let it = macros::resolve(clients, project, team, to_iteration).await?;
    let ops = vec![patch::add_field("System.IterationPath", it.path.clone())];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: "backlog.move".into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"iterationPath": it.path}),
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
        operation: "backlog.move".into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: json!({"rev": updated.rev, "iterationPath": it.path}),
    }))
}

/// The process's configured effort field, from backlogconfiguration.
pub async fn effort_field(
    clients: &Clients,
    project: &str,
    team: &str,
) -> Result<String, CliError> {
    let value = clients
        .raw()
        .get_json(&format!(
            "{}/_apis/work/backlogconfiguration?api-version=7.1",
            team_url(clients, project, team)
        ))
        .await?;
    Ok(value
        .get("backlogFields")
        .and_then(|b| b.get("typeFields"))
        .and_then(|t| t.get("Effort"))
        .and_then(|e| e.as_str())
        .unwrap_or("Microsoft.VSTS.Scheduling.StoryPoints")
        .to_string())
}

async fn forecast(
    clients: &Clients,
    project: &str,
    team: &str,
    velocity: f64,
    field: Option<&str>,
    level: Option<&str>,
) -> Result<CommandOutput, CliError> {
    if velocity <= 0.0 {
        return Err(CliError::Validation("--velocity must be positive".into()));
    }
    let effort = match field {
        Some(f) => f.to_string(),
        None => effort_field(clients, project, team).await?,
    };
    let level_value = resolve_level(clients, project, team, level).await?;
    let level_id = level_value
        .get("id")
        .and_then(|i| i.as_str())
        .ok_or_else(|| CliError::General("backlog level has no id".into()))?;
    let ids = backlog_ids(clients, project, team, level_id).await?;
    let fields = vec![
        "System.Title".to_string(),
        "System.State".to_string(),
        "System.WorkItemType".to_string(),
        effort.clone(),
    ];
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;

    // Future sprints to map onto.
    let iterations = macros::team_iterations(clients, project, team).await?;
    let future: Vec<_> = iterations
        .iter()
        .filter(|i| {
            i.time_frame.as_deref() == Some("future") || i.time_frame.as_deref() == Some("current")
        })
        .collect();

    let mut cumulative = 0.0;
    let rows: Vec<Value> = hydrated
        .items
        .iter()
        .map(|item| {
            let f = item.get("fields").cloned().unwrap_or(Value::Null);
            let points = f.get(&effort).and_then(|p| p.as_f64()).unwrap_or(0.0);
            cumulative += points;
            let sprint_index = if cumulative <= 0.0 {
                0
            } else {
                ((cumulative - f64::EPSILON) / velocity) as usize
            };
            let sprint = future
                .get(sprint_index)
                .map(|s| Value::String(s.name.clone()))
                .unwrap_or(Value::String("(beyond planned sprints)".into()));
            json!({
                "id": item.get("id").cloned().unwrap_or(Value::Null),
                "title": f.get("System.Title").cloned().unwrap_or(Value::Null),
                "effort": points,
                "cumulativeEffort": cumulative,
                "forecastSprint": sprint,
            })
        })
        .collect();
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": "cumulative effort walked in backlog priority order; item lands in sprint floor(cumulative/velocity), mapped onto current+future team sprints",
            "fieldsUsed": {"effort": effort},
            "velocity": velocity,
            "backlogLevel": level_value.get("name").cloned().unwrap_or(Value::Null),
            "items": rows,
        }),
        columns: vec![],
    })
}
