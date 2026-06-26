//! Flow metrics. Analytics OData (v4.0-preview) is the primary source where
//! history is required; bounded computations use the work item API directly.
//! Every output documents `calculation`, `dateBasis`, `fieldsUsed`, and
//! `stateCategories` so consumers know exactly what was measured.

use crate::cli::metrics::MetricsCmd;
use crate::client::Clients;
use crate::context::Ctx;
use crate::domain::{analytics, classification, hydrate, macros, states};
use crate::error::CliError;
use crate::output::CommandOutput;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub async fn run(
    cmd: &MetricsCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    match cmd {
        MetricsCmd::Velocity { iterations, field } => {
            velocity(clients, ctx, project, *iterations, field.as_deref()).await
        }
        MetricsCmd::Throughput { from, to, period } => {
            throughput(clients, project, from, to.as_deref(), period).await
        }
        MetricsCmd::LeadTime { from, to, r#type } => {
            duration_metric(
                clients,
                project,
                from,
                to.as_deref(),
                r#type.as_deref(),
                Duration::Lead,
            )
            .await
        }
        MetricsCmd::CycleTime { from, to, r#type } => {
            duration_metric(
                clients,
                project,
                from,
                to.as_deref(),
                r#type.as_deref(),
                Duration::Cycle,
            )
            .await
        }
        MetricsCmd::Burndown { iteration, measure } => {
            burndown(clients, ctx, project, iteration, measure).await
        }
        MetricsCmd::Wip => wip(clients, ctx, project).await,
        MetricsCmd::Cfd { from, to, by } => {
            cfd(clients, ctx, project, from, to.as_deref(), by).await
        }
    }
}

fn parse_day(s: &str) -> Result<time::Date, CliError> {
    classification::parse_date(s)
        .ok_or_else(|| CliError::Validation(format!("invalid date '{s}' (YYYY-MM-DD)")))
}

fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

fn range(from: &str, to: Option<&str>) -> Result<(time::Date, time::Date), CliError> {
    let from = parse_day(from)?;
    let to = match to {
        Some(t) => parse_day(t)?,
        None => today(),
    };
    if from > to {
        return Err(CliError::Validation(format!(
            "--from {from} is after --to {to}"
        )));
    }
    Ok((from, to))
}

// ---------------------------------------------------------------------------
// velocity
// ---------------------------------------------------------------------------

async fn velocity(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    count: usize,
    field: Option<&str>,
) -> Result<CommandOutput, CliError> {
    let team = ctx.team()?;
    let effort = match field {
        Some(f) => f.to_string(),
        None => super::backlog::effort_field(clients, project, team).await?,
    };
    let iterations = macros::team_iterations(clients, project, team).await?;
    let mut past: Vec<_> = iterations
        .iter()
        .filter(|i| i.time_frame.as_deref() == Some("past"))
        .collect();
    past.sort_by(|a, b| a.start_date.cmp(&b.start_date));
    let window: Vec<_> = past.into_iter().rev().take(count).rev().collect();
    if window.is_empty() {
        return Err(CliError::NotFound("no completed team iterations".into()));
    }

    let state_map = states::discover(clients, project, &[]).await?;
    let mut sprints = Vec::new();
    let mut total = 0.0;
    for it in &window {
        let escaped = it.path.replace('\'', "''");
        let wiql = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [System.IterationPath] = '{escaped}'",
            project.replace('\'', "''")
        );
        let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
        let ids = super::wiql_cmd::collect_ids(&result);
        let fields = vec![
            "System.State".to_string(),
            "System.WorkItemType".to_string(),
            effort.clone(),
        ];
        let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;
        let mut completed_effort = 0.0;
        let mut completed_count = 0usize;
        for item in &hydrated.items {
            let f = item.get("fields").cloned().unwrap_or(Value::Null);
            let ty = f
                .get("System.WorkItemType")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let state = f.get("System.State").and_then(|s| s.as_str()).unwrap_or("");
            if state_map.category(ty, state) == Some("Completed") {
                completed_count += 1;
                completed_effort += f.get(&effort).and_then(|p| p.as_f64()).unwrap_or(0.0);
            }
        }
        total += completed_effort;
        sprints.push(json!({
            "name": it.name,
            "path": it.path,
            "startDate": it.start_date,
            "finishDate": it.finish_date,
            "completedEffort": completed_effort,
            "completedCount": completed_count,
        }));
    }
    let average = total / window.len() as f64;
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": "per past team iteration: sum of effort over items whose current state maps to the Completed category",
            "dateBasis": "iteration membership at query time (System.IterationPath)",
            "fieldsUsed": {"effort": effort},
            "stateCategories": state_map.to_json(),
            "iterations": sprints,
            "averageVelocity": (average * 10.0).round() / 10.0,
        }),
        columns: vec![],
    })
}

