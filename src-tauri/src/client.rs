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
    println!(
        "daemon still running after SIGTERM, check manually (pid {})",
        pid
    );
    Ok(())
}

pub fn clear() -> Result<()> {
    let _ = rt().block_on(http_post("/clear", serde_json::json!({})))?;
    println!("cleared");
    Ok(())
}

/// Resolved display language preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Zh,
    En,
    Both,
}

/// Resolve `--lang` argument, with `auto` falling back to locale detection.
///
/// We peek at LC_ALL / LC_MESSAGES / LANG env vars and treat anything that
/// starts with `zh` (e.g. `zh_CN.UTF-8`, `zh-Hans`) as Chinese. Otherwise
/// default to English.
fn resolve_lang(arg: &str) -> Lang {
    match arg {
        "zh" => Lang::Zh,
        "en" => Lang::En,
        "both" => Lang::Both,
        "auto" | "" => {
            let locale = std::env::var("LC_ALL")
                .or_else(|_| std::env::var("LC_MESSAGES"))
                .or_else(|_| std::env::var("LANG"))
                .unwrap_or_default()
                .to_lowercase();
            if locale.starts_with("zh") {
                Lang::Zh
            } else {
                Lang::En
            }
        }
        other => {
            eprintln!(
                "warning: unknown --lang value '{}', falling back to auto",
                other
            );
            resolve_lang("auto")
        }
    }
}

/// Measured display width of a string, counting CJK characters as 2 columns
/// (the same convention most monospace terminals use for rendering).
fn display_width(s: &str) -> usize {
    s.chars()
        .fold(0usize, |acc, c| acc + if is_wide_char(c) { 2 } else { 1 })
}

/// East Asian Wide / Fullwidth approximation — good enough for sticker
/// descriptions and tags without pulling in a whole unicode-width crate.
fn is_wide_char(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F |        // Hangul Jamo
        0x2E80..=0x303E |        // CJK Radicals, Kangxi
        0x3041..=0x33FF |        // Hiragana, Katakana, CJK
        0x3400..=0x4DBF |        // CJK Unified Ideographs Ext A
        0x4E00..=0x9FFF |        // CJK Unified Ideographs
        0xA000..=0xA4CF |        // Yi
        0xAC00..=0xD7A3 |        // Hangul Syllables
        0xF900..=0xFAFF |        // CJK Compatibility Ideographs
        0xFE30..=0xFE4F |        // CJK Compatibility Forms
        0xFF00..=0xFF60 |        // Fullwidth ASCII
        0xFFE0..=0xFFE6 |        // Fullwidth signs
        0x20000..=0x2FFFD |      // CJK Ext B–F
        0x30000..=0x3FFFD        // CJK Ext G
    )
}

/// Pad `s` on the right with spaces so its display width is `width`.
/// If `s` is already wider, we truncate with a single-column ellipsis.
fn pad_right(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w <= width {
        let mut out = s.to_string();
        out.push_str(&" ".repeat(width - w));
        out
    } else {
        // truncate, leaving room for ellipsis
        let target = width.saturating_sub(1);
        let mut acc = 0usize;
        let mut out = String::new();
        for c in s.chars() {
            let cw = if is_wide_char(c) { 2 } else { 1 };
            if acc + cw > target {
                break;
            }
            out.push(c);
            acc += cw;
        }
        while display_width(&out) < target {
            out.push(' ');
        }
        out.push('…');
        out
    }
}

/// Truncate `s` to display width `width` (with trailing ellipsis if cut).
fn truncate_width(s: &str, width: usize) -> String {
    if display_width(s) <= width {
        return s.to_string();
    }
    let target = width.saturating_sub(1);
    let mut acc = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let cw = if is_wide_char(c) { 2 } else { 1 };
        if acc + cw > target {
            break;
        }
        out.push(c);
        acc += cw;
    }
    out.push('…');
    out
}

