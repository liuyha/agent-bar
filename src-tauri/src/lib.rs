mod account_statistics;
mod account_statistics_cache;
mod models;
mod panel;
mod providers;
mod state;
mod statistics;
mod storage;
mod tray_summary;

use std::time::Duration;

use models::{AppSettings, CodexStatisticsSource, DashboardSnapshot, ProviderId};
use state::AppState;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent,
};

#[tauri::command]
fn get_dashboard(state: State<'_, AppState>) -> Result<DashboardSnapshot, String> {
    state.snapshot()
}

#[tauri::command]
async fn refresh_dashboard(app: AppHandle) -> Result<DashboardSnapshot, String> {
    let providers = app.state::<AppState>().settings()?.enabled_providers;
    if providers.contains(&ProviderId::Codex) {
        let _ = app.emit("refresh-account-statistics", ());
    }
    for provider in providers {
        let _ = app.emit("refresh-token-statistics", provider);
    }
    refresh_and_emit(app).await
}

#[tauri::command]
async fn refresh_provider_dashboard(
    app: AppHandle,
    provider: ProviderId,
) -> Result<DashboardSnapshot, String> {
    if provider == ProviderId::Codex {
        let _ = app.emit("refresh-account-statistics", ());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = app.state::<AppState>().refresh_provider(provider);
        // A manual refresh also reloads local history when account collection
        // fails or returns the same unavailable state.
        let _ = app.emit("refresh-token-statistics", provider);
        let snapshot = result?;
        emit_snapshot(&app, &snapshot);
        Ok(snapshot)
    })
    .await
    .map_err(|_| "读取本机账号任务异常，请重试".to_string())?
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state.settings()
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: AppSettings) -> Result<AppSettings, String> {
    let worker_app = app.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let saved = state.save_settings(settings)?;
        #[cfg(target_os = "macos")]
        update_native_theme(&worker_app);
        if let Err(error) = worker_app.emit("settings-updated", &saved) {
            eprintln!("发送设置更新事件失败：{error}");
        }
        if let Ok(snapshot) = state.snapshot() {
            emit_snapshot(&worker_app, &snapshot);
        }
        Ok::<_, String>(saved)
    })
    .await
    .map_err(|_| "保存设置任务异常，请重试".to_string())??;
    Ok(saved)
}

#[cfg(target_os = "macos")]
fn update_native_theme(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        // Read the latest saved setting on the UI thread so delayed updates
        // cannot give the native glass a different appearance from the webview.
        let Ok(settings) = handle.state::<AppState>().settings() else {
            return;
        };
        handle.set_theme(match settings.theme {
            models::Theme::System => None,
            models::Theme::Light => Some(tauri::Theme::Light),
            models::Theme::Dark => Some(tauri::Theme::Dark),
        });
    }) {
        eprintln!("调度窗口主题更新失败：{error}");
    }
}

async fn refresh_and_emit(app: AppHandle) -> Result<DashboardSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = app.state::<AppState>().refresh()?;
        emit_snapshot(&app, &snapshot);
        Ok(snapshot)
    })
    .await
    .map_err(|_| "读取本机账号任务异常，请重试".to_string())?
}

fn emit_snapshot(app: &AppHandle, snapshot: &DashboardSnapshot) {
    update_tray_summary(app);
    // Saving settings already succeeded, so an event delivery failure must not turn
    // the save into an apparent failure. The frontend can always reload the cache.
    if let Err(error) = app.emit("usage-updated", snapshot) {
        eprintln!("发送用量更新事件失败：{error}");
    }
}

