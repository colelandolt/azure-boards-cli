use clap::Subcommand;

#[derive(Subcommand)]
pub enum GithubCmd {
    /// Link a GitHub pull request to a work item
    LinkPr {
        work_item_id: i32,
        /// https://github.com/{owner}/{repo}/pull/{number}
        pr_url: String,
        /// Boards GitHub repo connection GUID (skips discovery)
        #[arg(long)]
        connection_id: Option<String>,
        /// Fall back to a plain hyperlink relation instead of an artifact link
        #[arg(long)]
        as_hyperlink: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Link a GitHub commit to a work item
    LinkCommit {
        work_item_id: i32,
        /// https://github.com/{owner}/{repo}/commit/{sha}
        commit_url: String,
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long)]
        as_hyperlink: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Link a GitHub branch to a work item
    LinkBranch {
        work_item_id: i32,
        /// https://github.com/{owner}/{repo}/tree/{branch}
        branch_url: String,
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long)]
        as_hyperlink: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Link a GitHub issue to a work item
    LinkIssue {
        work_item_id: i32,
        /// https://github.com/{owner}/{repo}/issues/{number}
        issue_url: String,
        #[arg(long)]
        connection_id: Option<String>,
        #[arg(long)]
        as_hyperlink: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// List GitHub links on a work item
    Links { work_item_id: i32 },
    /// Detect the GitHub repo from git remotes and check Boards connectivity
    Detect,
}
