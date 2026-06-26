use super::Column;
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

/// Look up a dotted path ("fields.System.Title" walks fields -> System.Title
/// first, falling back to a literal key containing dots) and stringify.
pub fn lookup(value: &Value, path: &str) -> String {
    fn get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
        if let Some(v) = value.get(path) {
            return Some(v);
        }
        let (head, tail) = path.split_once('.')?;
        get(value.get(head)?, tail)
    }
    match get(value, path) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        // ADO identity fields are objects with displayName.
        Some(obj @ Value::Object(_)) => obj
            .get("displayName")
            .and_then(|d| d.as_str())
            .map(String::from)
            .unwrap_or_else(|| obj.to_string()),
        Some(other) => other.to_string(),
    }
}

/// az-style borderless table: left-aligned columns, two-space gutters,
/// dashed underline under the headers.
pub fn render_table(rows: &[Value], columns: &[Column]) -> String {
    if columns.is_empty() || rows.is_empty() {
        return if rows.is_empty() {
            String::new()
        } else {
            rows.iter()
                .map(|r| serde_json::to_string(r).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| columns.iter().map(|c| lookup(row, c.path)).collect())
        .collect();
    let mut widths: Vec<usize> = columns.iter().map(|c| c.header.width()).collect();
    for row in &cells {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.width());
        }
    }
    let mut out = String::new();
    let emit_row = |out: &mut String, cells: &[String]| {
        let last = cells.len() - 1;
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            line.push_str(cell);
            if i != last {
                let pad = widths[i].saturating_sub(cell.width()) + 2;
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    };
    let headers: Vec<String> = columns.iter().map(|c| c.header.to_string()).collect();
    emit_row(&mut out, &headers);
    let dashes: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    emit_row(&mut out, &dashes);
    for row in &cells {
        emit_row(&mut out, row);
    }
    out
}

/// Two-column key/value detail view for a single item.
pub fn render_detail(value: &Value, columns: &[Column]) -> String {
    let pairs: Vec<(String, String)> = if columns.is_empty() {
        flatten(value)
    } else {
        columns
            .iter()
            .map(|c| (c.header.to_string(), lookup(value, c.path)))
            .collect()
    };
    let key_width = pairs.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let mut out = String::new();
    for (k, v) in pairs {
        let pad = key_width.saturating_sub(k.width()) + 2;
        out.push_str(&k);
        out.extend(std::iter::repeat_n(' ', pad));
        out.push_str(&v);
        out.push('\n');
    }
    out
}

fn flatten(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let rendered = match v {
                    Value::String(s) => s.clone(),
                    Value::Object(_) | Value::Array(_) => {
                        serde_json::to_string(v).unwrap_or_default()
                    }
                    other => other.to_string(),
                };
                (k.clone(), rendered)
            })
            .collect(),
        other => vec![("value".into(), other.to_string())],
    }
}

/// TSV: raw values, tab-joined, no header (az parity; script-friendly).
pub fn render_tsv(rows: &[Value], columns: &[Column]) -> String {
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = if columns.is_empty() {
            vec![serde_json::to_string(row).unwrap_or_default()]
        } else {
            columns.iter().map(|c| lookup(row, c.path)).collect()
        };
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cols() -> Vec<Column> {
        vec![
            Column::new("ID", "id"),
            Column::new("Title", "fields.System.Title"),
            Column::new("Assigned To", "fields.System.AssignedTo"),
        ]
    }

    #[test]
    fn renders_borderless_table() {
        let rows = vec![
            json!({"id": 1, "fields": {"System.Title": "Fix bug", "System.AssignedTo": {"displayName": "Cole"}}}),
            json!({"id": 22, "fields": {"System.Title": "Ship it"}}),
        ];
        let t = render_table(&rows, &cols());
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "ID  Title    Assigned To");
        assert_eq!(lines[1], "--  -------  -----------");
        assert_eq!(lines[2], "1   Fix bug  Cole");
        assert_eq!(lines[3], "22  Ship it");
    }

    #[test]
    fn dotted_path_prefers_nested_then_literal() {
        let v = json!({"fields": {"System.Title": "literal key wins"}});
        assert_eq!(lookup(&v, "fields.System.Title"), "literal key wins");
        let v2 = json!({"a": {"b": {"c": 5}}});
        assert_eq!(lookup(&v2, "a.b.c"), "5");
    }

    #[test]
    fn tsv_has_no_header_and_raw_tabs() {
        let rows = vec![json!({"id": 1, "fields": {"System.Title": "x"}})];
        assert_eq!(render_tsv(&rows, &cols()), "1\tx\t\n");
    }

    #[test]
    fn detail_aligns_keys() {
        let v = json!({"id": 7, "state": "Active"});
        let d = render_detail(
            &v,
            &[Column::new("ID", "id"), Column::new("State", "state")],
        );
        assert_eq!(d, "ID     7\nState  Active\n");
    }
}
