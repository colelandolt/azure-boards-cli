//! Git-remote auto-detection of Azure DevOps org/project and GitHub owner/repo.
//! Shells out to `git remote -v` (no libgit2 dependency).

use percent_encoding::percent_decode_str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzureRemote {
    pub organization: String,
    pub project: String,
    pub repo: String,
    pub remote_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRemote {
    pub owner: String,
    pub repo: String,
    pub remote_name: String,
}

#[derive(Debug, Default, Clone)]
pub struct Detected {
    pub azure: Option<AzureRemote>,
    pub github: Option<GithubRemote>,
}

/// Run `git remote -v` in `cwd` and parse. Errors (not a repo, git missing)
/// degrade to an empty result — detection is best-effort by design.
pub fn detect(cwd: &std::path::Path) -> Detected {
    let output = std::process::Command::new("git")
        .arg("remote")
        .arg("-v")
        .current_dir(cwd)
        .output();
    match output {
        Ok(out) if out.status.success() => parse_remotes(&String::from_utf8_lossy(&out.stdout)),
        _ => Detected::default(),
    }
}

/// Parse `git remote -v` output. `origin` wins; otherwise the first matching
/// remote by sorted remote name (deterministic).
pub fn parse_remotes(text: &str) -> Detected {
    let mut entries: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?.to_string();
            let url = parts.next()?.to_string();
            Some((name, url))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup();

    let pick = |matches: Vec<(String, String)>| -> Option<(String, String)> {
        matches
            .iter()
            .find(|(n, _)| n == "origin")
            .cloned()
            .or_else(|| matches.first().cloned())
    };

    let azure_candidates: Vec<(String, String)> = entries
        .iter()
        .filter(|(_, url)| parse_azure_url(url).is_some())
        .cloned()
        .collect();
    let github_candidates: Vec<(String, String)> = entries
        .iter()
        .filter(|(_, url)| parse_github_url(url).is_some())
        .cloned()
        .collect();

    Detected {
        azure: pick(azure_candidates).and_then(|(name, url)| {
            parse_azure_url(&url).map(|(org, project, repo)| AzureRemote {
                organization: org,
                project,
                repo,
                remote_name: name,
            })
        }),
        github: pick(github_candidates).and_then(|(name, url)| {
            parse_github_url(&url).map(|(owner, repo)| GithubRemote {
                owner,
                repo,
                remote_name: name,
            })
        }),
    }
}

/// Supported forms:
///   https://dev.azure.com/{org}/{project}/_git/{repo}
///   https://{user}@dev.azure.com/{org}/{project}/_git/{repo}
///   git@ssh.dev.azure.com:v3/{org}/{project}/{repo}
///   https://{org}.visualstudio.com/{project}/_git/{repo}   (legacy)
fn parse_azure_url(url: &str) -> Option<(String, String, String)> {
    let decode = |s: &str| {
        percent_decode_str(s)
            .decode_utf8()
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| s.to_string())
    };
    if let Some(rest) = url.strip_prefix("git@ssh.dev.azure.com:v3/") {
        let parts: Vec<&str> = rest.trim_end_matches(".git").splitn(3, '/').collect();
        if let [org, project, repo] = parts[..] {
            return Some((org.into(), decode(project), decode(repo)));
        }
        return None;
    }
    let after_scheme = url
        .strip_prefix("https://")
        .or(url.strip_prefix("http://"))?;
    // Strip optional user@ prefix.
    let host_and_path = after_scheme
        .split_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(after_scheme);
    let (host, path) = host_and_path.split_once('/')?;
    let segments: Vec<&str> = path.trim_end_matches(".git").split('/').collect();
    if host == "dev.azure.com" {
        if let [org, project, "_git", repo] = segments[..] {
            return Some((org.into(), decode(project), decode(repo)));
        }
    } else if let Some(org) = host.strip_suffix(".visualstudio.com") {
        if let [project, "_git", repo] = segments[..] {
            return Some((org.into(), decode(project), decode(repo)));
        }
    }
    None
}

/// Supported forms: https://github.com/{owner}/{repo}(.git), git@github.com:{owner}/{repo}(.git)
fn parse_github_url(url: &str) -> Option<(String, String)> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let parts: Vec<&str> = rest.trim_end_matches(".git").splitn(2, '/').collect();
    if let [owner, repo] = parts[..] {
        if !owner.is_empty() && !repo.is_empty() && !repo.contains('/') {
            return Some((owner.into(), repo.into()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_azure_remote_with_spaces() {
        let d = parse_remotes(
            "origin\thttps://dev.azure.com/nrgmr/Yellow%20Hat/_git/cinesys (fetch)\norigin\thttps://dev.azure.com/nrgmr/Yellow%20Hat/_git/cinesys (push)\n",
        );
        let a = d.azure.unwrap();
        assert_eq!(a.organization, "nrgmr");
        assert_eq!(a.project, "Yellow Hat");
        assert_eq!(a.repo, "cinesys");
    }

    #[test]
    fn parses_ssh_azure_remote() {
        let d = parse_remotes("origin\tgit@ssh.dev.azure.com:v3/nrgmr/Proj/repo (fetch)\n");
        let a = d.azure.unwrap();
        assert_eq!(
            (a.organization.as_str(), a.project.as_str(), a.repo.as_str()),
            ("nrgmr", "Proj", "repo")
        );
    }

    #[test]
    fn parses_user_at_and_legacy_hosts() {
        assert_eq!(
            parse_azure_url("https://cole@dev.azure.com/nrgmr/P/_git/r"),
            Some(("nrgmr".into(), "P".into(), "r".into()))
        );
        assert_eq!(
            parse_azure_url("https://nrgmr.visualstudio.com/P/_git/r"),
            Some(("nrgmr".into(), "P".into(), "r".into()))
        );
    }

    #[test]
    fn parses_github_forms() {
        assert_eq!(
            parse_github_url("git@github.com:nrgmr/cinesys.git"),
            Some(("nrgmr".into(), "cinesys".into()))
        );
        assert_eq!(
            parse_github_url("https://github.com/nrgmr/cinesys"),
            Some(("nrgmr".into(), "cinesys".into()))
        );
    }

    #[test]
    fn origin_wins_over_other_remotes() {
        let d = parse_remotes(
            "alt\thttps://dev.azure.com/other/O/_git/x (fetch)\norigin\thttps://dev.azure.com/nrgmr/P/_git/r (fetch)\n",
        );
        assert_eq!(d.azure.unwrap().organization, "nrgmr");
    }

    #[test]
    fn first_sorted_remote_when_no_origin() {
        let d = parse_remotes(
            "zeta\thttps://dev.azure.com/zorg/Z/_git/z (fetch)\nalpha\thttps://dev.azure.com/aorg/A/_git/a (fetch)\n",
        );
        assert_eq!(d.azure.unwrap().organization, "aorg");
    }

    #[test]
    fn github_remote_never_fills_azure() {
        let d = parse_remotes("origin\thttps://github.com/nrgmr/cinesys.git (fetch)\n");
        assert!(d.azure.is_none());
        assert_eq!(d.github.unwrap().owner, "nrgmr");
    }
}
