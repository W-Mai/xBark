// Build-time: package the bundled sticker pack into a compressed
// tar.zst blob that we include_bytes! at compile time. That way the
// release binary is self-contained — first time a user runs the daemon
// it unpacks the blob into their config dir.
//
// The blob is regenerated whenever stickers/ changes (cargo rerun-if).

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    tauri_build::build();

    // Where is the project root (workspace-wise repository root)?
    // CARGO_MANIFEST_DIR = .../xBark/src-tauri
    // We want .../xBark
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir
        .parent()
        .expect("src-tauri must have a parent")
        .to_path_buf();
    let stickers_dir = repo_root.join("stickers");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let blob_path = out_dir.join("stickers.tar.zst");

    // Tell cargo to re-run if anything under stickers/ changes
    println!("cargo:rerun-if-changed={}", stickers_dir.display());

    if !stickers_dir.exists() {
        // Happens e.g. when someone vendors only src-tauri/ — emit an empty
        // marker so include_bytes! in src/ still compiles.
        fs::write(&blob_path, []).expect("write empty stickers blob");
        println!(
            "cargo:warning=stickers/ not found at {}, built with empty sticker pack",
            stickers_dir.display()
        );
        return;
    }

    if let Err(e) = pack(&stickers_dir, &blob_path) {
        panic!("failed to pack stickers from {:?}: {}", stickers_dir, e);
    }

    let size = fs::metadata(&blob_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "cargo:warning=bundled sticker pack: {} bytes ({})",
        size,
        blob_path.display()
    );
}

fn pack(src: &Path, out: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // 1. tar the source directory into memory
    let mut tar_buf = Vec::with_capacity(20 * 1024 * 1024);
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        // Sort entries for deterministic output
        let mut entries: Vec<_> = walkdir::WalkDir::new(src)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .collect();
        entries.sort_by(|a, b| a.path().cmp(b.path()));

        for entry in entries {
            let full = entry.path();
            let rel = full.strip_prefix(src)?;
            builder.append_path_with_name(full, rel)?;
        }
        builder.finish()?;
    }

    // 2. zstd compress the tarball to the output file (level 19)
    let out_file = fs::File::create(out)?;
    let mut encoder = zstd::stream::Encoder::new(out_file, 19)?;
    std::io::copy(&mut tar_buf.as_slice(), &mut encoder)?;
    encoder.finish()?;
    Ok(())
}
