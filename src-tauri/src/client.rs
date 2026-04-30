// CLI client-side commands: send/status/stop/clear/list/autostart.
// Talks to the running daemon over HTTP (127.0.0.1:<port_file>).

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli_types::AutostartAction;
use crate::config::Config;

const PID_FILE_NAME: &str = "xbark.pid";
const PORT_FILE_NAME: &str = "xbark.port";

fn config_dir() -> PathBuf {
    Config::config_dir()
}

fn read_port() -> Option<u16> {
    std::fs::read_to_string(config_dir().join(PORT_FILE_NAME))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn read_pid() -> Option<u32> {
    std::fs::read_to_string(config_dir().join(PID_FILE_NAME))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

async fn http_get(path: &str) -> Result<serde_json::Value> {
    let port = read_port().context("daemon not running (no port file)")?;
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}{}", port, path))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?;
    Ok(resp.json().await?)
}

async fn http_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let port = read_port().context("daemon not running (no port file)")?;
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}{}", port, path))
        .json(&body)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?;
    Ok(resp.json().await?)
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn health_check(port: u16, timeout_ms: u64) -> bool {
    let client = reqwest::Client::new();
    match client
        .get(format!("http://127.0.0.1:{}/health", port))
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Auto-start daemon if not running, then wait until it's reachable.
fn ensure_daemon_running() -> Result<()> {
    if let Some(port) = read_port() {
        if rt().block_on(health_check(port, 500)) {
            return Ok(());
        }
    }

    // spawn daemon in background
    let exe = std::env::current_exe()?;
    std::process::Command::new(&exe)
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    // wait for up to 5s
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Some(port) = read_port() {
            if rt().block_on(health_check(port, 300)) {
                return Ok(());
            }
        }
    }
    anyhow::bail!("daemon failed to start within 5s");
}

pub fn send(
    keyword: String,
    duration: Option<f32>,
    size: Option<u32>,
    position: Option<String>,
) -> Result<()> {
    ensure_daemon_running()?;
    let body = serde_json::json!({
        "keyword": keyword,
        "duration": duration,
        "size": size,
        "position": position,
    });
    let resp = rt().block_on(http_post("/sticker", body))?;
    if resp["ok"].as_bool().unwrap_or(false) {
        if let Some(name) = resp["sticker"]["filename"].as_str() {
            println!("sent: {}", name);
        } else {
            println!("sent");
        }
        Ok(())
    } else {
        let err = resp["error"].as_str().unwrap_or("unknown error");
        anyhow::bail!("{}", err)
    }
}

pub fn status() -> Result<()> {
    let pid = read_pid();
    let port = read_port();
    match (pid, port) {
        (Some(pid), Some(port)) if is_alive(pid) => {
            // try health
            let resp = rt().block_on(http_get("/health"));
            match resp {
                Ok(v) => {
                    println!("daemon running");
                    println!("  pid:     {}", pid);
                    println!("  port:    {}", port);
                    println!("  version: {}", v["version"].as_str().unwrap_or("?"));
                }
                Err(_) => {
                    println!("daemon pid {} alive but HTTP not responding", pid);
                }
            }
        }
        _ => {
            println!("daemon not running");
        }
    }
    Ok(())
}

pub fn stop() -> Result<()> {
    let Some(pid) = read_pid() else {
        println!("daemon not running");
        return Ok(());
    };
    if !is_alive(pid) {
        println!("daemon not running (stale pid file)");
        let _ = std::fs::remove_file(config_dir().join(PID_FILE_NAME));
        let _ = std::fs::remove_file(config_dir().join(PORT_FILE_NAME));
        return Ok(());
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    // wait up to 3s
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !is_alive(pid) {
            println!("stopped (pid {})", pid);
            return Ok(());
        }
    }
    println!("daemon still running after SIGTERM, check manually (pid {})", pid);
    Ok(())
}

pub fn clear() -> Result<()> {
    let _ = rt().block_on(http_post("/clear", serde_json::json!({})))?;
    println!("cleared");
    Ok(())
}

