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
) -> Result<VirtualLibraryInfo, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    guard.virtual_library.set_refresh_minutes(refresh_minutes)?;
    guard
        .virtual_library
        .set_strm_base_url(strm_base_url.unwrap_or_default())?;
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
                "虚拟库同步完成：{}（{} 个 STRM，{} 个元数据）",
                mapping.name, summary.strm_files, summary.metadata_files
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
    });
    Ok(())
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
