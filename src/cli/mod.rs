pub mod boards;
pub mod github;
pub mod items;
pub mod metrics;
pub mod planning;
pub mod work_item;

use crate::output::OutputFormat;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "azure-boards",
    version,
    about = "Azure Boards from the command line: work items, sprints, backlogs, and developer workflow automation.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    #[command(flatten)]
    pub global: GlobalArgs,
}

#[derive(Args, Clone, Debug, Default)]
pub struct GlobalArgs {
    /// Organization name or URL (e.g. "nrgmr" or https://dev.azure.com/nrgmr)
    #[arg(long, global = true)]
    pub org: Option<String>,
    /// Project name or ID
    #[arg(long, global = true)]
    pub project: Option<String>,
    /// Team name (used by team-scoped commands)
    #[arg(long, global = true)]
    pub team: Option<String>,
    /// Enable/disable git-remote auto-detection of org/project
    #[arg(long, global = true, value_parser = clap::builder::BoolishValueParser::new())]
    pub detect: Option<bool>,
    /// Output format
    #[arg(short = 'o', long, global = true, value_enum)]
    pub output: Option<OutputFormat>,
    /// Shorthand for -o json
    #[arg(long, global = true, conflicts_with = "output")]
    pub json: bool,
    /// JMESPath query applied to the result before rendering
    #[arg(long, global = true, value_name = "JMESPATH")]
    pub query: Option<String>,
    /// Assume "yes" on confirmation prompts (skip confirmation)
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    /// Only show errors on stderr
    #[arg(long, global = true)]
    pub only_show_errors: bool,
    /// Increase stderr logging (info level)
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Full debug logging on stderr (auth material redacted)
    #[arg(long, global = true)]
    pub debug: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Sign in and manage credentials
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
    /// Set persistent defaults (organization, project, team, output)
    Configure(ConfigureArgs),
    /// Show or explain the resolved org/project/team context
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
    /// Create, inspect, update, and delete work items
    #[command(name = "work-item", visible_alias = "wi")]
    WorkItem {
        #[command(subcommand)]
        cmd: work_item::WorkItemCmd,
    },
    /// Run raw WIQL queries
    Wiql {
        #[command(subcommand)]
        cmd: work_item::WiqlCmd,
    },
    /// Manage and run saved queries
    Query {
        #[command(subcommand)]
        cmd: work_item::QueryCmd,
    },
    /// Work item comments
    Comment {
        #[command(subcommand)]
        cmd: items::CommentCmd,
    },
    /// Work item relations (links)
    Relation {
        #[command(subcommand)]
        cmd: items::RelationCmd,
    },
    /// Work item file attachments
    Attachment {
        #[command(subcommand)]
        cmd: items::AttachmentCmd,
    },
    /// Inline images embedded in work item descriptions
    Image {
        #[command(subcommand)]
        cmd: items::ImageCmd,
    },
    /// Work item tags
    Tag {
        #[command(subcommand)]
        cmd: items::TagCmd,
    },
    /// Area paths (project tree and team assignments)
    Area {
        #[command(subcommand)]
        cmd: planning::AreaCmd,
    },
    /// Iterations (project tree, team sprints, import)
    Iteration {
        #[command(subcommand)]
        cmd: planning::IterationCmd,
    },
    /// Team sprint convenience commands
    Sprint {
        #[command(subcommand)]
        cmd: planning::SprintCmd,
    },
    /// Kanban boards
    Board {
        #[command(subcommand)]
        cmd: boards::BoardCmd,
    },
    /// Backlogs: levels, ordering, forecasting
    Backlog {
        #[command(subcommand)]
        cmd: boards::BacklogCmd,
    },
    /// Flow metrics (velocity, lead/cycle time, burndown, CFD)
    Metrics {
        #[command(subcommand)]
        cmd: metrics::MetricsCmd,
    },
    /// GitHub artifact links on work items
    Github {
        #[command(subcommand)]
        cmd: github::GithubCmd,
    },
    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
pub enum AuthCmd {
    /// Sign in with Microsoft Entra (device code), or store a PAT with --pat
    Login {
        /// Store a Personal Access Token instead (read from stdin, never argv)
        #[arg(long)]
        pat: bool,
    },
    /// Show the active credential source and identity (never prints secrets)
    Status,
    /// Remove stored credentials
    Logout,
}

#[derive(Args)]
pub struct ConfigureArgs {
    /// Defaults to set, e.g. organization=nrgmr project="Yellow Hat" team=...
    #[arg(long, value_parser = parse_key_val, num_args = 1.., value_name = "KEY=VALUE")]
    pub defaults: Vec<(String, String)>,
    /// Write to the repo-local .azure-boards.toml instead of user config
    #[arg(long)]
    pub local: bool,
    /// List current configuration
    #[arg(long, short = 'l')]
    pub list: bool,
}

#[derive(Subcommand)]
pub enum ContextCmd {
    /// Show the resolved context and where each value came from
    Show,
    /// Run detection and optionally explain every resolution step
    Detect {
        /// Print the full rung-by-rung resolution trace
        #[arg(long)]
        explain: bool,
    },
}

/// Parse "key=value" pairs for --defaults and --field.
pub fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected KEY=VALUE, got '{s}'"))?;
    if k.trim().is_empty() {
        return Err(format!("empty key in '{s}'"));
    }
    Ok((k.trim().to_string(), v.to_string()))
}

/// Resolve an argument value that may reference external content:
/// - `@-` reads stdin (cached, so multiple `@-` in one invocation see the same
///   content rather than the second read getting an empty stream)
/// - `@path` reads the file at `path`
/// - anything else is returned verbatim
///
/// To pass a literal value that begins with `@`, source it from a file or
/// stdin instead. Used for every free-text field value (descriptions,
/// acceptance criteria, comments, WIQL, and `-f Ref=@file`).
pub fn read_arg_or_file(value: &str) -> Result<String, crate::error::CliError> {
    use crate::error::CliError;
    // One-time stdin cache: a single process can only consume stdin once, so
    // every `@-` reference resolves to the same captured content.
    static STDIN_CACHE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    match value.strip_prefix('@') {
        Some("-") => {
            let mut cache = STDIN_CACHE.lock().unwrap();
            if cache.is_none() {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| CliError::Validation(format!("failed to read stdin (@-): {e}")))?;
                *cache = Some(buf);
            }
            Ok(cache.as_ref().unwrap().clone())
        }
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| CliError::Validation(format!("cannot read @{path}: {e}"))),
        None => Ok(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_key_val_basics() {
        assert_eq!(
            parse_key_val("organization=nrgmr").unwrap(),
            ("organization".into(), "nrgmr".into())
        );
        assert_eq!(
            parse_key_val("System.Title=a=b").unwrap(),
            ("System.Title".into(), "a=b".into())
        );
        assert!(parse_key_val("no-equals").is_err());
    }

    #[test]
    fn global_args_accepted_anywhere() {
        let cli = Cli::try_parse_from([
            "ab",
            "work-item",
            "show",
            "123",
            "--org",
            "nrgmr",
            "-o",
            "json",
        ])
        .unwrap();
        assert_eq!(cli.global.org.as_deref(), Some("nrgmr"));
        let cli2 =
            Cli::try_parse_from(["ab", "--org", "nrgmr", "work-item", "show", "123"]).unwrap();
        assert_eq!(cli2.global.org.as_deref(), Some("nrgmr"));
    }

    #[test]
    fn json_conflicts_with_output() {
        assert!(Cli::try_parse_from(["ab", "context", "show", "--json", "-o", "yaml"]).is_err());
    }
}
