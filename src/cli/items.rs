use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum CommentCmd {
    /// List all comments on a work item (follows pagination)
    List { work_item_id: i32 },
    /// Add a comment
    Add {
        work_item_id: i32,
        /// Comment text, or @file.md / @- for stdin
        #[arg(long)]
        text: String,
    },
    /// Update a comment
    Update {
        work_item_id: i32,
        comment_id: i32,
        /// Comment text, or @file.md / @- for stdin
        #[arg(long)]
        text: String,
    },
    /// Delete a comment
    Delete {
        work_item_id: i32,
        comment_id: i32,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum RelationCmd {
    /// List relations, classified (Parent/Child/Related/GitHub PR/...)
    List { id: i32 },
    /// Add a relation
    Add {
        id: i32,
        /// parent | child | related | predecessor | successor | duplicate |
        /// duplicate-of | a full reference name (System.LinkTypes.*)
        #[arg(long, short = 't')]
        r#type: String,
        /// Target work item ID (or full URL for artifact/hyperlink types)
        #[arg(long)]
        target: String,
        /// Optional link comment
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a relation by the index shown in `relation list`
    Remove {
        id: i32,
        /// Relation index from `relation list` (rev-checked at removal time)
        #[arg(long)]
        relation_id: usize,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show the parent/child tree from a root work item
    Tree {
        id: i32,
        #[arg(long, default_value_t = 3)]
        depth: u32,
    },
    /// List the relation types defined in the organization
    ListType,
}

#[derive(Subcommand)]
pub enum AttachmentCmd {
    /// List file attachments on a work item
    List { id: i32 },
    /// Download attachments
    Download {
        id: i32,
        /// Only download the attachment with this file name
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = ".")]
        output_dir: PathBuf,
    },
    /// Upload a file and attach it
    Add {
        id: i32,
        file: PathBuf,
        #[arg(long)]
        comment: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an attachment by file name
    Remove {
        id: i32,
        #[arg(long)]
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum ImageCmd {
    /// List inline images in Description/Acceptance Criteria with text context
    List { id: i32 },
    /// Download inline images to a directory
    Download {
        id: i32,
        #[arg(long, default_value = "./ado-images")]
        output_dir: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum TagCmd {
    /// List tags on a work item
    List { id: i32 },
    /// Add a tag (preserves existing tags; case-insensitive dedupe)
    Add { id: i32, tag: String },
    /// Remove a tag
    Remove { id: i32, tag: String },
    /// Replace all tags ("tag1; tag2")
    Set {
        id: i32,
        tags: String,
        #[arg(long)]
        dry_run: bool,
    },
}
