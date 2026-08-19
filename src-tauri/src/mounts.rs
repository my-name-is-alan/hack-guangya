//! WebDAV/原生挂载与虚拟媒体库命令。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MountInfo {
    pub(crate) enabled: bool,
    pub(crate) running: bool,
    pub(crate) configured: bool,
    pub(crate) local_only: bool,
    pub(crate) endpoint: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) error: Option<String>,
    pub(crate) protocol: String,
}


pub(crate) fn mount_info(state: &RuntimeState) -> MountInfo {
    MountInfo {
        enabled: state.webdav_enabled,
        running: state.webdav_running,
        configured: !state.webdav_username.is_empty() && !state.webdav_password.is_empty(),
        local_only: true,
        endpoint: format!("http://127.0.0.1:{}/", state.webdav_port),
        username: state.webdav_username.clone(),
        password: String::new(),
        error: state.webdav_error.clone(),
        protocol: "webdav".to_string(),
    }
}

#[tauri::command]
pub(crate) fn get_mount_info(state: tauri::State<'_, SharedState>) -> Result<MountInfo, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(mount_info(&guard))
}

#[tauri::command]
pub(crate) fn update_mount_credentials(
    state: tauri::State<'_, SharedState>,
    username: String,
    password: String,
) -> Result<MountInfo, String> {
    let username = normalize_webdav_username(&username)?;
    let password = normalize_webdav_password(&password)?;
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.webdav_username = username;
    guard.webdav_password = password;
    save_config(&guard);
    Ok(mount_info(&guard))
}

#[tauri::command]
pub(crate) fn get_native_mount_info(state: tauri::State<'_, SharedState>) -> Result<NativeMountInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    Ok(guard.native_mount.info())
}

#[tauri::command]
pub(crate) fn update_native_mount_options(
    state: tauri::State<'_, SharedState>,
    options: NativeMountOptions,
) -> Result<NativeMountInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.native_mount.set_options(options)?;
    save_config(&guard);
    Ok(guard.native_mount.info())
}

#[tauri::command]
pub(crate) fn start_native_mount(state: tauri::State<'_, SharedState>) -> Result<NativeMountInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    if !guard.webdav_running {
        return Err(guard
            .webdav_error
            .clone()
            .unwrap_or_else(|| "WebDAV 本地服务尚未就绪".to_string()));
    }
    let endpoint = format!("http://127.0.0.1:{}/", guard.webdav_port);
    let username = guard.webdav_username.clone();
    let password = guard.webdav_password.clone();
    guard.native_mount.start(&endpoint, &username, &password)
}

#[tauri::command]
pub(crate) fn stop_native_mount(state: tauri::State<'_, SharedState>) -> Result<NativeMountInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.native_mount.stop()
}

#[tauri::command]
pub(crate) fn select_native_mount_target() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub(crate) fn select_rclone_binary() -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    #[cfg(windows)]
    {
        dialog = dialog.add_filter("rclone", &["exe"]);
    }
    dialog
        .pick_file()
        .map(|path| path.to_string_lossy().to_string())
}

pub(crate) fn virtual_library_info(state: &RuntimeState) -> VirtualLibraryInfo {
    state.virtual_library.info()
}

#[tauri::command]
pub(crate) fn get_virtual_library_info(
    state: tauri::State<'_, SharedState>,
) -> Result<VirtualLibraryInfo, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(virtual_library_info(&guard))
}

#[tauri::command]
pub(crate) fn update_virtual_library_settings(
    state: tauri::State<'_, SharedState>,
    refresh_minutes: u64,
    strm_base_url: Option<String>,
    emby_upstream: Option<String>,
    emby_api_key: Option<String>,
) -> Result<VirtualLibraryInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.virtual_library.set_refresh_minutes(refresh_minutes)?;
    guard
        .virtual_library
        .set_strm_base_url(strm_base_url.unwrap_or_default())?;
    guard
        .virtual_library
        .set_emby_upstream(emby_upstream.unwrap_or_default())?;
    // API Key 留空保持不变（不回显）；提交 "off" 清除。
    if let Some(value) = emby_api_key {
        let trimmed = value.trim().to_string();
        if !trimmed.is_empty() {
            guard
                .virtual_library
                .set_emby_api_key(if trimmed == "off" { String::new() } else { trimmed });
        }
    }
    save_config(&guard);
    // 直链地址决定 STRM 服务监听范围（回环 / 所有网卡），通知其重新绑定。
    guard.strm_rebind.send_modify(|value| *value = value.wrapping_add(1));
    Ok(virtual_library_info(&guard))
}

#[tauri::command]
pub(crate) fn upsert_virtual_library_mapping(
    state: tauri::State<'_, SharedState>,
    mapping: VirtualLibraryMapping,
) -> Result<VirtualLibraryInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.virtual_library.upsert_mapping(mapping)?;
    save_config(&guard);
    Ok(virtual_library_info(&guard))
}

#[tauri::command]
pub(crate) fn remove_virtual_library_mapping(
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<VirtualLibraryInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.virtual_library.remove_mapping(id.trim())?;
    save_config(&guard);
    Ok(virtual_library_info(&guard))
}

pub(crate) fn publish_virtual_library(app: &tauri::AppHandle, state: &SharedState) {
    if let Ok(guard) = state.lock() {
        emit(
            app,
            json!({
                "type": "virtual-library",
                "data": virtual_library_info(&guard)
            }),
        );
    }
}

