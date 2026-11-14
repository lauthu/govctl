//! Repository-authoritative control publication — [[RFC-0012:C-CLI]].
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Clone, Debug)]
pub(crate) enum CoordCommand {
    /// Discover cross-clone coordination capabilities
    Capabilities,
    /// Read requests and ownership from canonical main
    List {
        #[arg(long)]
        context_file: PathBuf,
    },
    /// Conditionally publish one signed control operation from JSON stdin
    Apply {
        #[arg(long)]
        context_file: PathBuf,
        #[arg(long)]
        sign_key_file: PathBuf,
        #[arg(long, required = true)]
        stdin: bool,
    },
}
