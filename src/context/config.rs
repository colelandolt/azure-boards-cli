use crate::error::CliError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// User config: ~/.config/azure-boards/config.toml
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub defaults: Defaults,
    /// "owner/repo" -> Boards GitHub repo GUID used in vstfs artifact links.
    #[serde(default, rename = "github")]
    pub github: GithubSection,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GithubSection {
    #[serde(default)]
    pub connections: BTreeMap<String, String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Defaults {
    pub organization: Option<String>,
    pub project: Option<String>,
    pub team: Option<String>,
    pub output: Option<String>,
    pub detect: Option<bool>,
}

/// Repo-local config: .azure-boards.toml at the git root. Whitelist-only —
/// any unknown key is rejected so secrets are structurally impossible here.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub organization: Option<String>,
    pub project: Option<String>,
    pub team: Option<String>,
    #[serde(rename = "default-output")]
    pub default_output: Option<String>,
    pub detect: Option<bool>,
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("azure-boards")
}

pub fn user_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load_user_config() -> Result<UserConfig, CliError> {
    load_user_config_from(&user_config_path())
}

pub fn load_user_config_from(path: &Path) -> Result<UserConfig, CliError> {
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text)
        .map_err(|e| CliError::Validation(format!("invalid user config {}: {e}", path.display())))
}

pub fn save_user_config(cfg: &UserConfig) -> Result<PathBuf, CliError> {
    let path = user_config_path();
    save_user_config_to(cfg, &path)?;
    Ok(path)
}

pub fn save_user_config_to(cfg: &UserConfig, path: &Path) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let text = toml::to_string_pretty(cfg)
        .map_err(|e| CliError::General(format!("config encode: {e}")))?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Locate the repo-local config by walking up from cwd to the git root (or
/// filesystem root). Checks `.azure-boards.toml` then `.azure-boards/config.toml`.
pub fn find_repo_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for candidate in [
            d.join(".azure-boards.toml"),
            d.join(".azure-boards").join("config.toml"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if d.join(".git").exists() {
            break;
        }
        dir = d.parent();
    }
    None
}

pub fn load_repo_config(path: &Path) -> Result<RepoConfig, CliError> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|e| {
        CliError::Validation(format!(
            "invalid repo config {} (allowed keys: organization, project, team, default-output, detect): {e}",
            path.display()
        ))
    })
}

pub fn save_repo_config(cfg: &RepoConfig, dir: &Path) -> Result<PathBuf, CliError> {
    let path = dir.join(".azure-boards.toml");
    let text = toml::to_string_pretty(cfg)
        .map_err(|e| CliError::General(format!("config encode: {e}")))?;
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Create a directory with 0700 permissions on Unix.
pub fn create_private_dir(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_config_rejects_unknown_keys() {
        let err = toml::from_str::<RepoConfig>("organization = \"org\"\npat = \"secret\"\n");
        assert!(err.is_err(), "secret-bearing keys must be rejected");
    }

    #[test]
    fn repo_config_accepts_whitelisted_keys() {
        let cfg: RepoConfig = toml::from_str(
            "organization = \"nrgmr\"\nproject = \"Yellow Hat\"\nteam = \"YH\"\ndefault-output = \"json\"\ndetect = false\n",
        )
        .unwrap();
        assert_eq!(cfg.organization.as_deref(), Some("nrgmr"));
        assert_eq!(cfg.default_output.as_deref(), Some("json"));
        assert_eq!(cfg.detect, Some(false));
    }

    #[test]
    fn finds_repo_config_walking_up_to_git_root() {
        let tmp = std::env::temp_dir().join(format!("ab-test-{}", std::process::id()));
        let nested = tmp.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::write(tmp.join(".azure-boards.toml"), "project = \"P\"\n").unwrap();
        let found = find_repo_config(&nested).unwrap();
        assert_eq!(found, tmp.join(".azure-boards.toml"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn user_config_github_connections_roundtrip() {
        let mut cfg = UserConfig::default();
        cfg.github
            .connections
            .insert("nrgmr/cinesys".into(), "abc-123".into());
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: UserConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.github.connections["nrgmr/cinesys"], "abc-123");
    }
}
