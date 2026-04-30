# Releasing xBark

This document captures the full release/distribution plan so future-me doesn't have to rediscover it.

Status: v0.1.0 shipped as source-only. Everything below is the TODO for v0.1.1+.

---

## Big picture

Four user-facing install paths, in priority order:

1. `brew install w-mai/tap/xbark` — the default path for mac users
2. `curl | sh` shell installer from GitHub releases
3. `cargo binstall xbark` — for rust users, no source build
4. `cargo install --git …` — absolute worst-case fallback

All four share the same artifacts: GitHub Release assets produced by **cargo-dist** in CI.

```
┌──────────────┐   git tag vX.Y.Z   ┌──────────────────┐
│  local dev   │ ─────────────────▶ │  GitHub Actions  │
└──────────────┘                    │  (cargo-dist)    │
                                    └────────┬─────────┘
                                             │
                   ┌─────────────────────────┼──────────────────────────┐
                   ▼                         ▼                          ▼
          ┌────────────────┐       ┌──────────────────┐      ┌──────────────────┐
          │ Release assets │       │ homebrew-tap     │      │ install shell    │
          │ (.tar.gz per   │       │  auto-commit     │      │  auto-served     │
          │  platform)     │       │  formula         │      │                  │
          └────────────────┘       └──────────────────┘      └──────────────────┘
```

---

## Step-by-step for v0.1.1

### 1. Set up a Homebrew tap repo

One-time:

```bash
gh repo create W-Mai/homebrew-tap --public \
  --description "Homebrew formulae for W-Mai projects"
```

The naming convention is important: `<user>/homebrew-<anything>`. The `homebrew-` prefix lets users write `brew tap W-Mai/tap` (Homebrew strips the prefix).

### 2. Install cargo-dist

```bash
cargo install cargo-dist
# or from homebrew:
brew install axodotdev/tap/cargo-dist
```

### 3. Initialise cargo-dist config

```bash
cd ~/Projects/xBark
cargo dist init
```

This will interactively ask:
- Which platforms: pick `aarch64-apple-darwin`, `x86_64-apple-darwin`. Add Linux + Windows later
- Which installers: pick `shell` and `homebrew`
- Homebrew tap: `W-Mai/homebrew-tap`
- CI: yes, GitHub

It generates:
- `dist-workspace.toml` — config
- `.github/workflows/release.yml` — CI definition
- Adds a `[workspace.metadata.dist]` section to `Cargo.toml`

**Tauri caveat**: cargo-dist by default builds the package's default binary. xBark's binary is at `src-tauri/` subpackage, which is the workspace member — this should work out of the box, but the frontend files in `src/` must be embedded at build time (they already are, via `tauri::generate_context!`). Verify `cargo build --release` from scratch on a clean CI runner produces a working binary.

If Tauri's build requires extra system deps (e.g. webkit on Linux), cargo-dist needs a `build-local-artifacts` hook or GitHub Actions setup step. See https://opensource.axo.dev/cargo-dist/book/quickstart/rust.html

### 4. First release

```bash
# bump version in src-tauri/Cargo.toml to 0.1.1
# (also root Cargo.toml workspace.package.version)

git add -A
git commit -m "🔖(release): bump to 0.1.1"
git tag v0.1.1
git push origin main v0.1.1
```

CI will:
1. Build binaries for each target platform in parallel
2. Package them with the `xbark-installer.sh` wrapper
3. Upload as GitHub Release assets
4. Commit the new Homebrew formula to `W-Mai/homebrew-tap/Formula/xbark.rb`

### 5. Test the install paths

On a fresh machine (or inside a clean Docker on Linux):

```bash
# Path 1: homebrew
brew tap W-Mai/tap
brew install xbark
xbark --version

# Path 2: shell installer
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/W-Mai/xBark/releases/download/v0.1.1/xbark-installer.sh | sh

# Path 3: cargo-binstall (after cargo-dist generates the required metadata)
cargo binstall xbark

# Path 4: cargo install (fallback, compiles from source)
cargo install --git https://github.com/W-Mai/xBark --tag v0.1.1 xbark
```

### 6. Update README with install instructions

```markdown
## Install

### Homebrew (macOS)
\`\`\`bash
brew install w-mai/tap/xbark
\`\`\`

### Shell installer
\`\`\`bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/W-Mai/xBark/releases/latest/download/xbark-installer.sh | sh
\`\`\`

### From source
\`\`\`bash
cargo install --git https://github.com/W-Mai/xBark
\`\`\`
```

---

## Version bump checklist

Every release should touch these places (TODO: script it):

- [ ] `Cargo.toml` — `workspace.package.version`
- [ ] `src-tauri/tauri.conf.json` — `version`
- [ ] `src-tauri/Cargo.toml` — not needed (inherits from workspace)
- [ ] `CHANGELOG.md` — add entry (not yet created)
- [ ] Run `cargo build --release` to refresh `Cargo.lock`
- [ ] Commit: `🔖(release): bump to X.Y.Z`
- [ ] Tag: `git tag vX.Y.Z`
- [ ] Push: `git push origin main vX.Y.Z`

---

## Signing / notarisation (macOS)

Unsigned binaries on macOS show the scary "can't be opened" warning. To fix:

1. Enrol in [Apple Developer Program](https://developer.apple.com/programs/) ($99/year) — probably not worth it for a sticker tool
2. Generate a "Developer ID Application" certificate
3. Give cargo-dist your signing identity + Apple ID app-specific password via GitHub secrets:
   - `APPLE_CERTIFICATE` (base64 p12)
   - `APPLE_CERTIFICATE_PASSWORD`
   - `APPLE_ID`
   - `APPLE_APP_PASSWORD`
   - `APPLE_TEAM_ID`
4. cargo-dist will `codesign` + `notarytool submit` automatically

**Alternative**: tell users to `xattr -d com.apple.quarantine $(which xbark)` after install. Ugly but free.

---

## Cross-platform status

| Platform | Status | Notes |
|----------|--------|-------|
| macOS aarch64 | ✅ works | primary target, tested |
| macOS x86_64 | ⚠️ untested | should work, Tauri v2 supports it |
| Linux x86_64 | ❌ untested | needs `webkit2gtk-4.1` system dep |
| Linux aarch64 | ❌ untested | same |
| Windows | ❌ untested | needs WebView2 runtime |

Windows and Linux support are not blockers for v0.1.x. They need:
- Testing the overlay window attributes (transparent + always-on-top + skip-taskbar)
- Platform-specific autostart (systemd user unit on Linux, Task Scheduler on Windows)
- Additional CI build targets in cargo-dist config

---

## References

- cargo-dist docs: https://opensource.axo.dev/cargo-dist/book/
- Homebrew formula cookbook: https://docs.brew.sh/Formula-Cookbook
- Tauri distribution guide: https://v2.tauri.app/distribute/
- cargo-binstall metadata: https://github.com/cargo-bins/cargo-binstall
