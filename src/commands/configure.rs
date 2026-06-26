use crate::cli::ConfigureArgs;
use crate::context::config;
use crate::error::CliError;
use crate::output::CommandOutput;
use serde_json::json;

pub fn run(args: &ConfigureArgs) -> Result<CommandOutput, CliError> {
    if args.list || args.defaults.is_empty() {
        return list();
    }
    if args.local {
        set_repo_defaults(&args.defaults)
    } else {
        set_user_defaults(&args.defaults)
    }
}

const ALLOWED: &[&str] = &["organization", "project", "team", "output", "detect"];

fn set_user_defaults(pairs: &[(String, String)]) -> Result<CommandOutput, CliError> {
    let mut cfg = config::load_user_config()?;
    apply(pairs, |key, value| match key {
        "organization" => cfg.defaults.organization = some_or_clear(value),
        "project" => cfg.defaults.project = some_or_clear(value),
        "team" => cfg.defaults.team = some_or_clear(value),
        "output" => cfg.defaults.output = some_or_clear(value),
        "detect" => cfg.defaults.detect = parse_bool(value),
        _ => unreachable!(),
    })?;
    let path = config::save_user_config(&cfg)?;
    Ok(CommandOutput::Item {
        value: json!({
            "written": path.display().to_string(),
            "defaults": serde_json::to_value(&cfg.defaults)?,
        }),
        columns: vec![],
    })
}

fn set_repo_defaults(pairs: &[(String, String)]) -> Result<CommandOutput, CliError> {
    let cwd = std::env::current_dir()?;
    let root = git_root(&cwd).unwrap_or(cwd);
    let existing = config::find_repo_config(&root)
        .map(|p| config::load_repo_config(&p))
        .transpose()?
        .unwrap_or_default();
    let mut cfg = existing;
    apply(pairs, |key, value| match key {
        "organization" => cfg.organization = some_or_clear(value),
        "project" => cfg.project = some_or_clear(value),
        "team" => cfg.team = some_or_clear(value),
        "output" => cfg.default_output = some_or_clear(value),
        "detect" => cfg.detect = parse_bool(value),
        _ => unreachable!(),
    })?;
    let path = config::save_repo_config(&cfg, &root)?;
    Ok(CommandOutput::Item {
        value: json!({
            "written": path.display().to_string(),
            "config": serde_json::to_value(&cfg)?,
        }),
        columns: vec![],
    })
}

fn apply(pairs: &[(String, String)], mut set: impl FnMut(&str, &str)) -> Result<(), CliError> {
    for (key, value) in pairs {
        let key = key.as_str();
        if !ALLOWED.contains(&key) {
            return Err(CliError::Validation(format!(
                "unknown default '{key}' (allowed: {})",
                ALLOWED.join(", ")
            )));
        }
        set(key, value);
    }
    Ok(())
}

fn some_or_clear(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        "" => None,
        _ => None,
    }
}

fn list() -> Result<CommandOutput, CliError> {
    let user = config::load_user_config()?;
    let cwd = std::env::current_dir()?;
    let repo_path = config::find_repo_config(&cwd);
    let repo = repo_path
        .as_ref()
        .map(|p| config::load_repo_config(p))
        .transpose()?;
    Ok(CommandOutput::Item {
        value: json!({
            "userConfig": config::user_config_path().display().to_string(),
            "userDefaults": serde_json::to_value(&user.defaults)?,
            "repoConfig": repo_path.map(|p| p.display().to_string()),
            "repoValues": repo.map(|r| serde_json::to_value(&r)).transpose()?,
        }),
        columns: vec![],
    })
}

fn git_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}
