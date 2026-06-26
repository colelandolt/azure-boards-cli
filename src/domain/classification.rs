//! Project classification nodes (areas and iterations): tree fetch, path
//! normalization, leaf walking, and CRUD over the raw REST API.

use crate::client::{enc, Clients};
use crate::error::CliError;
use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Areas,
    Iterations,
}

impl Group {
    pub fn segment(&self) -> &'static str {
        match self {
            Group::Areas => "areas",
            Group::Iterations => "iterations",
        }
    }
}

fn base_url(clients: &Clients, project: &str, group: Group) -> String {
    format!(
        "{}/{}/_apis/wit/classificationnodes/{}",
        clients.org_url(),
        enc(project),
        group.segment()
    )
}

/// Normalize a node path to field format: drop the leading backslash and the
/// structural "\Area"/"\Iteration" second segment.
/// "\Proj\Iteration\2026 Q2\Lost" -> "Proj\2026 Q2\Lost"
pub fn normalize_path(raw: &str) -> String {
    let trimmed = raw.trim_start_matches('\\');
    let parts: Vec<&str> = trimmed.split('\\').collect();
    if parts.len() > 1
        && (parts[1].eq_ignore_ascii_case("iteration") || parts[1].eq_ignore_ascii_case("area"))
    {
        let mut out = vec![parts[0]];
        out.extend(&parts[2..]);
        out.join("\\")
    } else {
        trimmed.to_string()
    }
}

/// The path segments *under* the root (for API URLs): strip the project name.
/// "Proj\2026 Q2\Lost" -> "2026 Q2/Lost" (URL-encoded per segment)
pub fn api_subpath(field_path: &str, project: &str) -> String {
    let parts: Vec<&str> = field_path.split('\\').filter(|s| !s.is_empty()).collect();
    let rest: Vec<&str> = if parts.first().map(|p| p.eq_ignore_ascii_case(project)) == Some(true) {
        parts[1..].to_vec()
    } else {
        parts
    };
    rest.iter().map(|s| enc(s)).collect::<Vec<_>>().join("/")
}

pub async fn get_tree(
    clients: &Clients,
    project: &str,
    group: Group,
    depth: u32,
) -> Result<Value, CliError> {
    let url = format!(
        "{}?$depth={depth}&api-version=7.1",
        base_url(clients, project, group)
    );
    clients.raw().get_json(&url).await
}

pub async fn get_node(
    clients: &Clients,
    project: &str,
    group: Group,
    path: &str,
    depth: u32,
) -> Result<Value, CliError> {
    let sub = api_subpath(path, project);
    let url = format!(
        "{}/{sub}?$depth={depth}&api-version=7.1",
        base_url(clients, project, group)
    );
    clients.raw().get_json(&url).await
}

pub async fn create_node(
    clients: &Clients,
    project: &str,
    group: Group,
    parent_path: Option<&str>,
    name: &str,
    start: Option<&str>,
    finish: Option<&str>,
) -> Result<Value, CliError> {
    let base = base_url(clients, project, group);
    let url = match parent_path {
        Some(p) if !p.is_empty() => {
            format!("{base}/{}?api-version=7.1", api_subpath(p, project))
        }
        _ => format!("{base}?api-version=7.1"),
    };
    let mut body = json!({"name": name});
    if let (Some(s), Some(f)) = (start, finish) {
        body["attributes"] = json!({
            "startDate": format!("{s}T00:00:00Z"),
            "finishDate": format!("{f}T00:00:00Z"),
        });
    }
    clients.raw().post_json(&url, body).await
}

pub async fn update_node(
    clients: &Clients,
    project: &str,
    group: Group,
    path: &str,
    body: Value,
) -> Result<Value, CliError> {
    let url = format!(
        "{}/{}?api-version=7.1",
        base_url(clients, project, group),
        api_subpath(path, project)
    );
    clients
        .raw()
        .patch_json(&url, body, "application/json")
        .await
}

pub async fn delete_node(
    clients: &Clients,
    project: &str,
    group: Group,
    path: &str,
    reclassify_id: Option<i32>,
) -> Result<Value, CliError> {
    let mut url = format!(
        "{}/{}?api-version=7.1",
        base_url(clients, project, group),
        api_subpath(path, project)
    );
    if let Some(id) = reclassify_id {
        url.push_str(&format!("&$reclassifyId={id}"));
    }
    clients.raw().delete(&url).await
}

