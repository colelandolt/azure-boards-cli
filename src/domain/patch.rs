//! JSON-patch document builder for work item writes.

use azure_devops_rust_api::wit::models::{json_patch_operation::Op, JsonPatchOperation};
use serde_json::Value;

pub fn add_field(field: &str, value: impl Into<Value>) -> JsonPatchOperation {
    JsonPatchOperation {
        op: Some(Op::Add),
        path: Some(format!("/fields/{field}")),
        value: Some(value.into()),
        from: None,
    }
}

/// Optimistic-concurrency guard: fails the whole patch (HTTP 409/400 -> exit 6)
/// unless the work item is at exactly this revision.
pub fn test_rev(rev: i32) -> JsonPatchOperation {
    JsonPatchOperation {
        op: Some(Op::Test),
        path: Some("/rev".into()),
        value: Some(Value::from(rev)),
        from: None,
    }
}

pub fn add_relation(rel: &str, url: &str, attributes: Value) -> JsonPatchOperation {
    JsonPatchOperation {
        op: Some(Op::Add),
        path: Some("/relations/-".into()),
        value: Some(serde_json::json!({
            "rel": rel,
            "url": url,
            "attributes": attributes,
        })),
        from: None,
    }
}

pub fn remove_relation(index: usize) -> JsonPatchOperation {
    JsonPatchOperation {
        op: Some(Op::Remove),
        path: Some(format!("/relations/{index}")),
        value: None,
        from: None,
    }
}

/// Build a create patch from a `--from-json` document. Accepts either
/// `{"fields": {ref: value, ...}}` or a bare object whose top-level keys are
/// fields (minus the reserved `type`/`parent`/`relations`/`fields` keys).
/// `parent` adds a Hierarchy-Reverse link; `relations` adds raw relation ops.
/// Field values pass through as typed JSON (no `@file` expansion inside JSON).
pub fn ops_from_json(doc: &Value, org_url: &str) -> Result<Vec<JsonPatchOperation>, String> {
    const RESERVED: [&str; 4] = ["type", "parent", "relations", "fields"];
    let fields: serde_json::Map<String, Value> = match doc.get("fields") {
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("\"fields\" must be a JSON object".into()),
        None => match doc {
            Value::Object(top) => top
                .iter()
                .filter(|(k, _)| !RESERVED.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => return Err("--from-json document must be a JSON object".into()),
        },
    };

    let mut ops = Vec::new();
    for (field, value) in fields {
        ops.push(JsonPatchOperation {
            op: Some(Op::Add),
            path: Some(format!("/fields/{field}")),
            value: Some(value),
            from: None,
        });
    }
    if let Some(parent) = doc.get("parent") {
        let pid = parent
            .as_i64()
            .ok_or("\"parent\" must be an integer work item id")?;
        ops.push(add_relation(
            "System.LinkTypes.Hierarchy-Reverse",
            &format!("{org_url}/_apis/wit/workItems/{pid}"),
            serde_json::json!({"comment": ""}),
        ));
    }
    if let Some(relations) = doc.get("relations") {
        let arr = relations
            .as_array()
            .ok_or("\"relations\" must be a JSON array")?;
        for rel in arr {
            ops.push(JsonPatchOperation {
                op: Some(Op::Add),
                path: Some("/relations/-".into()),
                value: Some(rel.clone()),
                from: None,
            });
        }
    }
    if ops.is_empty() {
        return Err("--from-json produced no fields or relations".into());
    }
    Ok(ops)
}

/// Parse a user-supplied op name.
pub fn parse_op(name: &str) -> Option<Op> {
    match name.to_ascii_lowercase().as_str() {
        "add" => Some(Op::Add),
        "replace" => Some(Op::Replace),
        "remove" => Some(Op::Remove),
        "test" => Some(Op::Test),
        "copy" => Some(Op::Copy),
        "move" => Some(Op::Move),
        _ => None,
    }
}

/// User-supplied values: JSON if it parses as a non-string JSON literal,
/// otherwise the raw string. "1" stays a string (ADO coerces field values);
/// "{...}"/"[...]"/true/false/null become structured.
pub fn parse_value(raw: &str) -> Value {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
    } else {
        Value::String(raw.to_string())
    }
}

