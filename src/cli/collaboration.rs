//! Attached collaboration commands; these are not a fourth business entity.
use super::OutputFormat;
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
pub(crate) struct CollabTargetArgs {
    /// Existing RFC, ADR or work-item ID
    pub id: String,
    /// Stable repository identity (for example owner/repository)
    #[arg(long)]
    pub repo_id: String,
    #[arg(short = 'o', long, value_enum, default_value = "json")]
    pub output: OutputFormat,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CollabListArgs {
    /// Existing RFC, ADR or work-item ID
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    pub id: Option<String>,
    /// Read the complete project in one request
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub repo_id: String,
    /// Explicitly trusted Ed25519 public key; repeat for multiple issuers
    #[arg(long)]
    pub trust_key_file: Vec<PathBuf>,
    #[arg(short = 'o', long, value_enum, default_value = "json")]
    pub output: OutputFormat,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CollabAppendArgs {
    #[command(flatten)]
    pub target: CollabTargetArgs,
    /// Read one bounded event envelope from stdin
    #[arg(long, required = true)]
    pub stdin: bool,
    /// Preserve a historical revision without making it a current approval
    #[arg(long)]
    pub allow_stale: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CollabAddArgs {
    #[command(flatten)]
    pub target: CollabTargetArgs,
    /// comment, review, authorship or resolution
    #[arg(long = "type", value_parser = ["comment", "review", "authorship", "resolution"])]
    pub record_type: String,
    #[arg(long)]
    pub actor_file: PathBuf,
    #[arg(long)]
    pub payload_file: PathBuf,
    #[arg(long)]
    pub idempotency_key: String,
    #[arg(long)]
    pub target_file: Option<PathBuf>,
    /// Local private Ed25519 PKCS#8 key; never copied into the repository
    #[arg(long)]
    pub sign_key_file: Option<PathBuf>,
    #[arg(long)]
    pub allow_stale: bool,
}

#[derive(Subcommand, Clone, Debug)]
pub(crate) enum CollabCommand {
    /// Discover the portable collaboration protocol
    Capabilities {
        #[arg(short = 'o', long, value_enum, default_value = "json")]
        output: OutputFormat,
    },
    /// Compute an exact source target (collaboration records are excluded)
    Target(CollabTargetArgs),
    /// Read attached records, authors and review state
    List(CollabListArgs),
    /// Render the discussion and review state without an external client
    Show(CollabListArgs),
    /// Validate and append an existing event without changing its identity
    Append(CollabAppendArgs),
    /// Create an attributed comment, review, authorship or resolution
    Add(CollabAddArgs),
}
impl CollabCommand {
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::Capabilities { .. } | Self::Target(_) | Self::List(_) | Self::Show(_)
        )
    }
}
