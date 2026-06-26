//! Resolution of @current / @next / @previous iteration macros via the
//! team-settings iterations API (which supplies timeFrame per iteration).

use crate::client::Clients;
use crate::error::CliError;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct IterationRef {
    /// Iteration identifier GUID (team-settings APIs want this).
    pub id: Option<String>,
    /// Full iteration path for System.IterationPath writes.
    pub path: String,
    pub name: String,
    pub start_date: Option<String>,
    pub finish_date: Option<String>,
    pub time_frame: Option<String>,
}

fn from_value(v: &Value) -> IterationRef {
    let attrs = v.get("attributes").cloned().unwrap_or(Value::Null);
    IterationRef {
        id: v.get("id").and_then(|s| s.as_str()).map(String::from),
        path: v
            .get("path")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        name: v
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        start_date: attrs
            .get("startDate")
            .and_then(|s| s.as_str())
            .map(|s| s[..10.min(s.len())].to_string()),
        finish_date: attrs
            .get("finishDate")
            .and_then(|s| s.as_str())
            .map(|s| s[..10.min(s.len())].to_string()),
        time_frame: attrs
            .get("timeFrame")
            .and_then(|s| s.as_str())
            .map(String::from),
    }
}

/// List the team's subscribed iterations (team-settings API).
pub async fn team_iterations(
    clients: &Clients,
    project: &str,
    team: &str,
) -> Result<Vec<IterationRef>, CliError> {
    let url = format!(
        "{}/{}/{}/_apis/work/teamsettings/iterations?api-version=7.1",
        clients.org_url(),
        crate::client::enc(project),
        crate::client::enc(team),
    );
    let value = clients.raw().get_json(&url).await?;
    let mut refs: Vec<IterationRef> = value
        .get("value")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(from_value).collect())
        .unwrap_or_default();
    refs.sort_by(|a, b| a.start_date.cmp(&b.start_date));
    Ok(refs)
}

/// Resolve "@current", "@next", "@previous" (and "@next+N") to an iteration.
/// Anything not starting with '@' is returned as a plain path (id resolved if
/// the team subscribes to it).
pub async fn resolve(
    clients: &Clients,
    project: &str,
    team: &str,
    spec: &str,
) -> Result<IterationRef, CliError> {
    if !spec.starts_with('@') {
        let iterations = team_iterations(clients, project, team).await?;
        if let Some(found) = iterations.iter().find(|i| i.path == spec || i.name == spec) {
            return Ok(found.clone());
        }
        return Ok(IterationRef {
            id: None,
            path: spec.to_string(),
            name: spec.rsplit('\\').next().unwrap_or(spec).to_string(),
            start_date: None,
            finish_date: None,
            time_frame: None,
        });
    }
    let iterations = team_iterations(clients, project, team).await?;
    pick(&iterations, spec)
}

pub fn pick(iterations: &[IterationRef], spec: &str) -> Result<IterationRef, CliError> {
    let frame = |f: &str| -> Vec<&IterationRef> {
        iterations
            .iter()
            .filter(|i| i.time_frame.as_deref() == Some(f))
            .collect()
    };
    match spec.to_ascii_lowercase().as_str() {
        "@current" | "@currentiteration" => {
            frame("current").first().copied().cloned().ok_or_else(|| {
                let hint = frame("future")
                    .first()
                    .map(|n| {
                        format!(
                            "; nearest future sprint is '{}' (starts {})",
                            n.name,
                            n.start_date.as_deref().unwrap_or("?")
                        )
                    })
                    .unwrap_or_default();
                CliError::NotFound(format!("no current team iteration{hint}"))
            })
        }
        "@next" => frame("future")
            .first()
            .copied()
            .cloned()
            .ok_or_else(|| CliError::NotFound("no future team iteration".into())),
        "@previous" => frame("past")
            .last()
            .copied()
            .cloned()
            .ok_or_else(|| CliError::NotFound("no past team iteration".into())),
        other => {
            if let Some(n) = other
                .strip_prefix("@next+")
                .and_then(|n| n.parse::<usize>().ok())
            {
                frame("future").get(n).copied().cloned().ok_or_else(|| {
                    CliError::NotFound(format!("fewer than {} future iterations", n + 1))
                })
            } else {
                Err(CliError::Validation(format!(
                    "unknown iteration macro '{spec}' (use @current, @next, @next+N, @previous, or a path)"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iter(name: &str, frame: &str, start: &str) -> IterationRef {
        IterationRef {
            id: Some(format!("guid-{name}")),
            path: format!("Proj\\2026 Q2\\{name}"),
            name: name.into(),
            start_date: Some(start.into()),
            finish_date: None,
            time_frame: Some(frame.into()),
        }
    }

    #[test]
    fn picks_current_next_previous() {
        let sprints = vec![
            iter("Justified", "past", "2026-05-04"),
            iter("Killing Eve", "past", "2026-05-18"),
            iter("Lost", "current", "2026-06-01"),
            iter("Mad Men", "future", "2026-06-15"),
            iter("Narcos", "future", "2026-06-29"),
        ];
        assert_eq!(pick(&sprints, "@current").unwrap().name, "Lost");
        assert_eq!(pick(&sprints, "@next").unwrap().name, "Mad Men");
        assert_eq!(pick(&sprints, "@next+1").unwrap().name, "Narcos");
        assert_eq!(pick(&sprints, "@previous").unwrap().name, "Killing Eve");
    }

    #[test]
    fn missing_current_names_nearest_future() {
        let sprints = vec![iter("Mad Men", "future", "2026-06-15")];
        let e = pick(&sprints, "@current").unwrap_err();
        assert_eq!(e.exit_code(), 5);
        assert!(e.to_string().contains("Mad Men"));
    }

    #[test]
    fn unknown_macro_is_validation() {
        assert_eq!(pick(&[], "@bogus").unwrap_err().exit_code(), 7);
    }
}
