use crate::cli::boards::BoardCmd;
use crate::client::{enc, Clients};
use crate::context::Ctx;
use crate::domain::hydrate;
use crate::error::CliError;
use crate::output::{Column, CommandOutput};
use serde_json::Value;

pub async fn run(cmd: &BoardCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let team = ctx.team()?;
    match cmd {
        BoardCmd::List => list(clients, project, team).await,
        BoardCmd::Show { board } => show(clients, project, team, board).await,
        BoardCmd::Columns { board } => columns(clients, project, team, board).await,
        BoardCmd::WorkItems {
            board,
            column,
            fields,
        } => {
            work_items(
                clients,
                project,
                team,
                board,
                column.as_deref(),
                fields.as_deref(),
            )
            .await
        }
    }
}

fn boards_url(clients: &Clients, project: &str, team: &str) -> String {
    format!(
        "{}/{}/{}/_apis/work/boards",
        clients.org_url(),
        enc(project),
        enc(team)
    )
}

async fn list(clients: &Clients, project: &str, team: &str) -> Result<CommandOutput, CliError> {
    let value = clients
        .raw()
        .get_json(&format!(
            "{}?api-version=7.1",
            boards_url(clients, project, team)
        ))
        .await?;
    Ok(CommandOutput::List {
        value: value
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        columns: vec![Column::new("ID", "id"), Column::new("Name", "name")],
    })
}

pub async fn fetch_board(
    clients: &Clients,
    project: &str,
    team: &str,
    board: &str,
) -> Result<Value, CliError> {
    clients
        .raw()
        .get_json(&format!(
            "{}/{}?api-version=7.1",
            boards_url(clients, project, team),
            enc(board)
        ))
        .await
}

async fn show(
    clients: &Clients,
    project: &str,
    team: &str,
    board: &str,
) -> Result<CommandOutput, CliError> {
    Ok(CommandOutput::Item {
        value: fetch_board(clients, project, team, board).await?,
        columns: vec![],
    })
}

async fn columns(
    clients: &Clients,
    project: &str,
    team: &str,
    board: &str,
) -> Result<CommandOutput, CliError> {
    let value = clients
        .raw()
        .get_json(&format!(
            "{}/{}/columns?api-version=7.1",
            boards_url(clients, project, team),
            enc(board)
        ))
        .await?;
    Ok(CommandOutput::List {
        value: value
            .get("value")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        columns: vec![
            Column::new("Name", "name"),
            Column::new("Type", "columnType"),
            Column::new("WIP Limit", "itemLimit"),
        ],
    })
}

/// The team-specific board column field (WEF_{guid}_Kanban.Column) — the
/// per-team source of truth; System.BoardColumn only reflects the area-owning
/// team for items shared across boards.
pub fn column_field(board: &Value) -> String {
    board
        .get("fields")
        .and_then(|f| f.get("columnField"))
        .and_then(|c| c.get("referenceName"))
        .and_then(|r| r.as_str())
        .unwrap_or("System.BoardColumn")
        .to_string()
}

/// Work item types that feed this board (from allowedMappings).
fn board_types(board: &Value) -> Vec<String> {
    let mut types = std::collections::BTreeSet::new();
    if let Some(mappings) = board.get("allowedMappings").and_then(|m| m.as_object()) {
        for category in mappings.values() {
            if let Some(obj) = category.as_object() {
                for type_name in obj.keys() {
                    types.insert(type_name.clone());
                }
            }
        }
    }
    types.into_iter().collect()
}

pub async fn team_area_clause(
    clients: &Clients,
    project: &str,
    team: &str,
) -> Result<String, CliError> {
    let url = format!(
        "{}/{}/{}/_apis/work/teamsettings/teamfieldvalues?api-version=7.1",
        clients.org_url(),
        enc(project),
        enc(team)
    );
    let value = clients.raw().get_json(&url).await?;
    let clauses: Vec<String> = value
        .get("values")
        .and_then(|v| v.as_array())
        .map(|vals| {
            vals.iter()
                .filter_map(|v| {
                    let path = v.get("value").and_then(|p| p.as_str())?;
                    let include = v
                        .get("includeChildren")
                        .and_then(|i| i.as_bool())
                        .unwrap_or(false);
                    let escaped = path.replace('\'', "''");
                    Some(if include {
                        format!("[System.AreaPath] UNDER '{escaped}'")
                    } else {
                        format!("[System.AreaPath] = '{escaped}'")
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND ({})", clauses.join(" OR "))
    })
}

async fn work_items(
    clients: &Clients,
    project: &str,
    team: &str,
    board: &str,
    column: Option<&str>,
    fields: Option<&[String]>,
) -> Result<CommandOutput, CliError> {
    let board_value = fetch_board(clients, project, team, board).await?;
    let col_field = column_field(&board_value);
    let types = board_types(&board_value);
    let area_clause = team_area_clause(clients, project, team).await?;

    let mut wiql = format!(
        "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}'",
        project.replace('\'', "''")
    );
    if !types.is_empty() {
        let list = types
            .iter()
            .map(|t| format!("'{}'", t.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        wiql.push_str(&format!(" AND [System.WorkItemType] IN ({list})"));
    }
    match column {
        Some(c) => wiql.push_str(&format!(" AND [{col_field}] = '{}'", c.replace('\'', "''"))),
        None => wiql.push_str(&format!(" AND [{col_field}] <> ''")),
    }
    wiql.push_str(&area_clause);
    wiql.push_str(" ORDER BY [Microsoft.VSTS.Common.StackRank] ASC");

    let result = super::wiql_cmd::execute(clients, project, &wiql, None).await?;
    let ids = super::wiql_cmd::collect_ids(&result);
    let mut field_list = fields
        .map(|f| f.to_vec())
        .unwrap_or_else(hydrate::default_fields);
    if !field_list.iter().any(|f| f == &col_field) {
        field_list.push(col_field.clone());
    }
    let hydrated = hydrate::fetch(clients, project, &ids, Some(&field_list)).await?;
    let mut columns = hydrate::work_item_columns();
    columns.push(Column::new("Board Column", "fields.System.BoardColumn"));
    Ok(CommandOutput::List {
        value: hydrated.items,
        columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn column_field_prefers_wef_field() {
        let board =
            json!({"fields": {"columnField": {"referenceName": "WEF_ABC123_Kanban.Column"}}});
        assert_eq!(column_field(&board), "WEF_ABC123_Kanban.Column");
        assert_eq!(column_field(&json!({})), "System.BoardColumn");
    }

    #[test]
    fn board_types_from_allowed_mappings() {
        let board = json!({"allowedMappings": {
            "incomingColumn": {"User Story": ["New"], "Bug": ["New"]},
            "outgoingColumn": {"User Story": ["Closed"]}
        }});
        assert_eq!(board_types(&board), vec!["Bug", "User Story"]);
    }
}
