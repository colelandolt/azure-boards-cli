use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AreaCmd {
    /// Project-level area path tree
    Project {
        #[command(subcommand)]
        cmd: AreaProjectCmd,
    },
    /// Team area assignments
    Team {
        #[command(subcommand)]
        cmd: AreaTeamCmd,
    },
}

#[derive(Subcommand)]
pub enum AreaProjectCmd {
    /// List area paths
    List {
        #[arg(long, default_value_t = 4)]
        depth: u32,
    },
    /// Show one area node
    Show { path: String },
    /// Create an area path
    Create {
        /// New node name
        #[arg(long)]
        name: String,
        /// Parent path (default: project root)
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Rename an area path
    Update {
        path: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete an area path
    Delete {
        path: String,
        /// Node ID to reclassify existing work items to
        #[arg(long)]
        reclassify_id: Option<i32>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum AreaTeamCmd {
    /// List the team's area assignments
    List,
    /// Add an area path to the team
    Add {
        path: String,
        #[arg(long)]
        include_sub_areas: bool,
        /// Also make this the team's default area
        #[arg(long)]
        set_default: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an area path from the team
    Remove {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update an area assignment (toggle sub-area inclusion / default)
    Update {
        path: String,
        #[arg(long)]
        include_sub_areas: Option<bool>,
        #[arg(long)]
        set_default: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum IterationCmd {
    /// Project-level iteration tree
    Project {
        #[command(subcommand)]
        cmd: IterationProjectCmd,
    },
    /// Team sprint subscriptions and settings
    Team {
        #[command(subcommand)]
        cmd: IterationTeamCmd,
    },
    /// Show the team's current iteration
    Current,
    /// Create/refresh an iteration tree from a YAML/JSON plan file
    Import {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum IterationProjectCmd {
    /// List iterations (enriched with current/endsSoon)
    List {
        #[arg(long, default_value_t = 10)]
        depth: u32,
        /// Only iterations under this path prefix
        #[arg(long)]
        under: Option<String>,
    },
    /// Show one iteration node
    Show { path: String },
    /// Create an iteration
    Create {
        #[arg(long)]
        name: String,
        /// Parent path (default: project root)
        #[arg(long)]
        path: Option<String>,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        start_date: Option<String>,
        /// Finish date (YYYY-MM-DD)
        #[arg(long)]
        finish_date: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Update an iteration's name or dates
    Update {
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        start_date: Option<String>,
        #[arg(long)]
        finish_date: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete an iteration
    Delete {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum IterationTeamCmd {
    /// List the team's subscribed iterations
    List {
        /// Filter: past | current | future
        #[arg(long)]
        timeframe: Option<String>,
    },
    /// Subscribe the team to a project iteration (by path)
    Add {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// List work items in a team iteration
    ListWorkItems {
        /// Iteration path, or @current/@next/@previous
        iteration: String,
    },
    /// Unsubscribe the team from an iteration
    Remove {
        /// Iteration path, or @current/@next/@previous
        iteration: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Set the team's backlog iteration
    SetBacklogIteration {
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the team's backlog iteration
    ShowBacklogIteration,
    /// Set the team's default iteration (path or @currentIteration macro)
    SetDefaultIteration {
        /// Iteration path, or "@currentIteration"
        path: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the team's default iteration
    ShowDefaultIteration,
}

#[derive(Subcommand)]
pub enum SprintCmd {
    /// List the team's sprints
    List,
    /// Show the current sprint
    Current,
    /// Work items in the current sprint
    Backlog {
        /// Sprint path or @current/@next/@previous (default @current)
        #[arg(long, default_value = "@current")]
        sprint: String,
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
    },
    /// Move a work item into a sprint
    AddWorkItem {
        id: i32,
        /// Sprint path or @current/@next/@previous
        #[arg(long, default_value = "@current")]
        iteration: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a work item to another sprint
    MoveWorkItem {
        id: i32,
        /// Sprint path or @current/@next/@previous
        #[arg(long)]
        to: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Sprint capacity
    Capacity {
        #[command(subcommand)]
        cmd: CapacityCmd,
    },
    /// Remaining-work burndown for a sprint
    Burndown {
        /// Sprint path or @current (default)
        #[arg(long, default_value = "@current")]
        sprint: String,
    },
}

#[derive(Subcommand)]
pub enum CapacityCmd {
    /// Show per-member capacity and days off for a sprint
    Show {
        /// Sprint path or @current (default)
        #[arg(long, default_value = "@current")]
        sprint: String,
    },
}
