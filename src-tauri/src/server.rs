// HTTP server (axum) embedded in the daemon.
// Endpoints:
//   POST /sticker  — send a sticker (by keyword or image_path)
//   POST /clear    — clear all active stickers
//   GET  /health   — liveness
//   GET  /stickers — list available stickers (optionally filtered)

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::config::Config;
use crate::overlay::{self, StickerPayload};
use crate::resolver::{Resolver, StickerMeta};

pub struct AppState {
    pub app_handle: AppHandle,
    pub config: Config,
    pub resolver: Arc<Resolver>,
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    /// Keyword/aiName/tag/filename — resolved against meta.json
    pub keyword: Option<String>,
    /// Absolute path to image, bypasses resolver
    pub path: Option<String>,
    pub duration: Option<f32>,
    pub size: Option<u32>,
    pub position: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub ok: bool,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticker: Option<StickerMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Serve forever. Returns the bound port.
pub async fn serve(state: AppState, desired_port: u16) -> Result<u16> {
    let state = Arc::new(state);

    let app = Router::new()
        .route("/health", get(health))
        .route("/sticker", post(send_sticker))
        .route("/clear", post(clear))
        .route("/stickers", get(list_stickers))
        .with_state(state.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], desired_port));
    let listener = TcpListener::bind(addr).await.context("bind HTTP port")?;
    let actual_port = listener.local_addr()?.port();
    tracing::info!("HTTP server listening on http://127.0.0.1:{}", actual_port);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("server error: {}", e);
        }
    });

    Ok(actual_port)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn send_sticker(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendRequest>,
) -> impl IntoResponse {
    let id = Uuid::new_v4().to_string();

    // Resolve image path
    let (image_path, sticker_meta) = if let Some(kw) = req.keyword.as_ref() {
        match state.resolver.resolve(kw) {
            Some(m) => {
                let p = state.resolver.resolve_path(&m);
                (p, Some(m))
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(SendResponse {
                        ok: false,
                        id,
                        sticker: None,
                        error: Some(format!("no sticker matches keyword: {}", kw)),
                    }),
                );
            }
        }
    } else if let Some(p) = req.path.as_ref() {
        (std::path::PathBuf::from(p), None)
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(SendResponse {
                ok: false,
                id,
                sticker: None,
                error: Some("either keyword or path must be provided".into()),
            }),
        );
    };

    if !image_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(SendResponse {
                ok: false,
                id,
                sticker: None,
                error: Some(format!("file not found: {:?}", image_path)),
            }),
        );
    }

    // Load the image as a base64 data URL. This bypasses Tauri's
    // asset:// protocol entirely — no URL escaping headaches, no
    // scope mismatches, works identically for JPG/PNG/GIF. Cost is
    // negligible for 256×256 stickers (few ms fs read + b64 encode).
    let image_url = match std::fs::read(&image_path) {
        Ok(bytes) => {
            use base64::Engine;
            let mime = match image_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase())
            {
                Some(ref e) if e == "jpg" || e == "jpeg" => "image/jpeg",
                Some(ref e) if e == "png" => "image/png",
                Some(ref e) if e == "gif" => "image/gif",
                Some(ref e) if e == "webp" => "image/webp",
                _ => "application/octet-stream",
            };
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            format!("data:{};base64,{}", mime, b64)
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SendResponse {
                    ok: false,
                    id,
                    sticker: None,
                    error: Some(format!("read image failed: {}", e)),
                }),
            );
        }
    };

    let duration_ms = (req.duration.unwrap_or(state.config.duration) * 1000.0) as u32;
    let size = req.size.unwrap_or(state.config.size);
    let position = req
        .position
        .unwrap_or_else(|| state.config.position.clone());

    let payload = StickerPayload {
        id: id.clone(),
        image_url,
        duration_ms,
        size,
        position,
        description: sticker_meta.as_ref().map(|m| m.description.clone()).unwrap_or_default(),
        ai_name: sticker_meta.as_ref().map(|m| m.ai_name.clone()).unwrap_or_default(),
    };

    if let Err(e) = overlay::show_sticker(&state.app_handle, payload) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SendResponse {
                ok: false,
                id,
                sticker: sticker_meta,
                error: Some(e.to_string()),
            }),
        );
    }

    (
        StatusCode::OK,
        Json(SendResponse {
            ok: true,
            id,
            sticker: sticker_meta,
            error: None,
        }),
    )
}

async fn clear(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let _ = overlay::clear_all(&state.app_handle);
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub filter: Option<String>,
}

async fn list_stickers(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let items = state.resolver.list(q.filter.as_deref());
    Json(serde_json::json!({ "ok": true, "count": items.len(), "items": items }))
}

// Suppress unused warning for HashMap import when all fields used
#[allow(dead_code)]
fn _unused() -> HashMap<String, String> {
    HashMap::new()
}