/// Current terminal width in columns. Falls back to 120 if unknown.
fn terminal_width() -> usize {
    // $COLUMNS is set by shells that care; honour it first
    if let Ok(s) = std::env::var("COLUMNS") {
        if let Ok(n) = s.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    // fallback via tput if it's available
    if let Ok(out) = std::process::Command::new("tput").arg("cols").output() {
        if out.status.success() {
            if let Ok(s) = std::str::from_utf8(&out.stdout) {
                if let Ok(n) = s.trim().parse::<usize>() {
                    if n > 0 {
                        return n;
                    }
                }
            }
        }
    }
    120
}

pub fn list(filter: Option<String>, lang: String, detail: bool) -> Result<()> {
    ensure_daemon_running()?;
    let path = match filter {
        Some(f) => format!("/stickers?filter={}", urlencoding::encode(&f)),
        None => "/stickers".to_string(),
    };
    let resp = rt().block_on(http_get(&path))?;
    let items = resp["items"].as_array().cloned().unwrap_or_default();

    if items.is_empty() {
        println!("(no stickers found)");
        return Ok(());
    }

    let lang = resolve_lang(&lang);

    // Extract rows: each row is a set of strings per column (filename, ai, tags, desc).
    struct Row {
        filename: String,
        ainame: String,
        tags: String,
        description: String,
    }

    fn pick(m: &serde_json::Value, field: &str, lang: Lang) -> String {
        let v = &m[field];
        // May be a bare string (old format), an {en, zh} object, or missing
        if let Some(s) = v.as_str() {
            return s.to_string();
        }
        let en = v["en"].as_str().unwrap_or("").to_string();
        let zh = v["zh"].as_str().unwrap_or("").to_string();
        match lang {
            Lang::Zh => {
                if !zh.is_empty() {
                    zh
                } else {
                    en
                }
            }
            Lang::En => {
                if !en.is_empty() {
                    en
                } else {
                    zh
                }
            }
            Lang::Both => {
                if !en.is_empty() && !zh.is_empty() {
                    format!("{} / {}", en, zh)
                } else if !en.is_empty() {
                    en
                } else {
                    zh
                }
            }
        }
    }

    fn pick_tags(m: &serde_json::Value, lang: Lang) -> String {
        let v = &m["tags"];
        // Bare array?
        if let Some(arr) = v.as_array() {
            return arr
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(",");
        }
        let en: Vec<&str> = v["en"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let zh: Vec<&str> = v["zh"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let picked: Vec<&str> = match lang {
            Lang::Zh => {
                if !zh.is_empty() {
                    zh
                } else {
                    en
                }
            }
            Lang::En => {
                if !en.is_empty() {
                    en
                } else {
                    zh
                }
            }
            Lang::Both => {
                let mut v = en.clone();
                v.extend(zh.iter());
                v
            }
        };
        picked.join(",")
    }

    let rows: Vec<Row> = items
        .iter()
        .map(|m| Row {
            filename: m["filename"].as_str().unwrap_or("").to_string(),
            ainame: pick(m, "aiName", lang),
            tags: pick_tags(m, lang),
            description: pick(m, "description", lang),
        })
        .collect();

    // Compute column widths, capped per terminal width.
    let total_width = terminal_width();
    // Max raw widths
    let max_fn = rows
        .iter()
        .map(|r| display_width(&r.filename))
        .max()
        .unwrap_or(0);
    let max_ai = rows
        .iter()
        .map(|r| display_width(&r.ainame))
        .max()
        .unwrap_or(0);
    let max_tags = rows
        .iter()
        .map(|r| display_width(&r.tags))
        .max()
        .unwrap_or(0);

    // Hard cap per column to keep the table usable on normal terminals.
    let col_fn = max_fn.min(46);
    let col_ai = max_ai.min(28);
    let col_tags = max_tags.min(42);

    // Spacing: 2 spaces between columns.
    let spacer = "  ";
    let fixed_width = col_fn + spacer.len() + col_ai + spacer.len() + col_tags;
    let col_desc = if detail {
        total_width
            .saturating_sub(fixed_width + spacer.len())
            .max(20)
    } else {
        0
    };

    // Header
    let mut header = String::new();
    header.push_str(&pad_right("FILENAME", col_fn));
    header.push_str(spacer);
    header.push_str(&pad_right("AINAME", col_ai));
    header.push_str(spacer);
    header.push_str(&pad_right("TAGS", col_tags));
    if detail {
        header.push_str(spacer);
        header.push_str("DESCRIPTION");
    }
    println!("\x1b[1m{}\x1b[0m", header);

    // Separator
    let mut sep = String::new();
    sep.push_str(&"─".repeat(col_fn));
    sep.push_str(spacer);
    sep.push_str(&"─".repeat(col_ai));
    sep.push_str(spacer);
    sep.push_str(&"─".repeat(col_tags));
    if detail {
        sep.push_str(spacer);
        sep.push_str(&"─".repeat(col_desc));
    }
    println!("\x1b[2m{}\x1b[0m", sep);

    // Rows
    for r in &rows {
        let mut line = String::new();
        line.push_str(&pad_right(&r.filename, col_fn));
        line.push_str(spacer);
        line.push_str(&pad_right(&r.ainame, col_ai));
        line.push_str(spacer);
        line.push_str(&pad_right(&r.tags, col_tags));
        if detail && col_desc > 0 {
            line.push_str(spacer);
            line.push_str(&truncate_width(&r.description, col_desc));
        }
        println!("{}", line);
    }

    println!();
    let mode = match lang {
        Lang::Zh => "zh",
        Lang::En => "en",
        Lang::Both => "both",
    };
    println!(
        "\x1b[2m{} stickers · lang={} · tip: --lang <zh|en|both>, --detail for descriptions\x1b[0m",
        rows.len(),
        mode
    );
    Ok(())
}

pub fn autostart(action: AutostartAction) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        autostart_macos(action)
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

    let was_running = read_port().is_some();
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
