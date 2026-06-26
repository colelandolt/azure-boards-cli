#!/usr/bin/env bash
set -euo pipefail

: "${ADO_ORG:?Set ADO_ORG (e.g. nrgmr)}"
: "${ADO_PROJECT:?Set ADO_PROJECT}"
: "${ADO_PAT:?Set ADO_PAT}"

# Normalize ADO_ORG: accept either "nrgmr" or "https://dev.azure.com/nrgmr"
if [[ "$ADO_ORG" != http* ]]; then
  ADO_ORG="https://dev.azure.com/${ADO_ORG}"
fi

API_VERSION="${ADO_API_VERSION:-7.1}"

# URL-encode ADO_PROJECT once so project names with spaces (e.g. "Yellow Hat")
# don't produce malformed URLs. All URL constructions use ENCODED_PROJECT.
# ADO_PROJECT is kept as-is for display and Python-side use.
ENCODED_PROJECT=""  # populated after PYTHON is resolved (see below)

if python3 -c "" 2>/dev/null; then
  PYTHON=python3
elif py -c "" 2>/dev/null; then
  PYTHON=py
elif python -c "" 2>/dev/null; then
  PYTHON=python
else
  echo "Error: no python interpreter found (tried python3, py, python)" >&2
  exit 1
fi

ENCODED_PROJECT=$($PYTHON -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$ADO_PROJECT")

auth_header() {
  printf ":%s" "$ADO_PAT" | base64 | tr -d '\n'
}

# ---------------------------------------------------------------------------
# READ OPERATIONS
# ---------------------------------------------------------------------------

get_work_item() {
  local id="$1"
  local fields="${2:-System.Id,System.Title,System.State,System.AssignedTo,System.WorkItemType,System.AreaPath,System.IterationPath,System.Tags,System.CreatedDate,System.ChangedDate,System.Description,Microsoft.VSTS.Common.AcceptanceCriteria}"

  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}?api-version=${API_VERSION}&fields=${fields}"
}

wiql_query() {
  local wiql="$1"
  curl -sS \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    -d "$($PYTHON -c "import json,sys; print(json.dumps({'query': sys.argv[1]}))" "$wiql")" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/wiql?api-version=${API_VERSION}"
}

# Batch fetch by IDs (comma-separated)
get_work_items_batch() {
  local ids_csv="$1"
  local fields="${2:-System.Id,System.Title,System.State,System.AssignedTo,System.WorkItemType,System.IterationPath,System.Tags,System.ChangedDate}"

  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems?api-version=${API_VERSION}&ids=${ids_csv}&fields=${fields}"
}

