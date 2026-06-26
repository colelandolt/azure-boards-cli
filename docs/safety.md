# Destructive-operation safety

Safe by default; `--yes` means "skip confirmation", never "make the operation more destructive". There is no `--force`.

## Tiers

| Tier | Examples | Gate |
|---|---|---|
| Mutating | update, create, tag add, sprint move | none (reversible via another update) |
| Destructive (recoverable) | `work-item delete` (recycle bin), `relation remove`, `attachment remove`, `comment delete`, `area/iteration delete`, `query delete` | TTY confirmation, or `--yes` |
| Permanent | `work-item delete --destroy` | `--destroy` **and** `--confirm-id <id>` matching the target, **plus** confirmation/`--yes` |

## Work item deletion

```sh
ab work-item delete 123 --yes                      # recycle bin (recoverable)
ab work-item restore 123                           # bring it back
ab work-item delete 123 --destroy --confirm-id 123 --yes   # permanent, cannot be undone
```

A missing or mismatched `--confirm-id` is exit 9 even with `--yes`. Permanent destroy is single-item only — it cannot be batched.

## Non-interactive behavior

Without a TTY, any operation that would prompt fails closed with exit 9 (never blocks, never proceeds). Add `--yes` deliberately.

## Dry runs

All destructive and bulk commands accept `--dry-run`: the exact planned request and resolved target are printed, nothing is sent, exit 0. Reads needed to *plan* (e.g. the read-modify-write in `tag add`) are allowed; writes are not.

## Race safety

`relation remove` and `attachment remove` resolve the target index from a fresh read and pair the removal with a JSON-patch revision test — if anything else modified the work item in between, the operation fails with exit 6 instead of removing the wrong relation. `work-item update --expected-rev <n>` offers the same guard for field updates.
