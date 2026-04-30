// Overlay window management.
// A single full-screen transparent, click-through, always-on-top webview
// hosts all sticker popups. We communicate with the frontend via Tauri events.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const OVERLAY_LABEL: &str = "overlay";

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

/// Show a sticker via event. The frontend listens for `sticker:show` events.
pub fn show_sticker(app: &AppHandle, payload: StickerPayload) -> Result<()> {
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .context("overlay window missing")?;

    if !window.is_visible().unwrap_or(false) {
        window.show()?;
    }

    app.emit_to(OVERLAY_LABEL, "sticker:show", &payload)?;
    Ok(())
}

/// Tell the frontend to clear all active stickers.
pub fn clear_all(app: &AppHandle) -> Result<()> {
    app.emit_to(OVERLAY_LABEL, "sticker:clear", &())?;
    Ok(())
}
