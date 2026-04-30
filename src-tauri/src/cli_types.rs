// Types shared between main.rs (clap parsing) and client.rs (handlers)

use clap::Subcommand;

#[derive(Subcommand, Clone, Copy, Debug)]
pub enum AutostartAction {
    /// Install launchd plist, daemon starts at login
    Install,
    /// Remove launchd plist
    Uninstall,
    /// Show autostart status
    Status,
}