/// Build the field ops for create/update from sugar + explicit --field pairs.
/// Later entries win over earlier ones for the same field.
pub fn field_ops(pairs: &[(String, String)]) -> Vec<JsonPatchOperation> {
    let mut seen = std::collections::HashMap::new();
    for (i, (field, _)) in pairs.iter().enumerate() {
        seen.insert(field.clone(), i);
    }
    pairs
        .iter()
        .enumerate()
        .filter(|(i, (field, _))| seen[field] == *i)
        .map(|(_, (field, value))| add_field(field, parse_value(value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_field_shape() {
        let op = add_field("System.Title", "Hello");
        let v = serde_json::to_value(&op).unwrap();
        assert_eq!(v["op"], "add");
        assert_eq!(v["path"], "/fields/System.Title");
        assert_eq!(v["value"], "Hello");
    }

    #[test]
    fn rev_test_op_shape() {
        let v = serde_json::to_value(test_rev(7)).unwrap();
        assert_eq!(v["op"], "test");
        assert_eq!(v["path"], "/rev");
        assert_eq!(v["value"], 7);
    }

    #[test]
    fn later_duplicate_fields_win() {
        let ops = field_ops(&[
            ("System.Title".into(), "first".into()),
            ("System.Title".into(), "second".into()),
        ]);
        assert_eq!(ops.len(), 1);
        assert_eq!(serde_json::to_value(&ops[0]).unwrap()["value"], "second");
    }

    #[test]
    fn json_values_parsed_strings_kept() {
        assert_eq!(parse_value("plain"), Value::String("plain".into()));
        assert_eq!(parse_value("1"), Value::String("1".into()));
        assert_eq!(parse_value(r#"{"a":1}"#), serde_json::json!({"a":1}));
    }

    #[test]
    fn relation_op_includes_attributes() {
        let v = serde_json::to_value(add_relation(
            "ArtifactLink",
            "vstfs:///GitHub/PullRequest/abc%2F45",
            serde_json::json!({"name": "GitHub Pull Request"}),
        ))
        .unwrap();
        assert_eq!(v["value"]["rel"], "ArtifactLink");
        assert_eq!(v["value"]["attributes"]["name"], "GitHub Pull Request");
    }

    fn op_for_path<'a>(
        ops: &'a [JsonPatchOperation],
        path: &str,
    ) -> Option<&'a JsonPatchOperation> {
        ops.iter().find(|o| o.path.as_deref() == Some(path))
    }

    #[test]
    fn from_json_with_explicit_fields_and_parent() {
        let doc = serde_json::json!({
            "type": "User Story",
            "fields": {
                "System.Title": "T",
                "Microsoft.VSTS.Common.Priority": 1
            },
            "parent": 100
        });
        let ops = ops_from_json(&doc, "https://dev.azure.com/org").unwrap();
        assert_eq!(
            op_for_path(&ops, "/fields/System.Title").unwrap().value,
            Some(Value::String("T".into()))
        );
        // Numbers stay typed (not coerced to strings).
        assert_eq!(
            op_for_path(&ops, "/fields/Microsoft.VSTS.Common.Priority")
                .unwrap()
                .value,
            Some(Value::from(1))
        );
        let parent = op_for_path(&ops, "/relations/-").unwrap();
        let v = serde_json::to_value(parent).unwrap();
        assert_eq!(v["value"]["rel"], "System.LinkTypes.Hierarchy-Reverse");
        assert!(v["value"]["url"]
            .as_str()
            .unwrap()
            .ends_with("/workItems/100"));
    }

    #[test]
    fn from_json_bare_object_treated_as_fields() {
        let doc = serde_json::json!({"System.Title": "Bare", "System.State": "New"});
        let ops = ops_from_json(&doc, "https://dev.azure.com/org").unwrap();
        assert_eq!(ops.len(), 2);
        assert!(op_for_path(&ops, "/fields/System.Title").is_some());
        assert!(op_for_path(&ops, "/fields/System.State").is_some());
    }

    #[test]
    fn from_json_rejects_empty_and_bad_shapes() {
        assert!(ops_from_json(&serde_json::json!({}), "u").is_err());
        assert!(ops_from_json(&serde_json::json!({"fields": "nope"}), "u").is_err());
        assert!(ops_from_json(&serde_json::json!([1, 2]), "u").is_err());
        assert!(
            ops_from_json(&serde_json::json!({"fields": {"a": 1}, "parent": "x"}), "u").is_err()
        );
    }

    #[test]
    fn from_json_passes_relations_through() {
        let doc = serde_json::json!({
            "fields": {"System.Title": "T"},
            "relations": [
                {"rel": "System.LinkTypes.Related", "url": "https://x/_apis/wit/workItems/9"}
            ]
        });
        let ops = ops_from_json(&doc, "u").unwrap();
        let rel = ops
            .iter()
            .filter(|o| o.path.as_deref() == Some("/relations/-"))
            .count();
        assert_eq!(rel, 1);
    }
}