get_inline_images() {
  local id="$1"
  local workitem
  workitem=$(get_work_item "$id")

  local images_json
  images_json=$(echo "$workitem" | $PYTHON -c "
import sys, json, re

def strip_tags(html):
    return re.sub(r'<[^>]+>', ' ', html).strip()

def collapse_space(s):
    return re.sub(r'\s+', ' ', s).strip()

data = json.load(sys.stdin)
fields = data.get('fields', {})
html = (fields.get('System.Description') or '') + (fields.get('Microsoft.VSTS.Common.AcceptanceCriteria') or '')

results = []
parts = re.split(r'(<img[^>]+>)', html)
preceding_text = ''
for part in parts:
    if re.match(r'<img', part):
        m = re.search(r'src=\"([^\"]+)\"', part)
        if m:
            context = collapse_space(strip_tags(preceding_text))[-300:]
            results.append({'url': m.group(1), 'context': context})
        preceding_text = ''
    else:
        preceding_text += part

print(json.dumps(results))
")

  local results="["
  local first=true
  local i=1
  while IFS= read -r entry; do
    [[ -z "$entry" ]] && continue
    local url context tmpfile winpath
    url=$(echo "$entry" | $PYTHON -c "import sys,json; print(json.loads(sys.stdin.read())['url'])")
    context=$(echo "$entry" | $PYTHON -c "import sys,json; print(json.loads(sys.stdin.read())['context'])")
    tmpfile=$(mktemp /tmp/ado_img_XXXXXX.png)
    curl -sS \
      -H "Authorization: Basic $(auth_header)" \
      "$url" -o "$tmpfile"
    winpath=$(cygpath -w "$tmpfile" 2>/dev/null || echo "$tmpfile")
    context_json=$($PYTHON -c "import json,sys; print(json.dumps(sys.argv[1]))" "$context")
    if [[ "$first" == "true" ]]; then
      first=false
    else
      results+=","
    fi
    results+="{\"index\":$i,\"file\":\"$winpath\",\"context\":$context_json}"
    ((i++))
  done < <(echo "$images_json" | $PYTHON -c "
import sys, json
for item in json.load(sys.stdin):
    print(json.dumps(item))
")
  results+="]"
  echo "$results"
}

get_relations() {
  local id="$1"
  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}?%24expand=relations&api-version=${API_VERSION}" \
  | $PYTHON -c "
import sys, json, re

def rel_type(rel, url, name):
    if rel == 'System.LinkTypes.Hierarchy-Reverse':
        return 'Parent'
    if rel == 'System.LinkTypes.Hierarchy-Forward':
        return 'Child'
    if rel == 'System.LinkTypes.Related':
        return 'Related'
    if rel == 'ArtifactLink':
        if '/GitHub/PullRequest/' in url:
            return 'GitHub PR'
        if '/GitHub/Commit/' in url:
            return 'GitHub Commit'
        if '/GitHub/Branch/' in url:
            return 'GitHub Branch'
        return 'Artifact'
    return name or rel

data = json.load(sys.stdin)
relations = data.get('relations') or []
result = []
for r in relations:
    rel = r.get('rel', '')
    url = r.get('url', '')
    attrs = r.get('attributes', {})
    name = attrs.get('name', '')
    rtype = rel_type(rel, url, name)
    # Extract work item ID from ADO work item URLs
    wi_match = re.search(r'/workItems/(\d+)$', url)
    result.append({
        'type': rtype,
        'url': url,
        'workItemId': wi_match.group(1) if wi_match else None,
        'name': name,
    })
print(json.dumps({'count': len(result), 'relations': result}))
"
}

get_attachments() {
  local id="$1"
  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}?%24expand=relations&api-version=${API_VERSION}" \
  | $PYTHON -c "
import sys, json
data = json.load(sys.stdin)
relations = data.get('relations') or []
attachments = [r for r in relations if r.get('rel') == 'AttachedFile']
if not attachments:
    print(json.dumps({'count': 0, 'attachments': []}))
else:
    result = []
    for a in attachments:
        attrs = a.get('attributes', {})
        result.append({
            'name': attrs.get('name', ''),
            'size': attrs.get('resourceSize', 0),
            'url': a.get('url', ''),
            'comment': attrs.get('comment', '')
        })
    print(json.dumps({'count': len(result), 'attachments': result}))
"
}

# link-pr <id> <github-pr-url>
# e.g. link-pr 123 https://github.com/nrgmr/cinesys/pull/45
link_pr() {
  local id="$1"
  local pr_url="$2"

  # Parse pr number from URL: https://github.com/{owner}/{repo}/pull/{number}
  local pr
  pr=$($PYTHON -c "
import sys, re
url = sys.argv[1]
m = re.match(r'https://github\.com/[^/]+/[^/]+/pull/(\d+)', url)
if not m:
    print('ERROR: URL must be https://github.com/{owner}/{repo}/pull/{number}', file=sys.stderr)
    sys.exit(1)
print(m.group(1))
" "$pr_url") || exit 1

  # ADO GitHub connection ID — repo-specific, set via ADO_GITHUB_CONNECTION_ID env var.
  # Find it by inspecting existing artifact links on any work item linked to the repo.
  # It does not appear in the githubconnections API.
  : "${ADO_GITHUB_CONNECTION_ID:?Set ADO_GITHUB_CONNECTION_ID for this repo (see project CLAUDE.md)}"
  local connection_id="$ADO_GITHUB_CONNECTION_ID"

  local artifact_url="vstfs:///GitHub/PullRequest/${connection_id}%2F${pr}"

  curl -sS \
    -X PATCH \
    -H "Accept: application/json" \
    -H "Content-Type: application/json-patch+json" \
    -H "Authorization: Basic $(auth_header)" \
    -d "[{\"op\":\"add\",\"path\":\"/relations/-\",\"value\":{\"rel\":\"ArtifactLink\",\"url\":\"${artifact_url}\",\"attributes\":{\"name\":\"GitHub Pull Request\"}}}]" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}?api-version=${API_VERSION}"
}

