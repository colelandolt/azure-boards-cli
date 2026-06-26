# Azure CLI compatibility

`ab` keeps `az boards` muscle memory working: the same global flags (`--org`, `--project`, `--detect`, `-o/--output`, `--query` JMESPath, `-y/--yes`, `--only-show-errors`, `--verbose`, `--debug`) and a superset of the GA command surface.

## Command mapping

| az boards | ab |
|---|---|
| `az boards work-item show --id 1 --fields a,b --expand relations` | `ab work-item show 1 --fields a,b --expand relations` |
| `az boards work-item create --type Bug --title T --fields k=v` | `ab work-item create --type Bug --title T --field k=v` |
| `az boards work-item update --id 1 --state Active --reason R` | `ab work-item update 1 --state Active --reason R` |
| `az boards work-item delete --id 1 [--destroy] --yes` | `ab work-item delete 1 --yes` / `--destroy --confirm-id 1` |
| `az boards work-item relation add --id 1 --relation-type parent --target-id 2` | `ab relation add 1 --type parent --target 2` |
| `az boards work-item relation show --id 1` | `ab relation list 1` |
| `az boards work-item relation list-type` | `ab relation list-type` |
| `az boards query --wiql "..."` | `ab wiql fetch "..."` |
| `az boards query --id <saved-query-id>` | `ab query run <id-or-path>` |
| `az boards area project list/create/...` | `ab area project list/create/...` |
| `az boards area team add/list/remove/update` | `ab area team add/list/remove/update` |
| `az boards iteration project list/create/...` | `ab iteration project list/create/...` |
| `az boards iteration team add/list/remove` | `ab iteration team add/list/remove` |
| `az boards iteration team list-work-items --id X` | `ab iteration team list-work-items <path-or-@current>` |
| `az boards iteration team set-backlog-iteration / set-default-iteration` | `ab iteration team set-backlog-iteration / set-default-iteration` |
| `az boards work-item show --open` | `ab work-item show 1 --web` / `ab work-item open 1` |

## Deliberate differences

- **`--field k=v` (repeatable)** instead of az's space-separated `--fields k=v k2=v2` — unambiguous with values containing spaces.
- **Positional IDs** (`ab work-item show 123`) instead of `--id 123`.
- **Permanent destroy** requires `--destroy --confirm-id <id>`; `--yes` alone never escalates destructiveness. There is no ambiguous `--force`.
- **TTY-aware default output**: tables on a terminal, JSON when piped (az always defaults to JSON). Force with `-o json` or `--json`.
- Team-scoped iteration commands accept **paths and `@current`/`@next`/`@previous` macros** instead of raw iteration GUIDs.

## Beyond az boards

Everything az can't do: read comments (`ab comment list`), attachments up/download, inline-image extraction (`ab image`), safe tag append (`ab tag add`), GitHub artifact links (`ab github link-pr/commit/branch/issue`), saved-query CRUD, board column queries, backlog reordering (`ab backlog prioritize`), sprint capacity/burndown, and flow metrics (`ab metrics`).