// ---------------------------------------------------------------------------
// throughput
// ---------------------------------------------------------------------------

fn bucket_key(date: &str, period: &str) -> String {
    // date is ISO (YYYY-MM-DD...). week -> ISO year-week, month -> YYYY-MM.
    let day = classification::parse_date(&date[..10.min(date.len())]);
    match (period, day) {
        ("day", _) => date[..10.min(date.len())].to_string(),
        ("month", _) => date[..7.min(date.len())].to_string(),
        (_, Some(d)) => {
            let (year, week, _) = d.to_iso_week_date();
            format!("{year}-W{week:02}")
        }
        _ => date[..10.min(date.len())].to_string(),
    }
}

async fn completed_dates(
    clients: &Clients,
    project: &str,
    from: time::Date,
    to: time::Date,
    work_item_type: Option<&str>,
) -> Result<(Vec<Value>, &'static str), CliError> {
    // Analytics first: WorkItems with CompletedDate in range.
    if analytics::available(clients, project).await {
        let mut filter =
            format!("CompletedDate ge {from}T00:00:00Z and CompletedDate le {to}T23:59:59Z");
        if let Some(t) = work_item_type {
            filter.push_str(&format!(" and WorkItemType eq {}", analytics::literal(t)));
        }
        let rows = analytics::query(
            clients,
            project,
            "WorkItems",
            &[
                ("filter", filter),
                (
                    "select",
                    "WorkItemId,WorkItemType,CompletedDate,LeadTimeDays,CycleTimeDays".into(),
                ),
            ],
        )
        .await?;
        return Ok((rows, "analytics"));
    }
    // Fallback: WIQL on ClosedDate.
    let mut wiql = format!(
        "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [Microsoft.VSTS.Common.ClosedDate] >= '{from}' AND [Microsoft.VSTS.Common.ClosedDate] <= '{to}'",
        project.replace('\'', "''")
    );
    if let Some(t) = work_item_type {
        wiql.push_str(&format!(
            " AND [System.WorkItemType] = '{}'",
            t.replace('\'', "''")
        ));
    }
    let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
    let ids = super::wiql_cmd::collect_ids(&result);
    let fields = vec![
        "System.WorkItemType".to_string(),
        "System.CreatedDate".to_string(),
        "Microsoft.VSTS.Common.ClosedDate".to_string(),
        "Microsoft.VSTS.Common.ActivatedDate".to_string(),
    ];
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;
    let rows = hydrated
        .items
        .iter()
        .map(|i| {
            let f = i.get("fields").cloned().unwrap_or(Value::Null);
            json!({
                "WorkItemId": i.get("id").cloned().unwrap_or(Value::Null),
                "WorkItemType": f.get("System.WorkItemType").cloned().unwrap_or(Value::Null),
                "CompletedDate": f.get("Microsoft.VSTS.Common.ClosedDate").cloned().unwrap_or(Value::Null),
                "CreatedDate": f.get("System.CreatedDate").cloned().unwrap_or(Value::Null),
                "ActivatedDate": f.get("Microsoft.VSTS.Common.ActivatedDate").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok((rows, "workItemApi"))
}

async fn throughput(
    clients: &Clients,
    project: &str,
    from: &str,
    to: Option<&str>,
    period: &str,
) -> Result<CommandOutput, CliError> {
    if !["day", "week", "month"].contains(&period) {
        return Err(CliError::Validation(format!(
            "--period must be day, week, or month (got '{period}')"
        )));
    }
    let (from_d, to_d) = range(from, to)?;
    let (rows, source) = completed_dates(clients, project, from_d, to_d, None).await?;
    let mut buckets: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        if let Some(d) = row.get("CompletedDate").and_then(|v| v.as_str()) {
            *buckets.entry(bucket_key(d, period)).or_default() += 1;
        }
    }
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": format!("count of work items completed per {period}"),
            "dateBasis": if source == "analytics" { "Analytics CompletedDate" } else { "Microsoft.VSTS.Common.ClosedDate (Analytics unavailable)" },
            "fieldsUsed": {"completed": "CompletedDate/ClosedDate"},
            "source": source,
            "from": from_d.to_string(),
            "to": to_d.to_string(),
            "totalCompleted": rows.len(),
            "buckets": buckets,
        }),
        columns: vec![],
    })
}

