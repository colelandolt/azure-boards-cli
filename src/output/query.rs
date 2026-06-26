use crate::error::CliError;
use serde_json::Value;

/// Sole jmespath touchpoint: apply a JMESPath expression to a JSON value.
/// Swapping implementations later is a one-file change.
pub fn apply(expr: &str, value: &Value) -> Result<Value, CliError> {
    let compiled = jmespath::compile(expr)
        .map_err(|e| CliError::Validation(format!("invalid --query expression: {e}")))?;
    let var = jmespath::Variable::try_from(value.clone())
        .map_err(|e| CliError::General(format!("--query input: {e}")))?;
    let result = compiled
        .search(var)
        .map_err(|e| CliError::Validation(format!("--query evaluation failed: {e}")))?;
    serde_json::to_value(&*result).map_err(|e| CliError::General(format!("--query result: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_list_wrapper() {
        let v = json!({"count": 2, "value": [{"id": 1, "state": "Active"}, {"id": 2, "state": "Closed"}]});
        let out = apply("value[?state=='Active'].id", &v).unwrap();
        assert_eq!(out, json!([1]));
    }

    #[test]
    fn invalid_expression_is_validation_error() {
        let e = apply("value[?", &json!({})).unwrap_err();
        assert_eq!(e.exit_code(), 7);
    }
}
