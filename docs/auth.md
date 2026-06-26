# Authentication

Entra-first, with PAT fallback. Azure DevOps **Services** only (`https://dev.azure.com/<org>`).

## Credential precedence

Each invocation resolves a credential in this order; the first hit wins:

1. `ADO_TOKEN` env var — a Microsoft Entra bearer token, used as-is (agents/CI).
2. `ADO_PAT` env var — a Personal Access Token (agents/CI).
3. `AZURE_DEVOPS_EXT_PAT` env var — honored for az-CLI migration compatibility.
4. **Stored Entra sign-in** — from `ab auth login` (device code); access tokens are refreshed silently and rotated refresh tokens are re-persisted.
5. **Azure CLI** — if you've run `az login`, tokens are acquired from `az` with zero setup.
6. **Stored PAT** — from `ab auth login --pat`.
7. **Interactive device-code prompt** — only when stdin and stderr are TTYs. Non-interactive runs fail with exit 3 and a list of the sources tried.

`ab auth status` shows which source is active, the signed-in identity, and where credentials are stored. It never prints token material.

## Commands

| Command | What it does |
|---|---|
| `ab auth login` | Entra device-code sign-in (prints a URL + code on stderr); persists tokens |
| `ab auth login --pat` | Reads a PAT **from stdin** (never argv), validates it against the org, then stores it |
| `ab auth status` | Active source, identity, org, token expiry, storage backend |
| `ab auth logout` | Removes stored Entra and PAT credentials |

```sh
# PAT login non-interactively:
echo "$MY_PAT" | ab auth login --pat --org nrgmr
```

## Storage

Secrets prefer the OS keychain. Where no keychain is available (typical on WSL2/headless Linux), they fall back to `~/.config/azure-boards/credentials.json` created with `0600` permissions inside a `0700` directory. Secrets are **never** written to repo-local config.

## Device-code details

The flow runs against `login.microsoftonline.com/organizations` using the well-known Azure CLI public client ID (pre-consented for Azure DevOps in most tenants), requesting the Azure DevOps scope (`499b84ac-…/.default`) plus `offline_access`. If your tenant's Conditional Access policies block that client, override it with `AZURE_BOARDS_CLIENT_ID=<your-public-client-app-id>`.

## PAT scopes

| Feature | Required scope |
|---|---|
| Work items, comments, queries, iterations, boards | `vso.work_write` (read-only: `vso.work`) |
| `ab metrics` via Analytics | `vso.analytics` |
| `ab github detect` connection listing | `vso.githubconnections` |

403 errors name the likely missing scope.

## CI patterns

```yaml
# GitHub Actions
env:
  ADO_PAT: ${{ secrets.ADO_PAT }}
  ADO_ORG: nrgmr
  ADO_PROJECT: Yellow Hat

# Azure Pipelines
env:
  ADO_TOKEN: $(System.AccessToken)
```
