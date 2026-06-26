use super::parse_key_val;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum WorkItemCmd {
    /// Show a work item
    Show {
        id: i32,
        /// Comma-separated field reference names (mutually exclusive with --expand on the API)
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
        /// Expand: relations or all
        #[arg(long, value_parser = ["relations", "all", "links", "fields", "none"])]
        expand: Option<String>,
        /// Show the work item as of a UTC date/time (e.g. 2026-01-15 or 2026-01-15T12:00:00Z)
        #[arg(long, value_name = "DATETIME")]
        as_of: Option<String>,
        /// Open in the browser instead of printing
        #[arg(long, visible_alias = "open")]
        web: bool,
    },
    /// Fetch many work items by ID in batches
    BatchGet {
        /// Comma-separated work item IDs
        #[arg(long, value_delimiter = ',', required = true)]
        ids: Vec<i32>,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
    /// Create a work item
    Create {
        /// Work item type (any type the process defines, e.g. "User Story", "Bug").
        /// Required unless supplied via --from-json's "type" key.
        #[arg(long, short = 't', value_name = "TYPE")]
        r#type: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Description text, or @file / @- to read from a file or stdin
        #[arg(long)]
        description: Option<String>,
        /// Acceptance criteria text, or @file / @-
        #[arg(long)]
        acceptance_criteria: Option<String>,
        /// Parent work item ID (adds a Hierarchy-Reverse relation)
        #[arg(long)]
        parent: Option<i32>,
        #[arg(long)]
        area: Option<String>,
        #[arg(long)]
        iteration: Option<String>,
        #[arg(long)]
        assigned_to: Option<String>,
        /// Arbitrary fields: --field Ref.Name=value (repeatable). The value may
        /// be @file / @- to read large/structured content without shell escaping.
        #[arg(long = "field", short = 'f', value_parser = parse_key_val, value_name = "REF=VALUE")]
        fields: Vec<(String, String)>,
        /// Build the whole item from a JSON document (inline JSON, @file, or @-).
        /// Shape: {"type":"...","fields":{Ref:value,...},"parent":<id>,"relations":[...]}.
        /// Mutually exclusive with the field flags above.
        #[arg(long, value_name = "JSON|@FILE")]
        from_json: Option<String>,
        /// Treat --description / --acceptance-criteria as Markdown and convert to
        /// HTML (the format Azure DevOps stores these fields in)
        #[arg(long)]
        markdown: bool,
        #[arg(long)]
        dry_run: bool,
        /// Open the created work item in the browser
        #[arg(long, visible_alias = "open")]
        web: bool,
    },
    /// Update fields on a work item
    Update {
        id: i32,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        state: Option<String>,
        /// Reason accompanying a state change
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        assigned_to: Option<String>,
        #[arg(long)]
        area: Option<String>,
        #[arg(long)]
        iteration: Option<String>,
        /// Description text, or @file / @- to read from a file or stdin
        #[arg(long)]
        description: Option<String>,
        /// Acceptance criteria text, or @file / @-
        #[arg(long)]
        acceptance_criteria: Option<String>,
        #[arg(long = "field", short = 'f', value_parser = parse_key_val, value_name = "REF=VALUE")]
        fields: Vec<(String, String)>,
        /// Treat --description / --acceptance-criteria as Markdown and convert to HTML
        #[arg(long)]
        markdown: bool,
        /// Fail with a conflict (exit 6) unless the work item is at this revision
        #[arg(long)]
        expected_rev: Option<i32>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply a raw JSON-patch operation
    Patch {
        id: i32,
        /// Patch operation: add, replace, remove, test, copy, move
        #[arg(long)]
        op: String,
        /// Patch path, e.g. /fields/System.Tags
        #[arg(long)]
        path: String,
        /// Value (JSON if it parses, else string). Omit for remove.
        #[arg(long)]
        value: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a work item (recoverable recycle-bin delete by default)
    Delete {
        id: i32,
        /// PERMANENTLY destroy (cannot be undone); requires --confirm-id
        #[arg(long)]
        destroy: bool,
        /// Must equal the target id; required with --destroy
        #[arg(long, requires = "destroy")]
        confirm_id: Option<i64>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Restore a work item from the recycle bin
    Restore { id: i32 },
    /// Open a work item in the browser
    Open { id: i32 },
}

#[derive(Subcommand)]
pub enum WiqlCmd {
    /// Run a WIQL query and return matching IDs/relations
    Run {
        /// WIQL text, or @file.wiql / @- for stdin
        wiql: String,
        /// Cap the number of results
        #[arg(long)]
        top: Option<i32>,
    },
    /// Run a WIQL query and hydrate full work items (batched, order-preserving)
    Fetch {
        /// WIQL text, or @file.wiql / @- for stdin
        wiql: String,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
        #[arg(long)]
        top: Option<i32>,
    },
}

#[derive(Subcommand)]
pub enum QueryCmd {
    /// List saved queries
    List {
        /// Folder path to list (default: root)
        #[arg(long)]
        folder: Option<String>,
    },
    /// Show a saved query by path or ID
    Show { query: String },
    /// Run a saved query and hydrate the results
    Run {
        query: String,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
    /// Create a saved query
    Create {
        #[arg(long)]
        name: String,
        /// Parent folder path or ID (e.g. "Shared Queries/Team")
        #[arg(long, default_value = "Shared Queries")]
        folder: String,
        /// WIQL text, or @file.wiql / @- for stdin
        #[arg(long)]
        wiql: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update a saved query's WIQL or name
    Update {
        query: String,
        #[arg(long)]
        wiql: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a saved query or folder
    Delete {
        query: String,
        #[arg(long)]
        dry_run: bool,
    },
}

/// Shared sugar-to-field mapping used by create and update.
#[derive(Args, Debug, Default)]
pub struct FieldSugar {}

#[allow(clippy::too_many_arguments)]
pub fn sugar_fields(
    title: Option<&str>,
    state: Option<&str>,
    reason: Option<&str>,
    assigned_to: Option<&str>,
    area: Option<&str>,
    iteration: Option<&str>,
    description: Option<&str>,
    acceptance_criteria: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |field: &str, value: Option<&str>| {
        if let Some(v) = value {
            out.push((field.to_string(), v.to_string()));
        }
    };
    push("System.Title", title);
    push("System.State", state);
    push("System.Reason", reason);
    push("System.AssignedTo", assigned_to);
    push("System.AreaPath", area);
    push("System.IterationPath", iteration);
    push("System.Description", description);
    push(
        "Microsoft.VSTS.Common.AcceptanceCriteria",
        acceptance_criteria,
    );
    out
}
