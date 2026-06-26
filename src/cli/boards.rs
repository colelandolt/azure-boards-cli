use clap::Subcommand;

#[derive(Subcommand)]
pub enum BoardCmd {
    /// List the team's boards
    List,
    /// Show a board (columns, rows, field mappings)
    Show { board: String },
    /// List a board's columns
    Columns { board: String },
    /// Work items on a board, optionally filtered to a column
    WorkItems {
        board: String,
        #[arg(long)]
        column: Option<String>,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
}

#[derive(Subcommand)]
pub enum BacklogCmd {
    /// List the team's backlog levels
    Levels,
    /// List backlog work items in priority order
    List {
        /// Backlog level id or name (e.g. "Microsoft.RequirementCategory" or "Stories")
        #[arg(long)]
        level: Option<String>,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
    /// Reorder: move a work item before/after another in the backlog
    Prioritize {
        #[arg(long)]
        id: i32,
        /// Place the item immediately before this work item
        #[arg(long, conflicts_with = "after")]
        before: Option<i32>,
        /// Place the item immediately after this work item
        #[arg(long)]
        after: Option<i32>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a backlog item to an iteration
    Move {
        #[arg(long)]
        id: i32,
        /// Iteration path or @current/@next/@previous
        #[arg(long)]
        to_iteration: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Forecast which sprint each backlog item lands in, given a velocity
    Forecast {
        /// Effort per sprint (in the effort field's units)
        #[arg(long)]
        velocity: f64,
        /// Effort field override (default: the process's configured effort field)
        #[arg(long)]
        field: Option<String>,
        #[arg(long)]
        level: Option<String>,
    },
}
