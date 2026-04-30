// Sticker keyword resolver.
// Reads _meta.json with bilingual fields and scores each sticker against
// the input keyword, returning the best match.
//
// Matching philosophy
// -------------------
// Input normalisation: lowercase + split on whitespace/-/_//'/ → token set.
// The same normalisation is applied to all match targets. Scoring prefers:
//
//   1. Exact filename hit (incl. with and without extension)         100
//   2. Exact aiName hit (en or zh)                                    90
//   3. Keyword verbatim substring of aiName (either lang)             75
//   4. Exact tag hit (en or zh)                                       70
//   5. Keyword verbatim substring of any tag                          55
//   6. Full token-set coverage on aiName token-split                  45
//   7. Full token-set coverage on description token-split             35
//   8. Partial token-set coverage on tags (ratio * 25)               ≤25
//
// Whichever sticker wins with score > 0 gets returned. Ties → stable
// sort by filename so behaviour is deterministic.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// A string that can be monolingual or bilingual.
/// JSON acceptable shapes:
///
///   "..."                                  → bare string
///   {"en": "...", "zh": "..."}             → both
///   {"zh": "..."} / {"en": "..."}          → one side only
///
/// When a bare string is given, the caller's `.en()` / `.zh()` accessors
/// decide which language it belongs to — see `EnStr` / `ZhStr` wrappers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum Bilingual {
    /// Bare string. Language is disambiguated by wrapper type at the
    /// StickerMeta level so old monolingual entries still work.
    Mono(String),
    /// Explicit bilingual.
    Pair {
        #[serde(default)]
        en: String,
        #[serde(default)]
        zh: String,
    },
    #[default]
    #[serde(skip)]
    Empty,
}

/// Wrapper: a Bilingual whose Mono form defaults to English.
/// Used for aiName — historically the ai_name field has always been English.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct EnBias(pub Bilingual);