// ---------------------------------------------------------------------------
// lead-time / cycle-time
// ---------------------------------------------------------------------------

enum Duration {
    Lead,
    Cycle,
}

async fn duration_metric(
    clients: &Clients,
    project: &str,
    from: &str,
    to: Option<&str>,
    work_item_type: Option<&str>,
    kind: Duration,
) -> Result<CommandOutput, CliError> {
    let (from_d, to_d) = range(from, to)?;
    let (rows, source) = completed_dates(clients, project, from_d, to_d, work_item_type).await?;
    let (name, analytics_col, fallback_start) = match kind {
        Duration::Lead => ("leadTime", "LeadTimeDays", "CreatedDate"),
        Duration::Cycle => ("cycleTime", "CycleTimeDays", "ActivatedDate"),
    };
    let mut durations = Vec::new();
    for row in &rows {
        if source == "analytics" {
            if let Some(d) = row.get(analytics_col).and_then(|v| v.as_f64()) {
                durations.push(d);
            }
        } else {
            let start = row.get(fallback_start).and_then(|v| v.as_str());
            let end = row.get("CompletedDate").and_then(|v| v.as_str());
            if let (Some(s), Some(e)) = (
                start.and_then(|s| classification::parse_date(&s[..10.min(s.len())])),
                end.and_then(|e| classification::parse_date(&e[..10.min(e.len())])),
            ) {
                durations.push(((e - s).whole_days()).max(0) as f64);
            }
        }
    }
    let basis = match (&kind, source) {
        (Duration::Lead, "analytics") => "Analytics LeadTimeDays (CreatedDate -> CompletedDate)",
        (Duration::Cycle, "analytics") => "Analytics CycleTimeDays (first InProgress -> CompletedDate)",
        (Duration::Lead, _) => "System.CreatedDate -> ClosedDate, whole days (Analytics unavailable)",
        (Duration::Cycle, _) => "Microsoft.VSTS.Common.ActivatedDate -> ClosedDate, whole days (Analytics unavailable; approximates first InProgress entry)",
    };
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": format!("{name} distribution over items completed in the window"),
            "dateBasis": basis,
            "fieldsUsed": {"duration": analytics_col},
            "source": source,
            "from": from_d.to_string(),
            "to": to_d.to_string(),
            "distribution": analytics::distribution(durations),
        }),
        columns: vec![],
    })
}

// ---------------------------------------------------------------------------
// burndown
// ---------------------------------------------------------------------------

const MAX_FALLBACK_DAYS: i64 = 62;

