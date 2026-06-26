//! GitHub artifact links on work items (Boards perspective only).
//!
//! vstfs URIs require a Boards-internal GitHub repo GUID that no documented
//! API exposes, so linking uses a resolution ladder:
//!   1. --connection-id flag
//!   2. ADO_GITHUB_CONNECTION_ID env var (bash-script compat)
//!   3. [github.connections] map in user config ("owner/repo" -> guid)
//!   4. scan recent work items' existing GitHub artifact links (then cache)
//!   5. fail with an actionable error

use crate::cli::github::GithubCmd;
use crate::client::{enc, Clients};
use crate::context::{config, Ctx};
use crate::domain::{hydrate, patch};
use crate::error::CliError;
use crate::output::{Column, CommandOutput, MutationEnvelope};
use serde_json::{json, Value};

pub async fn run(cmd: &GithubCmd, ctx: &Ctx, clients: &Clients) -> Result<CommandOutput, CliError> {
    match cmd {
        GithubCmd::LinkPr {
            work_item_id,
            pr_url,
            connection_id,
            as_hyperlink,
            dry_run,
        } => {
            link(
                clients,
                ctx,
                *work_item_id,
                pr_url,
                LinkKind::PullRequest,
                connection_id.as_deref(),
                *as_hyperlink,
                *dry_run,
            )
            .await
        }
        GithubCmd::LinkCommit {
            work_item_id,
            commit_url,
            connection_id,
            as_hyperlink,
            dry_run,
        } => {
            link(
                clients,
                ctx,
                *work_item_id,
                commit_url,
                LinkKind::Commit,
                connection_id.as_deref(),
                *as_hyperlink,
                *dry_run,
            )
            .await
        }
        GithubCmd::LinkBranch {
            work_item_id,
            branch_url,
            connection_id,
            as_hyperlink,
            dry_run,
        } => {
            link(
                clients,
                ctx,
                *work_item_id,
                branch_url,
                LinkKind::Branch,
                connection_id.as_deref(),
                *as_hyperlink,
                *dry_run,
            )
            .await
        }
        GithubCmd::LinkIssue {
            work_item_id,
            issue_url,
            connection_id,
            as_hyperlink,
            dry_run,
        } => {
            link(
                clients,
                ctx,
                *work_item_id,
                issue_url,
                LinkKind::Issue,
                connection_id.as_deref(),
                *as_hyperlink,
                *dry_run,
            )
            .await
        }
        GithubCmd::Links { work_item_id } => links(clients, ctx, *work_item_id).await,
        GithubCmd::Detect => detect(clients, ctx).await,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum LinkKind {
    PullRequest,
    Commit,
    Branch,
    Issue,
}

impl LinkKind {
    fn vstfs_segment(&self) -> &'static str {
        match self {
            LinkKind::PullRequest => "PullRequest",
            LinkKind::Commit => "Commit",
            LinkKind::Branch => "Branch",
            LinkKind::Issue => "Issue",
        }
    }
    fn attribute_name(&self) -> &'static str {
        match self {
            LinkKind::PullRequest => "GitHub Pull Request",
            LinkKind::Commit => "GitHub Commit",
            LinkKind::Branch => "GitHub Branch",
            LinkKind::Issue => "GitHub Issue",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ParsedUrl {
    pub owner: String,
    pub repo: String,
    /// PR/issue number, commit SHA, or branch name.
    pub suffix: String,
}

/// Parse a GitHub web URL for the given link kind.
pub fn parse_github_url(url: &str, kind: LinkKind) -> Result<ParsedUrl, CliError> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .ok_or_else(|| CliError::Validation(format!("not a github.com URL: {url}")))?;
    let parts: Vec<&str> = rest.splitn(4, '/').collect();
    let (expect, label) = match kind {
        LinkKind::PullRequest => ("pull", "https://github.com/{owner}/{repo}/pull/{number}"),
        LinkKind::Commit => ("commit", "https://github.com/{owner}/{repo}/commit/{sha}"),
        LinkKind::Branch => ("tree", "https://github.com/{owner}/{repo}/tree/{branch}"),
        LinkKind::Issue => (
            "issues",
            "https://github.com/{owner}/{repo}/issues/{number}",
        ),
    };
    match parts.as_slice() {
        [owner, repo, seg, suffix] if *seg == expect && !suffix.is_empty() => {
            let suffix = match kind {
                LinkKind::PullRequest | LinkKind::Issue => {
                    let digits: String =
                        suffix.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if digits.is_empty() {
                        return Err(CliError::Validation(format!("expected {label}")));
                    }
                    digits
                }
                _ => suffix.trim_end_matches('/').to_string(),
            };
            Ok(ParsedUrl {
                owner: owner.to_string(),
                repo: repo.to_string(),
                suffix,
            })
        }
        _ => Err(CliError::Validation(format!("expected {label}"))),
    }
}

/// vstfs:///GitHub/{Kind}/{repoGuid}%2F{suffix} — the encoded '/' is
/// load-bearing; branch names additionally percent-encode their own slashes.
pub fn vstfs_url(kind: LinkKind, repo_guid: &str, suffix: &str) -> String {
    let encoded_suffix = suffix.replace('%', "%25").replace('/', "%2F");
    format!(
        "vstfs:///GitHub/{}/{repo_guid}%2F{encoded_suffix}",
        kind.vstfs_segment()
    )
}

/// Harvest repo GUIDs from existing GitHub artifact links on a relations array.
pub fn harvest_guids(relations: &[Value]) -> std::collections::BTreeSet<String> {
    let mut guids = std::collections::BTreeSet::new();
    for r in relations {
        if r.get("rel").and_then(|v| v.as_str()) != Some("ArtifactLink") {
            continue;
        }
        let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(rest) = url.strip_prefix("vstfs:///GitHub/") {
            // {Kind}/{guid}%2F{suffix}
            if let Some((_, tail)) = rest.split_once('/') {
                if let Some((guid, _)) = tail.split_once("%2F") {
                    if !guid.is_empty() {
                        guids.insert(guid.to_string());
                    }
                }
            }
        }
    }
    guids
}

async fn resolve_repo_guid(
    clients: &Clients,
    ctx: &Ctx,
    parsed: &ParsedUrl,
    flag: Option<&str>,
) -> Result<(String, &'static str), CliError> {
    if let Some(id) = flag {
        return Ok((id.to_string(), "flag"));
    }
    if let Ok(env_id) = std::env::var("ADO_GITHUB_CONNECTION_ID") {
        if !env_id.trim().is_empty() {
            return Ok((env_id.trim().to_string(), "env"));
        }
    }
    let key = format!("{}/{}", parsed.owner, parsed.repo);
    if let Some(id) = ctx.github_connections.get(&key) {
        return Ok((id.clone(), "config"));
    }
    // Scan recent work items for existing GitHub artifact links.
    let project = ctx.project()?;
    let wiql = format!(
        "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = '{}' AND [System.ChangedDate] >= @Today - 90 ORDER BY [System.ChangedDate] DESC",
        project.replace('\'', "''")
    );
    let result = super::wiql_cmd::execute(clients, project, &wiql, Some(200)).await?;
    let ids = super::wiql_cmd::collect_ids(&result);
    let mut guids = std::collections::BTreeSet::new();
    use azure_devops_rust_api::wit::models::{
        work_item_batch_get_request::ErrorPolicy, work_item_batch_get_request::Expand,
        WorkItemBatchGetRequest,
    };
    for chunk in ids.chunks(hydrate::CHUNK) {
        let body = WorkItemBatchGetRequest {
            ids: chunk.to_vec(),
            expand: Some(Expand::Relations),
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
            if let Some(rels) = v.get("relations").and_then(|r| r.as_array()) {
                guids.extend(harvest_guids(rels));
            }
        }
    }
    match guids.len() {
        1 => {
            let guid = guids.into_iter().next().unwrap();
            // Cache for next time (user config, never repo-local).
            if let Ok(mut cfg) = config::load_user_config() {
                cfg.github.connections.insert(key.clone(), guid.clone());
                if config::save_user_config(&cfg).is_ok() {
                    tracing::info!("cached GitHub repo GUID for {key} in user config");
                }
            }
            Ok((guid, "scan"))
        }
        0 => Err(CliError::NotFound(format!(
            "no GitHub repo GUID resolvable for {key}: no existing GitHub artifact links found in the last 90 days. \
             Link any work item to a PR once in the Azure Boards web UI, or pass --connection-id / set \
             [github.connections] \"{key}\" in user config"
        ))),
        n => Err(CliError::Validation(format!(
            "{n} distinct GitHub repo GUIDs found in existing links ({}); pass --connection-id to disambiguate \
             or set [github.connections] \"{key}\" in user config",
            guids.into_iter().collect::<Vec<_>>().join(", ")
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn link(
    clients: &Clients,
    ctx: &Ctx,
    work_item_id: i32,
    url: &str,
    kind: LinkKind,
    connection_id: Option<&str>,
    as_hyperlink: bool,
    dry_run: bool,
) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let parsed = parse_github_url(url, kind)?;
    let operation = format!("github.link-{}", kind.vstfs_segment().to_ascii_lowercase());

    let (rel, link_url, attributes) = if as_hyperlink {
        (
            "Hyperlink".to_string(),
            url.to_string(),
            json!({"comment": kind.attribute_name()}),
        )
    } else {
        let (guid, source) = resolve_repo_guid(clients, ctx, &parsed, connection_id).await?;
        tracing::info!("GitHub repo GUID via {source}: {guid}");
        (
            "ArtifactLink".to_string(),
            vstfs_url(kind, &guid, &parsed.suffix),
            json!({"name": kind.attribute_name()}),
        )
    };
    let ops = vec![patch::add_relation(&rel, &link_url, attributes)];
    if dry_run {
        return Ok(CommandOutput::Mutation(MutationEnvelope {
            operation,
            target: ctx.target(Some(work_item_id as i64))?,
            dry_run: true,
            result: serde_json::to_value(&ops)?,
        }));
    }
    clients
        .wit()
        .work_items_client()
        .update(&clients.org, ops, work_item_id, project)
        .send()
        .await?
        .into_body()?;
    // Verify the relation actually landed (vstfs forms are not validated
    // server-side at PATCH time).
    let item = super::relation::fetch_with_relations(clients, project, work_item_id).await?;
    let landed = item
        .get("relations")
        .and_then(|r| r.as_array())
        .map(|rels| {
            rels.iter()
                .any(|r| r.get("url").and_then(|u| u.as_str()) == Some(link_url.as_str()))
        })
        .unwrap_or(false);
    if !landed {
        return Err(CliError::General(format!(
            "link PATCH succeeded but the relation did not appear on work item {work_item_id}"
        )));
    }
    Ok(CommandOutput::Mutation(MutationEnvelope {
        operation,
        target: ctx.target(Some(work_item_id as i64))?,
        dry_run: false,
        result: json!({"linked": link_url, "github": url, "verified": true}),
    }))
}

async fn links(clients: &Clients, ctx: &Ctx, work_item_id: i32) -> Result<CommandOutput, CliError> {
    let project = ctx.project()?;
    let item = super::relation::fetch_with_relations(clients, project, work_item_id).await?;
    let relations = item
        .get("relations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let github_links: Vec<Value> = super::relation::decorate(&relations)
        .into_iter()
        .filter(|r| {
            let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
            url.starts_with("vstfs:///GitHub/") || url.contains("github.com/")
        })
        .collect();
    Ok(CommandOutput::List {
        value: github_links,
        columns: vec![
            Column::new("Index", "index"),
            Column::new("Type", "type"),
            Column::new("URL", "url"),
        ],
    })
}

async fn detect(clients: &Clients, ctx: &Ctx) -> Result<CommandOutput, CliError> {
    let repo = ctx
        .github_repo
        .as_ref()
        .ok_or_else(|| CliError::NotFound("no GitHub remote detected in this repository".into()))?;
    let key = format!("{}/{}", repo.owner, repo.repo);
    let project = ctx.project()?;
    // githubconnections is a preview API not in the SDK.
    let url = format!(
        "{}/{}/_apis/githubconnections?api-version=7.1-preview.1",
        clients.org_url(),
        enc(project)
    );
    let mut connected = false;
    let mut connections_info = Vec::new();
    match clients.raw().get_json(&url).await {
        Ok(value) => {
            let connections = value
                .get("value")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for conn in &connections {
                let conn_id = conn.get("id").and_then(|i| i.as_str()).unwrap_or("");
                let repos_url = format!(
                    "{}/{}/_apis/githubconnections/{conn_id}/repos?api-version=7.1-preview.1",
                    clients.org_url(),
                    enc(project)
                );
                let repos = clients
                    .raw()
                    .get_json(&repos_url)
                    .await
                    .unwrap_or(Value::Null);
                let urls: Vec<String> = repos
                    .get("value")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|r| {
                                r.get("gitHubRepositoryUrl")
                                    .and_then(|u| u.as_str())
                                    .map(String::from)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let matches = urls.iter().any(|u| {
                    u.trim_end_matches(".git")
                        .to_ascii_lowercase()
                        .ends_with(&format!("github.com/{}", key.to_ascii_lowercase()))
                });
                if matches {
                    connected = true;
                }
                connections_info.push(json!({
                    "connectionId": conn_id,
                    "isValid": conn.get("isConnectionValid").cloned().unwrap_or(Value::Null),
                    "repoCount": urls.len(),
                    "containsThisRepo": matches,
                }));
            }
        }
        Err(e) => {
            tracing::warn!("githubconnections API unavailable: {e}");
        }
    }
    let cached_guid = ctx.github_connections.get(&key).cloned();
    Ok(CommandOutput::Item {
        value: json!({
            "githubRepo": key,
            "remote": repo.remote_name,
            "connectedToBoards": connected,
            "connections": connections_info,
            "cachedRepoGuid": cached_guid,
            "note": if connected || cached_guid_present(&ctx.github_connections, &key) {
                "ready: github link-* commands can resolve this repo"
            } else {
                "repo not visibly connected: link one work item via the web UI once, or pass --connection-id"
            },
        }),
        columns: vec![],
    })
}

fn cached_guid_present(map: &std::collections::BTreeMap<String, String>, key: &str) -> bool {
    map.contains_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pr_commit_branch_issue_urls() {
        let pr = parse_github_url(
            "https://github.com/nrgmr/cinesys/pull/45",
            LinkKind::PullRequest,
        )
        .unwrap();
        assert_eq!(
            (pr.owner.as_str(), pr.repo.as_str(), pr.suffix.as_str()),
            ("nrgmr", "cinesys", "45")
        );

        let commit =
            parse_github_url("https://github.com/o/r/commit/deadbeef", LinkKind::Commit).unwrap();
        assert_eq!(commit.suffix, "deadbeef");

        let branch =
            parse_github_url("https://github.com/o/r/tree/feature/x", LinkKind::Branch).unwrap();
        assert_eq!(branch.suffix, "feature/x");

        let issue = parse_github_url("https://github.com/o/r/issues/12", LinkKind::Issue).unwrap();
        assert_eq!(issue.suffix, "12");

        assert_eq!(
            parse_github_url("https://github.com/o/r/pull/abc", LinkKind::PullRequest)
                .unwrap_err()
                .exit_code(),
            7
        );
        assert_eq!(
            parse_github_url("https://gitlab.com/o/r/pull/1", LinkKind::PullRequest)
                .unwrap_err()
                .exit_code(),
            7
        );
    }

    #[test]
    fn vstfs_urls_encode_slashes() {
        assert_eq!(
            vstfs_url(LinkKind::PullRequest, "abc-123", "45"),
            "vstfs:///GitHub/PullRequest/abc-123%2F45"
        );
        assert_eq!(
            vstfs_url(LinkKind::Branch, "abc-123", "feature/x"),
            "vstfs:///GitHub/Branch/abc-123%2Ffeature%2Fx"
        );
    }

    #[test]
    fn harvests_guids_from_artifact_links() {
        let relations = vec![
            json!({"rel": "ArtifactLink", "url": "vstfs:///GitHub/PullRequest/guid-1%2F45"}),
            json!({"rel": "ArtifactLink", "url": "vstfs:///GitHub/Commit/guid-1%2Fdeadbeef"}),
            json!({"rel": "ArtifactLink", "url": "vstfs:///Git/Commit/other"}),
            json!({"rel": "Hyperlink", "url": "https://github.com/o/r/pull/9"}),
        ];
        let guids = harvest_guids(&relations);
        assert_eq!(guids.len(), 1);
        assert!(guids.contains("guid-1"));
    }
}