fn update_tray_summary(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        // Read the latest cache on the UI thread so delayed events cannot restore
        // an older account or a provider that was disabled in Settings.
        let Ok(snapshot) = handle.state::<AppState>().snapshot() else {
            return;
        };
        let Some(tray) = handle.tray_by_id(panel::TRAY_ID) else {
            return;
        };
        let now = chrono::Utc::now();
        let title = tray_summary::title(&snapshot, now).unwrap_or_default();
        // On macOS None leaves the previous title unchanged; an empty string clears it.
        if let Err(error) = tray.set_title(Some(title)) {
            eprintln!("更新菜单栏用量摘要失败：{error}");
        }
        if let Err(error) = tray.set_tooltip(Some(tray_summary::tooltip(&snapshot, now))) {
            eprintln!("更新菜单栏提示失败：{error}");
        }
    }) {
        eprintln!("调度菜单栏更新失败：{error}");
    }
}

fn start_tray_clock(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Recalculate the countdown from cached real data, even while the panel is
        // hidden. This does not make extra account/network requests.
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            update_tray_summary(&app);
        }
    });
}

#[tauri::command]
fn hide_panel(app: AppHandle) -> Result<(), String> {
    panel::hide(&app).map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_settings(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    show_settings(&app).map_err(|error| error.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    panel::hide(app)?;
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

#[tauri::command]
fn show_statistics_panel(
    app: AppHandle,
    window: WebviewWindow,
    provider: ProviderId,
    anchor: panel::AnchorRect,
    focus: bool,
    update_only: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只能从主面板展开统计".into());
    }
    if !anchor.valid() {
        return Err("统计面板位置无效".into());
    }
    if !app
        .state::<AppState>()
        .settings()?
        .enabled_providers
        .contains(&provider)
    {
        return Err("当前服务未启用".into());
    }
    panel::show_statistics(&app, provider, anchor, focus, update_only)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_statistics_panel(
    app: AppHandle,
    window: WebviewWindow,
    focus_main: bool,
) -> Result<(), String> {
    require_panel_window(&window)?;
    panel::hide_statistics(&app, focus_main).map_err(|error| error.to_string())
}

#[tauri::command]
fn dismiss_panel(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_panel_window(&window)?;
    panel::dismiss(&app).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_panel_interaction(
    app: AppHandle,
    window: WebviewWindow,
    hovered: bool,
    keyboard: bool,
    intent: String,
) -> Result<(), String> {
    require_panel_window(&window)?;
    if !matches!(intent.as_str(), "pointer" | "leave" | "keyboard" | "focus") {
        return Err("面板交互来源无效".into());
    }
    panel::interaction_changed(&app, window.label(), hovered, keyboard, &intent)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn resize_content_window(
    app: AppHandle,
    window: WebviewWindow,
    height: f64,
    revision: Option<u64>,
) -> Result<(), String> {
    panel::resize_content_window(&app, &window, height, revision).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_statistics_panel_state(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<panel::StatisticsPanelState, String> {
    require_panel_window(&window)?;
    Ok(panel::statistics_state(&app))
}

#[tauri::command]
fn present_statistics_panel(
    app: AppHandle,
    window: WebviewWindow,
    revision: u64,
) -> Result<(), String> {
    if window.label() != "statistics" {
        return Err("只能从统计窗口确认显示".into());
    }
    panel::present_statistics(&app, revision).map_err(|error| error.to_string())
}

fn require_panel_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "statistics") {
        Ok(())
    } else {
        Err("该窗口不能控制统计面板".into())
    }
}

#[tauri::command]
async fn get_cached_token_statistics(
    app: AppHandle,
    provider: ProviderId,
) -> Result<Option<statistics::TokenStatistics>, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "无法定位用户目录，请重启 AgentBar 后重试".to_string())?;
    let data_dir = storage::data_dir(&home);
    tauri::async_runtime::spawn_blocking(move || statistics::read_cached(provider, &data_dir))
        .await
        .map_err(|_| "读取统计缓存任务异常，请重试".to_string())?
}

#[tauri::command]
async fn get_token_statistics(
    app: AppHandle,
    provider: ProviderId,
) -> Result<statistics::TokenStatistics, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "无法定位用户目录，请重启 AgentBar 后重试".to_string())?;
    let data_dir = storage::data_dir(&home);
    tauri::async_runtime::spawn_blocking(move || statistics::collect(provider, Some(&data_dir)))
        .await
        .map_err(|_| "统计本机会话任务异常，请重试".to_string())?
}

fn validate_account_statistics_request(
    settings: &AppSettings,
    source: CodexStatisticsSource,
) -> Result<(), String> {
    if !settings.enabled_providers.contains(&ProviderId::Codex) {
        return Err("Codex 已停用，请先启用后读取使用统计".into());
    }
    if source != CodexStatisticsSource::Auto || settings.codex_statistics_source != source {
        return Err("统计来源已变更，请按当前来源重新读取".into());
    }
    Ok(())
}

fn require_local_settings_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "settings" | "statistics") {
        Ok(())
    } else {
        Err("该窗口不能访问本机账号统计".into())
    }
}