async fn burndown(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    iteration: &str,
    measure: &str,
) -> Result<CommandOutput, CliError> {
    let team = ctx.team()?;
    let it = macros::resolve(clients, project, team, iteration).await?;
    let (start, finish) = match (it.start_date.as_deref(), it.finish_date.as_deref()) {
        (Some(s), Some(f)) => (parse_day(s)?, parse_day(f)?),
        _ => {
            return Err(CliError::Validation(format!(
                "iteration '{}' has no start/finish dates",
                it.path
            )))
        }
    };
    let end = finish.min(today());
    let escaped_path = it.path.replace('\'', "''");

    if analytics::available(clients, project).await {
        let agg = match measure {
            "remaining-work" => "aggregate(RemainingWork with sum as Total)",
            "effort" => "aggregate(StoryPoints with sum as Total)",
            _ => "aggregate($count as Total)",
        };
        let apply = format!(
            "filter(Iteration/IterationPath eq {} and DateValue ge {start}T00:00:00Z and DateValue le {end}T00:00:00Z and StateCategory ne 'Completed' and StateCategory ne 'Removed')/groupby((DateValue), {agg})",
            analytics::literal(&it.path)
        );
        let rows =
            analytics::query(clients, project, "WorkItemSnapshot", &[("apply", apply)]).await?;
        let mut days: BTreeMap<String, Value> = BTreeMap::new();
        for row in &rows {
            if let Some(d) = row.get("DateValue").and_then(|v| v.as_str()) {
                days.insert(
                    d[..10.min(d.len())].to_string(),
                    row.get("Total").cloned().unwrap_or(Value::Null),
                );
            }
        }
        return Ok(CommandOutput::Item {
            value: json!({
                "calculation": format!("per-day remaining ({measure}) for items not in Completed/Removed categories"),
                "dateBasis": "Analytics WorkItemSnapshot daily snapshots",
                "fieldsUsed": {"measure": measure},
                "source": "analytics",
                "iteration": it.path,
                "days": days,
            }),
            columns: vec![],
        });
    }

    // Fallback: per-day WIQL ASOF + state categorization. Expensive — warn.
    let total_days = (end - start).whole_days() + 1;
    if total_days > MAX_FALLBACK_DAYS {
        return Err(CliError::Validation(format!(
            "Analytics unavailable and the {total_days}-day window exceeds the {MAX_FALLBACK_DAYS}-day fallback cap"
        )));
    }
    tracing::warn!(
        "Analytics unavailable; computing burndown from {total_days} per-day ASOF queries"
    );
    let state_map = states::discover(clients, project, &[]).await?;
    let mut days: BTreeMap<String, Value> = BTreeMap::new();
    let mut day = start;
    while day <= end {
        let wiql = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [System.IterationPath] = '{escaped_path}' ASOF '{day}'",
            project.replace('\'', "''")
        );
        let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
        let ids = super::wiql_cmd::collect_ids(&result);
        let remaining = as_of_remaining(clients, project, &ids, day, measure, &state_map).await?;
        days.insert(day.to_string(), remaining);
        day = day.next_day().unwrap_or(day);
        if day == start {
            break;
        }
    }
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": format!("per-day remaining ({measure}) for items not in Completed/Removed categories"),
            "dateBasis": "work item API ASOF queries (Analytics unavailable)",
            "fieldsUsed": {"measure": measure},
            "stateCategories": state_map.to_json(),
            "source": "workItemApi",
            "iteration": it.path,
            "days": days,
        }),
        columns: vec![],
    })
}

async fn as_of_remaining(
    clients: &Clients,
    project: &str,
    ids: &[i32],
    day: time::Date,
    measure: &str,
    state_map: &states::StateMap,
) -> Result<Value, CliError> {
    use azure_devops_rust_api::wit::models::{
        work_item_batch_get_request::ErrorPolicy, WorkItemBatchGetRequest,
    };
    if ids.is_empty() {
        return Ok(json!(0));
    }
    let mut count = 0usize;
    let mut total = 0.0;
    for chunk in ids.chunks(hydrate::CHUNK) {
        let body = WorkItemBatchGetRequest {
            ids: chunk.to_vec(),
            fields: vec![
                "System.State".into(),
                "System.WorkItemType".into(),
                "Microsoft.VSTS.Scheduling.RemainingWork".into(),
                "Microsoft.VSTS.Scheduling.StoryPoints".into(),
            ],
            as_of: Some(day.midnight().assume_utc()),
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
            let v = serde_json::to_value(&item)?;
            let f = v.get("fields").cloned().unwrap_or(Value::Null);
            let ty = f
                .get("System.WorkItemType")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let state = f.get("System.State").and_then(|s| s.as_str()).unwrap_or("");
            let category = state_map.category(ty, state).unwrap_or("Unknown");
            if category != "Completed" && category != "Removed" {
                count += 1;
                total += match measure {
                    "remaining-work" => f
                        .get("Microsoft.VSTS.Scheduling.RemainingWork")
                        .and_then(|r| r.as_f64())
                        .unwrap_or(0.0),
                    "effort" => f
                        .get("Microsoft.VSTS.Scheduling.StoryPoints")
                        .and_then(|r| r.as_f64())
                        .unwrap_or(0.0),
                    _ => 0.0,
                };
            }
        }
    }
    Ok(if measure == "count" {
        json!(count)
    } else {
        json!(total)
    })
}

