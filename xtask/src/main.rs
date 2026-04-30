// xtask — project automation entrypoint.
//
// Inspired by Cha (https://github.com/W-Mai/Cha)'s xtask. We don't need the
// full plugin/lsp/integration-test machinery here, just the two operations
// that matter for maintaining xbark:
//
//   cargo xtask bump <major|minor|patch>
//       Bump the workspace version across every Cargo.toml and
//       src-tauri/tauri.conf.json, then refresh Cargo.lock.
//
//   cargo xtask release
//       Guard-rail release: require clean working tree, push main to
//       origin, create vX.Y.Z tag, push the tag to trigger cargo-dist.
//
//   cargo xtask build
//       Production build shortcut (cargo build --release -p xbark).
//
//   cargo xtask check
//       Fast sanity: cargo fmt --check + cargo clippy.

use std::process::Command;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "bump" => {
            let level = args.get(1).map(|s| s.as_str()).unwrap_or("");
            cmd_bump(level)
        }
        "release" => cmd_release(),
        "build" => cmd_build(),
        "check" => cmd_check(),
        _ => {
            eprintln!("usage: cargo xtask <bump <major|minor|patch> | release | build | check>");
            std::process::exit(1);
        }
    }
}

// ---------------- bump ----------------

fn cmd_bump(level: &str) -> Result {
    if !matches!(level, "major" | "minor" | "patch") {
        return Err("usage: cargo xtask bump <major|minor|patch>".into());
    }
    let root = project_root();
    let current = read_workspace_version(&root)?;
    let next = bump_semver(&current, level)?;
    println!("  → bumping {current} → {next}");

    // 1. Workspace Cargo.toml
    rewrite_version_in_file(&format!("{root}/Cargo.toml"), &current, &next)?;

    // 2. tauri.conf.json (has its own version field)
    rewrite_tauri_conf_version(&format!("{root}/src-tauri/tauri.conf.json"), &next)?;

    // 3. Refresh Cargo.lock so dependents see the new version
    println!("  → refreshing Cargo.lock");
    let status = Command::new("cargo")
        .args(["update", "--workspace"])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("cargo update --workspace failed".into());
    }

    println!("  ✅ bumped to {next}");
    println!();
    println!("  next steps:");
    println!("    git add -A");
    println!("    git commit -m \"🔖(release): bump to {next}\"");
    println!("    cargo xtask release");
    Ok(())
}

fn read_workspace_version(root: &str) -> Result<String> {
    let content = std::fs::read_to_string(format!("{root}/Cargo.toml"))?;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("version =") && !t.contains("workspace") {
            if let Some(v) = t.split('"').nth(1) {
                return Ok(v.to_string());
            }
        }
    }
    Err("could not find version in workspace Cargo.toml".into())
}

