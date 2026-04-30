// xBark — Desktop sticker popup daemon
// Main entry: CLI dispatcher for subcommands

use anyhow::Result;
use clap::{Parser, Subcommand};

mod assets;
mod cli_types;
mod client;
mod config;
mod daemon;
mod discovery;
mod overlay;
mod resolver;
mod server;

use cli_types::AutostartAction;

#[derive(Parser)]
#[command(
    name = "xbark",
    version,
    about = "Desktop sticker popup daemon",
    long_about = "xBark — fire a desktop sticker popup from anywhere.\n\nRun without arguments for an interactive tour."
)]
struct Cli {
    /// Subcommand. If omitted, runs `welcome` (interactive tour).
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive first-run walkthrough
    Welcome,
    /// Run the xBark daemon (foreground)
    Daemon {
        /// Port to listen on (0 = random, default from config)
        #[arg(long)]
        port: Option<u16>,
        /// Debug mode: visible opaque window + devtools
        #[arg(long)]
        debug: bool,
    },
    /// Send a sticker by keyword or filename
    Send {
        /// Sticker keyword, aiName, tag, or filename
        keyword: String,
        /// Override display duration in seconds
        #[arg(long)]
        duration: Option<f32>,
        /// Override size in pixels
        #[arg(long)]
        size: Option<u32>,
        /// Override position: bottom-right|bottom-left|top-right|top-left|center|random
        #[arg(long)]
        position: Option<String>,
    },
    /// Check daemon status
    Status,
    /// Stop running daemon
    Stop,
    /// Clear all currently visible stickers
    Clear,
    /// List all available stickers
    List {
        /// Filter by keyword (matches filename/aiName/tag/description)
        #[arg(long)]
        filter: Option<String>,
        /// Language preference for displayed fields.
        /// Defaults to "auto" which picks zh for zh* locales, en otherwise.
        #[arg(long, default_value = "auto")]
        lang: String,
        /// Show the description column (hidden by default for a compact view)
        #[arg(long)]
        detail: bool,
    },
    /// Manage launchd autostart (macOS only)
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("XBARK_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("xbark=info,warn")),
        )
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Welcome);

    match command {
        Command::Welcome => client::welcome(),
        Command::Daemon { port, debug } => {
            if debug {
                // Propagate as env var since overlay.rs checks XBARK_DEBUG
                // (keeping env var support so internal callers still work)
                std::env::set_var("XBARK_DEBUG", "1");
            }
            daemon::run(port)
        }
        Command::Send {
            keyword,
            duration,
            size,
            position,
        } => client::send(keyword, duration, size, position),
        Command::Status => client::status(),
        Command::Stop => client::stop(),
        Command::Clear => client::clear(),
        Command::List {
            filter,
            lang,
            detail,
        } => client::list(filter, lang, detail),
        Command::Autostart { action } => client::autostart(action),
    }
}