pub fn list(filter: Option<String>) -> Result<()> {
    ensure_daemon_running()?;
    let path = match filter {
        Some(f) => format!("/stickers?filter={}", urlencoding::encode(&f)),
        None => "/stickers".to_string(),
    };
    let resp = rt().block_on(http_get(&path))?;
    let items = resp["items"].as_array().cloned().unwrap_or_default();
    for m in items {
        println!(
            "{:<40}  {:<30}  {}",
            m["filename"].as_str().unwrap_or(""),
            m["aiName"].as_str().unwrap_or(""),
            m["description"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

pub fn autostart(action: AutostartAction) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return autostart_macos(action);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = action;
        anyhow::bail!("autostart is only implemented for macOS right now");
    }
}

#[cfg(target_os = "macos")]
fn autostart_macos(action: AutostartAction) -> Result<()> {
    let label = "sh.w-mai.xbark";
    let plist_dir = directories::BaseDirs::new()
        .context("no home")?
        .home_dir()
        .join("Library/LaunchAgents");
    std::fs::create_dir_all(&plist_dir)?;
    let plist_path = plist_dir.join(format!("{}.plist", label));

    match action {
        AutostartAction::Install => {
            let exe = std::env::current_exe()?;
            let exe_str = exe.to_string_lossy();
            let log_out = config_dir().join("daemon.log");
            std::fs::create_dir_all(config_dir())?;
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#,
                label = label,
                exe = exe_str,
                log = log_out.to_string_lossy(),
            );
            std::fs::write(&plist_path, plist)?;
            // load (bootstrap in newer launchctl)
            let uid = unsafe { libc::getuid() };
            let _ = std::process::Command::new("launchctl")
                .args([
                    "bootout",
                    &format!("gui/{}", uid),
                    &plist_path.to_string_lossy(),
                ])
                .output();
            let out = std::process::Command::new("launchctl")
                .args([
                    "bootstrap",
                    &format!("gui/{}", uid),
                    &plist_path.to_string_lossy(),
                ])
                .output()?;
            if !out.status.success() {
                anyhow::bail!(
                    "launchctl bootstrap failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            println!("autostart installed: {:?}", plist_path);
        }
        AutostartAction::Uninstall => {
            let uid = unsafe { libc::getuid() };
            let _ = std::process::Command::new("launchctl")
                .args([
                    "bootout",
                    &format!("gui/{}", uid),
                    &plist_path.to_string_lossy(),
                ])
                .output();
            if plist_path.exists() {
                std::fs::remove_file(&plist_path)?;
            }
            println!("autostart removed");
        }
        AutostartAction::Status => {
            if plist_path.exists() {
                println!("installed: {:?}", plist_path);
            } else {
                println!("not installed");
            }
        }
    }
    Ok(())
}

/// Interactive first-run walkthrough.
///
/// Runs when the user types `xbark` with no subcommand. We:
///   1. Describe what xbark does in one paragraph.
///   2. Auto-spawn the daemon if it isn't running (that also unpacks
///      the bundled sticker pack on first use).
///   3. Fire a demo sticker so they see something immediately.
///   4. Print the 5 most useful commands.
pub fn welcome() -> Result<()> {
    println!();
    println!("  ✨ \x1b[1mxBark\x1b[0m — desktop sticker popups from anywhere");
    println!();
    println!("     A small daemon that draws its own popups: large, animated,");
    println!("     macOS-Spaces-aware, click-through. Talk to it over HTTP from");
    println!("     any script — or embed :sticker[keyword]: in AI replies.");
    println!();

    let was_running = matches!(read_port(), Some(_));
    if !was_running {
        println!("  → starting daemon for the first time…");
    }

    if let Err(e) = ensure_daemon_running() {
        eprintln!("  ✗ could not start daemon: {}", e);
        eprintln!();
        eprintln!("    you can retry with: xbark daemon --debug");
        return Ok(());
    }

    println!("  → sending you a sticker right now…");
    println!();

    // Fire a demo sticker. Swallow errors — it's just a greeting.
    let body = serde_json::json!({
        "keyword": "smiling-man-thumbs-up",
        "duration": 4.0,
    });
    let _ = rt().block_on(http_post("/sticker", body));

    println!("  👀 look at the bottom-right of your screen.");
    println!();
    println!("  \x1b[1mNext, try:\x1b[0m");
    println!();
    println!("    xbark send 点赞                   fire a sticker by keyword");
    println!("    xbark list                        browse all 82 bundled stickers");
    println!("    xbark send 拿捏 --duration 5      override display time");
    println!("    xbark autostart install           run at login (macOS)");
    println!("    xbark --help                      see every subcommand");
    println!();
    println!("  daemon is running on port {}.", read_port().unwrap_or(0));
    println!("  stop it anytime with: xbark stop");
    println!();
    Ok(())
}
