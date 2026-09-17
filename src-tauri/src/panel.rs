use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::models::ProviderId;

pub const TRAY_ID: &str = "agentbar-tray";
const PANEL_WIDTH: f64 = 320.0;
const MENU_BAR_GAP: f64 = 1.0;
// Only used before the webview has supplied its first content measurement.
const INITIAL_PANEL_HEIGHT: f64 = 600.0;
const STATISTICS_WIDTH: f64 = 360.0;
const STATISTICS_MIN_WIDTH: f64 = 280.0;
const STATISTICS_LABEL: &str = "statistics";
const STATISTICS_EVENT: &str = "statistics-panel-changed";
const CASCADE_CLOSE_DELAY: Duration = Duration::from_millis(250);
const POINTER_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct PanelState {
    revision: AtomicU64,
    visible_on_press: Mutex<Option<bool>>,
    runtime: Mutex<PanelRuntime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatisticsSide {
    Left,
    Right,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatisticsPanelState {
    pub provider: Option<ProviderId>,
    pub side: Option<StatisticsSide>,
    pub revision: u64,
}

impl StatisticsPanelState {
    fn can_present(&self, revision: u64, main_visible: bool) -> bool {
        main_visible && self.provider.is_some() && self.revision == revision
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct AnchorRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl AnchorRect {
    pub fn valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Default)]
struct PanelRuntime {
    geometry: Option<PanelGeometry>,
    main_height: Option<f64>,
    statistics_height: Option<f64>,
    settings_height: Option<f64>,
    statistics: StatisticsPanelState,
    statistics_anchor: Option<AnchorRect>,
    statistics_bounds: Option<Bounds>,
    card_bounds: Option<Bounds>,
    pointer_watch: Option<PointerWatch>,
    pending_focus: bool,
    interaction: PanelInteraction,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum InteractionMode {
    #[default]
    Pointer,
    Keyboard,
}

#[derive(Default)]
struct PanelInteraction {
    main_keyboard: bool,
    statistics_keyboard: bool,
    mode: InteractionMode,
}

impl PanelInteraction {
    fn keyboard_active(&self) -> bool {
        self.mode == InteractionMode::Keyboard && (self.main_keyboard || self.statistics_keyboard)
    }

    fn update(&mut self, label: &str, keyboard: bool, intent: &str) {
        let mode = match intent {
            "pointer" | "leave" => InteractionMode::Pointer,
            "keyboard" => InteractionMode::Keyboard,
            "focus" => self.mode,
            _ => return,
        };
        match label {
            "main" => self.main_keyboard = keyboard,
            STATISTICS_LABEL => self.statistics_keyboard = keyboard,
            _ => return,
        }
        self.mode = mode;
    }

    fn begin_keyboard_navigation(&mut self) {
        self.mode = InteractionMode::Keyboard;
        // Keep explicit entry alive until the rendered companion takes focus.
        self.statistics_keyboard = true;
    }

    fn reset_statistics(&mut self) {
        self.statistics_keyboard = false;
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PointerPosition {
    x: f64,
    y: f64,
}

#[derive(Debug, Eq, PartialEq)]
enum WatchDecision {
    Keep,
    Close,
    Stop,
}

struct PointerWatch {
    revision: u64,
    last_pointer: Option<PointerPosition>,
    outside_since: Option<Instant>,
}

impl PointerWatch {
    fn new(revision: u64, pointer: Option<PointerPosition>) -> Self {
        Self {
            revision,
            last_pointer: pointer,
            outside_since: None,
        }
    }

    fn rebase(&mut self, pointer: Option<PointerPosition>) {
        self.last_pointer = pointer;
        self.outside_since = None;
    }

    fn sample(
        &mut self,
        revision: u64,
        pointer: PointerPosition,
        inside: bool,
        interaction: &mut PanelInteraction,
        now: Instant,
    ) -> WatchDecision {
        if self.revision != revision {
            return WatchDecision::Stop;
        }
        if self
            .last_pointer
            .is_some_and(|previous| previous != pointer)
        {
            interaction.mode = InteractionMode::Pointer;
        }
        self.last_pointer = Some(pointer);
        if inside || interaction.keyboard_active() {
            self.outside_since = None;
            return WatchDecision::Keep;
        }
        let since = *self.outside_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= CASCADE_CLOSE_DELAY {
            WatchDecision::Close
        } else {
            WatchDecision::Keep
        }
    }
}

#[derive(Clone, Copy)]
struct PanelGeometry {
    collapsed: Bounds,
    work: Bounds,
    anchor: Option<Bounds>,
    scale: f64,
}

impl PanelGeometry {
    fn new(work: Bounds, anchor: Option<Bounds>, scale: f64, height: f64) -> Self {
        Self {
            collapsed: placement(work, anchor, scale, height),
            work,
            anchor,
            scale,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

fn card_placement(main: Bounds, scale: f64, anchor: AnchorRect) -> Option<Bounds> {
    if !anchor.valid() {
        return None;
    }
    let left = (main.x + anchor.x * scale).max(main.x);
    let top = (main.y + anchor.y * scale).max(main.y);
    let right = (main.x + (anchor.x + anchor.width) * scale).min(main.x + main.width);
    let bottom = (main.y + (anchor.y + anchor.height) * scale).min(main.y + main.height);
    (right > left && bottom > top).then_some(Bounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn pointer_inside(
    pointer: PointerPosition,
    card: Option<Bounds>,
    visible_statistics: Option<Bounds>,
    scale: f64,
) -> bool {
    let bridge = card.zip(visible_statistics).and_then(|(card, statistics)| {
        let (left, right) = if statistics.x >= card.x + card.width {
            (card.x + card.width, statistics.x)
        } else if card.x >= statistics.x + statistics.width {
            (statistics.x + statistics.width, card.x)
        } else {
            return None;
        };
        let top = card.y.max(statistics.y);
        let bottom = (card.y + card.height).min(statistics.y + statistics.height);
        (right - left <= 32.0 * scale && bottom > top).then_some(Bounds {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    });
    // Bridge only the narrow horizontal gap at the card's overlapping height.
    // Main-window blank space below the card never becomes an active region.
    [card, visible_statistics, bridge]
        .into_iter()
        .flatten()
        .any(|bounds| bounds.contains(pointer.x, pointer.y))
}

#[cfg(any(target_os = "macos", test))]
fn cocoa_pointer_position(x: f64, y: f64, primary_scale: f64) -> PointerPosition {
    PointerPosition {
        x: x / primary_scale,
        y: y / primary_scale,
    }
}

fn native_pointer_position(app: &AppHandle) -> tauri::Result<PointerPosition> {
    let pointer = app.cursor_position()?;
    #[cfg(target_os = "macos")]
    {
        // Tao multiplies global Cocoa coordinates by the PRIMARY display scale,
        // even when the pointer is over a different-DPI monitor. Undo exactly
        // that factor to match macos_placement's desktop points.
        let primary = app.primary_monitor()?.ok_or(tauri::Error::WindowNotFound)?;
        Ok(cocoa_pointer_position(
            pointer.x,
            pointer.y,
            primary.scale_factor(),
        ))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(PointerPosition {
        x: pointer.x,
        y: pointer.y,
    })
}

fn valid_content_height(height: f64) -> bool {
    height.is_finite() && height > 0.0
}

fn fitted_height(content_height: f64, scale: f64, available: f64) -> f64 {
    (content_height.ceil().max(1.0) * scale).min(available)
}

// Keep the settings window's existing top edge until the new content would
// leave the work area. Decorations consume space outside the measured webview.
fn settings_placement(
    work: Bounds,
    current: Bounds,
    scale: f64,
    content_height: f64,
    decoration_height: f64,
) -> Bounds {
    let margin = (6.0 * scale).min(work.height / 4.0);
    let height = fitted_height(
        content_height,
        scale,
        (work.height - margin * 2.0 - decoration_height).max(scale),
    )
    .floor()
    .max(1.0);
    // Native getters report integer physical pixels. Normalize the calculated
    // bounds too, so fractional display scaling cannot cause repeated updates.
    let top = (work.y + margin).ceil();
    let bottom = (work.y + work.height - margin - height - decoration_height)
        .floor()
        .max(top);
    Bounds {
        y: current.y.clamp(top, bottom),
        height,
        ..current
    }
}

// Work in physical pixels, including monitors with negative desktop coordinates.
// The nearest work-area edge also handles an auto-hidden taskbar inside the screen.
fn placement(work: Bounds, anchor: Option<Bounds>, scale: f64, content_height: f64) -> Bounds {
    let margin = (6.0 * scale).min(work.width / 4.0).min(work.height / 4.0);
    let top_margin = (MENU_BAR_GAP * scale).min(work.height / 4.0);
    let gap = 4.0 * scale;
    let width = (PANEL_WIDTH * scale).min(work.width - margin * 2.0);
    let height = fitted_height(content_height, scale, work.height - top_margin - margin);
    let (mut x, mut y) = (work.x + work.width - width - margin, work.y + top_margin);

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
            0 => y = (tray.y + tray.height).max(work.y) + top_margin,
            1 => y = tray.y - height - gap,
            2 => x = tray.x + tray.width + gap,
            _ => x = tray.x - width - gap,
        }
    }

    Bounds {
        x: x.clamp(work.x + margin, work.x + work.width - width - margin),
        y: y.clamp(work.y + top_margin, work.y + work.height - height - margin),
        width,
        height,
    }
}

// Use Cocoa points on macOS and physical pixels elsewhere, matching the main
// panel. Keep their top edges aligned even when the companion needs scrolling.
// The primary panel never moves when the companion changes sides.
fn statistics_placement(
    work: Bounds,
    main: Bounds,
    scale: f64,
    content_height: f64,
) -> (Bounds, StatisticsSide) {
    let margin = (6.0 * scale).min(work.width / 4.0).min(work.height / 4.0);
    let gap = 4.0 * scale;
    let desired_width = (STATISTICS_WIDTH * scale).min(work.width - margin * 2.0);
    let min_width = STATISTICS_MIN_WIDTH * scale;
    let right_space = (work.x + work.width - margin - main.x - main.width - gap).max(0.0);
    let left_space = (main.x - gap - work.x - margin).max(0.0);
    let side = if right_space >= desired_width {
        StatisticsSide::Right
    } else if left_space >= desired_width {
        StatisticsSide::Left
    } else if right_space >= left_space {
        StatisticsSide::Right
    } else {
        StatisticsSide::Left
    };
    let available = match side {
        StatisticsSide::Right => right_space,
        StatisticsSide::Left => left_space,
    };
    // Small screens may not fit even a readable narrow companion. In that case
    // overlay within the work area instead of moving the primary or clipping
    // controls. The statistics view itself remains independently scrollable.
    let width = if available >= min_width {
        desired_width.min(available)
    } else {
        desired_width
    };
    let height = fitted_height(
        content_height,
        scale,
        work.y + work.height - margin - main.y,
    );
    let x = match side {
        StatisticsSide::Right => main.x + main.width + gap,
        StatisticsSide::Left => main.x - width - gap,
    };
    (
        Bounds {
            x: x.clamp(work.x + margin, work.x + work.width - width - margin),
            y: main.y,
            width,
            height,
        },
        side,
    )
}

fn apply_position(window: &WebviewWindow, bounds: Bounds, scale: f64) -> tauri::Result<()> {
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
    Ok(())
}

fn apply_bounds(window: &WebviewWindow, bounds: Bounds, scale: f64) -> tauri::Result<()> {
    apply_position(window, bounds, scale)?;
    window.set_size(tauri::LogicalSize::new(
        bounds.width / scale,
        bounds.height / scale,
    ))
}

fn position_panel(
    window: &WebviewWindow,
    anchor: Option<Rect>,
    height: f64,
) -> tauri::Result<Option<PanelGeometry>> {
    #[cfg(target_os = "macos")]
    if let Some(geometry) = macos_placement(window, height)? {
        apply_bounds(window, geometry.collapsed, geometry.scale)?;
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
            height,
        );
        apply_bounds(window, geometry.collapsed, geometry.scale)?;
        return Ok(Some(geometry));
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
fn macos_placement(window: &WebviewWindow, height: f64) -> tauri::Result<Option<PanelGeometry>> {
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
            height,
        ))
    })
}

pub fn initialize(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(STATISTICS_LABEL).is_some() {
        return Ok(());
    }
    let Some(main) = app.get_webview_window("main") else {
        return Ok(());
    };
    let builder = WebviewWindowBuilder::new(
        app,
        STATISTICS_LABEL,
        WebviewUrl::App("index.html#statistics".into()),
    )
    .title("AgentBar · 使用统计")
    .inner_size(STATISTICS_WIDTH, INITIAL_PANEL_HEIGHT)
    .resizable(false)
    .decorations(false)
    .minimizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    .shadow(true)
    .visible(false)
    .focused(false)
    // This webview owns remote refreshes and receives the render handshake
    // while hidden. Do not let WebKit suspend it between tray openings.
    .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
    .parent(&main)?;
    #[cfg(target_os = "macos")]
    let builder = builder
        .accept_first_mouse(true)
        .transparent(true)
        // Match the main window's native glass in tauri.macos.conf.json.
        // Keep vibrancy active while the pointer moves between both panels.
        .effects(
            tauri::window::EffectsBuilder::new()
                .effect(tauri::window::Effect::UnderWindowBackground)
                .state(tauri::window::EffectState::Active)
                .radius(12.0)
                .build(),
        );
    builder.build()?;
    Ok(())
}

fn emit_statistics_state(app: &AppHandle, state: &StatisticsPanelState) -> tauri::Result<()> {
    for label in ["main", STATISTICS_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            window.emit(STATISTICS_EVENT, state)?;
        }
    }
    Ok(())
}

pub fn statistics_state(app: &AppHandle) -> StatisticsPanelState {
    app.state::<PanelState>()
        .runtime
        .lock()
        .expect("panel runtime lock")
        .statistics
        .clone()
}

pub fn show(app: &AppHandle, anchor: Option<Rect>, page: &str) -> tauri::Result<()> {
    hide_statistics(app, false)?;
    if let Some(window) = app.get_webview_window("main") {
        let state = app.state::<PanelState>();
        state.revision.fetch_add(1, Ordering::Relaxed);
        let anchor = anchor.or(app
            .tray_by_id(TRAY_ID)
            .map(|tray| tray.rect())
            .transpose()?
            .flatten());
        {
            let mut runtime = state.runtime.lock().expect("panel runtime lock");
            let height = runtime.main_height.unwrap_or(INITIAL_PANEL_HEIGHT);
            runtime.geometry = position_panel(&window, anchor, height)?;
            runtime.interaction.reset();
        }
        window.emit(page, ())?;
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

pub fn resize_content_window(
    app: &AppHandle,
    window: &WebviewWindow,
    height: f64,
    revision: Option<u64>,
) -> tauri::Result<()> {
    if !valid_content_height(height) {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "窗口内容高度无效").into(),
        );
    }
    if window.label() == "settings" {
        app.state::<PanelState>()
            .runtime
            .lock()
            .expect("panel runtime lock")
            .settings_height = Some(height);
        return resize_settings(window, height);
    }
    let state = app.state::<PanelState>();
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    match window.label() {
        "main" => runtime.main_height = Some(height),
        STATISTICS_LABEL => {
            // A previous provider's late observer result must not resize the
            // current selection or a dismissed companion.
            if runtime.statistics.provider.is_none()
                || revision.is_some_and(|revision| revision != runtime.statistics.revision)
            {
                return Ok(());
            }
            runtime.statistics_height = Some(height);
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "该窗口不支持内容自适应高度",
            )
            .into());
        }
    }
    let Some(mut geometry) = runtime.geometry else {
        // Hidden main panels still retain their full desired height. Opening
        // on another monitor clamps that measurement to the new work area.
        return Ok(());
    };
    let main_changed = if window.label() == "main" {
        let bounds = placement(geometry.work, geometry.anchor, geometry.scale, height);
        let changed = bounds != geometry.collapsed;
        if changed {
            apply_bounds(window, bounds, geometry.scale)?;
            geometry.collapsed = bounds;
            runtime.geometry = Some(geometry);
        }
        changed
    } else {
        false
    };
    if let Some(anchor) = runtime.statistics_anchor {
        runtime.card_bounds = card_placement(geometry.collapsed, geometry.scale, anchor);
        let (bounds, side) = statistics_placement(
            geometry.work,
            geometry.collapsed,
            geometry.scale,
            runtime.statistics_height.unwrap_or(INITIAL_PANEL_HEIGHT),
        );
        // Native child windows can move with their parent. Reapply the desired
        // companion position even if its recorded bounds did not change.
        if main_changed || runtime.statistics_bounds != Some(bounds) {
            if let Some(statistics) = app.get_webview_window(STATISTICS_LABEL) {
                apply_bounds(&statistics, bounds, geometry.scale)?;
            }
            runtime.statistics_bounds = Some(bounds);
            if let Some(watch) = runtime.pointer_watch.as_mut() {
                watch.rebase(native_pointer_position(app).ok());
            }
        }
        if runtime.statistics.side != Some(side) {
            runtime.statistics.side = Some(side);
            // Geometry changes preserve the content-render handshake revision.
            emit_statistics_state(app, &runtime.statistics)?;
        }
    }
    Ok(())
}

pub fn settings_monitor_changed(app: &AppHandle) -> tauri::Result<()> {
    // Initial native move events can precede setup and the first DOM measure.
    let Some(state) = app.try_state::<PanelState>() else {
        return Ok(());
    };
    let height = state
        .runtime
        .lock()
        .expect("panel runtime lock")
        .settings_height;
    // Release the lock before changing the native window, which may emit Moved.
    if let (Some(height), Some(window)) = (height, app.get_webview_window("settings")) {
        resize_settings(&window, height)?;
    }
    Ok(())
}

fn resize_settings(window: &WebviewWindow, height: f64) -> tauri::Result<()> {
    let inner = window.inner_size()?;
    let outer = window.outer_size()?;
    let scale = window.scale_factor()?;
    let Some(monitor) = window.current_monitor()?.or(window.primary_monitor()?) else {
        if (height.ceil() * scale).round() != inner.height as f64 {
            window.set_size(tauri::LogicalSize::new(
                inner.width as f64 / scale,
                height.ceil(),
            ))?;
        }
        return Ok(());
    };
    let area = monitor.work_area();
    let position = window.outer_position()?;
    let bounds = settings_placement(
        Bounds {
            x: area.position.x as f64,
            y: area.position.y as f64,
            width: area.size.width as f64,
            height: area.size.height as f64,
        },
        Bounds {
            x: position.x as f64,
            y: position.y as f64,
            width: inner.width as f64,
            height: inner.height as f64,
        },
        scale,
        height,
        outer.height.saturating_sub(inner.height) as f64,
    );
    // A native Moved callback also reaches this path. Only change each property
    // when necessary, so correcting the position cannot create a resize loop.
    if bounds.height != inner.height as f64 {
        window.set_size(tauri::LogicalSize::new(
            bounds.width / scale,
            bounds.height / scale,
        ))?;
    }
    if bounds.y != position.y as f64 {
        apply_position(window, bounds, scale)?;
    }
    Ok(())
}

pub fn show_statistics(
    app: &AppHandle,
    provider: ProviderId,
    anchor: AnchorRect,
    focus: bool,
    update_only: bool,
) -> tauri::Result<()> {
    if !anchor.valid() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "统计卡片位置无效").into(),
        );
    }
    let Some(main) = app.get_webview_window("main") else {
        return Ok(());
    };
    let Some(window) = app.get_webview_window(STATISTICS_LABEL) else {
        return Ok(());
    };
    let state = app.state::<PanelState>();
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    // Scroll/resize observations only refresh the currently selected card.
    // A queued geometry update cannot reopen or switch a dismissed selection.
    if update_only && runtime.statistics.provider != Some(provider) {
        return Ok(());
    }
    // A hover command that arrives after dismissal cannot reopen the group.
    let Some(geometry) = runtime
        .geometry
        .filter(|_| main.is_visible().unwrap_or(false))
    else {
        return Ok(());
    };
    runtime.statistics_anchor = Some(anchor);
    runtime.card_bounds = card_placement(geometry.collapsed, geometry.scale, anchor);
    let (bounds, side) = statistics_placement(
        geometry.work,
        geometry.collapsed,
        geometry.scale,
        runtime.statistics_height.unwrap_or(INITIAL_PANEL_HEIGHT),
    );
    let provider_changed = runtime.statistics.provider != Some(provider);
    if provider_changed {
        runtime.pending_focus = focus;
    } else if focus {
        runtime.pending_focus = true;
    }
    if focus && !provider_changed {
        runtime.interaction.begin_keyboard_navigation();
        if let Some(watch) = runtime.pointer_watch.as_mut() {
            // Movement before this explicit entry must not cancel its keyboard
            // intent at the next native sample.
            watch.rebase(native_pointer_position(app).ok());
        }
    }
    if !provider_changed && runtime.statistics_bounds == Some(bounds) {
        if focus && window.is_visible()? {
            window.set_focus()?;
            runtime.pending_focus = false;
        }
        return Ok(());
    }
    if provider_changed {
        // Keep the group focused while replacing a currently-key companion.
        if window.is_focused()? {
            main.set_focus()?;
        }
        window.hide()?;
        runtime.interaction.reset_statistics();
        if focus {
            runtime.interaction.begin_keyboard_navigation();
        }
    }
    apply_bounds(&window, bounds, geometry.scale)?;
    runtime.statistics_bounds = Some(bounds);
    let watch_revision = if provider_changed || runtime.statistics.side != Some(side) {
        runtime.statistics.provider = Some(provider);
        runtime.statistics.side = Some(side);
        runtime.statistics.revision = runtime.statistics.revision.wrapping_add(1);
        let revision = runtime.statistics.revision;
        runtime.pointer_watch = Some(PointerWatch::new(
            revision,
            native_pointer_position(app).ok(),
        ));
        emit_statistics_state(app, &runtime.statistics)?;
        Some(revision)
    } else {
        None
    };
    if !provider_changed && focus && window.is_visible()? {
        window.set_focus()?;
        runtime.pending_focus = false;
    }
    drop(runtime);
    if let Some(revision) = watch_revision {
        start_pointer_watch(app, revision);
    }
    Ok(())
}

fn sample_native_pointer(
    app: &AppHandle,
    runtime: &mut PanelRuntime,
    revision: u64,
    now: Instant,
) -> tauri::Result<WatchDecision> {
    if runtime.statistics.provider.is_none()
        || runtime.statistics.revision != revision
        || runtime.geometry.is_none()
    {
        return Ok(WatchDecision::Stop);
    }
    let Some(main) = app.get_webview_window("main") else {
        return Ok(WatchDecision::Stop);
    };
    if !main.is_visible()? {
        return Ok(WatchDecision::Stop);
    }
    let pointer = native_pointer_position(app)?;
    let statistics_visible = app
        .get_webview_window(STATISTICS_LABEL)
        .map(|window| window.is_visible())
        .transpose()?
        .unwrap_or(false);
    let inside = pointer_inside(
        pointer,
        runtime.card_bounds,
        runtime.statistics_bounds.filter(|_| statistics_visible),
        runtime.geometry.expect("checked panel geometry").scale,
    );
    Ok(match runtime.pointer_watch.as_mut() {
        Some(watch) => watch.sample(revision, pointer, inside, &mut runtime.interaction, now),
        None => WatchDecision::Stop,
    })
}

fn poll_pointer_watch(app: &AppHandle, revision: u64) -> tauri::Result<bool> {
    let decision = {
        let state = app.state::<PanelState>();
        let mut runtime = state.runtime.lock().expect("panel runtime lock");
        sample_native_pointer(app, &mut runtime, revision, Instant::now())?
    };
    match decision {
        WatchDecision::Keep => Ok(true),
        WatchDecision::Stop => Ok(false),
        WatchDecision::Close => {
            // Recheck the real pointer and revision in the final close path.
            close_statistics(app, false, Some(revision))?;
            Ok(statistics_state(app).can_present(revision, true))
        }
    }
}

fn start_pointer_watch(app: &AppHandle, revision: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut error_reported = false;
        loop {
            tokio::time::sleep(POINTER_POLL_INTERVAL).await;
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let handle = app.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                let _ = sender.send(poll_pointer_watch(&handle, revision));
            }) {
                eprintln!("检测统计窗口鼠标位置失败：{error}");
                break;
            }
            // At most one main-thread sample is queued, including while a native
            // popup menu runs its own tracking loop. Nothing polls while hidden.
            match receiver.await {
                Ok(Ok(true)) => error_reported = false,
                Ok(Ok(false)) | Err(_) => break,
                Ok(Err(error)) => {
                    if !error_reported {
                        eprintln!("检测统计窗口鼠标位置失败：{error}");
                        error_reported = true;
                    }
                }
            }
        }
    });
}

