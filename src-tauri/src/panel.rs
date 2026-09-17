use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Rect, WebviewWindow};

pub const TRAY_ID: &str = "agentbar-tray";
const PANEL_WIDTH: f64 = 360.0;
const PANEL_HEIGHT: f64 = 600.0;
const EXPANDED_PANEL_WIDTH: f64 = 700.0;

#[derive(Default)]
pub struct PanelState {
    revision: AtomicU64,
    visible_on_press: Mutex<Option<bool>>,
    geometry: Mutex<Option<PanelGeometry>>,
}

#[derive(Clone, Copy)]
struct PanelGeometry {
    collapsed: Bounds,
    work: Bounds,
    scale: f64,
    expanded: bool,
}

impl PanelGeometry {
    fn new(work: Bounds, anchor: Option<Bounds>, scale: f64) -> Self {
        Self {
            collapsed: placement(work, anchor, scale),
            work,
            scale,
            expanded: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Bounds {
    fn from_rect(rect: Rect, scale: f64) -> Self {
        let position = rect.position.to_physical::<f64>(scale);
        let size = rect.size.to_physical::<f64>(scale);
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }

    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

// Work in physical pixels, including monitors with negative desktop coordinates.
// The nearest work-area edge also handles an auto-hidden taskbar inside the screen.
fn placement(work: Bounds, anchor: Option<Bounds>, scale: f64) -> Bounds {
    let margin = (6.0 * scale).min(work.width / 4.0).min(work.height / 4.0);
    let gap = 4.0 * scale;
    let width = (PANEL_WIDTH * scale).min(work.width - margin * 2.0);
    let height = (PANEL_HEIGHT * scale).min(work.height - margin * 2.0);
    let (mut x, mut y) = (work.x + work.width - width - margin, work.y + margin);

    if let Some(tray) = anchor {
        let cx = tray.x + tray.width / 2.0;
        let cy = tray.y + tray.height / 2.0;
        let distances = [
            (cy - work.y).abs(),
            (cy - work.y - work.height).abs(),
            (cx - work.x).abs(),
            (cx - work.x - work.width).abs(),
        ];
        let edge = distances
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(i, _)| i)
            .unwrap_or(0);
        x = cx - width / 2.0;
        y = cy - height / 2.0;
        match edge {
            0 => y = tray.y + tray.height + gap,
            1 => y = tray.y - height - gap,
            2 => x = tray.x + tray.width + gap,
            _ => x = tray.x - width - gap,
        }
    }

    Bounds {
        x: x.clamp(work.x + margin, work.x + work.width - width - margin),
        y: y.clamp(work.y + margin, work.y + work.height - height - margin),
        width,
        height,
    }
}

// Extend to the right while keeping the card in place whenever the work area
// allows it. All calculations use the same coordinate space as the original
// placement: Cocoa points on macOS, physical pixels on other platforms.
fn expanded_placement(work: Bounds, collapsed: Bounds, scale: f64) -> Bounds {
    let margin = (6.0 * scale).min(work.width / 4.0).min(work.height / 4.0);
    let width = (EXPANDED_PANEL_WIDTH * scale).min(work.width - margin * 2.0);
    let height = collapsed.height.min(work.height - margin * 2.0);
    Bounds {
        x: collapsed
            .x
            .clamp(work.x + margin, work.x + work.width - width - margin),
        y: collapsed
            .y
            .clamp(work.y + margin, work.y + work.height - height - margin),
        width,
        height,
    }
}

fn apply_bounds(
    window: &WebviewWindow,
    bounds: Bounds,
    scale: f64,
    resize_first: bool,
) -> tauri::Result<()> {
    let size = tauri::LogicalSize::new(bounds.width / scale, bounds.height / scale);
    // At the screen's right edge, moving the compact window left before
    // widening it would temporarily leave the pointer outside the window and
    // dispatch a mouseleave. Grow first so the old hover region stays covered.
    if resize_first {
        window.set_size(size)?;
    }
    // macOS converts physical positions with the window's old scale factor.
    // Use desktop points there so moving to a different-DPI screen is correct.
    #[cfg(target_os = "macos")]
    window.set_position(tauri::LogicalPosition::new(
        bounds.x / scale,
        bounds.y / scale,
    ))?;
    #[cfg(not(target_os = "macos"))]
    window.set_position(tauri::PhysicalPosition::new(
        bounds.x as i32,
        bounds.y as i32,
    ))?;
    if !resize_first {
        window.set_size(size)?;
    }
    Ok(())
}

fn position_panel(
    window: &WebviewWindow,
    anchor: Option<Rect>,
) -> tauri::Result<Option<PanelGeometry>> {
    #[cfg(target_os = "macos")]
    if let Some(geometry) = macos_placement(window)? {
        apply_bounds(window, geometry.collapsed, geometry.scale, false)?;
        return Ok(Some(geometry));
    }

    let monitors = window.available_monitors()?;
    // Tray events and monitor bounds are physical. Avoid monitor_from_point's
    // platform-dependent coordinate interpretation when selecting a display.
    let monitor = monitors
        .iter()
        .find(|monitor| {
            let Some(rect) = anchor else {
                return false;
            };
            let tray = Bounds::from_rect(rect, monitor.scale_factor());
            let position = monitor.position();
            let size = monitor.size();
            Bounds {
                x: position.x as f64,
                y: position.y as f64,
                width: size.width as f64,
                height: size.height as f64,
            }
            .contains(tray.x + tray.width / 2.0, tray.y + tray.height / 2.0)
        })
        .cloned()
        .or(window.current_monitor()?)
        .or(window.primary_monitor()?);

    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let geometry = PanelGeometry::new(
            Bounds {
                x: area.position.x as f64,
                y: area.position.y as f64,
                width: area.size.width as f64,
                height: area.size.height as f64,
            },
            anchor.map(|rect| Bounds::from_rect(rect, scale)),
            scale,
        );
        apply_bounds(window, geometry.collapsed, geometry.scale, false)?;
        return Ok(Some(geometry));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
fn macos_placement(window: &WebviewWindow) -> tauri::Result<Option<PanelGeometry>> {
    let Some(primary) = window.primary_monitor()? else {
        return Ok(None);
    };
    let primary_height = primary.size().height as f64 / primary.scale_factor();
    let Some(tray) = window.app_handle().tray_by_id(TRAY_ID) else {
        return Ok(None);
    };
    // macOS tray and monitor physical rectangles scale global origins separately.
    // Read the status item's actual screen and use Cocoa points throughout, so
    // adjacent Retina / non-Retina displays cannot be mistaken for each other.
    tray.with_inner_tray_icon(move |tray| {
        let mtm = objc2::MainThreadMarker::new()?;
        let item = tray.ns_status_item()?;
        let button = item.button(mtm)?;
        let tray_window = button.window()?;
        let frame = tray_window.frame();
        let visible = tray_window.screen()?.visibleFrame();
        Some(PanelGeometry::new(
            Bounds {
                x: visible.origin.x,
                y: primary_height - visible.origin.y - visible.size.height,
                width: visible.size.width,
                height: visible.size.height,
            },
            Some(Bounds {
                x: frame.origin.x,
                y: primary_height - frame.origin.y - frame.size.height,
                width: frame.size.width,
                height: frame.size.height,
            }),
            1.0,
        ))
    })
}

pub fn show(app: &AppHandle, anchor: Option<Rect>, page: &str) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        app.state::<PanelState>()
            .revision
            .fetch_add(1, Ordering::Relaxed);
        let anchor = anchor.or(app
            .tray_by_id(TRAY_ID)
            .map(|tray| tray.rect())
            .transpose()?
            .flatten());
        *app.state::<PanelState>()
            .geometry
            .lock()
            .expect("panel geometry lock") = position_panel(&window, anchor)?;
        window.emit(page, ())?;
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    app.state::<PanelState>()
        .revision
        .fetch_add(1, Ordering::Relaxed);
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
        // Restore the native size while hidden as well as forgetting the hover
        // state, so the next tray opening always starts with the compact panel.
        let result = set_expanded(app, false);
        *app.state::<PanelState>()
            .geometry
            .lock()
            .expect("panel geometry lock") = None;
        result?;
    }
    Ok(())
}

pub fn set_expanded(app: &AppHandle, expanded: bool) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    // A hover command that arrived after dismissal must not resize the hidden
    // window. Collapse is still allowed so hide() can restore its dimensions.
    if expanded && !window.is_visible()? {
        return Ok(());
    }
    let state = app.state::<PanelState>();
    let mut stored = state.geometry.lock().expect("panel geometry lock");
    let Some(geometry) = stored.as_mut() else {
        return Ok(());
    };
    if geometry.expanded == expanded {
        return Ok(());
    }
    let bounds = if expanded {
        expanded_placement(geometry.work, geometry.collapsed, geometry.scale)
    } else {
        geometry.collapsed
    };
    apply_bounds(&window, bounds, geometry.scale, expanded)?;
    geometry.expanded = expanded;
    Ok(())
}

pub fn press(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        *app.state::<PanelState>()
            .visible_on_press
            .lock()
            .expect("panel press lock") = Some(window.is_visible()?);
    }
    Ok(())
}