// ---------------------------------------------------------------------------
// wip
// ---------------------------------------------------------------------------

async fn wip(clients: &Clients, ctx: &Ctx, project: &str) -> Result<CommandOutput, CliError> {
    let state_map = states::discover(clients, project, &[]).await?;
    let in_progress: std::collections::BTreeSet<String> = state_map
        .by_type
        .values()
        .flat_map(|m| {
            m.iter()
                .filter(|(_, cat)| cat.as_str() == "InProgress")
                .map(|(s, _)| s.clone())
        })
        .collect();
    if in_progress.is_empty() {
        return Err(CliError::NotFound(
            "no InProgress states in this process".into(),
        ));
    }
    let states_list = in_progress
        .iter()
        .map(|s| format!("'{}'", s.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let mut wiql = format!(
        "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [System.State] IN ({states_list})",
        project.replace('\'', "''")
    );
    // Scope to the team's areas when a team is configured.
    if let Ok(team) = ctx.team() {
        wiql.push_str(&super::board::team_area_clause(clients, project, team).await?);
    }
    let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
    let ids = super::wiql_cmd::collect_ids(&result);
    let fields = vec![
        "System.WorkItemType".to_string(),
        "System.State".to_string(),
        "System.AssignedTo".to_string(),
    ];
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&fields)).await?;
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_assignee: BTreeMap<String, usize> = BTreeMap::new();
    for item in &hydrated.items {
        let f = item.get("fields").cloned().unwrap_or(Value::Null);
        let ty = f
            .get("System.WorkItemType")
            .and_then(|t| t.as_str())
            .unwrap_or("?")
            .to_string();
        let who = f
            .get("System.AssignedTo")
            .and_then(|a| a.get("displayName"))
            .and_then(|d| d.as_str())
            .unwrap_or("(unassigned)")
            .to_string();
        *by_type.entry(ty).or_default() += 1;
        *by_assignee.entry(who).or_default() += 1;
    }
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": "count of items currently in states mapping to the InProgress category, scoped to the team's areas when --team is set",
            "dateBasis": "current state",
            "stateCategories": state_map.to_json(),
            "totalInProgress": hydrated.items.len(),
            "byType": by_type,
            "byAssignee": by_assignee,
        }),
        columns: vec![],
    })
}

// ---------------------------------------------------------------------------
// cfd
// ---------------------------------------------------------------------------

async fn cfd(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    from: &str,
    to: Option<&str>,
    by: &str,
) -> Result<CommandOutput, CliError> {
    let (from_d, to_d) = range(from, to)?;
    let analytics_up = analytics::available(clients, project).await;
    match by {
        "column" => {
            if !analytics_up {
                return Err(CliError::Service(
                    "board-column CFD requires the Analytics service (column history is not reconstructable from work item revisions); try --by state".into(),
                ));
            }
            let apply = format!(
                "filter(DateValue ge {from_d}T00:00:00Z and DateValue le {to_d}T00:00:00Z)/groupby((DateValue, ColumnName), aggregate($count as Count))"
            );
            let rows = analytics::query(
                clients,
                project,
                "WorkItemBoardSnapshot",
                &[("apply", apply)],
            )
            .await?;
            Ok(cfd_output(rows, "ColumnName", "analytics", from_d, to_d))
        }
        "state" => {
            if analytics_up {
                let apply = format!(
                    "filter(DateValue ge {from_d}T00:00:00Z and DateValue le {to_d}T00:00:00Z)/groupby((DateValue, StateCategory), aggregate($count as Count))"
                );
                let rows =
                    analytics::query(clients, project, "WorkItemSnapshot", &[("apply", apply)])
                        .await?;
                return Ok(cfd_output(rows, "StateCategory", "analytics", from_d, to_d));
            }
            cfd_fallback(clients, ctx, project, from_d, to_d).await
        }
        other => Err(CliError::Validation(format!(
            "--by must be 'column' or 'state' (got '{other}')"
        ))),
    }
}