get_comments() {
  local id="$1"
  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workItems/${id}/comments?api-version=${API_VERSION}-preview.3"
}

# ---------------------------------------------------------------------------
# ITERATION OPERATIONS
# ---------------------------------------------------------------------------

# iterations check
# Fetches root-level iteration children and prints JSON array of names that
# start with "2026". Exit 0 always; caller inspects the array.
iterations_check() {
  local encoded_project
  encoded_project=$($PYTHON -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$ADO_PROJECT")

  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${encoded_project}/_apis/wit/classificationnodes/iterations?%24depth=1&api-version=${API_VERSION}" \
  | $PYTHON -c "
import json, sys
data = json.load(sys.stdin)
children = data.get('children') or []
found = [c['name'] for c in children if c['name'].startswith('2026')]
print(json.dumps(found))
"
}

# iterations list [n]
# Returns JSON array of up to n+1 sprint nodes (current + next n, default 3 → 4 total).
# Filters to "2026 *" quarter nodes when they exist; falls back to all nodes otherwise.
# Each object: {name, path, startDate, finishDate, current, ends_soon}
# "path" is already normalized to System.IterationPath format: e.g. Personas\2026 Q2\Killing Eve
iterations_list() {
  local n="${1:-3}"
  curl -sS \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/classificationnodes/iterations?api-version=${API_VERSION}&\$depth=10" \
  | $PYTHON -c "
import json, sys
from datetime import date

def to_date(s):
    try: return date.fromisoformat(s[:10]) if s else None
    except: return None

def normalize(raw):
    p = raw.lstrip('\\\\')
    parts = p.split('\\\\')
    if len(parts) > 1 and parts[1].lower() == 'iteration':
        parts = [parts[0]] + parts[2:]
    return '\\\\'.join(parts)

def walk(node, out):
    ch = node.get('children') or []
    if not ch:
        a = node.get('attributes') or {}
        out.append({
            'name': node['name'],
            'path': normalize(node.get('path', '')),
            'startDate': (a.get('startDate') or '')[:10],
            'finishDate': (a.get('finishDate') or '')[:10],
        })
    for c in ch:
        walk(c, out)

n = int(sys.argv[1])
data = json.load(sys.stdin)
today = date.today()

top = data.get('children') or []
yr2026 = [c for c in top if c.get('name', '').startswith('2026')]
roots = yr2026 if yr2026 else top

leaves = []
for r in roots:
    walk(r, leaves)

valid = sorted(
    [l for l in leaves if (to_date(l['finishDate']) or date.min) >= today],
    key=lambda x: x['startDate']
)
for l in valid:
    sd, fd = to_date(l['startDate']), to_date(l['finishDate'])
    l['current'] = bool(sd and fd and sd <= today <= fd)
    l['ends_soon'] = bool(l['current'] and fd and (fd - today).days <= 2)
print(json.dumps(valid[:n + 1], indent=2))
" "$n"
}

# iterations init [--force]
# Creates the standard 2026 Q1-Q4 iteration tree (quarters + named TV-show sprints).
# Without --force: aborts with exit 1 and prints a JSON conflict object if any
#   2026 iterations already exist.
# With --force: skips the conflict check and creates all nodes unconditionally
#   (duplicates will return an API error per node but won't stop the run).
iterations_init() {
  local force="${1:-}"
  local encoded_project
  encoded_project=$($PYTHON -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$ADO_PROJECT")
  local base_url="${ADO_ORG}/${encoded_project}/_apis/wit/classificationnodes/iterations"

  # --- conflict check ---
  if [[ "$force" != "--force" ]]; then
    local existing
    existing=$(iterations_check)
    local count
    count=$(echo "$existing" | $PYTHON -c "import json,sys; print(len(json.load(sys.stdin)))")
    if [[ "$count" -gt 0 ]]; then
      echo "{\"conflict\":true,\"existing\":${existing}}"
      exit 1
    fi
  fi

  # --- helper ---
  _create_node() {
    local parent_path="$1"   # empty = root, else URL-encoded parent name
    local name="$2"
    local start="$3"
    local finish="$4"
    local url
    if [[ -z "$parent_path" ]]; then
      url="${base_url}?api-version=${API_VERSION}"
    else
      url="${base_url}/${parent_path}?api-version=${API_VERSION}"
    fi
    local body
    body=$($PYTHON -c "
import json, sys
name, start, finish = sys.argv[1], sys.argv[2], sys.argv[3]
obj = {'name': name}
if start:
    obj['attributes'] = {'startDate': start + 'T00:00:00Z', 'finishDate': finish + 'T00:00:00Z'}
print(json.dumps(obj))
" "$name" "$start" "$finish")
    local result
    result=$(curl -sS -X POST \
      -H "Accept: application/json" \
      -H "Content-Type: application/json" \
      -H "Authorization: Basic $(auth_header)" \
      -d "$body" "$url")
    local id err
    id=$(echo "$result" | grep -o '"id":[0-9]*' | head -1 | grep -o '[0-9]*')
    err=$(echo "$result" | $PYTHON -c "import json,sys; d=json.load(sys.stdin); print(d.get('message',''))" 2>/dev/null || true)
    if [[ -n "$id" ]]; then
      echo "  ok  [$id] $name"
    else
      echo "  err $name — $err"
    fi
  }

  # --- quarters ---
  echo '{"phase":"quarters","items":['
  _create_node "" "2026 Q1" "2026-01-01" "2026-03-31"
  _create_node "" "2026 Q2" "2026-04-01" "2026-06-30"
  _create_node "" "2026 Q3" "2026-07-01" "2026-09-30"
  _create_node "" "2026 Q4" "2026-10-01" "2026-12-31"
  echo ']}'

  # --- Q1 sprints ---
  echo '{"phase":"2026 Q1 sprints","items":['
  _create_node "2026%20Q1" "Arrested Development" "2025-12-29" "2026-01-09"
  _create_node "2026%20Q1" "Breaking Bad"         "2026-01-12" "2026-01-23"
  _create_node "2026%20Q1" "Curb Your Enthusiasm" "2026-01-26" "2026-02-06"
  _create_node "2026%20Q1" "Deadwood"             "2026-02-09" "2026-02-20"
  _create_node "2026%20Q1" "Extras"               "2026-02-23" "2026-03-06"
  _create_node "2026%20Q1" "Fargo"                "2026-03-09" "2026-03-20"
  _create_node "2026%20Q1" "Game of Thrones"      "2026-03-23" "2026-04-03"
  echo ']}'

  # --- Q2 sprints ---
  echo '{"phase":"2026 Q2 sprints","items":['
  _create_node "2026%20Q2" "House"       "2026-04-06" "2026-04-17"
  _create_node "2026%20Q2" "Invincible"  "2026-04-20" "2026-05-01"
  _create_node "2026%20Q2" "Justified"   "2026-05-04" "2026-05-15"
  _create_node "2026%20Q2" "Killing Eve" "2026-05-18" "2026-05-29"
  _create_node "2026%20Q2" "Lost"        "2026-06-01" "2026-06-12"
  _create_node "2026%20Q2" "Mad Men"     "2026-06-15" "2026-06-26"
  echo ']}'

  # --- Q3 sprints ---
  echo '{"phase":"2026 Q3 sprints","items":['
  _create_node "2026%20Q3" "Narcos"         "2026-06-29" "2026-07-10"
  _create_node "2026%20Q3" "Ozark"          "2026-07-13" "2026-07-24"
  _create_node "2026%20Q3" "Peaky Blinders" "2026-07-27" "2026-08-07"
  _create_node "2026%20Q3" "Queen's Gambit" "2026-08-10" "2026-08-21"
  _create_node "2026%20Q3" "Rings of Power" "2026-08-24" "2026-09-04"
  _create_node "2026%20Q3" "Sopranos"       "2026-09-07" "2026-09-18"
  _create_node "2026%20Q3" "Ted Lasso"      "2026-09-21" "2026-10-02"
  echo ']}'

  # --- Q4 sprints ---
  echo '{"phase":"2026 Q4 sprints","items":['
  _create_node "2026%20Q4" "Undone"       "2026-10-05" "2026-10-16"
  _create_node "2026%20Q4" "Veep"         "2026-10-19" "2026-10-30"
  _create_node "2026%20Q4" "Westworld"    "2026-11-02" "2026-11-13"
  _create_node "2026%20Q4" "X-Files"      "2026-11-16" "2026-11-27"
  _create_node "2026%20Q4" "Yellowstone"  "2026-11-30" "2026-12-11"
  _create_node "2026%20Q4" "ZeroZeroZero" "2026-12-14" "2026-12-25"
  echo ']}'

  echo '{"status":"done"}'
}

# ---------------------------------------------------------------------------
# WRITE OPERATIONS
# ---------------------------------------------------------------------------

# update <id> <field_ref_name> <value>
# Examples:
#   ado update 123 System.State Active
#   ado update 123 System.AssignedTo "John Paez <john@example.com>"
update_field() {
  local id="$1"
  local field="$2"
  local value="$3"

  curl -sS \
    -X PATCH \
    -H "Accept: application/json" \
    -H "Content-Type: application/json-patch+json" \
    -H "Authorization: Basic $(auth_header)" \
    -d "[{\"op\":\"add\",\"path\":\"/fields/${field}\",\"value\":$($PYTHON -c "import json,sys; print(json.dumps(sys.argv[1]))" "$value")}]" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}?api-version=${API_VERSION}"
}

# tags add <id> <tag>
# Safely appends a tag to a work item without overwriting existing tags.
# Reads current System.Tags, appends new_tag if absent (case-insensitive), writes back.
tags_add() {
  local id="$1"
  local new_tag="$2"

  local current_tags
  current_tags=$(get_work_item "$id" "System.Tags" | $PYTHON -c "
import json, sys
data = json.load(sys.stdin)
print(data.get('fields', {}).get('System.Tags') or '')
")

  local updated_tags
  updated_tags=$($PYTHON -c "
import sys
current = sys.argv[1].strip()
tag = sys.argv[2].strip()
if current:
    parts = [t.strip() for t in current.replace(',', ';').split(';') if t.strip()]
    if tag.lower() not in [p.lower() for p in parts]:
        parts.append(tag)
    print('; '.join(parts))
else:
    print(tag)
" "$current_tags" "$new_tag")

  update_field "$id" "System.Tags" "$updated_tags"
}

# create <type> <title> [description] [acceptance_criteria] [parent_id]
# type examples: "User Story", "Task", "Bug", "Feature"
create_work_item() {
  local type="$1"
  local title="$2"
  local description="${3:-}"
  local acceptance_criteria="${4:-}"
  local parent_id="${5:-}"

  local encoded_type
  encoded_type=$($PYTHON -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$type")

  local patch
  patch=$($PYTHON -c "
import json, sys, urllib.parse
title = sys.argv[1]
description = sys.argv[2]
ac = sys.argv[3]
parent_id = sys.argv[4]
org = sys.argv[5]
project = sys.argv[6]
encoded_project = urllib.parse.quote(project, safe='')
parent_url_base = org + '/' + encoded_project + '/_apis/wit/workitems/'

ops = [
  {'op': 'add', 'path': '/fields/System.Title', 'value': title},
]
if description:
  ops.append({'op': 'add', 'path': '/fields/System.Description', 'value': description})
if ac:
  ops.append({'op': 'add', 'path': '/fields/Microsoft.VSTS.Common.AcceptanceCriteria', 'value': ac})
if parent_id:
  ops.append({
    'op': 'add',
    'path': '/relations/-',
    'value': {
      'rel': 'System.LinkTypes.Hierarchy-Reverse',
      'url': parent_url_base + parent_id,
      'attributes': {'comment': ''}
    }
  })
print(json.dumps(ops))
" "$title" "$description" "$acceptance_criteria" "$parent_id" "$ADO_ORG" "$ADO_PROJECT")

  curl -sS \
    -X POST \
    -H "Accept: application/json" \
    -H "Content-Type: application/json-patch+json" \
    -H "Authorization: Basic $(auth_header)" \
    -d "$patch" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/\$${encoded_type}?api-version=${API_VERSION}"
}

# delete <id> [--destroy]
# Default: soft-delete (moves to recycle bin, recoverable).
# Pass --destroy for permanent deletion (cannot be undone).
delete_work_item() {
  local id="$1"
  local destroy="${2:-}"
  local encoded_project
  encoded_project=$($PYTHON -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1], safe=''))" "$ADO_PROJECT")

  local url="${ADO_ORG}/${encoded_project}/_apis/wit/workitems/${id}?api-version=${API_VERSION}"
  [[ "$destroy" == "--destroy" ]] && url="${url}&destroy=true"

  curl -sS \
    -X DELETE \
    -H "Accept: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    "$url"
}

# comment <id> <text>
add_comment() {
  local id="$1"
  local text="$2"

  curl -sS \
    -X POST \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    -H "Authorization: Basic $(auth_header)" \
    -d "{\"text\":$($PYTHON -c "import json,sys; print(json.dumps(sys.argv[1]))" "$text")}" \
    "${ADO_ORG}/${ENCODED_PROJECT}/_apis/wit/workitems/${id}/comments?api-version=${API_VERSION}-preview.3"
}

# ---------------------------------------------------------------------------
# USAGE
# ---------------------------------------------------------------------------

usage() {
  cat <<'EOF'
Requires: Python 3 (python3, py, or python on PATH)

Usage:
  ado get <id> [fields_csv]
  ado relations <id>
  ado wiql "<WIQL query>"
  ado wiql-fetch "<WIQL query>" [fields_csv]
  ado attachments <id>
  ado update <id> <field_ref_name> <value>
  ado comment <id> <text>

Common field reference names for update:
  System.State                     (e.g. Active, Resolved, Closed)
  System.AssignedTo                (display name or email)
  System.Title
  System.Tags
  System.IterationPath
  Microsoft.VSTS.Common.Priority   (1-4)

Examples:
  ado get 123
  ado wiql "SELECT [System.Id] FROM WorkItems WHERE [System.AssignedTo] = @Me AND [System.State] <> 'Closed'"
  ado wiql-fetch "SELECT [System.Id] FROM WorkItems WHERE [System.IterationPath] = @CurrentIteration('[MyProject]')"
  ado attachments 123
  ado update 123 System.State Active
  ado update 123 System.AssignedTo "John Paez"
  ado comment 123 "Addressed in commit abc123"
  ado iterations list 3
  ado tags add 123 manual-step
EOF
}

# ---------------------------------------------------------------------------
# DISPATCH
# ---------------------------------------------------------------------------

cmd="${1:-}"
case "$cmd" in
  get)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    get_work_item "$2" "${3:-}"
    ;;
  wiql)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    wiql_query "$2"
    ;;
  wiql-fetch)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    wiql="$2"
    fields="${3:-}"
    ids=$(wiql_query "$wiql" | $PYTHON -c "
import sys, json
data = json.load(sys.stdin)
items = data.get('workItems') or []
print(','.join(str(x['id']) for x in items))
")
    if [[ -z "$ids" ]]; then
      echo '{"count":0,"value":[]}'
      exit 0
    fi
    get_work_items_batch "$ids" "$fields"
    ;;
  relations)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    get_relations "$2"
    ;;
  attachments)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    get_attachments "$2"
    ;;
  comments)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    get_comments "$2"
    ;;
  link-pr)
    [[ $# -ge 3 ]] || { usage; exit 2; }
    link_pr "$2" "$3"
    ;;
  inline-images)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    get_inline_images "$2"
    ;;
  update)
    [[ $# -ge 4 ]] || { usage; exit 2; }
    update_field "$2" "$3" "$4"
    ;;
  create)
    [[ $# -ge 3 ]] || { usage; exit 2; }
    create_work_item "$2" "$3" "${4:-}" "${5:-}" "${6:-}"
    ;;
  delete)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    delete_work_item "$2" "${3:-}"
    ;;
  iterations)
    sub="${2:-}"
    case "$sub" in
      check) iterations_check ;;
      init)  iterations_init "${3:-}" ;;
      list)  iterations_list "${3:-3}" ;;
      *) echo "Usage: ado iterations check|init [--force]|list [n]"; exit 2 ;;
    esac
    ;;
  comment)
    [[ $# -ge 3 ]] || { usage; exit 2; }
    add_comment "$2" "$3"
    ;;
  tags)
    sub="${2:-}"
    case "$sub" in
      add)
        [[ $# -ge 4 ]] || { echo "Usage: ado tags add <id> <tag>"; exit 2; }
        tags_add "$3" "$4"
        ;;
      *) echo "Usage: ado tags add <id> <tag>"; exit 2 ;;
    esac
    ;;
  *)
    usage
    exit 2
    ;;
esac