/// A flattened node with normalized path and (for iterations) dates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FlatNode {
    pub id: Option<i64>,
    /// Identifier GUID (team-settings APIs want this).
    pub identifier: Option<String>,
    pub name: String,
    pub path: String,
    #[serde(rename = "startDate", skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(rename = "finishDate", skip_serializing_if = "Option::is_none")]
    pub finish_date: Option<String>,
    #[serde(rename = "hasChildren")]
    pub has_children: bool,
}

/// Depth-first flatten of a classification tree (root included).
pub fn flatten(node: &Value) -> Vec<FlatNode> {
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn walk(node: &Value, out: &mut Vec<FlatNode>) {
    let attrs = node.get("attributes").cloned().unwrap_or(Value::Null);
    let children = node.get("children").and_then(|c| c.as_array());
    out.push(FlatNode {
        id: node.get("id").and_then(|i| i.as_i64()),
        identifier: node
            .get("identifier")
            .and_then(|s| s.as_str())
            .map(String::from),
        name: node
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        path: normalize_path(
            node.get("path")
                .and_then(|s| s.as_str())
                .unwrap_or_default(),
        ),
        start_date: attrs
            .get("startDate")
            .and_then(|s| s.as_str())
            .map(|s| s[..10.min(s.len())].to_string()),
        finish_date: attrs
            .get("finishDate")
            .and_then(|s| s.as_str())
            .map(|s| s[..10.min(s.len())].to_string()),
        has_children: children.map(|c| !c.is_empty()).unwrap_or(false),
    });
    if let Some(children) = children {
        for child in children {
            walk(child, out);
        }
    }
}

/// Enrich an iteration node list with current/endsSoon flags (bash parity:
/// endsSoon = current && finish within 2 days).
pub fn enrich_iterations(nodes: &[FlatNode], today: time::Date) -> Vec<Value> {
    nodes
        .iter()
        .map(|n| {
            let start = n.start_date.as_deref().and_then(parse_date);
            let finish = n.finish_date.as_deref().and_then(parse_date);
            let current = match (start, finish) {
                (Some(s), Some(f)) => s <= today && today <= f,
                _ => false,
            };
            let ends_soon = current
                && finish
                    .map(|f| (f - today).whole_days() <= 2)
                    .unwrap_or(false);
            json!({
                "name": n.name,
                "path": n.path,
                "startDate": n.start_date,
                "finishDate": n.finish_date,
                "identifier": n.identifier,
                "current": current,
                "endsSoon": ends_soon,
            })
        })
        .collect()
}

pub fn parse_date(s: &str) -> Option<time::Date> {
    time::Date::parse(
        s,
        &time::macros::format_description!("[year]-[month]-[day]"),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_structural_segment() {
        assert_eq!(
            normalize_path("\\Personas\\Iteration\\2026 Q2\\Killing Eve"),
            "Personas\\2026 Q2\\Killing Eve"
        );
        assert_eq!(normalize_path("\\Proj\\Area\\Web"), "Proj\\Web");
        assert_eq!(normalize_path("Proj\\2026 Q2"), "Proj\\2026 Q2");
    }

    #[test]
    fn api_subpath_strips_project_and_encodes() {
        assert_eq!(api_subpath("Proj\\2026 Q2\\Lost", "Proj"), "2026%20Q2/Lost");
        assert_eq!(api_subpath("2026 Q2\\Lost", "Proj"), "2026%20Q2/Lost");
        assert_eq!(api_subpath("Proj", "Proj"), "");
    }

    #[test]
    fn flatten_and_enrich() {
        let tree = json!({
            "id": 1, "identifier": "root-guid", "name": "Proj", "path": "\\Proj\\Iteration",
            "children": [{
                "id": 2, "identifier": "q2-guid", "name": "2026 Q2", "path": "\\Proj\\Iteration\\2026 Q2",
                "attributes": {"startDate": "2026-04-01T00:00:00Z", "finishDate": "2026-06-30T00:00:00Z"},
                "children": [{
                    "id": 3, "identifier": "lost-guid", "name": "Lost", "path": "\\Proj\\Iteration\\2026 Q2\\Lost",
                    "attributes": {"startDate": "2026-06-01T00:00:00Z", "finishDate": "2026-06-12T00:00:00Z"}
                }]
            }]
        });
        let flat = flatten(&tree);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[2].path, "Proj\\2026 Q2\\Lost");
        assert_eq!(flat[2].start_date.as_deref(), Some("2026-06-01"));

        let today = parse_date("2026-06-11").unwrap();
        let enriched = enrich_iterations(&flat[2..], today);
        assert_eq!(enriched[0]["current"], true);
        assert_eq!(enriched[0]["endsSoon"], true); // finishes 2026-06-12
    }
}
