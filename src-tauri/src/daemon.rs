// Daemon runner: initialize Tauri app, set up async runtime for HTTP/mDNS,
// keep everything alive.

use anyhow::Result;
use std::sync::Arc;
use tauri::Manager;

use crate::config::Config;
use crate::discovery::Discovery;
use crate::overlay;
use crate::resolver::Resolver;
use crate::server::{self, AppState};

const PID_FILE_NAME: &str = "xbark.pid";
const PORT_FILE_NAME: &str = "xbark.port";

pub fn run(port_override: Option<u16>) -> Result<()> {
    // Ensure config dir
    let config_dir = Config::config_dir();
    std::fs::create_dir_all(&config_dir)?;

    // Check for existing daemon via pid file
    if let Some(other_pid) = read_existing_pid(&config_dir) {
        if is_process_alive(other_pid) {
            anyhow::bail!(
                "xbark daemon already running (pid {}). use `xbark stop` first.",
                other_pid
            );
        } else {
            // stale pid file, clean up
            let _ = std::fs::remove_file(config_dir.join(PID_FILE_NAME));
            let _ = std::fs::remove_file(config_dir.join(PORT_FILE_NAME));
        }
    }

    let config = Config::load()?;
    let desired_port = port_override.unwrap_or(config.port);

    // Write our pid
    let pid = std::process::id();
    std::fs::write(config_dir.join(PID_FILE_NAME), pid.to_string())?;

    // Cleanup pid file on panic/exit (best-effort)
    let config_dir_for_cleanup = config_dir.clone();
    let _guard = ScopeGuard::new(move || {
        let _ = std::fs::remove_file(config_dir_for_cleanup.join(PID_FILE_NAME));
        let _ = std::fs::remove_file(config_dir_for_cleanup.join(PORT_FILE_NAME));
    });

    // If the user hasn't provided a sticker pack (common on first install),
    // materialise the bundled pack into ~/.config/xbark/stickers/ so the
    // daemon has something to show out of the box.
    let user_sticker_dir = Config::config_dir().join("stickers");
    match crate::assets::ensure_unpacked(&user_sticker_dir) {
        Ok(crate::assets::UnpackOutcome::Unpacked) => {
            tracing::info!("unpacked bundled sticker pack to {:?}", user_sticker_dir);
        }
        Ok(crate::assets::UnpackOutcome::AlreadyCurrent) => {
            tracing::debug!(
                "bundled sticker pack already current at {:?}",
                user_sticker_dir
            );
        }
        Ok(crate::assets::UnpackOutcome::UserOwnedSkipped) => {
            tracing::info!(
                "user-managed sticker dir at {:?}, leaving alone",
                user_sticker_dir
            );
        }
        Ok(crate::assets::UnpackOutcome::NoBundle) => {
            tracing::warn!("this xbark binary was built without a bundled sticker pack");
        }
        Err(e) => {
            tracing::warn!("could not unpack bundled sticker pack: {}", e);
        }
    }

    // Resolver — load stickers up front
    let sticker_dir = config.resolve_sticker_dir();
    let resolver = Arc::new(Resolver::new(sticker_dir.clone()));
    match resolver.reload() {
        Ok(n) => tracing::info!("loaded {} stickers from {:?}", n, sticker_dir),
        Err(e) => tracing::warn!("sticker reload failed: {}", e),
    }

    // Build & run Tauri app (this blocks)
    let config_for_app = config.clone();
    let resolver_for_app = resolver.clone();
    let desired_port_for_app = desired_port;
    let config_dir_for_app = config_dir.clone();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![frontend_ready])
        .setup(move |app| {
            // Create overlay window
            if let Err(e) = overlay::create_overlay(&app.handle()) {
                tracing::error!("failed to create overlay: {}", e);
                return Err(e.into());
            }

            // Spawn async runtime for HTTP + mDNS
            let app_handle = app.handle().clone();
            let cfg = config_for_app.clone();
            let resolver = resolver_for_app.clone();
            let cd = config_dir_for_app.clone();
            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("failed to create tokio runtime: {}", e);
                        return;
                    }
                };
                rt.block_on(async move {
                    let state = AppState {
                        app_handle,
                        config: cfg,
                        resolver,
                    };
                    match server::serve(state, desired_port_for_app).await {
                        Ok(port) => {
                            // Write port file for client discovery
                            let _ = std::fs::write(cd.join(PORT_FILE_NAME), port.to_string());
                            // Publish mDNS
                            match Discovery::publish(port) {
                                Ok(_disc) => {
                                    // Keep discovery alive for the lifetime of the runtime
                                    std::mem::forget(_disc);
                                }
                                Err(e) => tracing::warn!("mDNS publish failed: {}", e),
                            }
                            // Keep runtime alive
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                            }
                        }
                        Err(e) => {
                            tracing::error!("HTTP serve failed: {}", e);
                        }
                    }
                });
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("tauri run failed");

    Ok(())
}

fn read_existing_pid(config_dir: &std::path::Path) -> Option<u32> {
    let p = config_dir.join(PID_FILE_NAME);
    std::fs::read_to_string(&p).ok()?.trim().parse().ok()
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true // assume alive on non-unix
    }
}

struct ScopeGuard<F: FnOnce()> {
    f: Option<F>,
}

impl<F: FnOnce()> ScopeGuard<F> {
    fn new(f: F) -> Self {
        Self { f: Some(f) }
    }
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            f();
        }
    }
}

/// Invoked by the overlay webview once its event listeners are wired.
/// Any stickers that arrived during startup get flushed.
#[tauri::command]
fn frontend_ready(app: tauri::AppHandle) {
    overlay::mark_frontend_ready(&app);
}
