// xBark overlay frontend.
// Listens for sticker events from the Rust side and manages DOM lifecycle.

/**
 * @typedef {Object} StickerPayload
 * @property {string} id
 * @property {string} image_url
 * @property {number} duration_ms
 * @property {number} size
 * @property {"bottom-right"|"bottom-left"|"top-right"|"top-left"|"center"|"random"} position
 * @property {string} [description]
 * @property {string} [ai_name]
 */

const POSITIONS = ["bottom-right", "bottom-left", "top-right", "top-left", "center"];
const MAX_VISIBLE = 5;

/** Map<position, HTMLElement> */
const anchors = new Map();
/** Array<{id, el, timeoutId, startTime, remaining, position}> */
const active = [];

const root = document.getElementById("root");

function ensureAnchor(position) {
  if (!POSITIONS.includes(position)) position = "bottom-right";
  let anchor = anchors.get(position);
  if (!anchor) {
    anchor = document.createElement("div");
    anchor.className = `anchor ${position}`;
    root.appendChild(anchor);
    anchors.set(position, anchor);
  }
  return anchor;
}

function resolvePosition(requested) {
  if (requested === "random") {
    return POSITIONS[Math.floor(Math.random() * POSITIONS.length)];
  }
  return POSITIONS.includes(requested) ? requested : "bottom-right";
}

/**
 * @param {StickerPayload} payload
 */
function showSticker(payload) {
  const position = resolvePosition(payload.position);
  const anchor = ensureAnchor(position);

  // Enforce max visible: dismiss oldest
  while (active.length >= MAX_VISIBLE) {
    const oldest = active[0];
    dismissByIndex(0, true);
    if (!oldest) break;
  }

  const el = document.createElement("div");
  el.className = "sticker";
  el.style.width = `${payload.size}px`;
  el.style.height = `${payload.size}px`;
  el.dataset.id = payload.id;

  const img = document.createElement("img");
  img.src = payload.image_url;
  img.alt = payload.description || payload.ai_name || "";
  img.draggable = false;
  el.appendChild(img);

  anchor.appendChild(el);

  const record = {
    id: payload.id,
    el,
    position,
    remaining: payload.duration_ms,
    startTime: performance.now(),
    timeoutId: null,
    paused: false,
  };
  active.push(record);

  startDismissTimer(record);

  // Click to dismiss
  el.addEventListener("click", () => {
    dismiss(record);
  });

  // Hover to pause
  el.addEventListener("mouseenter", () => {
    if (record.paused || record.timeoutId === null) return;
    clearTimeout(record.timeoutId);
    const elapsed = performance.now() - record.startTime;
    record.remaining = Math.max(0, record.remaining - elapsed);
    record.paused = true;
    record.timeoutId = null;
  });
  el.addEventListener("mouseleave", () => {
    if (!record.paused) return;
    record.paused = false;
    record.startTime = performance.now();
    startDismissTimer(record);
  });
}

function startDismissTimer(record) {
  record.timeoutId = setTimeout(() => {
    dismiss(record);
  }, record.remaining);
}

function dismiss(record) {
  const idx = active.findIndex((r) => r.id === record.id);
  if (idx < 0) return;
  dismissByIndex(idx, false);
}

function dismissByIndex(idx, _squeezed) {
  const record = active[idx];
  if (!record) return;
  active.splice(idx, 1);
  if (record.timeoutId) clearTimeout(record.timeoutId);
  const { el } = record;
  el.classList.add("leaving");
  el.addEventListener(
    "animationend",
    () => {
      el.remove();
      maybeHide();
    },
    { once: true },
  );
  // Safety: if animationend doesn't fire in 500ms, force remove
  setTimeout(() => {
    if (el.parentNode) el.remove();
    maybeHide();
  }, 500);
}

function clearAll() {
  while (active.length > 0) {
    dismissByIndex(0, false);
  }
}

function maybeHide() {
  if (active.length > 0) return;
  // ask backend to hide window (via invoke)
  // We keep the window visible — hiding/showing adds latency to next sticker.
  // Comment out if you want to hide when idle:
  // tauriHide();
}

// ---- Tauri event wiring ----
// In Tauri v2 with withGlobalTauri=true, event.listen is exposed via
// window.__TAURI__.event, injected async after HTML parses. Poll until ready.
function wireEvents(attempt = 0) {
  const tauri = window.__TAURI__;
  if (!tauri || !tauri.event || !tauri.event.listen) {
    if (attempt > 200) {
      console.error("[xbark] __TAURI__.event never appeared after 10s");
      return;
    }
    setTimeout(() => wireEvents(attempt + 1), 50);
    return;
  }
  tauri.event.listen("sticker:show", (ev) => {
    showSticker(ev.payload);
  });
  tauri.event.listen("sticker:clear", () => {
    clearAll();
  });
  console.log("[xbark] event wiring ready");
}

wireEvents();

// Expose for debugging from webview devtools
window.xBark = { showSticker, clearAll, active };
