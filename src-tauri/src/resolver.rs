// Sticker keyword resolver.
// Loads _meta.json from sticker dir and offers fuzzy matching.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerMeta {
    pub filename: String,
    #[serde(rename = "aiName", default)]
    pub ai_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

pub struct Resolver {
    sticker_dir: PathBuf,
    meta: RwLock<HashMap<String, StickerMeta>>,
}

impl Resolver {
    pub fn new(sticker_dir: PathBuf) -> Self {
        Self {
            sticker_dir,
            meta: RwLock::new(HashMap::new()),
        }
    }

    pub fn reload(&self) -> Result<usize> {
        let meta_path = self.sticker_dir.join("_meta.json");
        if !meta_path.exists() {
            tracing::warn!("_meta.json not found at {:?}", meta_path);
            return Ok(0);
        }
        let text = fs::read_to_string(&meta_path)
            .with_context(|| format!("read {:?}", meta_path))?;
        let parsed: HashMap<String, StickerMeta> = serde_json::from_str(&text)
            .with_context(|| format!("parse {:?}", meta_path))?;
        let count = parsed.len();
        *self.meta.write().unwrap() = parsed;
        tracing::info!("loaded {} stickers from {:?}", count, meta_path);
        Ok(count)
    }

    pub fn sticker_dir(&self) -> &Path {
        &self.sticker_dir
    }

    /// Resolve a keyword to a sticker.
    /// Priority:
    ///   1. exact filename match
    ///   2. exact aiName match
    ///   3. any tag contains keyword (case-insensitive)
    ///   4. description contains keyword (case-insensitive)
    pub fn resolve(&self, keyword: &str) -> Option<StickerMeta> {
        let meta = self.meta.read().ok()?;
        if meta.is_empty() {
            return None;
        }

        // 1. exact filename
        if let Some(m) = meta.get(keyword) {
            return Some(m.clone());
        }

        // 2. exact aiName
        if let Some(m) = meta.values().find(|m| m.ai_name == keyword) {
            return Some(m.clone());
        }

        let lower = keyword.to_lowercase();

        // 3. tag fuzzy
        if let Some(m) = meta.values().find(|m| {
            m.tags
                .iter()
                .any(|t| t.to_lowercase().contains(&lower))
        }) {
            return Some(m.clone());
        }

        // 4. description fuzzy
        if let Some(m) = meta
            .values()
            .find(|m| m.description.to_lowercase().contains(&lower))
        {
            return Some(m.clone());
        }

        None
    }

    /// List all stickers, optionally filtered.
    pub fn list(&self, filter: Option<&str>) -> Vec<StickerMeta> {
        let meta = match self.meta.read() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut items: Vec<StickerMeta> = meta.values().cloned().collect();
        if let Some(f) = filter {
            let lf = f.to_lowercase();
            items.retain(|m| {
                m.filename.to_lowercase().contains(&lf)
                    || m.ai_name.to_lowercase().contains(&lf)
                    || m.description.to_lowercase().contains(&lf)
                    || m.tags.iter().any(|t| t.to_lowercase().contains(&lf))
            });
        }
        items.sort_by(|a, b| a.filename.cmp(&b.filename));
        items
    }

    pub fn resolve_path(&self, meta: &StickerMeta) -> PathBuf {
        self.sticker_dir.join(&meta.filename)
    }
}
