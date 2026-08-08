use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "dottyenv", version, about = "Validate .env files against a declared schema")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Env file to read
    #[arg(short, long, global = true, default_value = ".env")]
    pub file: PathBuf,

    /// Schema file to validate against
    #[arg(short, long, global = true, default_value = "dottyenv.toml")]
    pub schema: PathBuf,

    /// Emit machine-readable JSON on stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-error output
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate a schema from an existing .env file
    Init {
        /// Overwrite an existing schema file
        #[arg(long)]
        force: bool,
    },

    /// Validate the env file against the schema
    Check,

    /// Scan the codebase for env vars used but not declared
    Scan,

    /// List declared variables and whether each is set
    List,
}