pub fn toggle(app: &AppHandle, anchor: Rect) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        let was_visible = app
            .state::<PanelState>()
            .visible_on_press
            .lock()
            .expect("panel press lock")
            .take()
            .unwrap_or(window.is_visible()?);
        if was_visible {
            hide(app)?;
        } else {
            show(app, Some(anchor), "navigate-usage")?;
        }
    }
    Ok(())
}

pub fn focus_changed(app: &AppHandle, focused: bool) {
    let state = app.state::<PanelState>();
    if focused {
        state.revision.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let revision = state.revision.load(Ordering::Relaxed);
    let app = app.clone();
    // Allow the tray's mouse-down event to capture visibility before dismissal.
    // A focus change or explicit show/hide invalidates this pending close.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let handle = app.clone();
        if let Err(error) = app.run_on_main_thread(move || {
            if handle
                .state::<PanelState>()
                .revision
                .load(Ordering::Relaxed)
                != revision
            {
                return;
            }
            if let Some(window) = handle.get_webview_window("main") {
                if matches!(window.is_focused(), Ok(false)) {
                    super::log_result(hide(&handle));
                }
            }
        }) {
            eprintln!("收起统计面板失败：{error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    const WORK: Bounds = Bounds {
        x: 0.0,
        y: 24.0,
        width: 1440.0,
        height: 836.0,
    };

    #[test]
    fn menu_bar_panel_is_below_icon_and_clamped_at_right_edge() {
        let panel = placement(
            WORK,
            Some(Bounds {
                x: 1380.0,
                y: 0.0,
                width: 24.0,
                height: 24.0,
            }),
            1.0,
        );
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (1074.0, 30.0, 360.0, 600.0)
        );
    }

    #[test]
    fn bottom_taskbar_panel_opens_upward() {
        let panel = placement(
            WORK,
            Some(Bounds {
                x: 900.0,
                y: 860.0,
                width: 24.0,
                height: 40.0,
            }),
            1.0,
        );
        assert_eq!((panel.x, panel.y), (732.0, 254.0));
    }

    #[test]
    fn side_taskbars_open_towards_work_area() {
        let left = placement(
            WORK,
            Some(Bounds {
                x: -40.0,
                y: 400.0,
                width: 40.0,
                height: 24.0,
            }),
            1.0,
        );
        let right = placement(
            WORK,
            Some(Bounds {
                x: 1440.0,
                y: 400.0,
                width: 40.0,
                height: 24.0,
            }),
            1.0,
        );
        assert_eq!((left.x, right.x), (6.0, 1074.0));
    }

    #[test]
    fn retina_monitor_with_negative_origin_keeps_panel_inside_work_area() {
        let work = Bounds {
            x: -2880.0,
            y: -1600.0,
            width: 2880.0,
            height: 1550.0,
        };
        let panel = placement(
            work,
            Some(Bounds {
                x: -2800.0,
                y: -1648.0,
                width: 48.0,
                height: 48.0,
            }),
            2.0,
        );
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (-2868.0, -1588.0, 720.0, 1200.0)
        );
    }

    #[test]
    fn short_display_shrinks_panel_instead_of_putting_controls_offscreen() {
        let work = Bounds {
            x: 0.0,
            y: 24.0,
            width: 320.0,
            height: 400.0,
        };
        let panel = placement(work, None, 1.0);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (6.0, 30.0, 308.0, 388.0)
        );
    }

    #[test]
    fn expansion_keeps_left_edge_when_there_is_room_on_the_right() {
        let collapsed = Bounds {
            x: 200.0,
            y: 80.0,
            width: 360.0,
            height: 600.0,
        };
        let expanded = expanded_placement(WORK, collapsed, 1.0);
        assert_eq!(
            (expanded.x, expanded.y, expanded.width, expanded.height),
            (200.0, 80.0, 700.0, 600.0)
        );
    }

    #[test]
    fn expansion_at_right_edge_shifts_left_and_retains_original_bounds() {
        let geometry = PanelGeometry::new(WORK, None, 1.0);
        let expanded = expanded_placement(geometry.work, geometry.collapsed, geometry.scale);
        assert_eq!(
            (expanded.x, expanded.y, expanded.width, expanded.height),
            (734.0, 30.0, 700.0, 600.0)
        );
        assert_eq!(
            (
                geometry.collapsed.x,
                geometry.collapsed.y,
                geometry.collapsed.width,
                geometry.collapsed.height,
            ),
            (1074.0, 30.0, 360.0, 600.0)
        );
    }

    #[test]
    fn expansion_on_negative_retina_display_uses_physical_pixels() {
        let work = Bounds {
            x: -2880.0,
            y: -1600.0,
            width: 2880.0,
            height: 1550.0,
        };
        let collapsed = placement(work, None, 2.0);
        let expanded = expanded_placement(work, collapsed, 2.0);
        assert_eq!(
            (expanded.x, expanded.y, expanded.width, expanded.height),
            (-1412.0, -1588.0, 1400.0, 1200.0)
        );
    }

    #[test]
    fn expansion_on_small_display_remains_inside_work_area() {
        let work = Bounds {
            x: -320.0,
            y: 24.0,
            width: 320.0,
            height: 400.0,
        };
        let collapsed = placement(work, None, 1.0);
        let expanded = expanded_placement(work, collapsed, 1.0);
        assert_eq!(
            (expanded.x, expanded.y, expanded.width, expanded.height),
            (-314.0, 30.0, 308.0, 388.0)
        );
    }
}
