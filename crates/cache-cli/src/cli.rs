use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "dcc", version, about = "Developer Computation Cache", long_about = None)]
pub struct Cli {
    #[arg(short, long, global = true, help = "Path to the cache root directory")]
    pub cache_dir: Option<PathBuf>,

    #[arg(long, global = true, help = "Output result in JSON format")]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(about = "Initialize local cache directory and configuration")]
    Init {
        #[arg(
            short,
            long,
            help = "Max cache size (e.g. '500 MB', '2 GB', '10 GB' or raw bytes)"
        )]
        max_size: Option<String>,
    },

    #[command(about = "Execute a computation with caching")]
    Run(RunArgs),

    #[command(about = "Inspect a cached computation key")]
    Inspect {
        #[arg(help = "The SHA-256 computation key to inspect")]
        key: String,
    },

    #[command(about = "Display cache statistics and storage metrics")]
    Stats,

    #[command(about = "Verify integrity of stored objects")]
    Verify,

    #[command(about = "Clean entire cache or delete specific keys")]
    Clean {
        #[arg(short, long, help = "Specific computation key to delete")]
        key: Option<String>,
    },

    #[command(about = "Prune unreferenced objects or enforce max cache size")]
    Prune {
        #[arg(
            long,
            help = "Enforce max size (e.g. '500 MB', '2 GB', '10 GB' or raw bytes)"
        )]
        max_size: Option<String>,

        #[arg(
            long,
            default_value = "lru",
            help = "Eviction strategy: lru (least recently used), fifo (oldest created), lfu (least frequently used)"
        )]
        strategy: String,

        #[arg(
            long,
            help = "Perform a dry run without deleting any entries or objects"
        )]
        dry_run: bool,
    },

    #[command(about = "Inspect or manage cache configuration")]
    Config {
        #[arg(
            long,
            help = "Display configuration setting (e.g. 'max_size', 'cache_dir')"
        )]
        get: Option<String>,
    },

    #[command(about = "Diagnose cache health, environment, and permissions")]
    Doctor,

    #[command(about = "Manage and maintain local computation cache (clean, prune, verify, stats)")]
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum CacheCommands {
    #[command(about = "Clean entire cache or delete specific keys")]
    Clean {
        #[arg(short, long, help = "Specific computation key to delete")]
        key: Option<String>,
    },

    #[command(about = "Prune unreferenced objects or enforce max cache size")]
    Prune {
        #[arg(
            long,
            help = "Enforce max size (e.g. '500 MB', '2 GB', '10 GB' or raw bytes)"
        )]
        max_size: Option<String>,

        #[arg(
            long,
            default_value = "lru",
            help = "Eviction strategy: lru (least recently used), fifo (oldest created), lfu (least frequently used)"
        )]
        strategy: String,

        #[arg(
            long,
            help = "Perform a dry run without deleting any entries or objects"
        )]
        dry_run: bool,
    },

    #[command(about = "Verify integrity of stored objects")]
    Verify,

    #[command(about = "Display cache statistics and storage metrics")]
    Stats,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(short, long = "input", action = clap::ArgAction::Append, help = "Declared input files")]
    pub inputs: Vec<String>,

    #[arg(short, long = "output", action = clap::ArgAction::Append, help = "Declared output files")]
    pub outputs: Vec<String>,

    #[arg(short, long = "env", action = clap::ArgAction::Append, help = "Declared environment variables (KEY=VAL or KEY)")]
    pub env: Vec<String>,

    #[arg(long = "op", default_value = "default", help = "Operation identifier")]
    pub operation: String,

    #[arg(long, help = "Explain cache miss reasons in detail")]
    pub explain: bool,

    #[arg(
        long,
        default_value = "read-write",
        help = "Cache policy: read-write, read-only, write-only, bypass, force-recompute"
    )]
    pub policy: String,

    #[arg(
        last = true,
        required = true,
        help = "Command and arguments to execute"
    )]
    pub command: Vec<String>,
}