fn rewrite_version_in_file(path: &str, current: &str, next: &str) -> Result {
    let content = std::fs::read_to_string(path)?;
    let mut changed = false;
    let updated = content
        .lines()
        .map(|line| {
            let t = line.trim();
            if t.starts_with("version =")
                && !t.contains("workspace")
                && line.contains(&format!("\"{current}\""))
            {
                changed = true;
                line.replacen(&format!("\"{current}\""), &format!("\"{next}\""), 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let final_ = if content.ends_with('\n') {
        format!("{updated}\n")
    } else {
        updated
    };
    if changed {
        std::fs::write(path, final_)?;
        println!("  → updated {path}");
    }
    Ok(())
}

fn rewrite_tauri_conf_version(path: &str, next: &str) -> Result {
    if !std::path::Path::new(path).exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    let mut changed = false;
    let updated = content
        .lines()
        .map(|line| {
            let t = line.trim();
            if t.starts_with("\"version\":") {
                changed = true;
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                // Preserve trailing comma if present
                let has_comma = t.ends_with(',');
                let tail = if has_comma { "," } else { "" };
                format!("{indent}\"version\": \"{next}\"{tail}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let final_ = if content.ends_with('\n') {
        format!("{updated}\n")
    } else {
        updated
    };
    if changed {
        std::fs::write(path, final_)?;
        println!("  → updated {path}");
    }
    Ok(())
}

fn bump_semver(version: &str, level: &str) -> Result<String> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|p| {
            p.parse::<u64>()
                .map_err(|e| format!("invalid version: {e}"))
        })
        .collect::<std::result::Result<_, _>>()?;
    if parts.len() != 3 {
        return Err(format!("expected semver x.y.z, got {version}").into());
    }
    let (major, minor, patch) = (parts[0], parts[1], parts[2]);
    Ok(match level {
        "major" => format!("{}.0.0", major + 1),
        "minor" => format!("{major}.{}.0", minor + 1),
        "patch" => format!("{major}.{minor}.{}", patch + 1),
        _ => unreachable!(),
    })
}

// ---------------- release ----------------

fn cmd_release() -> Result {
    let root = project_root();

    // 1. Clean working tree check
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output()?;
    if !out.stdout.is_empty() {
        eprintln!("working tree is not clean:");
        eprintln!("{}", String::from_utf8_lossy(&out.stdout));
        return Err("commit or stash changes first".into());
    }

    // 2. Must be on main
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&root)
        .output()?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch != "main" {
        return Err(format!("release must happen on main, currently on {branch}").into());
    }

    // 3. Read version
    let version = read_workspace_version(&root)?;
    let tag = format!("v{version}");

    // 4. Tag mustn't already exist
    let out = Command::new("git")
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .current_dir(&root)
        .output()?;
    if out.status.success() {
        return Err(format!("tag {tag} already exists — bump version first").into());
    }

    println!("  → releasing {tag}");
    println!();

    // 5. Confirm with user. Skip when running in CI (no tty).
    if std::env::var("CI").is_err() && atty_like_stdin() {
        println!("  This will:");
        println!("    1. push main to origin");
        println!("    2. create annotated tag {tag}");
        println!("    3. push tag to origin (triggers cargo-dist release workflow)");
        println!();
        print!("  proceed? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
            return Err("aborted".into());
        }
    }

    // 6. Push main
    println!("  → git push origin main");
    let status = Command::new("git")
        .args(["push", "origin", "main"])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("git push main failed".into());
    }

    // 7. Tag
    let message = format!("xbark {version}");
    println!("  → git tag -a {tag}");
    let status = Command::new("git")
        .args(["tag", "-a", &tag, "-m", &message])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("git tag failed".into());
    }

    // 8. Push tag
    println!("  → git push origin {tag}");
    let status = Command::new("git")
        .args(["push", "origin", &tag])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("git push tag failed".into());
    }

    println!();
    println!("  ✅ tagged {tag} and pushed.");
    println!("  watch the release workflow:");
    println!("    https://github.com/W-Mai/xBark/actions");
    Ok(())
}

fn atty_like_stdin() -> bool {
    // Cheap heuristic — we don't want to pull in a full dep just for this.
    // If stdin is connected to something, assume interactive.
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

// ---------------- build ----------------

fn cmd_build() -> Result {
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "xbark"])
        .current_dir(project_root())
        .status()?;
    if !status.success() {
        return Err("cargo build failed".into());
    }
    println!("  ✅ release binary at target/release/xbark");
    Ok(())
}

// ---------------- check ----------------

fn cmd_check() -> Result {
    let root = project_root();

    println!("=== fmt --check ===");
    let status = Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("cargo fmt check failed — run `cargo fmt --all`".into());
    }

    println!("=== clippy ===");
    let status = Command::new("cargo")
        .args(["clippy", "--workspace", "--", "-D", "warnings"])
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err("clippy failed".into());
    }

    println!("  ✅ all checks passed");
    Ok(())
}

// ---------------- helpers ----------------

fn project_root() -> String {
    // xtask Cargo.toml lives at <root>/xtask/Cargo.toml. CARGO_MANIFEST_DIR
    // at runtime points to <root>/xtask, so we parent up one.
    std::env::var("CARGO_MANIFEST_DIR")
        .map(|d| {
            std::path::Path::new(&d)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|_| ".".to_string())
}