fn cfd_output(
    rows: Vec<Value>,
    group_col: &str,
    source: &str,
    from: time::Date,
    to: time::Date,
) -> CommandOutput {
    let mut days: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for row in &rows {
        let date = row
            .get("DateValue")
            .and_then(|v| v.as_str())
            .map(|d| d[..10.min(d.len())].to_string())
            .unwrap_or_default();
        let group = row
            .get(group_col)
            .and_then(|v| v.as_str())
            .unwrap_or("(none)")
            .to_string();
        let count = row.get("Count").and_then(|v| v.as_i64()).unwrap_or(0);
        days.entry(date).or_default().insert(group, count);
    }
    CommandOutput::Item {
        value: json!({
            "calculation": format!("per-day item count per {group_col}"),
            "dateBasis": "Analytics daily snapshots",
            "source": source,
            "from": from.to_string(),
            "to": to.to_string(),
            "days": days,
        }),
        columns: vec![],
    }
}

async fn cfd_fallback(
    clients: &Clients,
    ctx: &Ctx,
    project: &str,
    from: time::Date,
    to: time::Date,
) -> Result<CommandOutput, CliError> {
    let total_days = (to - from).whole_days() + 1;
    if total_days > MAX_FALLBACK_DAYS {
        return Err(CliError::Validation(format!(
            "Analytics unavailable and the {total_days}-day window exceeds the {MAX_FALLBACK_DAYS}-day fallback cap"
        )));
    }
    tracing::warn!("Analytics unavailable; computing CFD from {total_days} per-day ASOF queries");
    let state_map = states::discover(clients, project, &[]).await?;
    let area_clause = match ctx.team() {
        Ok(team) => super::board::team_area_clause(clients, project, team).await?,
        Err(_) => String::new(),
    };
    let mut days: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    let mut day = from;
    while day <= to {
        let wiql = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}'{area_clause} ASOF '{day}'",
            project.replace('\'', "''")
        );
        let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
        let ids = super::wiql_cmd::collect_ids(&result);
        let counts = as_of_category_counts(clients, project, &ids, day, &state_map).await?;
        days.insert(day.to_string(), counts);
        let next = day.next_day().unwrap_or(day);
        if next == day {
            break;
        }
        day = next;
    }
    Ok(CommandOutput::Item {
        value: json!({
            "calculation": "per-day item count per state category",
            "dateBasis": "work item API ASOF queries (Analytics unavailable)",
            "stateCategories": state_map.to_json(),
            "source": "workItemApi",
            "from": from.to_string(),
            "to": to.to_string(),
            "days": days,
        }),
        columns: vec![],
    })
}

async fn as_of_category_counts(
    clients: &Clients,
    project: &str,
    ids: &[i32],
    day: time::Date,
    state_map: &states::StateMap,
) -> Result<BTreeMap<String, i64>, CliError> {
    use azure_devops_rust_api::wit::models::{
        work_item_batch_get_request::ErrorPolicy, WorkItemBatchGetRequest,
    };
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for chunk in ids.chunks(hydrate::CHUNK) {
        let body = WorkItemBatchGetRequest {
            ids: chunk.to_vec(),
            fields: vec!["System.State".into(), "System.WorkItemType".into()],
            as_of: Some(day.midnight().assume_utc()),
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
            let v = serde_json::to_value(&item)?;
            let f = v.get("fields").cloned().unwrap_or(Value::Null);
            let ty = f
                .get("System.WorkItemType")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let state = f.get("System.State").and_then(|s| s.as_str()).unwrap_or("");
            let category = state_map
                .category(ty, state)
                .unwrap_or("Unknown")
                .to_string();
            *counts.entry(category).or_default() += 1;
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_by_period() {
        assert_eq!(bucket_key("2026-06-11T10:00:00Z", "day"), "2026-06-11");
        assert_eq!(bucket_key("2026-06-11T10:00:00Z", "month"), "2026-06");
        assert_eq!(bucket_key("2026-06-11T10:00:00Z", "week"), "2026-W24");
    }

    #[test]
    fn range_validation() {
        assert!(range("2026-01-01", Some("2026-02-01")).is_ok());
        assert_eq!(
            range("2026-02-01", Some("2026-01-01"))
                .unwrap_err()
                .exit_code(),
            7
        );
        assert_eq!(range("bogus", None).unwrap_err().exit_code(), 7);
    }
}
