use crate::cli::planning::{AreaCmd, AreaProjectCmd, AreaTeamCmd};
use crate::client::{enc, Clients};
use crate::context::{prompt, Ctx};
use crate::domain::classification::{self, Group};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(cmd: &AreaCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    match cmd {
        AreaCmd::Project { cmd } => project(cmd, ctx, clients).await,
        AreaCmd::Team { cmd } => team(cmd, ctx, clients).await,
    }
}

async fn project(
    cmd: &AreaProjectCmd,
    ctx: &Ctx,
    clients: &Clients,
) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    match cmd {
        AreaProjectCmd::List { depth } => {
            let tree = classification::get_tree(clients, proj, Group::Areas, *depth).await?;
            let nodes = classification::flatten(&tree);
            Ok(CommandOutput::List {
                value: nodes
                    .iter()
                    .map(|n| serde_json::to_value(n).unwrap_or_default())
                    .collect(),
                columns: vec![
                    Column::new("ID", "id"),
                    Column::new("Path", "path"),
                    Column::new("Has Children", "hasChildren"),
                ],
            })
        }
        AreaProjectCmd::Show { path } => {
            let node = classification::get_node(clients, proj, Group::Areas, path, 1).await?;
            Ok(CommandOutput::Item {
                value: node,
                columns: vec![],
            })
        }
        AreaProjectCmd::Create {
            name,
            path,
            dry_run,
        } => {
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "area.create".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"name": name, "parent": path}),
                }));
            }
            let created = classification::create_node(
                clients,
                proj,
                Group::Areas,
                path.as_deref(),
                name,
                None,
                None,
            )
            .await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "area.create".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result: created,
            }))
        }
        AreaProjectCmd::Update {
            path,
            name,
            dry_run,
        } => {
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "area.update".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"path": path, "newName": name}),
                }));
            }
            let updated = classification::update_node(
                clients,
                proj,
                Group::Areas,
                path,
                json!({"name": name}),
            )
            .await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "area.update".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result: updated,
            }))
        }
        AreaProjectCmd::Delete {
            path,
            reclassify_id,
            dry_run,
        } => {
            if *dry_run {
                return Ok(CommandOutput::Mutation(MutationEnvelope {
                    operation: "area.delete".into(),
                    target: ctx.target(None)?,
                    dry_run: true,
                    result: json!({"path": path, "reclassifyId": reclassify_id}),
                }));
            }
            prompt::confirm(
                ctx.assume_yes,
                prompt::Danger::Destructive,
                &format!("Delete area path '{path}'?"),
            )?;
            let result =
                classification::delete_node(clients, proj, Group::Areas, path, *reclassify_id)
                    .await?;
            Ok(CommandOutput::Mutation(MutationEnvelope {
                operation: "area.delete".into(),
                target: ctx.target(None)?,
                dry_run: false,
                result,
            }))
        }
    }
}

fn team_field_values_url(clients: &Clients, project: &str, team: &str) -> String {
    format!(
        "{}/{}/{}/_apis/work/teamsettings/teamfieldvalues?api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(team)
    )
}

async fn team(cmd: &AreaTeamCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let proj = ctx.project()?;
    let team_name = ctx.team()?;
    let url = team_field_values_url(clients, proj, team_name);
    let raw = clients.raw();
    match cmd {
        AreaTeamCmd::List => {
            let current = raw.get_json(&url).await?;
            let default = current.get("defaultValue").cloned().unwrap_or(Value::Null);
            let values: Vec<Value> = current
                .get("values")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut v| {
                    let is_default = v.get("value") == Some(&default);
                    v["isDefault"] = Value::Bool(is_default);
                    v
                })
                .collect();
            Ok(CommandOutput::List {
                value: values,
                columns: vec![
                    Column::new("Area", "value"),
                    Column::new("Include Sub-Areas", "includeChildren"),
                    Column::new("Default", "isDefault"),
                ],
            })
        }
        AreaTeamCmd::Add {
            path,
            include_sub_areas,
            set_default,
            dry_run,
        } => {
            let current = raw.get_json(&url).await?;
            let mut values = current
                .get("values")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if values
                .iter()
                .any(|v| v.get("value").and_then(|s| s.as_str()) == Some(path.as_str()))
            {
                return Err(CliError::Conflict(format!(
                    "area '{path}' is already assigned to team {team_name}"
                )));
            }
            values.push(json!({"value": path, "includeChildren": include_sub_areas}));
            let default = if *set_default {
                Value::String(path.clone())
            } else {
                current.get("defaultValue").cloned().unwrap_or(Value::Null)
            };
            patch_team_values(
                clients,
                ctx,
                &url,
                default,
                values,
                "area.team.add",
                *dry_run,
            )
            .await
        }
        AreaTeamCmd::Remove { path, dry_run } => {
            let current = raw.get_json(&url).await?;
            let before = current
                .get("values")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let values: Vec<Value> = before
                .iter()
                .filter(|v| v.get("value").and_then(|s| s.as_str()) != Some(path.as_str()))
                .cloned()
                .collect();
            if values.len() == before.len() {
                return Err(CliError::NotFound(format!(
                    "area '{path}' is not assigned to team {team_name}"
                )));
            }
            if values.is_empty() {
                return Err(CliError::Validation(
                    "cannot remove the team's last area assignment".into(),
                ));
            }
            let mut default = current.get("defaultValue").cloned().unwrap_or(Value::Null);
            if default.as_str() == Some(path.as_str()) {
                default = values[0].get("value").cloned().unwrap_or(Value::Null);
                tracing::warn!("removed area was the default; new default: {default}");
            }
            patch_team_values(
                clients,
                ctx,
                &url,
                default,
                values,
                "area.team.remove",
                *dry_run,
            )
            .await
        }
        AreaTeamCmd::Update {
            path,
            include_sub_areas,
            set_default,
            dry_run,
        } => {
            let current = raw.get_json(&url).await?;
            let mut values = current
                .get("values")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut found = false;
            for v in values.iter_mut() {
                if v.get("value").and_then(|s| s.as_str()) == Some(path.as_str()) {
                    found = true;
                    if let Some(inc) = include_sub_areas {
                        v["includeChildren"] = Value::Bool(*inc);
                    }
                }
            }
            if !found {
                return Err(CliError::NotFound(format!(
                    "area '{path}' is not assigned to team {team_name}"
                )));
            }
            let default = if *set_default {
                Value::String(path.clone())
            } else {
                current.get("defaultValue").cloned().unwrap_or(Value::Null)
            };
            patch_team_values(
                clients,
                ctx,
                &url,
                default,
                values,
                "area.team.update",
                *dry_run,
            )
            .await
        }
    }
}

async fn patch_team_values(
    clients: &Clients,
    ctx: &Ctx,
    url: &str,
    default: Value,
    values: Vec<Value>,
    operation: &str,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let body = json!({"defaultValue": default, "values": values});
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation: operation.into(),
            target: ctx.target(None)?,
            dry_run: true,
            result: body,
        }));
    }
    let result = clients
        .raw()
        .patch_json(url, body, "application/json")
        .await?;
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation: operation.into(),
        target: ctx.target(None)?,
        dry_run: false,
        result,
    }))
}
