<div align="center">

<img src="./static/logo.png" alt="xBark logo" width="140"/>

# xBark

**Desktop sticker popups for any command, any script, any AI reply.**

[![License](https://img.shields.io/github/license/W-Mai/xBark?style=flat-square)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/W-Mai/xBark?style=flat-square)](https://github.com/W-Mai/xBark/releases)
[![Stars](https://img.shields.io/github/stars/W-Mai/xBark?style=flat-square)](https://github.com/W-Mai/xBark/stargazers)

`xbark send 点赞` — and a sticker flies into the bottom-right of your screen.

![demo](./assets/demo.gif)

</div>

## 📖 Table of Contents

- [Why xBark](#-why-xbark)
- [Features](#-features)
- [Quick Start](#-quick-start)
- [Installation](#-installation)
- [Configuration](#-configuration)
- [HTTP API](#-http-api)
- [OpenCode Plugin](#-opencode-plugin)
- [Sticker Packs](#-sticker-packs)
- [Architecture](#-architecture)
- [Roadmap & Limitations](#-roadmap--limitations)
- [License](#-license)

## 🤔 Why xBark

macOS notifications are boring. A tiny grey banner in the top-right that slides away before you can read it, and the icon is basically a postage stamp. If you want **large, animated, memeable, on-screen reactions** — say, for an AI assistant that fires `:sticker[点赞]:` when it nails a task — the OS won't help you.

xBark is a small daemon that draws its own popups: a fullscreen transparent overlay that stacks sticker images in your preferred corner, plays GIFs natively, follows you across macOS Spaces, and dismisses itself on a timer. Talk to it over HTTP from anything — CLI, scripts, editor plugins, shell hooks.

## ✨ Features

- **🎯 Stacking popups** — multiple stickers stack up, oldest drops off after its duration
- **🪟 macOS Spaces aware** — the overlay follows you when you swipe between virtual desktops
- **👆 Click-through** — stickers never intercept clicks for the app underneath
- **🖱️ Interactive** — hover pauses the dismiss timer, click dismisses instantly
- **🌐 HTTP API** — send stickers from any language with a simple POST
- **📡 mDNS discovery** — daemon publishes `_xbark._tcp` for zero-config LAN reach
- **🔎 Fuzzy keyword resolution** — `:sticker[鼓掌]:` matches by filename / aiName / tag / description
- **🚀 Auto-start** — `xbark send` lazily spawns the daemon if it isn't running; `xbark autostart install` hooks it into launchd
- **🖼️ GIF support** — animated frames play natively in the WebView
- **⚙️ TOML config** — duration, size, sticker pack, max visible, etc.
- **📦 82-piece built-in pack** — curated Chinese meme stickers with AI-friendly metadata, or bring your own

## ⚡ Quick Start

```bash
# Send a sticker. If the daemon isn't running, it will be spawned.
xbark send 点赞

# List all available stickers (filter supported)
xbark list --filter 点赞

# Tweak per-sticker duration
xbark send 拿捏 --duration 5

# Daemon control
xbark status
xbark stop
xbark clear        # clear all active popups

# Run as a login item (macOS)
xbark autostart install
```

## 📦 Installation

> 🚧 **v0.1.0 is source-only.** Pre-built binaries + `brew install` arrive in v0.1.1 via [cargo-dist](https://opensource.axo.dev/cargo-dist/). See [`docs/RELEASING.md`](./docs/RELEASING.md) for the roadmap.

### From source

Prerequisites:
- [Rust toolchain](https://www.rust-lang.org/tools/install) (1.77+)
- macOS (Windows / Linux are on the roadmap)

```bash
git clone https://github.com/W-Mai/xBark
cd xBark
cargo build --release --manifest-path src-tauri/Cargo.toml
```

Drop the binary anywhere on your `PATH`:

```bash
cp target/release/xbark ~/.local/bin/
# or
ln -s "$(pwd)/target/release/xbark" ~/.bun/bin/xbark
```

### Coming soon

```bash
# Homebrew
brew install W-Mai/tap/xbark

# Shell installer
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/W-Mai/xBark/releases/latest/download/xbark-installer.sh | sh

# cargo-binstall
cargo binstall xbark
```

## 🛠️ Configuration

Put a `config.toml` at `~/.config/xbark/config.toml` (Linux / XDG) or `~/Library/Application Support/sh.w-mai.xbark/config.toml` (macOS). All fields are optional.

```toml
# Default display time per sticker (seconds)
duration = 2.0

# Default size in pixels (square)
size = 256

# Max stickers visible simultaneously (older ones get evicted)
max_visible = 5

# Gap between stacked stickers (pixels)
gap = 12

# Margin from screen edge (pixels)
margin = 32

# HTTP server port. 0 = random (port file written next to this config)
port = 0

# Point at a different sticker pack (overrides the bundled one)
# sticker_dir = "/path/to/my-stickers"
```

See [`config.example.toml`](./config.example.toml) for the full reference.

## 🌐 HTTP API

The daemon listens on `127.0.0.1:<port>` where port is auto-picked. Discover it via the port file (`xbark.port` next to the config), or via mDNS service `_xbark._tcp.local.`.

```bash
PORT=$(cat ~/Library/Application\ Support/sh.w-mai.xbark/xbark.port)

# Health check
curl http://127.0.0.1:$PORT/health

# Send by keyword (goes through the resolver)
curl -X POST http://127.0.0.1:$PORT/sticker \
  -H 'Content-Type: application/json' \
  -d '{"keyword":"点赞","duration":3,"size":256}'

# Send by absolute path (bypass resolver; any image file)
curl -X POST http://127.0.0.1:$PORT/sticker \
  -H 'Content-Type: application/json' \
  -d '{"path":"/path/to/image.gif"}'

# List available stickers (optionally filtered)
curl "http://127.0.0.1:$PORT/stickers?filter=点赞"

# Clear all active popups
curl -X POST http://127.0.0.1:$PORT/clear
```

## 🔌 OpenCode Plugin

xBark pairs naturally with [OpenCode](https://opencode.ai) — the AI writes `:sticker[点赞]:` inside its reply, and xBark pops the image into your screen as the tokens stream in.

Install the example plugin:

```bash
ln -s "$(pwd)/examples/opencode-plugin/sticker-notify.js" \
      ~/.config/opencode/plugins/sticker-notify.js
```

Then restart OpenCode. The plugin reads the daemon port from the port file, POSTs directly over HTTP (no subprocess fork — stickers fire in under 10ms), and de-duplicates per-message-part so streaming updates don't re-trigger the same sticker.

## 🎁 Sticker Packs

A sticker pack is a directory containing image files (`.jpg` / `.png` / `.gif` / `.webp`) plus a `_meta.json` describing each one:

```json
{
  "getimgdata-8.jpg": {
    "filename": "getimgdata-8.jpg",
    "aiName": "smiling-man-thumbs-up",
    "description": "Man smiling, thumbs up — approval or forced acceptance",
    "tags": ["点赞", "认可", "微笑"]
  }
}
```

- **filename** — the file on disk, must match the JSON key
- **aiName** — stable English slug AIs can cite reliably
- **description** — free-form, used as the fuzzy-match fallback
- **tags** — short keywords (Chinese or English), fastest match path

The bundled pack lives in [`stickers/`](./stickers/) with 82 entries.

## 🏗️ Architecture

```text
┌────────────────────┐    POST /sticker     ┌─────────────────────────────┐
│  OpenCode plugin   │ ───────────────────▶ │   xbark daemon (single      │
│  xbark CLI         │                      │   binary, Tauri + Rust)     │
│  curl / any client │                      │                             │
└────────────────────┘                      │   ┌─────────────────────┐   │
                                            │   │ axum HTTP server    │   │
                                            │   │  /sticker /clear    │   │
                                            │   │  /health /stickers  │   │
                                            │   └─────────────────────┘   │
                                            │   ┌─────────────────────┐   │
                                            │   │ mdns-sd publisher   │   │
                                            │   │   _xbark._tcp       │   │
                                            │   └─────────────────────┘   │
                                            │   ┌─────────────────────┐   │
                                            │   │ Tauri WebviewWindow │   │
                                            │   │ (WKWebView overlay, │   │
                                            │   │  transparent, all   │   │
                                            │   │  Spaces, click-thru)│   │
                                            │   └─────────────────────┘   │
                                            └─────────────────────────────┘
```

- Single Tauri `WebviewWindow` anchored to the primary screen's bottom-right
- Frontend is plain HTML + CSS + JS — no framework, no bundler
- Each sticker is an absolutely-positioned `<div>` managed by `flexbox` column-reverse; exit/entry are pure CSS keyframes
- Images are inlined as base64 data URLs in the HTTP response, so the WebView never needs filesystem scope over arbitrary paths
- HTTP server runs on its own Tokio runtime thread, doesn't share the Tauri main loop

## 🗺️ Roadmap & Limitations

| Status | Item |
|--------|------|
| ✅ | Stacking popups, animations, Spaces-aware, click-through |
| ✅ | HTTP API + mDNS + TOML config + launchd autostart (macOS) |
| ✅ | OpenCode plugin example with streaming-aware dispatch |
| 🚧 | Pre-built binaries + Homebrew tap ([v0.1.1 roadmap](./docs/RELEASING.md)) |
| 🚧 | Anchor positions other than `bottom-right` (needs multi-overlay refactor) |
| 📋 | Windows / Linux ports (Tauri v2 primitives portable, needs testing) |
| 📋 | Tray icon with show/hide/clear controls |
| 📋 | Sticker pack registry / `xbark install <pack>` |
| 📋 | Per-sticker sound effects |

## 📄 License

MIT. See [LICENSE](./LICENSE).

---

<div align="center">

Made at 3am while debugging something completely unrelated.

</div>
