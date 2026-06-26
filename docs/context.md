# Config & context resolution

Every command needs an organization, usually a project, and sometimes a team. Each resolves independently through this ladder (first value wins):

```
1. explicit flag        --org / --project / --team
2. environment          ADO_ORG / ADO_PROJECT / ADO_TEAM
3. repo-local config    .azure-boards.toml at the git root
4. user config          ~/.config/azure-boards/config.toml
5. git remote detection (org/project from an Azure DevOps remote)
6. interactive prompt   only on a TTY; otherwise exit 7
```

`ab context detect --explain` prints the full rung-by-rung trace — which sources had values and which one won.

## User config

`~/.config/azure-boards/config.toml` (set via `ab configure --defaults k=v ...`). The
location follows the OS convention (`%APPDATA%\azure-boards` on Windows) and can be
overridden with the `AZURE_BOARDS_CONFIG_DIR` environment variable — useful for pinning a
custom location or for fully isolated, scripted environments:

```toml
[defaults]
organization = "nrgmr"
project = "Yellow Hat"
team = "Yellow Hat Team"
output = "table"
detect = true

[github.connections]
"owner/repo" = "<boards-github-repo-guid>"   # cached by `ab github link-*`
```

## Repo-local config

`.azure-boards.toml` at the git root (write with `ab configure --defaults k=v --local`). **Whitelist-only** — exactly these keys are allowed, so secrets are structurally impossible in a committed file:

```toml
organization = "nrgmr"
project = "Yellow Hat"
team = "Yellow Hat Team"
default-output = "json"
detect = false
```

Any other key is rejected with a validation error (exit 7).

## Git remote detection

Enabled by default; disable with `--detect false` (flag, repo config, or user config). Recognized remote forms:

- `https://dev.azure.com/{org}/{project}/_git/{repo}` (with optional `user@`)
- `git@ssh.dev.azure.com:v3/{org}/{project}/{repo}`
- `https://{org}.visualstudio.com/{project}/_git/{repo}` (legacy)
- `github.com/{owner}/{repo}` — fills the GitHub repo context for `ab github` only, never org/project

`origin` wins over other remotes; otherwise the first matching remote in name order.

## Targets in mutations

Every mutating command embeds the fully resolved target in its `--dry-run` preview and its JSON result:

```json
{"operation": "work-item.update", "target": {"organization": "nrgmr", "project": "Yellow Hat", "workItemId": 123}, ...}
```
