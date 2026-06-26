# Claude Code workflows

`ab` is built to be driven by Claude Code (and other agents). Suggested setup in a project's `CLAUDE.md`:

```markdown
## Azure Boards

Use `ab` (azure-boards CLI). Context is auto-detected from this repo's config;
auth comes from `ab auth login` / ADO_PAT. All commands support `--json` and
stable exit codes (see `ab <group> --help`).

- Look up a work item: `ab work-item show <id> --json`
- My open items: `ab wiql fetch "SELECT [System.Id] FROM WorkItems WHERE [System.AssignedTo] = @Me AND [System.State] <> 'Closed'" --json`
- Read the screenshots on a bug: `ab image download <id> --output-dir /tmp/wi-images --json` (then Read the files; each entry includes the surrounding text context)
- Comment with the fix: `ab comment add <id> --text "Fixed in <sha>"`
- Link the PR: `ab github link-pr <id> <pr-url>`
- Move to done: `ab work-item update <id> --state Closed --yes`
- Always preview destructive/bulk changes with --dry-run first.
```

## Recipes

**Triage a bug end to end**

```sh
ab work-item show 4231 --expand relations --json   # description + normalizedText + links
ab comment list 4231 --json                         # discussion history
ab image download 4231 --output-dir /tmp/wi4231     # screenshots with text context
ab relation tree 4231 --depth 2 --json              # parent/child context
```

**Ship a fix**

```sh
ab work-item update 4231 --state Active --yes
# ... do the work, open PR #88 ...
ab github link-pr 4231 https://github.com/owner/repo/pull/88
ab comment add 4231 --text @pr-summary.md
ab tag add 4231 needs-qa
ab work-item update 4231 --state Resolved --reason "Code complete" --yes
```

**Sprint planning**

```sh
ab sprint current --json
ab sprint backlog --json
ab backlog list --json                       # priority order
ab backlog prioritize --id 4231 --before 4100
ab sprint add-work-item 4231 --iteration @next
ab metrics velocity --iterations 6 --json    # calculation basis documented in output
ab backlog forecast --velocity 25 --json
```

**Bulk hygiene (agent loop)**

```sh
# find stale active items, then for each id:
ab wiql fetch "SELECT [System.Id] FROM WorkItems WHERE [System.State] = 'Active' AND [System.ChangedDate] < @Today - 30" --json
ab work-item update "$id" --field System.Tags="stale-check" --dry-run --json   # preview
ab tag add "$id" stale-check                                                   # apply (append-safe)
```

## Why this works well for agents

- JSON by default when piped; exit codes carry the failure class (no output parsing).
- `--dry-run` previews show the exact patch and resolved target before writes.
- Non-TTY runs never hang on prompts; destructive ops fail closed (exit 9) without `--yes`.
- Inline-image extraction turns work-item screenshots into local files Claude can actually read, each paired with the text that preceded it.
