pub mod color;
pub mod query;
pub mod table;

use crate::error::CliError;
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum OutputFormat {
    Json,
    Jsonc,
    Table,
    Tsv,
    Yaml,
    Yamlc,
    None,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "jsonc" => Ok(Self::Jsonc),
            "table" => Ok(Self::Table),
            "tsv" => Ok(Self::Tsv),
            "yaml" => Ok(Self::Yaml),
            "yamlc" => Ok(Self::Yamlc),
            "none" => Ok(Self::None),
            other => Err(format!("unknown output format: {other}")),
        }
    }
}

/// A column for table rendering: header plus a dotted path into the JSON value
/// (e.g. "fields.System\\.Title" is not supported — paths are plain key chains).
#[derive(Clone)]
pub struct Column {
    pub header: &'static str,
    pub path: &'static str,
}

impl Column {
    pub const fn new(header: &'static str, path: &'static str) -> Self {
        Self { header, path }
    }
}

/// Target of a mutating command, embedded in every mutation envelope so agents
/// can verify where a write landed (or would land, under --dry-run).
#[derive(Clone, Debug, serde::Serialize)]
pub struct Target {
    pub organization: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(rename = "workItemId", skip_serializing_if = "Option::is_none")]
    pub work_item_id: Option<i64>,
}

pub struct MutationEnvelope {
    pub operation: String,
    pub target: Target,
    pub dry_run: bool,
    pub result: Value,
}

/// Everything a command handler can produce. Handlers never write to stdout;
/// only `render` does.
pub enum CommandOutput {
    List {
        value: Vec<Value>,
        columns: Vec<Column>,
    },
    Item {
        value: Value,
        columns: Vec<Column>,
    },
    Mutation(MutationEnvelope),
    /// Raw bytes (e.g. attachment downloaded to stdout). Never JSON-wrapped.
    Raw(Vec<u8>),
    None,
}

impl CommandOutput {
    /// Canonical JSON shape: lists -> {"count", "value"}, mutations -> envelope.
    pub fn to_value(&self) -> Value {
        match self {
            CommandOutput::List { value, .. } => json!({
                "count": value.len(),
                "value": value,
            }),
            CommandOutput::Item { value, .. } => value.clone(),
            CommandOutput::Mutation(m) => {
                let mut obj = json!({
                    "operation": m.operation,
                    "target": m.target,
                    "dryRun": m.dry_run,
                    "result": m.result,
                });
                // Surface the affected entity's id/rev at the top level so
                // `--query id` / `--query rev` work the same as on reads (which
                // nest them under the item). Falls back to the resolved work
                // item id when the result body doesn't carry one.
                let id = m
                    .result
                    .get("id")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .or_else(|| m.target.work_item_id.map(Value::from));
                if let Some(id) = id {
                    obj["id"] = id;
                }
                if let Some(rev) = m.result.get("rev").filter(|v| !v.is_null()).cloned() {
                    obj["rev"] = rev;
                }
                obj
            }
            CommandOutput::Raw(_) | CommandOutput::None => Value::Null,
        }
    }
}

/// Resolve the effective format: explicit flag > config default > TTY sniffing.
pub fn select_format(
    flag: Option<OutputFormat>,
    config_default: Option<OutputFormat>,
    stdout_tty: bool,
) -> OutputFormat {
    flag.or(config_default).unwrap_or(if stdout_tty {
        OutputFormat::Table
    } else {
        OutputFormat::Json
    })
}

pub fn render(
    out: &CommandOutput,
    format: OutputFormat,
    jmes: Option<&str>,
    stdout_tty: bool,
) -> Result<(), CliError> {
    let mut stdout = std::io::stdout().lock();

    if let CommandOutput::Raw(bytes) = out {
        stdout.write_all(bytes)?;
        return Ok(());
    }
    if matches!(out, CommandOutput::None) || format == OutputFormat::None {
        return Ok(());
    }

    let mut value = out.to_value();
    if let Some(expr) = jmes {
        value = query::apply(expr, &value)?;
    }

    let color = stdout_tty && std::env::var_os("NO_COLOR").is_none();
    match format {
        OutputFormat::Json => writeln!(stdout, "{}", serde_json::to_string_pretty(&value)?)?,
        OutputFormat::Jsonc => {
            let text = serde_json::to_string_pretty(&value)?;
            if color {
                writeln!(stdout, "{}", color::colorize_json(&text))?;
            } else {
                writeln!(stdout, "{text}")?;
            }
        }
        OutputFormat::Yaml => write!(stdout, "{}", to_yaml(&value)?)?,
        OutputFormat::Yamlc => {
            let text = to_yaml(&value)?;
            if color {
                write!(stdout, "{}", color::colorize_yaml(&text))?;
            } else {
                write!(stdout, "{text}")?;
            }
        }
        OutputFormat::Table => render_human(&mut stdout, out, &value, jmes.is_some())?,
        OutputFormat::Tsv => render_tsv(&mut stdout, out, &value, jmes.is_some())?,
        OutputFormat::None => {}
    }
    Ok(())
}

fn to_yaml(value: &Value) -> Result<String, CliError> {
    serde_norway::to_string(value).map_err(|e| CliError::General(format!("yaml encode: {e}")))
}

