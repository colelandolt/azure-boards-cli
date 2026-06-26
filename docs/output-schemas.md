# Output schemas

## Format selection

`-o/--output` flag > `--json` (alias for `-o json`) > repo config `default-output` > user config `output` > TTY sniffing (terminal → `table`, piped → `json`).

Formats: `json` `jsonc` (colored when TTY) `table` `tsv` `yaml` `yamlc` `none`.

`--query <jmespath>` is applied to the canonical JSON value **before** rendering, in every format. After a `--query` reshapes the data, `table`/`tsv` degrade like az: scalars print raw, scalar arrays one per line, everything else pretty JSON. Color is suppressed when piped or when `NO_COLOR` is set.

## List wrapper

Every list-shaped result:

```json
{
  "count": 2,
  "value": [ { "id": 1, "fields": { "...": "..." } }, { "id": 2 } ]
}
```

## Mutation envelope

Every mutating command (create/update/patch/delete/restore, tag/relation/attachment writes, iteration import, backlog prioritize, github link-*):

```json
{
  "operation": "work-item.update",
  "target": {
    "organization": "nrgmr",
    "project": "Yellow Hat",
    "team": "Yellow Hat Team",
    "workItemId": 123
  },
  "dryRun": false,
  "result": { "...": "API response, or the planned request under --dry-run" },
  "id": 123,
  "rev": 5
}
```

`target.team` and `target.workItemId` appear only when relevant. The affected entity's
`id` and `rev` are hoisted to the top level (from `result`, falling back to the resolved
work item id) so `--query id` / `--query rev` behave the same as on reads, which nest those
under the item; both are omitted when neither a result body nor a work item id is available.

## Error object (stderr, JSON modes)

```json
{"code": "conflict", "message": "conflict: the test operation for path /rev failed", "exitCode": 6, "details": {"optional": "..."} }
```

`code` is one of: `general` `usage` `auth` `forbidden` `notFound` `conflict` `validation` `serviceUnavailable` `unsafeBlocked` `partialSuccess`.

## Work items

Raw API shape passes through (`id`, `rev`, `fields`, `relations`, `url`). Rich-text fields additionally get normalized plain text:

```json
{
  "id": 123,
  "fields": { "System.Description": "<div>html...</div>" },
  "normalizedText": { "description": "plain text...", "acceptanceCriteria": "..." }
}
```

## Inline images (`ab image list/download`)

```json
{"count": 2, "value": [
  {"index": 1, "file": "./ado-images/wi123_img1.png", "url": "https://dev.azure.com/...", "context": "the 300 chars of text preceding the image"}
]}
```

## Metrics

Every `ab metrics` result documents its own basis:

```json
{
  "calculation": "human-readable formula actually used",
  "dateBasis": "Analytics CompletedDate | ClosedDate fallback | ASOF queries",
  "fieldsUsed": {"effort": "Microsoft.VSTS.Scheduling.StoryPoints"},
  "stateCategories": {"User Story": {"Active": "InProgress", "...": "..."}},
  "source": "analytics | workItemApi"
}
```