#[tauri::command]
async fn get_cached_codex_account_statistics(
    app: AppHandle,
    window: WebviewWindow,
    source: CodexStatisticsSource,
) -> Result<Option<account_statistics::AccountUsageSnapshot>, String> {
    require_local_settings_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let settings = app.state::<AppState>().settings()?;
        validate_account_statistics_request(&settings, source)?;
        let request_scope = providers::account_statistics_scope_key();
        let cache_scope = providers::account_statistics_cache_scope_key()
            .filter(|cache_scope| cache_scope == &request_scope);
        let home = app
            .path()
            .home_dir()
            .map_err(|_| "无法定位用户目录，请重启 AgentBar 后重试".to_string())?;
        let snapshot = account_statistics_cache::read_cached(
            &storage::data_dir(&home),
            source,
            cache_scope.as_deref(),
        )?;
        let current = app.state::<AppState>().settings()?;
        validate_account_statistics_request(&current, source)?;
        if request_scope != providers::account_statistics_scope_key() {
            return Err("Codex 登录状态已变化，请重新读取当前账号统计".into());
        }
        Ok(snapshot)
    })
    .await
    .map_err(|_| "读取服务端统计缓存任务异常，请重试".to_string())?
}

#[tauri::command]
async fn get_codex_account_statistics(
    app: AppHandle,
    window: WebviewWindow,
    source: CodexStatisticsSource,
) -> Result<account_statistics::AccountUsageSnapshot, String> {
    require_local_settings_window(&window)?;
    let requested = app.state::<AppState>().settings()?;
    validate_account_statistics_request(&requested, source)?;
    let ticket = account_statistics_cache::REQUESTS.begin()?;
    let scope = providers::account_statistics_scope_key();
    let cache_scope =
        providers::account_statistics_cache_scope_key().filter(|cache_scope| cache_scope == &scope);
    let snapshot =
        tauri::async_runtime::spawn_blocking(move || providers::collect_account_statistics(source))
            .await
            .map_err(|_| "读取服务端使用统计任务异常，请重试".to_string())?;
    let current = app.state::<AppState>().settings()?;
    validate_account_statistics_request(&current, source)?;
    if scope != providers::account_statistics_scope_key() {
        return Err("Codex 登录状态已变化，请重新读取当前账号统计".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let home = app
            .path()
            .home_dir()
            .map_err(|_| "无法定位用户目录，请重启 AgentBar 后重试".to_string())?;
        account_statistics_cache::REQUESTS.finish(
            ticket,
            &storage::data_dir(&home),
            source,
            cache_scope.as_deref(),
            snapshot,
            || {
                let current = app.state::<AppState>().settings()?;
                validate_account_statistics_request(&current, source)?;
                if scope != providers::account_statistics_scope_key() {
                    return Err("Codex 登录状态已变化，请重新读取当前账号统计".into());
                }
                Ok(())
            },
        )
    })
    .await
    .map_err(|_| "保存服务端统计缓存任务异常，请重试".to_string())?
}

fn log_result(result: Result<(), tauri::Error>) {
    if let Err(error) = result {
        eprintln!("操作 AgentBar 窗口失败：{error}");
    }
}

fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示面板", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "偏好设置…", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新用量", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出 AgentBar", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &refresh, &separator, &quit])?;

    TrayIconBuilder::with_id(panel::TRAY_ID)
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .icon_as_template(true)
        .tooltip("AgentBar · 本机账号用量")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => log_result(panel::show(app, None, "navigate-usage")),
            "settings" => {
                log_result(show_settings(app));
            }
            "refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = refresh_dashboard(app).await {
                        eprintln!("刷新用量失败：{error}");
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state,
                rect,
                ..
            } = event
            {
                match button_state {
                    MouseButtonState::Down => log_result(panel::press(tray.app_handle())),
                    MouseButtonState::Up => log_result(panel::toggle(tray.app_handle(), rect)),
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn start_refresh_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = refresh_and_emit(app.clone()).await {
                eprintln!("后台刷新用量失败：{error}");
            }
            let state = app.state::<AppState>();
            let interval = match state.settings() {
                Ok(settings) => settings.refresh_interval_seconds,
                Err(error) => {
                    eprintln!("停止后台刷新：{error}");
                    return;
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                _ = state.settings_changed.notified() => {
                    // Collect newly enabled providers immediately, even while hidden.
                }
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log_result(panel::show(app, None, "navigate-usage"));
        }))
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let data_dir = storage::data_dir(&app.path().home_dir()?);
            let state = AppState::load(settings_path, data_dir).map_err(std::io::Error::other)?;
            app.manage(state);
            #[cfg(target_os = "macos")]
            update_native_theme(app.handle());
            app.manage(panel::PanelState::default());
            create_tray(app)?;
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            panel::initialize(app.handle())?;
            start_refresh_loop(app.handle().clone());
            start_tray_clock(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        log_result(window.hide());
                    }
                    WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        log_result(panel::settings_monitor_changed(window.app_handle()));
                    }
                    _ => {}
                }
            }
            if matches!(window.label(), "main" | "statistics") {
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        // Retain the WebView and Rust background worker in the tray.
                        api.prevent_close();
                        if window.label() == "statistics" {
                            log_result(panel::hide_statistics(window.app_handle(), true));
                        } else {
                            log_result(panel::hide(window.app_handle()));
                        }
                    }
                    WindowEvent::Focused(focused) => {
                        panel::focus_changed(window.app_handle(), *focused)
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            refresh_dashboard,
            refresh_provider_dashboard,
            get_settings,
            save_settings,
            hide_panel,
            hide_settings,
            open_settings,
            quit_app,
            show_statistics_panel,
            hide_statistics_panel,
            dismiss_panel,
            set_panel_interaction,
            resize_content_window,
            get_statistics_panel_state,
            present_statistics_panel,
            get_cached_token_statistics,
            get_token_statistics,
            get_cached_codex_account_statistics,
            get_codex_account_statistics,
        ])
        .run(tauri::generate_context!())
        .expect("启动 AgentBar 失败");
}

#[cfg(test)]
mod account_request_tests {
    use super::*;

    #[test]
    fn remote_statistics_respect_saved_source_and_provider_opt_out() {
        let mut settings = AppSettings::default();
        assert!(
            validate_account_statistics_request(&settings, CodexStatisticsSource::Local).is_err()
        );
        assert!(
            validate_account_statistics_request(&settings, CodexStatisticsSource::Oauth).is_err()
        );
        settings.codex_statistics_source = CodexStatisticsSource::Auto;
        assert!(
            validate_account_statistics_request(&settings, CodexStatisticsSource::Auto).is_ok()
        );
        for explicit in [
            CodexStatisticsSource::Oauth,
            CodexStatisticsSource::Pat,
            CodexStatisticsSource::Cli,
        ] {
            assert!(validate_account_statistics_request(&settings, explicit).is_err());
        }
        settings.enabled_providers.clear();
        assert!(
            validate_account_statistics_request(&settings, CodexStatisticsSource::Auto).is_err()
        );
    }
}
