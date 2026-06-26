use crate::cli::planning::{CapacityCmd, SprintCmd};
use crate::client::{enc, Clients};
use crate::context::Ctx;
use crate::domain::{hydrate, macros, patch, states};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(cmd: &SprintCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let team = ctx.team()?;
    match cmd {
        SprintCmd::List => list(clients, project, team).await,
        SprintCmd::Current => current(clients, project, team).await,
        SprintCmd::Backlog { sprint, fields } => {
            backlog(clients, project, team, sprint, fields.as_deref()).await
        }
        SprintCmd::AddWorkItem {
            id,
            iteration,
            dry_run,
        } => {
            move_to_sprint(
                clients,
                ctx,
                project,
                team,
                *id,
                iteration,
                "sprint.add-work-item",
                *dry_run,
            )
            .await
        }
        SprintCmd::MoveWorkItem { id, to, dry_run } => {
            move_to_sprint(
                clients,
                ctx,
                project,
                team,
                *id,
                to,
                "sprint.move-work-item",
                *dry_run,
            )
            .await
        }
        SprintCmd::Capacity {
            cmd: CapacityCmd::Show { sprint },
        } => capacity(clients, project, team, sprint).await,
        SprintCmd::Burndown { sprint } => burndown(clients, project, team, sprint).await,
    }
}

fn sprint_columns() -> Vec<Column> {
    vec![
        Column::new("Name", "name"),
        Column::new("Path", "path"),
        Column::new("Start", "startDate"),
        Column::new("Finish", "finishDate"),
        Column::new("Timeframe", "timeFrame"),
    ]
}

fn sprint_to_value(it: &macros::IterationRef) -> Value {
    json!({
        "name": it.name,
        "path": it.path,
        "identifier": it.id,
        "startDate": it.start_date,
        "finishDate": it.finish_date,
        "timeFrame": it.time_frame,
    })
}

async fn list(clients: &Clients, project: &str, team: &str) -> Result<CommandOutput, CliError> {
    let iterations = macros::team_iterations(clients, project, team).await?;
    Ok(CommandOutput::List {
        value: iterations.iter().map(sprint_to_value).collect(),
        columns: sprint_columns(),
    })
}

async fn current(clients: &Clients, project: &str, team: &str) -> Result<CommandOutput, CliError> {
    let it = macros::resolve(clients, project, team, "@current").await?;
    Ok(CommandOutput::Item {
        value: sprint_to_value(&it),
        columns: vec![],
    })
}

async fn sprint_work_item_ids(
    clients: &Clients,
    project: &str,
    team: &str,
    sprint: &str,
) -> Result<(macros::IterationRef, Vec<i32>), CliError> {
    let it = macros::resolve(clients, project, team, sprint).await?;
    let id = it.id.clone().ok_or_else(|| {
        CliError::NotFound(format!(
            "sprint '{}' is not in the team's sprint list",
            it.path
        ))
    })?;
    let url = format!(
        "{}/{}/{}/_apis/work/teamsettings/iterations/{id}/workitems?api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(team)
    );
    let value = clients.raw().get_json(&url).await?;
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
    Ok((it, ids))
}

async fn backlog(
    clients: &Clients,
    project: &str,
    team: &str,
    sprint: &str,
    fields: Option<&[String]>,
) -> Result<CommandOutput, CliError> {
    let (_, ids) = sprint_work_item_ids(clients, project, team, sprint).await?;
    let mut field_list = fields
        .map(|f| f.to_vec())
        .unwrap_or_else(hydrate::default_fields);
    for extra in [
        "Microsoft.VSTS.Scheduling.RemainingWork",
        "Microsoft.VSTS.Scheduling.StoryPoints",
    ] {
        if !field_list.iter().any(|f| f == extra) {
            field_list.push(extra.to_string());
        }
    }
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&field_list)).await?;
    Ok(CommandOutput::List {
        value: hydrated.items,
        columns: hydrate::work_item_columns(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn move_to_sprint(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    team: &str,
    id: i32,
    sprint: &str,
    operation: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let it = macros::resolve(clients, project, team, sprint).await?;
    let ops = vec![patch::add_field("System.IterationPath", it.path.clone())];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(Some(id as i64))?,
            dry_run: true,
            result: json!({"iterationPath": it.path, "patch": serde_json::to_value(&ops)?}),
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
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(Some(id as i64))?,
        dry_run: false,
        result: json!({
            "iterationPath": value.get("fields").and_then(|f| f.get("System.IterationPath")).cloned().unwrap_or(Value::Null),
            "rev": value.get("rev").cloned().unwrap_or(Value::Null),
        }),
    }))
}

