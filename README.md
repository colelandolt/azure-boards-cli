# azure-boards-cli

Rust-native Azure Boards CLI for tracking work items, planning sprints, prioritizing backlogs, and automating developer workflows. Installed as `azure-boards` with an `ab` alias.

Built agent-first: stable JSON output, stable exit codes, never hangs waiting for input, every destructive operation is safe by default. Covers the full `az boards` GA surface plus everything the legacy `ado` bash script did (comments, attachments, inline-image extraction, GitHub PR links, safe tag append) — and more (saved queries, boards, backlog reordering, flow metrics).

## Install

```sh
# Prebuilt binaries (Linux, macOS, Windows) from GitHub Releases, or:
cargo install --git https://github.com/colelandolt/azure-boards-cli
```

## Quick start

```sh
ab auth login                       # Microsoft Entra device-code sign-in (or: az login just works)
ab configure --defaults organization=nrgmr project="Yellow Hat" team="Yellow Hat Team"
ab context show                     # what org/project/team resolved, and from where

ab work-item show 123
ab work-item create --type "User Story" --title "New thing" --parent 100
ab work-item update 123 --state Active --reason "Work started"
ab wiql fetch "SELECT [System.Id] FROM WorkItems WHERE [System.AssignedTo] = @Me AND [System.State] <> 'Closed'"

ab sprint current
ab sprint backlog
ab backlog prioritize --id 123 --before 456
ab metrics velocity --iterations 6

ab comment add 123 --text "Fixed in abc123"
ab tag add 123 manual-step
ab image download 123 --output-dir ./screenshots   # inline screenshots with text context
ab github link-pr 123 https://github.com/owner/repo/pull/45
```

Auth for agents/CI: set `ADO_PAT` (or `ADO_TOKEN` for an Entra bearer token) — no login command needed. `ADO_ORG` / `ADO_PROJECT` / `ADO_TEAM` set context. Org/project are also auto-detected from the git remote.

## Command groups

`auth` · `configure` · `context` · `work-item` · `wiql` · `query` (saved queries) · `comment` · `relation` · `attachment` · `image` · `tag` · `area` · `iteration` (incl. `import` from a YAML plan) · `sprint` · `board` · `backlog` · `metrics` · `github` · `completion`

Run `ab <group> --help` for subcommands. Familiar `az` conventions are supported: `--org`, `--project`, `--detect`, `-o/--output json|jsonc|table|tsv|yaml|yamlc|none`, `--query` (JMESPath), `-y/--yes`, `--only-show-errors`, `--verbose`, `--debug`.

## The agent contract

- Human-readable tables on a TTY; JSON when piped. `--json` always emits valid JSON on stdout; everything else (progress, prompts, errors) goes to stderr.
- Lists are `{"count": n, "value": [...]}`. Mutations are `{"operation", "target", "dryRun", "result"}`.
- Errors in JSON mode are a stable object on stderr: `{"code", "message", "exitCode"}`.
- Stable exit codes: 0 ok · 1 general · 2 usage · 3 auth · 4 forbidden · 5 not found · 6 conflict · 7 validation · 8 service unavailable · 9 unsafe blocked · 10 partial success.
- Non-TTY never prompts. Destructive operations need `--yes`; permanent destroy needs `--destroy --confirm-id <id>` on top. Bulk/destructive commands support `--dry-run`.

## Documentation

- [Install](docs/install.md)
- [Authentication](docs/auth.md)
- [Config & context resolution](docs/context.md)
- [Azure CLI compatibility](docs/az-compat.md)
- [Agent-safe usage](docs/agent-safe.md)
- [Output schemas](docs/output-schemas.md)
- [Destructive-operation safety](docs/safety.md)
- [Migration from the bash script](docs/migration.md)
- [Claude Code workflows](docs/claude-code.md)

## Legacy

The `ado` bash script in the repo root is the legacy predecessor, kept as a behavioral reference. See [docs/migration.md](docs/migration.md) for the command mapping.

## License

MIT
