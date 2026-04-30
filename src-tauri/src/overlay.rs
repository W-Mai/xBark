// Overlay window management.
// A single full-screen transparent, click-through, always-on-top webview
// hosts all sticker popups. We communicate with the frontend via Tauri events.
//
// Readiness protocol
// ------------------
// There's a race between the HTTP server accepting requests and the webview
// finishing HTML/JS load + wiring `window.__TAURI__.event.listen`. During
// that window, any `emit_to("sticker:show")` we fire gets dropped by the
// webview runtime (no listener yet) and the sticker silently never renders.
//
// To handle this reliably:
//   1. The frontend, once its listener is attached, invokes the Tauri
//      command `frontend_ready`. That sets a process-wide AtomicBool.
//   2. `show_sticker` checks the flag. If ready, emit immediately. If not
//      yet ready, the payload is pushed onto a bounded in-memory queue
//      (capped at MAX_PENDING — old entries get dropped, newest wins).
//   3. When `frontend_ready` fires, we drain the queue and emit everything
//      that was waiting.
//
// This also shields against any future transient "frontend went away"
// scenarios (webview crash, devtools reload, etc.).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const OVERLAY_LABEL: &str = "overlay";

/// Has the frontend registered its event listeners?
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);

/// Bounded queue of stickers received before the frontend was ready.
const MAX_PENDING: usize = 20;
static PENDING: Mutex<Vec<StickerPayload>> = Mutex::new(Vec::new());

pub fn is_frontend_ready() -> bool {
    FRONTEND_READY.load(Ordering::Acquire)
}

/// Invoked by the frontend once its event listeners are wired.
/// Drains any pending stickers that were enqueued during startup.
pub fn mark_frontend_ready(app: &AppHandle) {
    let was_ready = FRONTEND_READY.swap(true, Ordering::AcqRel);
    if was_ready {
        return;
    }
    let pending = {
        let mut q = PENDING.lock().expect("pending mutex poisoned");
        std::mem::take(&mut *q)
    };
    if !pending.is_empty() {
        tracing::info!("frontend ready — flushing {} pending stickers", pending.len());
    } else {
        tracing::info!("frontend ready");
    }
    for payload in pending {
        if let Err(e) = app.emit_to(OVERLAY_LABEL, "sticker:show", &payload) {
            tracing::warn!("failed to emit queued sticker: {}", e);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerPayload {
    pub id: String,
    pub image_url: String,
    pub duration_ms: u32,
    pub size: u32,
    pub position: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub ai_name: String,
}

/// Create the single overlay webview window. Hidden by default.
/// Shown lazily on first sticker.
pub fn create_overlay(app: &AppHandle) -> Result<()> {
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Ok(());
    }

    // Dump all monitors so we can see what Tauri reports.
    let monitors = app.available_monitors().unwrap_or_default();
    tracing::info!("available_monitors: {} found", monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        tracing::info!(
            "  monitor[{}] name={:?} pos=({},{})px size={}x{}px scale={}",
            i,
            m.name(),
            m.position().x, m.position().y,
            m.size().width, m.size().height,
            m.scale_factor(),
        );
    }
    let monitor = app
        .primary_monitor()
        .ok()
        .flatten()
        .context("no primary monitor")?;
    tracing::info!(
        "primary_monitor name={:?} pos=({},{})px size={}x{}px scale={}",
        monitor.name(),
        monitor.position().x, monitor.position().y,
        monitor.size().width, monitor.size().height,
        monitor.scale_factor(),
    );

    let debug_mode = std::env::var("XBARK_DEBUG").is_ok();
    tracing::info!("create_overlay: debug_mode={}", debug_mode);

    // Compute bottom-right of primary monitor in logical points
    let screen_size = monitor.size();
    let screen_pos = monitor.position();
    let scale = monitor.scale_factor();
    let screen_w_pt = screen_size.width as f64 / scale;
    let screen_h_pt = screen_size.height as f64 / scale;
    let screen_x_pt = screen_pos.x as f64 / scale;
    let screen_y_pt = screen_pos.y as f64 / scale;

    let overlay_w = 600.0_f64;
    let overlay_h = screen_h_pt.min(1600.0);
    let overlay_x = screen_x_pt + screen_w_pt - overlay_w;
    let overlay_y = screen_y_pt + screen_h_pt - overlay_h;
    tracing::info!(
        "overlay region: {}x{}pt at ({},{})pt on primary screen",
        overlay_w, overlay_h, overlay_x, overlay_y
    );

    let mut builder = WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title("xBark Overlay")
    .decorations(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)  // follow user across macOS Spaces
    .skip_taskbar(true)
    .resizable(false)
    .focused(false)
    .shadow(false);

    if debug_mode {
        builder = builder
            .visible(true)
            .transparent(true)
            .inner_size(600.0, 400.0)
            .position(50.0, 50.0);
        tracing::info!("debug mode: small 600x400 window at (50,50)");
    } else {
        // Non-fullscreen transparent overlay anchored at right side.
        builder = builder
            .transparent(true)
            .visible(true)
            .inner_size(overlay_w, overlay_h)
            .position(overlay_x, overlay_y);
        tracing::info!(
            "overlay region: {}x{} at ({},{})",
            overlay_w, overlay_h, overlay_x, overlay_y
        );
    }

    let window = builder.build()?;
    tracing::info!("window built ok, label={}", window.label());

    if !debug_mode {
        window.set_ignore_cursor_events(true)?;
    }

    if debug_mode {
        window.open_devtools();
        tracing::info!("open_devtools() called");
    }

    // macOS: set to floating + join all spaces so it shows on fullscreen apps too
    #[cfg(target_os = "macos")]
    {
        use tauri::utils::config::WindowEffectsConfig;
        let _ = window; // placeholder for future macOS-specific calls
                        // The macos-private-api feature gives us the cocoa window if needed later
    }

    tracing::info!("overlay window created");
    Ok(())
}

/// Show a sticker via event. If the frontend isn't ready yet, queue it.
pub fn show_sticker(app: &AppHandle, payload: StickerPayload) -> Result<()> {
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .context("overlay window missing")?;

    if !window.is_visible().unwrap_or(false) {
        window.show()?;
    }

    if is_frontend_ready() {
        app.emit_to(OVERLAY_LABEL, "sticker:show", &payload)?;
    } else {
        let mut q = PENDING.lock().expect("pending mutex poisoned");
        if q.len() >= MAX_PENDING {
            // Drop oldest to keep memory bounded
            q.remove(0);
        }
        q.push(payload);
        tracing::debug!(
            "frontend not ready, queued sticker (pending={}, cap={})",
            q.len(),
            MAX_PENDING
        );
    }
    Ok(())
}

/// Tell the frontend to clear all active stickers.
pub fn clear_all(app: &AppHandle) -> Result<()> {
    app.emit_to(OVERLAY_LABEL, "sticker:clear", &())?;
    Ok(())
}
