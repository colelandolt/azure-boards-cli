pub mod config;
pub mod detect;
pub mod prompt;

use crate::error::CliError;
use crate::output::OutputFormat;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Flag,
    Env(&'static str),
    RepoConfig,
    UserConfig,
    GitRemote(String),
    Default,
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Flag => "command-line flag".into(),
            Source::Env(name) => format!("env {name}"),
            Source::RepoConfig => "repo config (.azure-boards.toml)".into(),
            Source::UserConfig => "user config".into(),
            Source::GitRemote(remote) => format!("git remote {remote}"),
            Source::Default => "default".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved<T> {
    pub value: T,
    pub source: Source,
}

/// All raw inputs to context resolution, gathered up-front so resolution
/// itself is pure and unit-testable.
#[derive(Debug, Default)]
pub struct ResolveInputs {
    pub flag_org: Option<String>,
    pub flag_project: Option<String>,
    pub flag_team: Option<String>,
    pub flag_detect: Option<bool>,
    pub env_org: Option<String>,
    pub env_project: Option<String>,
    pub env_team: Option<String>,
    pub repo: Option<config::RepoConfig>,
    pub user: config::UserConfig,
    pub detected: detect::Detected,
}

/// Fully resolved invocation context.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Org short name (e.g. "nrgmr"). URL form is `org_url()`.
    pub org: Option<Resolved<String>>,
    pub project: Option<Resolved<String>>,
    pub team: Option<Resolved<String>>,
    pub detect_enabled: Resolved<bool>,
    /// Output default from config (flag-level format is applied in main).
    pub config_output: Option<OutputFormat>,
    pub github_repo: Option<detect::GithubRemote>,
    pub github_connections: std::collections::BTreeMap<String, String>,
    /// Global --yes: skip confirmation prompts (set by main after parsing).
    pub assume_yes: bool,
}

impl Ctx {
    pub fn org_name(&self) -> Result<&str, CliError> {
        self.org
            .as_ref()
            .map(|r| r.value.as_str())
            .ok_or_else(|| missing("organization", "--org", "ADO_ORG"))
    }

    pub fn org_url(&self) -> Result<String, CliError> {
        Ok(format!("https://dev.azure.com/{}", self.org_name()?))
    }

    pub fn project(&self) -> Result<&str, CliError> {
        self.project
            .as_ref()
            .map(|r| r.value.as_str())
            .ok_or_else(|| missing("project", "--project", "ADO_PROJECT"))
    }

    pub fn team(&self) -> Result<&str, CliError> {
        self.team
            .as_ref()
            .map(|r| r.value.as_str())
            .ok_or_else(|| missing("team", "--team", "ADO_TEAM"))
    }

    pub fn target(&self, work_item_id: Option<i64>) -> Result<crate::output::Target, CliError> {
        Ok(crate::output::Target {
            organization: self.org_name()?.to_string(),
            project: self.project()?.to_string(),
            team: self.team.as_ref().map(|t| t.value.clone()),
            work_item_id,
        })
    }
}

fn missing(what: &str, flag: &str, env: &str) -> CliError {
    CliError::Validation(format!(
        "no {what} resolved: pass {flag}, set {env}, run `ab configure --defaults`, or run inside a repo with an Azure DevOps remote"
    ))
}

/// Normalize an org value: accept "nrgmr" or "https://dev.azure.com/nrgmr".
/// Reject any other host (Azure DevOps Services only).
pub fn normalize_org(raw: &str) -> Result<String, CliError> {
    let raw = raw.trim().trim_end_matches('/');
    if let Some(rest) = raw
        .strip_prefix("https://dev.azure.com/")
        .or_else(|| raw.strip_prefix("http://dev.azure.com/"))
    {
        let name = rest.split('/').next().unwrap_or("");
        if name.is_empty() {
            return Err(CliError::Validation(
                "organization URL has no org name".into(),
            ));
        }
        return Ok(name.to_string());
    }
    if raw.contains("://") || raw.contains('/') {
        return Err(CliError::Validation(format!(
            "unsupported organization '{raw}': only Azure DevOps Services (https://dev.azure.com/<org>) is supported"
        )));
    }
    if raw.is_empty() {
        return Err(CliError::Validation("organization is empty".into()));
    }
    Ok(raw.to_string())
}

/// One rung of the resolution ladder for the --explain trace.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Step {
    pub source: String,
    pub value: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Default)]
pub struct Explanation {
    pub organization: Vec<Step>,
    pub project: Vec<Step>,
    pub team: Vec<Step>,
}

impl Explanation {
    pub fn to_json(&self) -> Value {
        json!({
            "organization": self.organization,
            "project": self.project,
            "team": self.team,
        })
    }
}

