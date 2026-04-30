// OpenCode plugin: scan AI replies for :sticker[keyword]: markers and
// dispatch them to a running xBark daemon via HTTP.
//
// Install:
//   ln -s $(pwd)/examples/opencode-plugin/sticker-notify.js \
//     ~/.config/opencode/plugins/sticker-notify.js
//
// Requires: xBark daemon running (`xbark daemon`), or `xbark` on PATH so
// we can spawn one on demand.
//
// Advantages of direct HTTP over `xbark send`:
//   - no per-sticker fork + Rust runtime startup (~80ms saved per sticker)
//   - fires as soon as the regex matches in the streaming text, not after
//     a subprocess can spin up
//
// Port is read from ~/.config/xbark/xbark.port (or
// ~/Library/Application Support/sh.w-mai.xbark/xbark.port on macOS).

import { existsSync, readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { homedir, platform } from "node:os";
import { join } from "node:path";

const STICKER_REGEX = /:{1,2}sticker\[([^\]]+)\]:{1,2}/g;
const XBARK_BIN = process.env.XBARK_BIN || "xbark";

// Resolve the daemon port file. ProjectDirs(sh, w-mai, xbark) puts it at
// $XDG_CONFIG_HOME/xbark/ on Linux and ~/Library/Application Support/sh.w-mai.xbark/
// on macOS.
function portFileCandidates() {
  const home = homedir();
  if (platform() === "darwin") {
    return [
      join(home, "Library/Application Support/sh.w-mai.xbark/xbark.port"),
      join(home, ".config/xbark/xbark.port"),
    ];
  }
  // Linux / others
  const xdg = process.env.XDG_CONFIG_HOME || join(home, ".config");
  return [join(xdg, "xbark/xbark.port")];
}

function readPort() {
  for (const p of portFileCandidates()) {
    if (existsSync(p)) {
      try {
        const n = parseInt(readFileSync(p, "utf-8").trim(), 10);
        if (!Number.isNaN(n)) return n;
      } catch {}
    }
  }
  return null;
}

let cachedPort = null;
let cachedPortExpiry = 0;

async function getPort() {
  const now = Date.now();
  if (cachedPort && now < cachedPortExpiry) return cachedPort;

  const port = readPort();
  if (port) {
    cachedPort = port;
    cachedPortExpiry = now + 60_000;
    return port;
  }
  return null;
}

async function postSticker(keyword) {
  const port = await getPort();
  if (!port) {
    // daemon not running; lazy-spawn via CLI
    spawnDaemon();
    return;
  }
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 800);
    await fetch(`http://127.0.0.1:${port}/sticker`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ keyword }),
      signal: controller.signal,
    });
    clearTimeout(timer);
  } catch {
    // daemon might've died — invalidate cache so we spawn next time
    cachedPort = null;
  }
}

function spawnDaemon() {
  try {
    const proc = spawn(XBARK_BIN, ["daemon"], {
      stdio: "ignore",
      detached: true,
    });
    proc.unref();
  } catch {
    // xbark not on PATH; give up silently
  }
}

function extractText(part) {
  if (!part) return "";
  if (typeof part.text === "string") return part.text;
  if (part.type === "text" && typeof part.content === "string") return part.content;
  return "";
}

export default async ({ client }) => {
  // xBark is intended cross-platform (via Tauri), no hard darwin gate.
  // But if the daemon binary isn't installed this simply becomes a no-op
  // after the first failed fetch.

  const seenByPart = new Map();

  async function processText(partId, text) {
    if (!text) return;
    const matched = new Set();
    STICKER_REGEX.lastIndex = 0;
    let m;
    while ((m = STICKER_REGEX.exec(text))) {
      const kw = m[1].trim();
      if (kw) matched.add(kw);
    }
    if (matched.size === 0) return;

    const seen = seenByPart.get(partId) || new Set();
    const fresh = [...matched].filter((k) => !seen.has(k));
    if (fresh.length === 0) return;
    fresh.forEach((k) => seen.add(k));
    seenByPart.set(partId, seen);

    // Fire them all in parallel — xBark daemon handles stacking + eviction
    await Promise.all(fresh.map((kw) => postSticker(kw)));
  }

  return {
    event: async ({ event }) => {
      try {
        if (event.type === "message.part.updated") {
          const part = event.properties?.part;
          if (!part) return;
          await processText(part.id, extractText(part));
        } else if (event.type === "message.part.removed") {
          const partId = event.properties?.part?.id;
          if (partId) seenByPart.delete(partId);
        } else if (event.type === "message.removed") {
          const info = event.properties?.info;
          if (info?.id) {
            for (const key of [...seenByPart.keys()]) {
              if (key.startsWith(info.id)) seenByPart.delete(key);
            }
          }
        } else if (event.type === "session.idle") {
          if (seenByPart.size > 100) {
            const keep = [...seenByPart.entries()].slice(-50);
            seenByPart.clear();
            keep.forEach(([k, v]) => seenByPart.set(k, v));
          }
        }
      } catch (err) {
        try {
          await client.app.log({
            body: {
              service: "xbark-opencode-plugin",
              level: "error",
              message: "unhandled error",
              extra: { error: String(err) },
            },
          });
        } catch {}
      }
    },
  };
};
