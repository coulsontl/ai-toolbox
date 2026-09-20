use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Manager, Runtime};

static LIGHTWEIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// Window geometry captured before the main window is destroyed, restored when
/// the window is rebuilt. Physical pixels are converted to logical units so the
/// values can be fed back into `WebviewWindowBuilder`.
static SAVED_GEOMETRY: Mutex<Option<WindowGeometry>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

impl WindowGeometry {
    /// Convert physical-pixel window metrics into logical units for
    /// `WebviewWindowBuilder`.
    pub fn from_physical(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_factor: f64,
        maximized: bool,
    ) -> WindowGeometry {
        let scale = if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        WindowGeometry {
            x: x as f64 / scale,
            y: y as f64 / scale,
            width: width as f64 / scale,
            height: height as f64 / scale,
            maximized,
        }
    }
}

fn store_saved_geometry(geometry: Option<WindowGeometry>) {
    if let Ok(mut guard) = SAVED_GEOMETRY.lock() {
        *guard = geometry;
    }
}

fn take_saved_geometry() -> Option<WindowGeometry> {
    SAVED_GEOMETRY
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
}

fn capture_window_geometry<R: Runtime>(window: &tauri::Window<R>) -> Option<WindowGeometry> {
    // A never-shown window (e.g. start-lightweight startup) has no meaningful
    // geometry worth restoring on rebuild.
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    let maximized = window.is_maximized().unwrap_or(false);
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    Some(WindowGeometry::from_physical(
        position.x,
        position.y,
        size.width,
        size.height,
        scale_factor,
        maximized,
    ))
}

pub fn is_lightweight_mode() -> bool {
    LIGHTWEIGHT_MODE.load(Ordering::Acquire)
}

fn refresh_tray_menus_async<R: Runtime>(app: &tauri::AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::tray::refresh_tray_menus(&app).await;
    });
}

fn show_and_focus_main_window<R: Runtime>(app: &tauri::AppHandle<R>, window: &tauri::Window<R>) {
    #[cfg(target_os = "macos")]
    {
        use tauri::ActivationPolicy;
        let _ = app.set_activation_policy(ActivationPolicy::Regular);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// Destroy the main window to release WebView memory while keeping the backend
/// (tray, gateway, schedulers) running. Idempotent: entering again while the
/// window is already absent just re-asserts the flag.
pub fn enter_lightweight_mode<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    let previous_mode = LIGHTWEIGHT_MODE.swap(true, Ordering::AcqRel);
    let deeplink_state = app.try_state::<crate::coding::deeplink::DeepLinkState>();
    let frontend_was_ready = deeplink_state
        .as_ref()
        .map(|state| state.mark_frontend_not_ready())
        .unwrap_or(false);

    if let Some(window) = crate::main_window(app) {
        store_saved_geometry(capture_window_geometry(&window));

        #[cfg(target_os = "macos")]
        {
            use tauri::ActivationPolicy;
            let _ = app.set_activation_policy(ActivationPolicy::Accessory);
        }

        if let Err(error) = window.destroy() {
            LIGHTWEIGHT_MODE.store(previous_mode, Ordering::Release);
            if frontend_was_ready {
                if let Some(state) = deeplink_state {
                    state.mark_frontend_ready();
                }
            }
            return Err(format!("Failed to destroy main window: {error}"));
        }
    }

    refresh_tray_menus_async(app);
    log::info!("Entered lightweight mode");
    Ok(())
}

/// Rebuild the main window from its saved geometry and leave lightweight mode.
/// When the window is already alive this only brings it to the front.
pub fn exit_lightweight_mode<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    if let Some(window) = crate::main_window(app) {
        show_and_focus_main_window(app, &window);
        LIGHTWEIGHT_MODE.store(false, Ordering::Release);
        refresh_tray_menus_async(app);
        return Ok(());
    }

    let geometry = take_saved_geometry();
    crate::build_main_window(app, geometry)
        .map_err(|e| format!("Failed to rebuild main window: {e}"))?;

    if let Some(window) = crate::main_window(app) {
        show_and_focus_main_window(app, &window);
    }
    LIGHTWEIGHT_MODE.store(false, Ordering::Release);
    refresh_tray_menus_async(app);
    log::info!("Exited lightweight mode");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_physical_metrics_to_logical_units() {
        let geometry = WindowGeometry::from_physical(150, 90, 1920, 1200, 1.5, false);
        assert_eq!(geometry.x, 100.0);
        assert_eq!(geometry.y, 60.0);
        assert_eq!(geometry.width, 1280.0);
        assert_eq!(geometry.height, 800.0);
        assert!(!geometry.maximized);
    }

    #[test]
    fn tolerates_invalid_scale_factor() {
        let geometry = WindowGeometry::from_physical(10, 20, 800, 600, 0.0, true);
        assert_eq!(geometry.x, 10.0);
        assert_eq!(geometry.width, 800.0);
        assert!(geometry.maximized);
    }

    #[test]
    fn saved_geometry_round_trips_once() {
        store_saved_geometry(None);
        assert!(take_saved_geometry().is_none());

        let geometry = WindowGeometry {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            maximized: false,
        };
        store_saved_geometry(Some(geometry));
        assert_eq!(take_saved_geometry(), Some(geometry));
        // Second take sees the cleared value.
        assert!(take_saved_geometry().is_none());

        store_saved_geometry(None);
    }
}