/// Gather raw inputs from the real environment.
pub fn gather_inputs(
    flag_org: Option<String>,
    flag_project: Option<String>,
    flag_team: Option<String>,
    flag_detect: Option<bool>,
) -> ResolveInputs {
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let repo = config::find_repo_config(&cwd).and_then(|p| config::load_repo_config(&p).ok());
    let user = config::load_user_config().unwrap_or_default();
    // Detection is gated on the resolved `detect` setting; only shell out when enabled.
    let detect_enabled = flag_detect
        .or(repo.as_ref().and_then(|r| r.detect))
        .or(user.defaults.detect)
        .unwrap_or(true);
    let detected = if detect_enabled {
        detect::detect(&cwd)
    } else {
        detect::Detected::default()
    };
    ResolveInputs {
        flag_org,
        flag_project,
        flag_team,
        flag_detect,
        env_org: std::env::var("ADO_ORG").ok().filter(|s| !s.is_empty()),
        env_project: std::env::var("ADO_PROJECT").ok().filter(|s| !s.is_empty()),
        env_team: std::env::var("ADO_TEAM").ok().filter(|s| !s.is_empty()),
        repo,
        user,
        detected,
    }
}

/// Pure precedence resolution: flag > env > repo config > user config > git remote.
/// Interactive prompting is deliberately NOT here; callers that allow it do so
/// explicitly after seeing a Validation error.
pub fn resolve(inputs: &ResolveInputs) -> Result<(Ctx, Explanation), CliError> {
    let detect_enabled = first(&[
        (Source::Flag, inputs.flag_detect),
        (
            Source::RepoConfig,
            inputs.repo.as_ref().and_then(|r| r.detect),
        ),
        (Source::UserConfig, inputs.user.defaults.detect),
    ])
    .unwrap_or(Resolved {
        value: true,
        source: Source::Default,
    });

    let git_org = inputs
        .detected
        .azure
        .as_ref()
        .map(|a| (a.remote_name.clone(), a.organization.clone()));
    let git_project = inputs
        .detected
        .azure
        .as_ref()
        .map(|a| (a.remote_name.clone(), a.project.clone()));

    let (org, org_steps) = ladder(
        "organization",
        &[
            (Source::Flag, inputs.flag_org.clone()),
            (Source::Env("ADO_ORG"), inputs.env_org.clone()),
            (
                Source::RepoConfig,
                inputs.repo.as_ref().and_then(|r| r.organization.clone()),
            ),
            (
                Source::UserConfig,
                inputs.user.defaults.organization.clone(),
            ),
            (
                git_org
                    .as_ref()
                    .map(|(n, _)| Source::GitRemote(n.clone()))
                    .unwrap_or(Source::GitRemote("none".into())),
                detect_enabled
                    .value
                    .then(|| git_org.as_ref().map(|(_, v)| v.clone()))
                    .flatten(),
            ),
        ],
    );
    let org = org
        .map(|r| -> Result<Resolved<String>, CliError> {
            Ok(Resolved {
                value: normalize_org(&r.value)?,
                source: r.source,
            })
        })
        .transpose()?;

    let (project, project_steps) = ladder(
        "project",
        &[
            (Source::Flag, inputs.flag_project.clone()),
            (Source::Env("ADO_PROJECT"), inputs.env_project.clone()),
            (
                Source::RepoConfig,
                inputs.repo.as_ref().and_then(|r| r.project.clone()),
            ),
            (Source::UserConfig, inputs.user.defaults.project.clone()),
            (
                git_project
                    .as_ref()
                    .map(|(n, _)| Source::GitRemote(n.clone()))
                    .unwrap_or(Source::GitRemote("none".into())),
                detect_enabled
                    .value
                    .then(|| git_project.as_ref().map(|(_, v)| v.clone()))
                    .flatten(),
            ),
        ],
    );

    let (team, team_steps) = ladder(
        "team",
        &[
            (Source::Flag, inputs.flag_team.clone()),
            (Source::Env("ADO_TEAM"), inputs.env_team.clone()),
            (
                Source::RepoConfig,
                inputs.repo.as_ref().and_then(|r| r.team.clone()),
            ),
            (Source::UserConfig, inputs.user.defaults.team.clone()),
        ],
    );

    let config_output = inputs
        .repo
        .as_ref()
        .and_then(|r| r.default_output.clone())
        .or(inputs.user.defaults.output.clone())
        .and_then(|s| s.parse::<OutputFormat>().ok());

    let ctx = Ctx {
        org,
        project,
        team,
        detect_enabled,
        config_output,
        github_repo: inputs.detected.github.clone(),
        github_connections: inputs.user.github.connections.clone(),
        assume_yes: false,
    };
    let explanation = Explanation {
        organization: org_steps,
        project: project_steps,
        team: team_steps,
    };
    Ok((ctx, explanation))
}

