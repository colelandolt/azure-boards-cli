# Agent-safe usage

`ab` is designed to be driven by AI agents and scripts without supervision.

## Guarantees

1. **Never hangs.** Non-TTY invocations never prompt. Anything that would need confirmation fails immediately with exit 9 and a message saying to add `--yes`. Unresolvable context fails with exit 7.
2. **stdout is data, stderr is everything else.** Progress, warnings, prompts, logs, and errors all go to stderr. `--json` guarantees valid JSON on stdout (or nothing, on error).
3. **Stable exit codes:**

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | general failure / user cancelled a prompt |
| 2 | usage or argument error |
| 3 | authentication failure |
| 4 | authorization (forbidden / missing PAT scope) |
| 5 | not found |
| 6 | conflict / optimistic-concurrency failure (`--expected-rev`, racing relation removal) |
| 7 | validation failed (bad input, bad config, unresolvable context) |
| 8 | network / service unavailable (incl. rate limits after retries) |
| 9 | unsafe operation blocked (confirmation needed; `--confirm-id` missing/mismatched) |
| 10 | partial success (bulk operation with mixed results) |

4. **Stable error objects.** In JSON output modes, errors are a single JSON line on stderr:

```json
{"code": "notFound", "message": "not found: TF401232: ...", "exitCode": 5}
```

5. **`--dry-run` everywhere it matters.** Mutating commands print the exact patch/request plus the resolved target and exit 0 without any write requests.
6. **No emojis in machine output.** Ever.

## Recommended agent invocation pattern

```sh
export ADO_PAT=...           # or ADO_TOKEN
export ADO_ORG=nrgmr ADO_PROJECT="Yellow Hat" ADO_TEAM="Yellow Hat Team"

ab work-item show 123 --json
ab work-item update 123 --state Active --dry-run --json   # preview
ab work-item update 123 --state Active --yes --json        # apply
```

Check the exit code first; on non-zero, parse the last stderr line as JSON.

## Optimistic concurrency

When multiple writers may touch the same work item, pass `--expected-rev <rev>` on `work-item update`. If the item changed since you read it, the call fails with exit 6 instead of clobbering. `relation remove` and `attachment remove` apply a revision test automatically.

## Retries

GETs are retried automatically on 408/429/5xx (writes only on 429), honoring `Retry-After`, with exponential backoff capped at 30s. After retries are exhausted you get exit 8.
