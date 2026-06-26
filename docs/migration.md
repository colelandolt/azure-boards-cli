# Migration from the `ado` bash script

The legacy `ado` script (repo root) is superseded by `ab`. Command mapping:

| ado (bash) | ab |
|---|---|
| `ado get 123 [fields_csv]` | `ab work-item show 123 [--fields a,b]` |
| `ado wiql "..."` | `ab wiql run "..."` |
| `ado wiql-fetch "..." [fields]` | `ab wiql fetch "..." [--fields a,b]` |
| `ado relations 123` | `ab relation list 123` |
| `ado attachments 123` | `ab attachment list 123` |
| `ado comments 123` | `ab comment list 123` |
| `ado comment 123 "text"` | `ab comment add 123 --text "text"` |
| `ado inline-images 123` | `ab image download 123 --output-dir ./ado-images` |
| `ado link-pr 123 <pr-url>` | `ab github link-pr 123 <pr-url>` |
| `ado update 123 System.State Active` | `ab work-item update 123 --state Active` (or `--field System.State=Active`) |
| `ado create "User Story" "Title" [desc] [ac] [parent]` | `ab work-item create --type "User Story" --title "Title" [--description d] [--acceptance-criteria ac] [--parent id]` |
| `ado delete 123 [--destroy]` | `ab work-item delete 123 --yes` / `--destroy --confirm-id 123` |
| `ado tags add 123 tag` | `ab tag add 123 tag` |
| `ado iterations list [n]` | `ab iteration project list` (or `ab sprint list` for team sprints) |
| `ado iterations check` | `ab iteration project list --under 2026` |
| `ado iterations init [--force]` | `ab iteration import --file examples/sprints-2026.yaml [--dry-run]` |

## Environment variables

| Old | New |
|---|---|
| `ADO_ORG` | `ADO_ORG` (unchanged; URLs and bare names both accepted) |
| `ADO_PROJECT` | `ADO_PROJECT` (unchanged) |
| `ADO_PAT` | `ADO_PAT` (unchanged) — but prefer `ab auth login` (Entra) interactively |
| `ADO_GITHUB_CONNECTION_ID` | still honored; better: let `ab github link-pr` discover and cache the GUID, or set `[github.connections]` in user config |
| `ADO_API_VERSION` | dropped — pinned to 7.1 internally (comments use their preview version automatically) |

## Behavioral upgrades to be aware of

- **HTTP failures actually fail.** The script exited 0 on 401/404/etc.; `ab` maps every failure to a stable non-zero exit code with a JSON error object. Remove any output-sniffing error handling.
- **Output shapes are stable**: lists are `{"count", "value"}` (the script's `wiql-fetch` shape), mutations carry the `{operation, target, result}` envelope.
- **Tag add / relation remove are race-safe** (revision tests).
- **Inline image downloads never send credentials to non-ADO hosts** (the script sent the PAT header to any `<img src>` URL).
- **`iterations init` is generalized**: the hardcoded 2026 schedule now lives in `examples/sprints-2026.yaml`; `ab iteration import` works for any year/naming via the same file format, with `--dry-run` diffing against the live tree (the conflict check replaces `--force`).
- **Sprint names/dates are never hardcoded**; `@current`/`@next`/`@previous` resolve from team settings.
- **Long text comes from files, not argv.** Any free-text value — `--description`,
  `--acceptance-criteria`, `--text`, `--wiql`, `patch --value`, and any `-f Ref=...` value —
  accepts `@file` or `@-` (stdin), so multi-line markdown never has to be shell-escaped. For
  multi-field creates, `ab work-item create --from-json @story.json` builds the whole item
  (fields + parent + relations) from one JSON document.