async fn capacity(
    clients: &Clients,
    project: &str,
    team: &str,
    sprint: &str,
) -> Result<CommandOutput, CliError> {
    let it = macros::resolve(clients, project, team, sprint).await?;
    let iteration_id = it.id.clone().ok_or_else(|| {
        CliError::NotFound(format!(
            "sprint '{}' is not in the team's sprint list",
            it.path
        ))
    })?;
    let raw = clients.raw();
    let base = format!(
        "{}/{}/{}/_apis/work/teamsettings/iterations/{iteration_id}",
        clients.org_url(),
        enc(project),
        enc(team)
    );
    let capacities = raw
        .get_json(&format!("{base}/capacities?api-version=7.1"))
        .await?;
    let days_off = raw
        .get_json(&format!("{base}/teamdaysoff?api-version=7.1"))
        .await
        .unwrap_or(Value::Null);

    let members: Vec<Value> = capacities
        .get("teamMembers")
        .or_else(|| capacities.get("value"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let summary: Vec<Value> = members
        .iter()
        .map(|m| {
            let per_day: f64 = m
                .get("activities")
                .and_then(|a| a.as_array())
                .map(|acts| {
                    acts.iter()
                        .filter_map(|a| a.get("capacityPerDay").and_then(|c| c.as_f64()))
                        .sum()
                })
                .unwrap_or(0.0);
            json!({
                "member": m.get("teamMember").and_then(|t| t.get("displayName")).cloned().unwrap_or(Value::Null),
                "capacityPerDay": per_day,
                "activities": m.get("activities").cloned().unwrap_or(Value::Null),
                "daysOff": m.get("daysOff").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let total_per_day: f64 = summary
        .iter()
        .filter_map(|m| m.get("capacityPerDay").and_then(|c| c.as_f64()))
        .sum();
    Ok(CommandOutput::Item {
        value: json!({
            "sprint": sprint_to_value(&it),
            "members": summary,
            "teamDaysOff": days_off.get("daysOff").cloned().unwrap_or(Value::Null),
            "totalCapacityPerDay": total_per_day,
        }),
        columns: vec![],
    })
}

/// Current-state sprint progress summary. A true per-day burndown needs
/// Analytics (see `ab metrics burndown`); this reports where the sprint
/// stands right now, with the calculation basis documented in the output.
async fn burndown(
    clients: &Clients,
    project: &str,
    team: &str,
    sprint: &str,
) -> Result<CommandOutput, CliError> {
    let (it, ids) = sprint_work_item_ids(clients, project, team, sprint).await?;
    let fields = vec![
        "System.State".to_string(),
        "System.WorkItemType".to_string(),
        "Microsoft.VSTS.Scheduling.RemainingWork".to_string(),
        "Microsoft.VSTS.Scheduling.StoryPoints".to_string(),
    ];
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;
    let types: Vec<String> = hydrated
        .items
        .iter()
        .filter_map(|i| {
            i.get("fields")
                .and_then(|f| f.get("System.WorkItemType"))
                .and_then(|t| t.as_str())
                .map(String::from)
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let state_map = states::discover(clients, project, &types).await?;

    let mut by_category: std::collections::BTreeMap<String, usize> = Default::default();
    let mut remaining_work = 0.0;
    let mut remaining_points = 0.0;
    let mut total_points = 0.0;
    for item in &hydrated.items {
        let f = item.get("fields").cloned().unwrap_or(Value::Null);
        let ty = f
            .get("System.WorkItemType")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let state = f.get("System.State").and_then(|s| s.as_str()).unwrap_or("");
        let category = state_map
            .category(ty, state)
            .unwrap_or("Unknown")
            .to_string();
        *by_category.entry(category.clone()).or_default() += 1;
        let points = f
            .get("Microsoft.VSTS.Scheduling.StoryPoints")
            .and_then(|p| p.as_f64())
            .unwrap_or(0.0);
        total_points += points;
        if category != "Completed" && category != "Removed" {
            remaining_points += points;
            remaining_work += f
                .get("Microsoft.VSTS.Scheduling.RemainingWork")
                .and_then(|r| r.as_f64())
                .unwrap_or(0.0);
        }
    }
    let today = time::OffsetDateTime::now_utc().date();
    let days = |d: Option<&str>| d.and_then(crate::domain::classification::parse_date);
    let (elapsed, remaining_days) = match (
        days(it.start_date.as_deref()),
        days(it.finish_date.as_deref()),
    ) {
        (Some(s), Some(f)) => (
            Some((today - s).whole_days().max(0)),
            Some((f - today).whole_days().max(0)),
        ),
        _ => (None, None),
    };
    Ok(CommandOutput::Item {
        value: json!({
            "sprint": sprint_to_value(&it),
            "calculation": "current snapshot: counts by state category; remaining = sum over items not in Completed/Removed categories. For per-day history use `ab metrics burndown`.",
            "stateCategories": state_map.to_json(),
            "totalItems": hydrated.items.len(),
            "itemsByCategory": by_category,
            "remainingWorkHours": remaining_work,
            "remainingStoryPoints": remaining_points,
            "totalStoryPoints": total_points,
            "daysElapsed": elapsed,
            "daysRemaining": remaining_days,
        }),
        columns: vec![],
    })
}
