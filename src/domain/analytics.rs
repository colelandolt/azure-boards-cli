//! Azure DevOps Analytics OData v4.0-preview client (raw REST; not covered by
//! the SDK). Follows @odata.nextLink pagination and probes availability.

use crate::client::{enc, Clients};
use crate::error::CliError;
use serde_json::Value;

pub fn base_url(clients: &Clients, project: &str) -> String {
    format!(
        "https://analytics.dev.azure.com/{}/{}/_odata/v4.0-preview",
        clients.org,
        enc(project)
    )
}

/// Minimal encoding for OData query option values (space and quote-safe).
pub fn odata_encode(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('"', "%22")
        .replace('#', "%23")
        .replace('&', "%26")
        .replace('+', "%2B")
        .replace('\\', "%5C")
}

/// True when the Analytics service answers for this project.
pub async fn available(clients: &Clients, project: &str) -> bool {
    let url = format!("{}/WorkItems?%24top=0", base_url(clients, project));
    clients.raw().get_json(&url).await.is_ok()
}

/// GET an entity set with raw OData query options (already encoded),
/// following nextLink pagination. Returns the concatenated "value" rows.
pub async fn query(
    clients: &Clients,
    project: &str,
    entity: &str,
    options: &[(&str, String)],
) -> Result<Vec<Value>, CliError> {
    let mut url = format!("{}/{}", base_url(clients, project), entity);
    let mut sep = '?';
    for (name, value) in options {
        url.push(sep);
        sep = '&';
        url.push_str(&format!("%24{}={}", name, odata_encode(value)));
    }
    let raw = clients.raw();
    let mut rows = Vec::new();
    let mut next = Some(url);
    while let Some(u) = next {
        let page = raw.get_json(&u).await?;
        if let Some(items) = page.get("value").and_then(|v| v.as_array()) {
            rows.extend(items.iter().cloned());
        }
        next = page
            .get("@odata.nextLink")
            .and_then(|l| l.as_str())
            .map(String::from);
    }
    Ok(rows)
}

/// Escape a string literal for an OData filter expression.
pub fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Distribution stats over a series of day-counts.
pub fn distribution(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return serde_json::json!({"count": 0});
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let count = values.len();
    let sum: f64 = values.iter().sum();
    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * (count as f64 - 1.0)).round() as usize;
        values[idx.min(count - 1)]
    };
    serde_json::json!({
        "count": count,
        "averageDays": (sum / count as f64 * 10.0).round() / 10.0,
        "medianDays": pct(50.0),
        "p85Days": pct(85.0),
        "p95Days": pct(95.0),
        "minDays": values[0],
        "maxDays": values[count - 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odata_encoding_and_literals() {
        assert_eq!(odata_encode("a b"), "a%20b");
        assert_eq!(literal("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn distribution_stats() {
        let d = distribution(vec![1.0, 2.0, 3.0, 4.0, 10.0]);
        assert_eq!(d["count"], 5);
        assert_eq!(d["medianDays"], 3.0);
        assert_eq!(d["maxDays"], 10.0);
        assert_eq!(d["averageDays"], 4.0);
        assert_eq!(distribution(vec![])["count"], 0);
    }
}