/// Human (table) rendering. After --query reshapes the value we can no longer
/// rely on the declared columns, so degrade gracefully like az does.
fn render_human(
    w: &mut impl Write,
    out: &CommandOutput,
    value: &Value,
    queried: bool,
) -> Result<(), CliError> {
    if queried {
        return render_degraded(w, value);
    }
    match out {
        CommandOutput::List {
            value: rows,
            columns,
        } => {
            write!(w, "{}", table::render_table(rows, columns))?;
        }
        CommandOutput::Item { value, columns } => {
            write!(w, "{}", table::render_detail(value, columns))?;
        }
        CommandOutput::Mutation(m) => {
            let verb = if m.dry_run { "Would run" } else { "Completed" };
            let mut line = format!(
                "{verb} {} in {}/{}",
                m.operation, m.target.organization, m.target.project
            );
            if let Some(id) = m.target.work_item_id {
                line.push_str(&format!(" (work item {id})"));
            }
            writeln!(w, "{line}")?;
            if m.dry_run {
                writeln!(w, "{}", serde_json::to_string_pretty(&m.result)?)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn render_tsv(
    w: &mut impl Write,
    out: &CommandOutput,
    value: &Value,
    queried: bool,
) -> Result<(), CliError> {
    if queried {
        return render_degraded(w, value);
    }
    match out {
        CommandOutput::List {
            value: rows,
            columns,
        } => {
            write!(w, "{}", table::render_tsv(rows, columns))?;
        }
        CommandOutput::Item { value, columns } => {
            let row = vec![value.clone()];
            write!(w, "{}", table::render_tsv(&row, columns))?;
        }
        _ => writeln!(w, "{}", serde_json::to_string(value)?)?,
    }
    Ok(())
}

/// az-style degradation for post-query values: scalar raw, scalar array one
/// per line, anything else pretty JSON.
fn render_degraded(w: &mut impl Write, value: &Value) -> Result<(), CliError> {
    match value {
        Value::String(s) => writeln!(w, "{s}")?,
        Value::Number(_) | Value::Bool(_) => writeln!(w, "{value}")?,
        Value::Null => {}
        Value::Array(items) if items.iter().all(|i| !i.is_object() && !i.is_array()) => {
            for item in items {
                match item {
                    Value::String(s) => writeln!(w, "{s}")?,
                    other => writeln!(w, "{other}")?,
                }
            }
        }
        other => writeln!(w, "{}", serde_json::to_string_pretty(other)?)?,
    }
    Ok(())
}

/// Emit an error to stderr. In JSON-ish modes this is the stable error object;
/// in human modes it is a plain message.
pub fn emit_error(err: &CliError, format: OutputFormat) {
    let machine = matches!(
        format,
        OutputFormat::Json | OutputFormat::Jsonc | OutputFormat::Yaml | OutputFormat::Yamlc
    );
    if machine {
        eprintln!(
            "{}",
            serde_json::to_string(&err.to_json()).unwrap_or_default()
        );
    } else {
        eprintln!("error: {err}");
    }
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::General(format!("json: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_wrapper_is_stable() {
        let out = CommandOutput::List {
            value: vec![json!({"id": 1}), json!({"id": 2})],
            columns: vec![],
        };
        let v = out.to_value();
        assert_eq!(v["count"], 2);
        assert_eq!(v["value"][1]["id"], 2);
    }

    #[test]
    fn mutation_envelope_shape() {
        let out = CommandOutput::Mutation(MutationEnvelope {
            operation: "work-item.update".into(),
            target: Target {
                organization: "nrgmr".into(),
                project: "Yellow Hat".into(),
                team: None,
                work_item_id: Some(123),
            },
            dry_run: true,
            result: json!([{"op": "add"}]),
        });
        let v = out.to_value();
        assert_eq!(v["operation"], "work-item.update");
        assert_eq!(v["target"]["workItemId"], 123);
        assert_eq!(v["dryRun"], true);
        assert!(v["target"].get("team").is_none());
        // id falls back to the resolved work item id when result has none.
        assert_eq!(v["id"], 123);
    }

    #[test]
    fn mutation_hoists_id_and_rev_from_result() {
        // A create whose result is the work item: id/rev come from the body and
        // become queryable as `--query id` / `--query rev`.
        let out = CommandOutput::Mutation(MutationEnvelope {
            operation: "work-item.create".into(),
            target: Target {
                organization: "o".into(),
                project: "p".into(),
                team: None,
                work_item_id: None,
            },
            dry_run: false,
            result: json!({"id": 42, "rev": 1, "fields": {}}),
        });
        let v = out.to_value();
        assert_eq!(v["id"], 42);
        assert_eq!(v["rev"], 1);
    }

    #[test]
    fn mutation_without_any_id_omits_the_key() {
        let out = CommandOutput::Mutation(MutationEnvelope {
            operation: "iteration.import".into(),
            target: Target {
                organization: "o".into(),
                project: "p".into(),
                team: None,
                work_item_id: None,
            },
            dry_run: true,
            result: json!({"plan": []}),
        });
        let v = out.to_value();
        assert!(v.get("id").is_none(), "no id should be invented: {v}");
        assert!(v.get("rev").is_none());
    }

    #[test]
    fn format_selection_precedence() {
        assert_eq!(
            select_format(Some(OutputFormat::Yaml), Some(OutputFormat::Json), true),
            OutputFormat::Yaml
        );
        assert_eq!(
            select_format(Option::None, Some(OutputFormat::Json), true),
            OutputFormat::Json
        );
        assert_eq!(
            select_format(Option::None, Option::None, true),
            OutputFormat::Table
        );
        assert_eq!(
            select_format(Option::None, Option::None, false),
            OutputFormat::Json
        );
    }
}