fn virtual_library_pending_resync() -> &'static Mutex<HashSet<String>> {
    static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(crate) fn spawn_virtual_library_sync(
    app: tauri::AppHandle,
    state: SharedState,
    id: String,
) -> Result<(), String> {
    let mapping = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        let mapping = guard
            .virtual_library
            .mapping(&id)
            .ok_or_else(|| "虚拟库配置不存在".to_string())?;
        if !mapping.enabled {
            return Err("该虚拟库已停用".to_string());
        }
        guard.virtual_library.begin_sync(&id)?;
        mapping
    };
    publish_virtual_library(&app, &state);
    tauri::async_runtime::spawn(async move {
        let result = virtual_library::sync_mapping(&state, &mapping).await;
        let outcome_message = match &result {
            Ok(summary) => format!(
                "虚拟库同步完成：{}（{} 个 STRM，{} 个元数据，{} 项变更）",
                mapping.name, summary.strm_files, summary.metadata_files, summary.changes.total
            ),
            Err(error) => format!("虚拟库同步失败：{}：{error}", mapping.name),
        };
        if let Ok(mut guard) = state.lock() {
            guard.virtual_library.finish_sync(&id, result.clone());
        }
        status(
            &app,
            if result.is_ok() { "success" } else { "error" },
            outcome_message,
        );
        publish_virtual_library(&app, &state);
        if let Ok(summary) = &result {
            // 同步有变更且配置了 Emby API Key 与 Emby 内路径时，按目录精确通知增量扫描。
            let (upstream, api_key) = state
                .lock()
                .ok()
                .map(|guard| {
                    let options = guard.virtual_library.options();
                    (options.emby_upstream, options.emby_api_key)
                })
                .unwrap_or_default();
            let notify = virtual_library::notify_emby(
                &upstream,
                &api_key,
                &mapping.emby_path,
                &summary.changes,
            )
            .await;
            match notify {
                Ok(Some(count)) => {
                    if let Ok(mut guard) = state.lock() {
                        guard
                            .virtual_library
                            .set_emby_notify_result(&id, Some(count), None);
                    }
                    publish_virtual_library(&app, &state);
                }
                Ok(None) => {}
                Err(error) => {
                    if let Ok(mut guard) = state.lock() {
                        guard
                            .virtual_library
                            .set_emby_notify_result(&id, None, Some(error.clone()));
                    }
                    status(&app, "warning", format!("{}：{error}", mapping.name));
                    publish_virtual_library(&app, &state);
                }
            }
        }
        // 同步期间又有触发请求：当前轮结束后自动再同步一次。
        let should_resync = virtual_library_pending_resync()
            .lock()
            .map(|mut pending| pending.remove(&id))
            .unwrap_or(false);
        if should_resync {
            let _ = queue_virtual_library_sync(app.clone(), state.clone(), id.clone());
        }
    });
    Ok(())
}

/// 触发同步；若该虚拟库正在同步则记为待重跑，当前轮结束后自动再同步一次。
pub(crate) fn queue_virtual_library_sync(
    app: tauri::AppHandle,
    state: SharedState,
    id: String,
) -> bool {
    match spawn_virtual_library_sync(app, state.clone(), id.clone()) {
        Ok(()) => true,
        Err(_) => {
            let running = state
                .lock()
                .ok()
                .map(|guard| {
                    guard
                        .virtual_library
                        .info()
                        .statuses
                        .get(&id)
                        .is_some_and(|status| status.running)
                })
                .unwrap_or(false);
            if running {
                if let Ok(mut pending) = virtual_library_pending_resync().lock() {
                    pending.insert(id);
                }
                return true;
            }
            false
        }
    }
}

/// 整理器等云端写入方完成后调用：找出覆盖该云端目录的虚拟库并触发同步。
pub(crate) fn sync_virtual_libraries_for_cloud_target(
    app: &tauri::AppHandle,
    state: &SharedState,
    dir_id: &str,
    cloud_path: &str,
) -> Vec<String> {
    let mappings = state
        .lock()
        .ok()
        .map(|guard| guard.virtual_library.options().mappings)
        .unwrap_or_default();
    let matched: Vec<String> = mappings
        .into_iter()
        .filter(|mapping| {
            mapping.enabled
                && ((!dir_id.trim().is_empty() && mapping.source_dir_id == dir_id.trim())
                    || virtual_library::cloud_paths_overlap(&mapping.source_path, cloud_path))
        })
        .map(|mapping| mapping.id)
        .collect();
    for id in &matched {
        let _ = queue_virtual_library_sync(app.clone(), state.clone(), id.clone());
    }
    matched
}

#[tauri::command]
pub(crate) fn sync_virtual_library(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<VirtualLibraryInfo, String> {
    spawn_virtual_library_sync(app, state.inner().clone(), id)?;
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(virtual_library_info(&guard))
}

#[tauri::command]
pub(crate) fn select_virtual_library_target() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

pub(crate) async fn virtual_library_refresh_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        let refresh_minutes = state
            .lock()
            .ok()
            .map(|guard| guard.virtual_library.options().refresh_minutes)
            .unwrap_or(virtual_library::default_refresh_minutes());
        sleep(Duration::from_secs(refresh_minutes.saturating_mul(60))).await;
        let mappings = state
            .lock()
            .ok()
            .map(|guard| guard.virtual_library.options().mappings)
            .unwrap_or_default();
        for mapping in mappings.into_iter().filter(|mapping| mapping.enabled) {
            let _ = spawn_virtual_library_sync(app.clone(), state.clone(), mapping.id);
        }
    }
}