fn first<T: Copy>(rungs: &[(Source, Option<T>)]) -> Option<Resolved<T>> {
    rungs.iter().find_map(|(source, value)| {
        value.map(|v| Resolved {
            value: v,
            source: source.clone(),
        })
    })
}

fn ladder(
    _field: &str,
    rungs: &[(Source, Option<String>)],
) -> (Option<Resolved<String>>, Vec<Step>) {
    let mut selected: Option<Resolved<String>> = None;
    let mut steps = Vec::new();
    for (source, value) in rungs {
        let is_selected = selected.is_none() && value.is_some();
        steps.push(Step {
            source: source.describe(),
            value: value.clone(),
            selected: is_selected,
        });
        if is_selected {
            selected = Some(Resolved {
                value: value.clone().unwrap(),
                source: source.clone(),
            });
        }
    }
    (selected, steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs() -> ResolveInputs {
        ResolveInputs {
            user: config::UserConfig::default(),
            ..Default::default()
        }
    }

    #[test]
    fn flag_beats_env_beats_configs_beats_git() {
        let mut inputs = base_inputs();
        inputs.env_org = Some("env-org".into());
        inputs.flag_org = Some("flag-org".into());
        inputs.user.defaults.organization = Some("user-org".into());
        inputs.detected.azure = Some(detect::AzureRemote {
            organization: "git-org".into(),
            project: "git-proj".into(),
            repo: "r".into(),
            remote_name: "origin".into(),
        });
        let (ctx, explain) = resolve(&inputs).unwrap();
        assert_eq!(ctx.org.as_ref().unwrap().value, "flag-org");
        assert_eq!(ctx.org.as_ref().unwrap().source, Source::Flag);
        // Project falls through to git detection.
        assert_eq!(ctx.project.as_ref().unwrap().value, "git-proj");
        assert!(matches!(
            ctx.project.as_ref().unwrap().source,
            Source::GitRemote(_)
        ));
        // Explain records the shadowed sources.
        let org_selected: Vec<_> = explain.organization.iter().filter(|s| s.selected).collect();
        assert_eq!(org_selected.len(), 1);
        assert_eq!(org_selected[0].source, "command-line flag");
    }

    #[test]
    fn detect_false_skips_git_rung() {
        let mut inputs = base_inputs();
        inputs.flag_detect = Some(false);
        inputs.detected.azure = Some(detect::AzureRemote {
            organization: "git-org".into(),
            project: "git-proj".into(),
            repo: "r".into(),
            remote_name: "origin".into(),
        });
        let (ctx, _) = resolve(&inputs).unwrap();
        assert!(ctx.org.is_none());
        assert!(ctx.project.is_none());
    }

    #[test]
    fn org_url_accepted_and_normalized() {
        let mut inputs = base_inputs();
        inputs.flag_org = Some("https://dev.azure.com/nrgmr".into());
        let (ctx, _) = resolve(&inputs).unwrap();
        assert_eq!(ctx.org_name().unwrap(), "nrgmr");
        assert_eq!(ctx.org_url().unwrap(), "https://dev.azure.com/nrgmr");
    }

    #[test]
    fn non_services_org_rejected() {
        assert_eq!(
            normalize_org("https://tfs.corp.local/collection")
                .unwrap_err()
                .exit_code(),
            7
        );
        assert!(normalize_org("nrgmr").is_ok());
        assert_eq!(
            normalize_org("https://dev.azure.com/nrgmr/").unwrap(),
            "nrgmr"
        );
    }

    #[test]
    fn missing_context_is_validation_error() {
        let inputs = base_inputs();
        let (ctx, _) = resolve(&inputs).unwrap();
        assert_eq!(ctx.org_name().unwrap_err().exit_code(), 7);
        assert_eq!(ctx.project().unwrap_err().exit_code(), 7);
    }

    #[test]
    fn repo_config_beats_user_config() {
        let mut inputs = base_inputs();
        inputs.repo = Some(config::RepoConfig {
            project: Some("repo-proj".into()),
            ..Default::default()
        });
        inputs.user.defaults.project = Some("user-proj".into());
        let (ctx, _) = resolve(&inputs).unwrap();
        assert_eq!(ctx.project.as_ref().unwrap().value, "repo-proj");
        assert_eq!(ctx.project.as_ref().unwrap().source, Source::RepoConfig);
    }

    #[test]
    fn config_output_prefers_repo_local() {
        let mut inputs = base_inputs();
        inputs.repo = Some(config::RepoConfig {
            default_output: Some("json".into()),
            ..Default::default()
        });
        inputs.user.defaults.output = Some("yaml".into());
        let (ctx, _) = resolve(&inputs).unwrap();
        assert_eq!(ctx.config_output, Some(OutputFormat::Json));
    }
}