impl EnBias {
    pub fn en(&self) -> &str {
        match &self.0 {
            Bilingual::Mono(s) => s,
            Bilingual::Pair { en, .. } => en,
            Bilingual::Empty => "",
        }
    }
    pub fn zh(&self) -> &str {
        match &self.0 {
            Bilingual::Mono(_) => "",
            Bilingual::Pair { zh, .. } => zh,
            Bilingual::Empty => "",
        }
    }
    pub fn both(&self) -> Vec<&str> {
        [self.en(), self.zh()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Wrapper: a Bilingual whose Mono form defaults to Chinese.
/// Used for description. (All 82 bundled descriptions were originally zh.)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ZhBias(pub Bilingual);

impl ZhBias {
    pub fn en(&self) -> &str {
        match &self.0 {
            Bilingual::Mono(_) => "",
            Bilingual::Pair { en, .. } => en,
            Bilingual::Empty => "",
        }
    }
    pub fn zh(&self) -> &str {
        match &self.0 {
            Bilingual::Mono(s) => s,
            Bilingual::Pair { zh, .. } => zh,
            Bilingual::Empty => "",
        }
    }
    pub fn both(&self) -> Vec<&str> {
        [self.en(), self.zh()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()
    }
}

/// Bilingual tag list. Bare array defaults to zh.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum BilingualTags {
    Mono(Vec<String>),
    Pair {
        #[serde(default)]
        en: Vec<String>,
        #[serde(default)]
        zh: Vec<String>,
    },
    #[default]
    #[serde(skip)]
    Empty,
}

impl BilingualTags {
    pub fn en(&self) -> &[String] {
        match self {
            BilingualTags::Mono(_) => &[],
            BilingualTags::Pair { en, .. } => en,
            BilingualTags::Empty => &[],
        }
    }
    pub fn zh(&self) -> &[String] {
        match self {
            BilingualTags::Mono(v) => v,
            BilingualTags::Pair { zh, .. } => zh,
            BilingualTags::Empty => &[],
        }
    }
    pub fn iter_all(&self) -> impl Iterator<Item = &String> {
        self.en().iter().chain(self.zh().iter())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerMeta {
    pub filename: String,
    #[serde(rename = "aiName", default)]
    pub ai_name: EnBias,
    #[serde(default)]
    pub description: ZhBias,
    #[serde(default)]
    pub tags: BilingualTags,
}

pub struct Resolver {
    sticker_dir: PathBuf,
    meta: RwLock<HashMap<String, StickerMeta>>,
}

// ---- Normalisation helpers ----

/// Lowercased token stream. Separators: whitespace, - _ / , . : ; '
fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| {
            c.is_whitespace()
                || matches!(c, '-' | '_' | '/' | ',' | '.' | ':' | ';' | '\'' | '"')
        })
        .filter(|t| !t.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn norm_lc(s: &str) -> String {
    s.to_lowercase()
}

// ---- Resolver ----

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

    /// Resolve a keyword to a sticker by running the scorer and returning
    /// the highest-scoring entry (score > 0). Ties broken by filename.
    pub fn resolve(&self, keyword: &str) -> Option<StickerMeta> {
        let meta = self.meta.read().ok()?;
        if meta.is_empty() {
            return None;
        }

        let kw_lc = norm_lc(keyword);
        let kw_tokens: Vec<String> = tokens(keyword);

        let mut best: Option<(i32, &StickerMeta)> = None;
        // Collect scores, deterministic tie-breaking by filename
        let mut scored: Vec<(i32, &StickerMeta)> = meta
            .values()
            .map(|m| (score_sticker(m, keyword, &kw_lc, &kw_tokens), m))
            .filter(|(s, _)| *s > 0)
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| a.1.filename.cmp(&b.1.filename))
        });

        if let Some(first) = scored.first() {
            best = Some((first.0, first.1));
            tracing::debug!(
                "resolve '{}' → {} (score={})",
                keyword,
                first.1.filename,
                first.0
            );
        }

        best.map(|(_, m)| m.clone())
    }

    /// List all stickers, optionally filtered. The filter applies the same
    /// scoring as resolve() but returns anything with score > 0.
    pub fn list(&self, filter: Option<&str>) -> Vec<StickerMeta> {
        let meta = match self.meta.read() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut items: Vec<StickerMeta> = if let Some(f) = filter {
            let f_lc = norm_lc(f);
            let f_tokens = tokens(f);
            meta.values()
                .filter(|m| score_sticker(m, f, &f_lc, &f_tokens) > 0)
                .cloned()
                .collect()
        } else {
            meta.values().cloned().collect()
        };
        items.sort_by(|a, b| a.filename.cmp(&b.filename));
        items
    }

    pub fn resolve_path(&self, meta: &StickerMeta) -> PathBuf {
        self.sticker_dir.join(&meta.filename)
    }
}

/// Score a sticker against an input keyword. Higher = better match.
/// See module doc for the scoring table.
fn score_sticker(m: &StickerMeta, keyword: &str, kw_lc: &str, kw_tokens: &[String]) -> i32 {
    // 1. Exact filename
    if m.filename == keyword {
        return 100;
    }
    // filename without extension exact
    if let Some(stem) = m.filename.rsplit_once('.').map(|(s, _)| s) {
        if stem == keyword {
            return 100;
        }
    }

    // Collect match targets in one pass for efficiency
    let ainame_variants = m.ai_name.both();
    let description_variants = m.description.both();
    let tags_all: Vec<&String> = m.tags.iter_all().collect();

    let mut best = 0i32;

    // 2. Exact aiName match (case-insensitive)
    for n in &ainame_variants {
        if norm_lc(n) == kw_lc {
            best = best.max(90);
        }
    }

    // 3. keyword verbatim substring in aiName
    for n in &ainame_variants {
        if norm_lc(n).contains(kw_lc) {
            best = best.max(75);
        }
    }

    // 4. Exact tag hit
    for t in &tags_all {
        if norm_lc(t) == kw_lc {
            best = best.max(70);
        }
    }

    // 5. keyword substring of tag
    for t in &tags_all {
        if norm_lc(t).contains(kw_lc) {
            best = best.max(55);
        }
    }

    // 6. Full token coverage of aiName's token-split
    if !kw_tokens.is_empty() {
        for n in &ainame_variants {
            let n_tokens = tokens(n);
            if !n_tokens.is_empty() && kw_tokens.iter().all(|t| n_tokens.contains(t)) {
                best = best.max(45);
            }
        }
    }

    // 7. Full token coverage inside description (substring over joined desc)
    if !kw_tokens.is_empty() {
        for d in &description_variants {
            let d_lc = norm_lc(d);
            if kw_tokens.iter().all(|t| d_lc.contains(t)) {
                best = best.max(35);
            }
        }
    }

    // 8. Partial token coverage across tags (en-side mostly)
    if !kw_tokens.is_empty() {
        let all_tag_tokens: Vec<String> = tags_all
            .iter()
            .flat_map(|t| tokens(t))
            .collect();
        if !all_tag_tokens.is_empty() {
            let hits = kw_tokens.iter().filter(|t| all_tag_tokens.contains(*t)).count();
            if hits > 0 {
                let ratio = hits as f32 / kw_tokens.len() as f32;
                let partial_score = (ratio * 25.0) as i32;
                best = best.max(partial_score);
            }
        }
    }

    // Also: keyword substring anywhere in description
    if !kw_lc.is_empty() {
        for d in &description_variants {
            if norm_lc(d).contains(kw_lc) {
                best = best.max(20);
            }
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticker(name: &str, ai_en: &str, ai_zh: &str, desc_en: &str, desc_zh: &str, tags_en: &[&str], tags_zh: &[&str]) -> StickerMeta {
        StickerMeta {
            filename: format!("{}.jpg", name),
            ai_name: EnBias(Bilingual::Pair { en: ai_en.into(), zh: ai_zh.into() }),
            description: ZhBias(Bilingual::Pair { en: desc_en.into(), zh: desc_zh.into() }),
            tags: BilingualTags::Pair {
                en: tags_en.iter().map(|s| s.to_string()).collect(),
                zh: tags_zh.iter().map(|s| s.to_string()).collect(),
            },
        }
    }

    fn make_resolver(s: StickerMeta) -> Resolver {
        let r = Resolver::new(PathBuf::from("/tmp"));
        let mut map = HashMap::new();
        map.insert(s.filename.clone(), s);
        *r.meta.write().unwrap() = map;
        r
    }

    #[test]
    fn exact_filename() {
        let r = make_resolver(sticker("hi", "smiling", "微笑", "", "", &[], &[]));
        assert!(r.resolve("hi.jpg").is_some());
        assert!(r.resolve("hi").is_some());
    }

    #[test]
    fn exact_ai_name() {
        let r = make_resolver(sticker("x", "smiling-man-thumbs-up", "竖大拇指", "", "", &[], &[]));
        assert!(r.resolve("smiling-man-thumbs-up").is_some());
        assert!(r.resolve("竖大拇指").is_some());
    }

    #[test]
    fn substring_ai_name() {
        let r = make_resolver(sticker("x", "smiling-man-thumbs-up", "", "", "", &[], &[]));
        assert!(r.resolve("thumbs-up").is_some());
        assert!(r.resolve("man").is_some());
    }

    #[test]
    fn tokens_in_ai_name() {
        let r = make_resolver(sticker("x", "smiling-man-thumbs-up", "", "", "", &[], &[]));
        // multi-word space-separated should still match
        assert!(r.resolve("smiling thumbs up").is_some());
        assert!(r.resolve("Thumbs Up").is_some());
    }

    #[test]
    fn case_insensitive() {
        let r = make_resolver(sticker("x", "smiling", "", "", "", &["Like"], &[]));
        assert!(r.resolve("SMILING").is_some());
        assert!(r.resolve("like").is_some());
        assert!(r.resolve("LIKE").is_some());
    }

    #[test]
    fn chinese_tag() {
        let r = make_resolver(sticker("x", "", "", "", "", &[], &["点赞", "认可"]));
        assert!(r.resolve("点赞").is_some());
        assert!(r.resolve("认可").is_some());
    }

    #[test]
    fn no_match_returns_none() {
        let r = make_resolver(sticker("x", "smiling", "", "", "", &[], &[]));
        assert!(r.resolve("不存在的关键词").is_none());
        assert!(r.resolve("xxxyyyzzz").is_none());
    }

    #[test]
    fn mono_backward_compat() {
        // Old format: aiName bare string (en), description bare (zh), tags bare array (zh)
        let json = r#"{
            "x.jpg": {
                "filename": "x.jpg",
                "aiName": "smiling",
                "description": "男子面带笑容",
                "tags": ["点赞", "认可"]
            }
        }"#;
        let parsed: HashMap<String, StickerMeta> = serde_json::from_str(json).unwrap();
        let m = &parsed["x.jpg"];
        // aiName is en-biased (historical)
        assert_eq!(m.ai_name.en(), "smiling");
        assert_eq!(m.ai_name.zh(), "");
        // description is zh-biased
        assert_eq!(m.description.en(), "");
        assert_eq!(m.description.zh(), "男子面带笑容");
        assert_eq!(m.tags.zh(), &["点赞".to_string(), "认可".to_string()]);
    }

    #[test]
    fn new_bilingual_format() {
        let json = r#"{
            "x.jpg": {
                "filename": "x.jpg",
                "aiName": {"en": "smiling-man", "zh": "微笑的男子"},
                "description": {"en": "Man smiling", "zh": "男子面带笑容"},
                "tags": {"en": ["smile", "like"], "zh": ["微笑", "点赞"]}
            }
        }"#;
        let parsed: HashMap<String, StickerMeta> = serde_json::from_str(json).unwrap();
        let m = &parsed["x.jpg"];
        assert_eq!(m.ai_name.en(), "smiling-man");
        assert_eq!(m.ai_name.zh(), "微笑的男子");
        assert_eq!(m.description.en(), "Man smiling");
        assert_eq!(m.description.zh(), "男子面带笑容");
        assert_eq!(m.tags.en(), &["smile".to_string(), "like".to_string()]);
        assert_eq!(m.tags.zh(), &["微笑".to_string(), "点赞".to_string()]);
    }
}