pub fn present_statistics(app: &AppHandle, revision: u64) -> tauri::Result<()> {
    let handle = app.clone();
    // AppKit ordering must run on the main thread. Recheck the selection inside
    // the dispatched closure so dismissal also invalidates queued presentation.
    app.run_on_main_thread(move || {
        super::log_result(present_statistics_on_main(&handle, revision));
    })
}

fn present_statistics_on_main(app: &AppHandle, revision: u64) -> tauri::Result<()> {
    let Some(main) = app.get_webview_window("main") else {
        return Ok(());
    };
    let state = app.state::<PanelState>();
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    // Late readiness from a previous provider or a dismissed panel is ignored.
    if runtime.geometry.is_some() && runtime.statistics.can_present(revision, main.is_visible()?) {
        if let Some(window) = app.get_webview_window(STATISTICS_LABEL) {
            #[cfg(target_os = "macos")]
            order_statistics_front(&window, &main)?;
            #[cfg(not(target_os = "macos"))]
            window.show()?;
            if runtime.pending_focus {
                window.set_focus()?;
                runtime.pending_focus = false;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn order_statistics_front(window: &WebviewWindow, main: &WebviewWindow) -> tauri::Result<()> {
    use objc2::{msg_send, runtime::AnyObject};

    debug_assert!(objc2::MainThreadMarker::new().is_some());
    let child = window.ns_window()?.cast::<AnyObject>();
    let parent = main.ns_window()?.cast::<AnyObject>();
    if child.is_null() || parent.is_null() {
        return Err(tauri::Error::WindowNotFound);
    }
    // SAFETY: These live Tauri windows own the NSWindow pointers, and the caller
    // runs on AppKit's main thread. orderOut: may detach a child; restore the
    // relationship before ordering it. NSWindowAbove is NSInteger value 1.
    unsafe {
        let previous_parent: *mut AnyObject = msg_send![child, parentWindow];
        if previous_parent != parent {
            if !previous_parent.is_null() {
                let _: () = msg_send![previous_parent, removeChildWindow: child];
            }
            let _: () = msg_send![parent, addChildWindow: child, ordered: 1_isize];
        }
        // Tauri show() calls makeKeyAndOrderFront: on macOS. Hover should only
        // order the companion forward, retaining whichever window is already key.
        let _: () = msg_send![child, orderFront: None::<&AnyObject>];
    }
    Ok(())
}

fn clear_statistics(runtime: &mut PanelRuntime) {
    runtime.statistics.provider = None;
    runtime.statistics.side = None;
    runtime.statistics.revision = runtime.statistics.revision.wrapping_add(1);
    runtime.statistics_bounds = None;
    runtime.statistics_anchor = None;
    runtime.card_bounds = None;
    runtime.pointer_watch = None;
    runtime.pending_focus = false;
    runtime.interaction.reset_statistics();
}

fn close_statistics(
    app: &AppHandle,
    focus_main: bool,
    idle_revision: Option<u64>,
) -> tauri::Result<()> {
    let state = app.state::<PanelState>();
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    if let Some(revision) = idle_revision {
        if sample_native_pointer(app, &mut runtime, revision, Instant::now())?
            != WatchDecision::Close
        {
            return Ok(());
        }
    }
    // A pointer departure only dismisses the companion. If it was key, return
    // focus to the visible primary before the deferred group-blur check fires.
    let restore_main_focus = focus_main
        || (idle_revision.is_some()
            && app
                .get_webview_window(STATISTICS_LABEL)
                .is_some_and(|window| window.is_focused().unwrap_or(false))
            && app
                .get_webview_window("main")
                .is_some_and(|window| window.is_visible().unwrap_or(false)));
    clear_statistics(&mut runtime);
    if restore_main_focus {
        // Closing the key child creates a temporary focus gap. Invalidate any
        // pending group dismissal before returning keyboard control to main.
        state.revision.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(window) = app.get_webview_window(STATISTICS_LABEL) {
        window.hide()?;
    }
    emit_statistics_state(app, &runtime.statistics)?;
    if restore_main_focus {
        if let Some(window) = app.get_webview_window("main") {
            if window.is_visible()? {
                window.set_focus()?;
            }
        }
    }
    Ok(())
}

pub fn hide_statistics(app: &AppHandle, focus_main: bool) -> tauri::Result<()> {
    close_statistics(app, focus_main, None)
}

pub fn hide(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<PanelState>();
    state.revision.fetch_add(1, Ordering::Relaxed);
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    runtime.geometry = None;
    runtime.interaction.reset();
    clear_statistics(&mut runtime);
    if let Some(window) = app.get_webview_window(STATISTICS_LABEL) {
        window.hide()?;
    }
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
    }
    emit_statistics_state(app, &runtime.statistics)?;
    Ok(())
}

pub fn dismiss(app: &AppHandle) -> tauri::Result<()> {
    if statistics_state(app).provider.is_some() {
        hide_statistics(app, true)
    } else {
        hide(app)
    }
}

pub fn interaction_changed(
    app: &AppHandle,
    label: &str,
    _hovered: bool,
    keyboard: bool,
    intent: &str,
) -> tauri::Result<()> {
    let state = app.state::<PanelState>();
    let mut runtime = state.runtime.lock().expect("panel runtime lock");
    if runtime.geometry.is_some() && matches!(label, "main" | STATISTICS_LABEL) {
        // DOM events convey input intent only. Native coordinates are the sole
        // authority for card/companion hover and automatic dismissal.
        runtime.interaction.update(label, keyboard, intent);
        if intent == "keyboard" && keyboard {
            if let Some(watch) = runtime.pointer_watch.as_mut() {
                // A key event is newer than any movement since the last poll.
                // Only movement after this event may switch back to pointer mode.
                watch.rebase(native_pointer_position(app).ok());
            }
        }
    }
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
            let group_focused = ["main", STATISTICS_LABEL].iter().any(|label| {
                handle.get_webview_window(label).is_some_and(|window| {
                    window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
                })
            });
            if !group_focused {
                super::log_result(hide(&handle));
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
    fn content_measurements_reject_invalid_or_empty_heights() {
        for height in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            assert!(!valid_content_height(height));
        }
        assert!(valid_content_height(0.5));
        assert!(valid_content_height(1800.0));
    }

    #[test]
    fn panel_height_grows_and_shrinks_from_content_without_a_fixed_minimum() {
        let compact = placement(WORK, None, 1.0, 243.2);
        let expanded = placement(WORK, None, 1.0, 725.0);
        assert_eq!((compact.height, expanded.height), (244.0, 725.0));
        assert_eq!((compact.x, compact.y), (expanded.x, expanded.y));
        assert_eq!(placement(WORK, None, 1.0, 243.2), compact);
    }

    #[test]
    fn content_height_is_clamped_to_screen_but_can_expand_on_a_larger_display() {
        let desired = 1100.0;
        let short = placement(WORK, None, 1.0, desired);
        let tall = placement(
            Bounds {
                height: 1400.0,
                ..WORK
            },
            None,
            1.0,
            desired,
        );
        assert_eq!(short.height, 829.0);
        assert_eq!(tall.height, desired);
        assert_eq!(fitted_height(100.1, 2.0, 1000.0), 202.0);
    }

    #[test]
    fn bottom_anchored_panel_preserves_its_bottom_when_content_changes() {
        let tray = Some(Bounds {
            x: 900.0,
            y: 860.0,
            width: 24.0,
            height: 40.0,
        });
        let compact = placement(WORK, tray, 1.0, 240.0);
        let expanded = placement(WORK, tray, 1.0, 700.0);
        assert_eq!(compact.y + compact.height, expanded.y + expanded.height);
        assert_eq!(expanded.y, compact.y - 460.0);
    }

    #[test]
    fn statistics_content_changes_keep_main_top_and_scroll_at_screen_bottom() {
        let main = placement(WORK, None, 1.0, 400.0);
        let (expanded, side) = statistics_placement(WORK, main, 1.0, 1000.0);
        let (compact, compact_side) = statistics_placement(WORK, main, 1.0, 180.0);
        assert_eq!((expanded.y, expanded.height), (25.0, 829.0));
        assert_eq!((compact.y, compact.height), (25.0, 180.0));
        assert_eq!(side, compact_side);
        assert_eq!(expanded.x, compact.x);
    }

    #[test]
    fn settings_height_reserves_titlebar_and_keeps_window_in_work_area() {
        let current = Bounds {
            x: 400.0,
            y: 550.0,
            width: 420.0,
            height: 200.0,
        };
        let compact = settings_placement(WORK, current, 1.0, 200.0, 28.0);
        assert_eq!((compact.y, compact.height), (550.0, 200.0));
        let expanded = settings_placement(WORK, current, 1.0, 1200.0, 28.0);
        assert_eq!((expanded.y, expanded.height), (30.0, 796.0));
        assert_eq!(expanded.x, current.x);
        assert_eq!(expanded.width, current.width);
        assert_eq!(
            expanded.y + expanded.height + 28.0,
            WORK.y + WORK.height - 6.0
        );
    }

    #[test]
    fn settings_restores_desired_height_when_moved_to_a_taller_monitor() {
        let desired = 900.0;
        let current = Bounds {
            x: 100.0,
            y: 80.0,
            width: 420.0,
            height: desired,
        };
        let short = settings_placement(WORK, current, 1.0, desired, 28.0);
        assert_eq!(short.height, 796.0);
        let tall_work = Bounds {
            x: 1440.0,
            height: 1200.0,
            ..WORK
        };
        let moved = Bounds { x: 1500.0, ..short };
        let restored = settings_placement(tall_work, moved, 1.0, desired, 28.0);
        assert_eq!(restored.height, desired);
        assert_eq!(restored.x, moved.x);
        assert_eq!(
            settings_placement(tall_work, restored, 1.0, desired, 28.0),
            restored
        );
    }

    #[test]
    fn settings_fractional_scale_produces_stable_physical_bounds() {
        let current = Bounds {
            x: 500.0,
            y: 750.0,
            width: 525.0,
            height: 500.0,
        };
        let fitted = settings_placement(WORK, current, 1.25, 1000.0, 35.0);
        assert_eq!(fitted.height.fract(), 0.0);
        assert_eq!(fitted.y.fract(), 0.0);
        assert_eq!(settings_placement(WORK, fitted, 1.25, 1000.0, 35.0), fitted);
        let taller = Bounds {
            height: 1400.0,
            ..WORK
        };
        assert_eq!(
            settings_placement(taller, fitted, 1.25, 1000.0, 35.0).height,
            1250.0
        );
    }

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
            INITIAL_PANEL_HEIGHT,
        );
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (1114.0, 25.0, 320.0, 600.0)
        );
    }

    #[test]
    fn menu_bar_gap_is_one_point_with_or_without_a_full_height_tray_icon() {
        for scale in [1.0, 1.25, 2.0] {
            let work = Bounds {
                x: WORK.x * scale,
                y: WORK.y * scale,
                width: WORK.width * scale,
                height: WORK.height * scale,
            };
            for anchor in [
                None,
                Some(Bounds {
                    x: 600.0 * scale,
                    y: 2.0 * scale,
                    width: 20.0 * scale,
                    height: 20.0 * scale,
                }),
            ] {
                let main = placement(work, anchor, scale, 400.0);
                let (statistics, _) = statistics_placement(work, main, scale, 500.0);
                assert_eq!(main.y - work.y, scale);
                assert_eq!(statistics.y, main.y);
            }
        }
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
            INITIAL_PANEL_HEIGHT,
        );
        assert_eq!((panel.x, panel.y), (752.0, 254.0));
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
            INITIAL_PANEL_HEIGHT,
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
            INITIAL_PANEL_HEIGHT,
        );
        assert_eq!((left.x, right.x), (6.0, 1114.0));
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
            INITIAL_PANEL_HEIGHT,
        );
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (-2868.0, -1598.0, 640.0, 1200.0)
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
        let panel = placement(work, None, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (6.0, 25.0, 308.0, 393.0)
        );
    }

    #[test]
    fn statistics_prefers_right_and_aligns_with_main_top() {
        let main = Bounds {
            x: 200.0,
            y: 80.0,
            width: 360.0,
            height: 600.0,
        };
        let (bounds, side) = statistics_placement(WORK, main, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(side, StatisticsSide::Right);
        assert_eq!(
            bounds,
            Bounds {
                x: 564.0,
                y: 80.0,
                width: 360.0,
                height: 600.0
            }
        );
        assert_eq!(main.x, 200.0);
    }

    #[test]
    fn statistics_flips_left_at_right_edge_without_changing_top_alignment() {
        let main = placement(WORK, None, 1.0, INITIAL_PANEL_HEIGHT);
        let (bounds, side) = statistics_placement(WORK, main, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(side, StatisticsSide::Left);
        assert_eq!(
            bounds,
            Bounds {
                x: 750.0,
                y: 25.0,
                width: 360.0,
                height: 600.0
            }
        );
        assert_eq!(main.x, 1114.0);
    }

    #[test]
    fn statistics_on_negative_retina_display_scales_anchor_and_gap() {
        let work = Bounds {
            x: -2880.0,
            y: -1600.0,
            width: 2880.0,
            height: 1550.0,
        };
        let main = placement(work, None, 2.0, INITIAL_PANEL_HEIGHT);
        let (bounds, side) = statistics_placement(work, main, 2.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(side, StatisticsSide::Left);
        assert_eq!(
            bounds,
            Bounds {
                x: -1380.0,
                y: -1598.0,
                width: 720.0,
                height: 1200.0
            }
        );
    }

    #[test]
    fn cocoa_points_preserve_negative_origin_on_mixed_scale_monitors() {
        // macos_placement normalizes both Retina and non-Retina displays to
        // points before this calculation; the old window scale is irrelevant.
        let work = Bounds {
            x: -1920.0,
            y: 24.0,
            width: 1920.0,
            height: 1056.0,
        };
        let main = Bounds {
            x: -1914.0,
            y: 30.0,
            width: 360.0,
            height: 600.0,
        };
        let (bounds, side) = statistics_placement(work, main, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(side, StatisticsSide::Right);
        assert_eq!(bounds.x, -1550.0);
        assert_eq!(bounds.y, main.y);
    }

    #[test]
    fn statistics_uses_readable_narrower_side_when_neither_fits_full_width() {
        let work = Bounds {
            x: 0.0,
            y: 24.0,
            width: 1000.0,
            height: 836.0,
        };
        let main = Bounds {
            x: 330.0,
            y: 30.0,
            width: 360.0,
            height: 600.0,
        };
        let (bounds, side) = statistics_placement(work, main, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(side, StatisticsSide::Left);
        assert_eq!(
            bounds,
            Bounds {
                x: 6.0,
                y: 30.0,
                width: 320.0,
                height: 600.0
            }
        );
    }

    #[test]
    fn tiny_display_uses_readable_overlay_inside_work_area() {
        let work = Bounds {
            x: -320.0,
            y: 24.0,
            width: 320.0,
            height: 400.0,
        };
        let main = placement(work, None, 1.0, INITIAL_PANEL_HEIGHT);
        let (bounds, _) = statistics_placement(work, main, 1.0, INITIAL_PANEL_HEIGHT);
        assert_eq!(
            bounds,
            Bounds {
                x: -314.0,
                y: 25.0,
                width: 308.0,
                height: 393.0
            }
        );
        assert_eq!(main, bounds);
    }

    #[test]
    fn tall_statistics_scroll_below_a_lower_main_panel_without_moving_above_it() {
        let main = Bounds {
            x: 200.0,
            y: 300.0,
            width: PANEL_WIDTH,
            height: 200.0,
        };
        let (statistics, _) = statistics_placement(WORK, main, 1.0, 1000.0);
        assert_eq!(statistics.y, main.y);
        assert_eq!(statistics.height, 554.0);
        assert_eq!(statistics.y + statistics.height, WORK.y + WORK.height - 6.0);
    }

    const CARD: Bounds = Bounds {
        x: 20.0,
        y: 60.0,
        width: 320.0,
        height: 160.0,
    };
    const STATISTICS: Bounds = Bounds {
        x: 364.0,
        y: 60.0,
        width: 360.0,
        height: 600.0,
    };

    fn watch_sample(
        watch: &mut PointerWatch,
        interaction: &mut PanelInteraction,
        pointer: PointerPosition,
        now: Instant,
    ) -> WatchDecision {
        watch.sample(
            7,
            pointer,
            pointer_inside(pointer, Some(CARD), Some(STATISTICS), 1.0),
            interaction,
            now,
        )
    }

    #[test]
    fn card_to_gap_to_statistics_stays_open_without_any_dom_enter() {
        let now = Instant::now();
        let card = PointerPosition { x: 300.0, y: 100.0 };
        let gap = PointerPosition { x: 362.0, y: 100.0 };
        let statistics = PointerPosition { x: 500.0, y: 100.0 };
        let mut watch = PointerWatch::new(7, Some(card));
        let mut interaction = PanelInteraction::default();
        assert_eq!(
            watch_sample(&mut watch, &mut interaction, card, now),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                gap,
                now + Duration::from_millis(50)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                statistics,
                now + Duration::from_millis(100)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                statistics,
                now + Duration::from_secs(10)
            ),
            WatchDecision::Keep
        );
        assert_eq!(watch.outside_since, None);
    }

    #[test]
    fn narrow_horizontal_bridge_keeps_slow_crossing_open_but_excludes_main_blank() {
        let now = Instant::now();
        let gap = PointerPosition { x: 350.0, y: 100.0 };
        let blank = PointerPosition { x: 350.0, y: 400.0 };
        let mut watch = PointerWatch::new(7, Some(gap));
        let mut interaction = PanelInteraction::default();
        assert_eq!(
            watch_sample(&mut watch, &mut interaction, gap, now),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                gap,
                now + Duration::from_secs(2)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_secs(3)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(3250)
            ),
            WatchDecision::Close
        );
        let distant = Bounds {
            x: 500.0,
            ..STATISTICS
        };
        assert!(!pointer_inside(gap, Some(CARD), Some(distant), 1.0));
        let left_statistics = Bounds {
            x: -364.0,
            ..STATISTICS
        };
        assert!(pointer_inside(
            PointerPosition { x: 10.0, y: 100.0 },
            Some(CARD),
            Some(left_statistics),
            1.0
        ));
    }

    #[test]
    fn late_dom_leave_and_focus_cannot_close_a_real_pointer_inside_statistics() {
        let now = Instant::now();
        let pointer = PointerPosition { x: 500.0, y: 100.0 };
        let mut watch = PointerWatch::new(7, Some(pointer));
        let mut interaction = PanelInteraction::default();
        interaction.update("main", false, "leave");
        interaction.update(STATISTICS_LABEL, false, "leave");
        interaction.update(STATISTICS_LABEL, true, "focus");
        assert_eq!(
            watch_sample(&mut watch, &mut interaction, pointer, now),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                pointer,
                now + Duration::from_secs(1)
            ),
            WatchDecision::Keep
        );
    }

    #[test]
    fn leaving_statistics_for_main_blank_space_closes_after_continuous_250ms() {
        let now = Instant::now();
        let statistics = PointerPosition { x: 500.0, y: 100.0 };
        let blank = PointerPosition { x: 200.0, y: 400.0 };
        let mut watch = PointerWatch::new(7, Some(statistics));
        let mut interaction = PanelInteraction::default();
        interaction.update(STATISTICS_LABEL, true, "focus");
        assert_eq!(
            watch_sample(&mut watch, &mut interaction, blank, now),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(249)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(250)
            ),
            WatchDecision::Close
        );
    }

    #[test]
    fn returning_to_card_resets_the_continuous_outside_timer() {
        let now = Instant::now();
        let card = PointerPosition { x: 300.0, y: 100.0 };
        let blank = PointerPosition { x: 200.0, y: 400.0 };
        let mut watch = PointerWatch::new(7, Some(card));
        let mut interaction = PanelInteraction::default();
        watch_sample(&mut watch, &mut interaction, blank, now);
        watch_sample(
            &mut watch,
            &mut interaction,
            card,
            now + Duration::from_millis(200),
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(300)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(549)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                blank,
                now + Duration::from_millis(550)
            ),
            WatchDecision::Close
        );
    }

    #[test]
    fn keyboard_with_stationary_outside_pointer_survives_until_real_mouse_movement() {
        let now = Instant::now();
        let outside = PointerPosition {
            x: 1000.0,
            y: 700.0,
        };
        let moved = PointerPosition {
            x: 1001.0,
            y: 700.0,
        };
        let mut watch = PointerWatch::new(7, Some(outside));
        let mut interaction = PanelInteraction::default();
        interaction.begin_keyboard_navigation();
        assert_eq!(
            watch_sample(&mut watch, &mut interaction, outside, now),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                outside,
                now + Duration::from_secs(10)
            ),
            WatchDecision::Keep
        );
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                moved,
                now + Duration::from_secs(11)
            ),
            WatchDecision::Keep
        );
        interaction.update(STATISTICS_LABEL, true, "focus");
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                moved,
                now + Duration::from_millis(11250)
            ),
            WatchDecision::Close
        );
        interaction.update(STATISTICS_LABEL, true, "keyboard");
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                moved,
                now + Duration::from_secs(12)
            ),
            WatchDecision::Keep
        );
    }

    #[test]
    fn keyboard_rebase_ignores_movement_before_the_newer_key_event() {
        let now = Instant::now();
        let previous = PointerPosition {
            x: 1000.0,
            y: 700.0,
        };
        let before_key = PointerPosition {
            x: 1001.0,
            y: 700.0,
        };
        let after_key = PointerPosition {
            x: 1002.0,
            y: 700.0,
        };
        let mut watch = PointerWatch::new(7, Some(previous));
        let mut interaction = PanelInteraction::default();
        watch_sample(&mut watch, &mut interaction, previous, now);
        interaction.update(STATISTICS_LABEL, true, "keyboard");
        watch.rebase(Some(before_key));
        assert_eq!(watch.outside_since, None);
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                before_key,
                now + Duration::from_secs(1)
            ),
            WatchDecision::Keep
        );
        assert!(interaction.keyboard_active());
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                after_key,
                now + Duration::from_secs(2)
            ),
            WatchDecision::Keep
        );
        assert!(!interaction.keyboard_active());
        assert_eq!(
            watch_sample(
                &mut watch,
                &mut interaction,
                after_key,
                now + Duration::from_millis(2250)
            ),
            WatchDecision::Close
        );
    }

    #[test]
    fn new_selection_invalidates_old_watcher_and_hiding_clears_watch() {
        let now = Instant::now();
        let outside = PointerPosition {
            x: 1000.0,
            y: 700.0,
        };
        let mut watch = PointerWatch::new(7, Some(outside));
        let mut interaction = PanelInteraction::default();
        watch.sample(7, outside, false, &mut interaction, now);
        assert_eq!(
            watch.sample(
                8,
                outside,
                false,
                &mut interaction,
                now + Duration::from_secs(1)
            ),
            WatchDecision::Stop
        );
        let mut next_watch = PointerWatch::new(8, Some(outside));
        assert_eq!(
            next_watch.sample(
                8,
                outside,
                false,
                &mut interaction,
                now + Duration::from_secs(1)
            ),
            WatchDecision::Keep
        );
        let mut runtime = PanelRuntime {
            pointer_watch: Some(next_watch),
            ..PanelRuntime::default()
        };
        clear_statistics(&mut runtime);
        assert!(runtime.pointer_watch.is_none());
    }

    #[test]
    fn cocoa_pointer_uses_primary_scale_even_over_negative_secondary_monitor() {
        let pointer = cocoa_pointer_position(-2600.0, -800.0, 2.0);
        assert_eq!(
            pointer,
            PointerPosition {
                x: -1300.0,
                y: -400.0
            }
        );
        let secondary = Bounds {
            x: -1920.0,
            y: -600.0,
            width: 1920.0,
            height: 1080.0,
        };
        assert!(pointer_inside(pointer, Some(secondary), None, 1.0));
        assert_eq!(cocoa_pointer_position(-1300.0, -400.0, 1.0), pointer);
    }

    #[test]
    fn card_rect_scales_and_clips_to_main_viewport_without_preserving_blank_space() {
        let main = Bounds {
            x: -720.0,
            y: -300.0,
            width: 720.0,
            height: 1200.0,
        };
        let anchor = AnchorRect {
            x: 12.0,
            y: -20.0,
            width: 336.0,
            height: 100.0,
        };
        let card = card_placement(main, 2.0, anchor).unwrap();
        assert_eq!(
            card,
            Bounds {
                x: -696.0,
                y: -300.0,
                width: 672.0,
                height: 160.0
            }
        );
        assert!(pointer_inside(
            PointerPosition {
                x: -500.0,
                y: -250.0
            },
            Some(card),
            None,
            1.0
        ));
        assert!(!pointer_inside(
            PointerPosition {
                x: -500.0,
                y: 100.0
            },
            Some(card),
            None,
            1.0
        ));
        assert!(card_placement(main, 2.0, AnchorRect { y: 700.0, ..anchor }).is_none());
        for invalid in [
            AnchorRect {
                x: f64::NAN,
                ..anchor
            },
            AnchorRect {
                width: 0.0,
                ..anchor
            },
            AnchorRect {
                height: -1.0,
                ..anchor
            },
        ] {
            assert!(!invalid.valid());
            assert!(card_placement(main, 2.0, invalid).is_none());
        }
    }

    #[test]
    fn hidden_statistics_bounds_do_not_keep_pointer_alive() {
        let pointer = PointerPosition { x: 500.0, y: 100.0 };
        assert!(pointer_inside(pointer, Some(CARD), Some(STATISTICS), 1.0));
        assert!(!pointer_inside(pointer, Some(CARD), None, 1.0));
    }

    #[test]
    fn readiness_for_old_provider_or_hidden_group_cannot_present_child() {
        let mut runtime = PanelRuntime::default();
        runtime.statistics.provider = Some(ProviderId::Codex);
        runtime.statistics.side = Some(StatisticsSide::Right);
        runtime.statistics.revision = 10;
        assert!(runtime.statistics.can_present(10, true));
        assert!(!runtime.statistics.can_present(9, true));
        assert!(!runtime.statistics.can_present(10, false));
        clear_statistics(&mut runtime);
        assert_eq!(runtime.statistics.revision, 11);
        assert!(!runtime.statistics.can_present(10, true));
        assert!(!runtime.statistics.can_present(11, true));
    }
}
