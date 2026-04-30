// Config loading from ~/.config/xbark/config.toml
// Missing fields get defaults, missing file is not an error.

use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directory containing sticker files and _meta.json.
    /// Defaults to $CONFIG_DIR/xbark/stickers/ (we also fall back to the
    /// bundled `stickers/` dir if that doesn't exist).
    pub sticker_dir: Option<PathBuf>,

    /// Default popup duration in seconds
    pub duration: f32,

    /// Default popup size in pixels (square)
    pub size: u32,

    /// Default anchor position (bottom-right|bottom-left|top-right|top-left|center|random)
    pub position: String,

    /// Max stickers visible simultaneously
    pub max_visible: usize,

    /// Gap between stacked stickers (pixels)
    pub gap: u32,

    /// Margin from screen edge (pixels)
    pub margin: u32,

    /// HTTP server port (0 = random)
    pub port: u16,

    /// Log file path (None = stderr only)
    pub log_file: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sticker_dir: None,
            duration: 2.0,
            size: 256,
            position: "bottom-right".to_string(),
            max_visible: 5,
            gap: 12,
            margin: 32,
            port: 0,
            log_file: None,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            tracing::info!("no config file at {:?}, using defaults", path);
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)?;
        let cfg: Self = toml::from_str(&text)?;
        tracing::info!("loaded config from {:?}", path);
        Ok(cfg)
    }

    pub fn config_dir() -> PathBuf {
        ProjectDirs::from("sh", "w-mai", "xbark")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| dirs_home().join(".config").join("xbark"))
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Resolve the effective sticker dir.
    /// Priority:
    ///   1. config.sticker_dir if set and exists
    ///   2. $XDG_CONFIG_HOME/xbark/stickers/ (user-managed)
    ///   3. stickers/ next to the REAL executable (after resolving symlinks)
    ///   4. dev fallback: ../../../stickers from target/release/xbark
    pub fn resolve_sticker_dir(&self) -> PathBuf {
        if let Some(p) = &self.sticker_dir {
            if p.exists() {
                return p.clone();
            }
            tracing::warn!("configured sticker_dir does not exist: {:?}", p);
        }

        let user_dir = Self::config_dir().join("stickers");
        if user_dir.exists() {
            return user_dir;
        }

        // Resolve symlinks so ~/.bun/bin/xbark -> repo/target/release/xbark works
        if let Ok(exe) = std::env::current_exe() {
            let real_exe = exe.canonicalize().unwrap_or(exe);
            if let Some(parent) = real_exe.parent() {
                // bundled right next to the binary (distribution layout)
                let bundled = parent.join("stickers");
                if bundled.exists() {
                    return bundled;
                }
                // dev layout: <repo>/target/release/xbark, walk up 2 levels
                let maybe_repo = parent.parent().and_then(|p| p.parent());
                if let Some(root) = maybe_repo {
                    let dev = root.join("stickers");
                    if dev.exists() {
                        return dev;
                    }
                }
            }
        }

        // Last-resort fallback
        Self::config_dir().join("stickers")
    }
}

fn dirs_home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/"))
}
