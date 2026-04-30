// Bundled sticker pack, embedded at compile time as a zstd-compressed tar.
//
// Layout:
//   <config_dir>/stickers/
//     _meta.json
//     *.{jpg,gif,png,webp}
//     .xbark-pack-version    <-- marks which bundle version is unpacked
//
// On daemon start we check whether the bundled version differs from the
// unpacked one, and unpack if needed. Users can override by pointing
// `sticker_dir` in config.toml somewhere else entirely (we never touch
// that path).

use anyhow::{Context, Result};
use std::fs;
use std::io::Cursor;
use std::path::Path;

/// zstd-compressed tarball of the bundled stickers/ directory.
/// Empty byte slice if stickers/ was absent at build time.
const BUNDLED_PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stickers.tar.zst"));

/// A stable "version" for the bundled pack — changes whenever the compressed
/// blob changes. We use a short hash so the marker file stays readable.
fn bundled_pack_fingerprint() -> String {
    // xxHash-style cheap mix. We don't need cryptographic integrity, just
    // a fingerprint that changes when the bundle changes.
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset
    for &b in BUNDLED_PACK {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{:016x}-{}", hash, BUNDLED_PACK.len())
}

const VERSION_MARKER: &str = ".xbark-pack-version";

/// Ensure the given directory contains the bundled sticker pack.
/// Unpacks if directory is empty OR the fingerprint differs.
/// Does nothing if the bundled pack is empty (e.g. built from a stripped source).
pub fn ensure_unpacked(target_dir: &Path) -> Result<UnpackOutcome> {
    if BUNDLED_PACK.is_empty() {
        return Ok(UnpackOutcome::NoBundle);
    }

    let fingerprint = bundled_pack_fingerprint();
    let marker_path = target_dir.join(VERSION_MARKER);

    // If marker matches, nothing to do.
    if let Ok(existing) = fs::read_to_string(&marker_path) {
        if existing.trim() == fingerprint {
            return Ok(UnpackOutcome::AlreadyCurrent);
        }
    }

    // If the target dir has content but no valid marker, respect the user's
    // customisations and don't overwrite.
    if target_dir.exists() && has_user_content(target_dir)? && !marker_path.exists() {
        return Ok(UnpackOutcome::UserOwnedSkipped);
    }

    fs::create_dir_all(target_dir)
        .with_context(|| format!("create sticker dir {:?}", target_dir))?;

    unpack(BUNDLED_PACK, target_dir)
        .with_context(|| format!("unpack sticker bundle to {:?}", target_dir))?;

    fs::write(&marker_path, &fingerprint)
        .with_context(|| format!("write version marker {:?}", marker_path))?;

    Ok(UnpackOutcome::Unpacked)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpackOutcome {
    /// Pack was already at the current version; nothing changed.
    AlreadyCurrent,
    /// We unpacked the bundled pack into the directory.
    Unpacked,
    /// Directory has user-owned content (no marker); we did not touch it.
    UserOwnedSkipped,
    /// This binary was built without a bundled pack.
    NoBundle,
}

fn has_user_content(dir: &Path) -> Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == VERSION_MARKER {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

fn unpack(blob: &[u8], target: &Path) -> Result<()> {
    let cursor = Cursor::new(blob);
    let decoder = zstd::stream::Decoder::new(cursor)?;
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable() {
        let a = bundled_pack_fingerprint();
        let b = bundled_pack_fingerprint();
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_contains_length() {
        let fp = bundled_pack_fingerprint();
        assert!(fp.contains(&BUNDLED_PACK.len().to_string()));
    }
}
