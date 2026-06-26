use clap::Subcommand;

#[derive(Subcommand)]
pub enum MetricsCmd {
    /// Completed effort per iteration over the last N completed sprints
    Velocity {
        #[arg(long, default_value_t = 6)]
        iterations: usize,
        /// Effort field (default: the process's configured effort field)
        #[arg(long)]
        field: Option<String>,
    },
    /// Completed item count per period
    Throughput {
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        from: String,
        /// End date (YYYY-MM-DD, default today)
        #[arg(long)]
        to: Option<String>,
        /// Bucket size: day | week | month
        #[arg(long, default_value = "week")]
        period: String,
    },
    /// Created -> completed duration distribution
    LeadTime {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        r#type: Option<String>,
    },
    /// In-progress -> completed duration distribution
    CycleTime {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        r#type: Option<String>,
    },
    /// Per-day remaining work for an iteration
    Burndown {
        /// Iteration path or @current
        #[arg(long, default_value = "@current")]
        iteration: String,
        /// What to sum: count | remaining-work | effort
        #[arg(long, default_value = "count")]
        measure: String,
    },
    /// Current work-in-progress by type and assignee
    Wip,
    /// Cumulative flow diagram data (per-day counts per column/state)
    Cfd {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: Option<String>,
        /// Group by: column (Analytics only) | state
        #[arg(long, default_value = "state")]
        by: String,
    },
}
