//! 备份任务（目录映射）管理命令。

use crate::prelude::*;

#[tauri::command]
pub(crate) fn select_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}
#[tauri::command]
pub(crate) fn add_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    local_path: String,
    remote_path: String,
    remote_parent_id: String,
    source_policy: String,
    archive_path: Option<String>,
    scan_existing: bool,
    sync_types: Vec<String>,
    monitor_mode: String,
    auto_share: bool,
    organizer_mapping_id: Option<String>,
) -> Result<Mapping, String> {
    if !["keep", "archive", "delete"].contains(&source_policy.as_str()) {
        return Err("无效的上传后源文件策略".into());
    }
    if auto_share {
        let guard = state.lock().map_err(|e| e.to_string())?;
        if guard.hdhive_base_url.is_empty() || guard.hdhive_secret.is_empty() {
            return Err("开启自动分享前请先配置 Hdhive 地址和密钥".to_string());
        }
    }
    let mapping = Mapping {
        id: Uuid::new_v4().to_string(),
        local_path: PathBuf::from(local_path).to_string_lossy().to_string(),
        remote_path: normalize_remote_path(&remote_path),
        remote_parent_id,
        enabled: true,
        source_policy,
        archive_path: archive_path.filter(|value| !value.trim().is_empty()),
        scan_existing,
        sync_types: normalize_sync_types(&sync_types),
        watch_error: None,
        monitor_mode: normalize_monitor_mode(&monitor_mode),
        auto_share,
        organizer_mapping_id: organizer_mapping_id.unwrap_or_default().trim().to_string(),
    };
    organizer::validate_backup_mapping_link(
        &app,
        &mapping.organizer_mapping_id,
        &mapping.remote_parent_id,
    )?;
    if !Path::new(&mapping.local_path).is_dir() {
        return Err("本地目录不存在".into());
    }
    if mapping.source_policy == "archive" {
        let archive_path = mapping
            .archive_path
            .as_ref()
            .ok_or_else(|| "归档策略需要选择归档目录".to_string())?;
        if Path::new(archive_path).starts_with(Path::new(&mapping.local_path)) {
            return Err("归档目录不能位于被监控目录内部".into());
        }
    }
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.mappings.push(mapping.clone());
        save_config(&guard);
    }
    if let Err(error) = install_watcher(state.inner(), &mapping) {
        if let Ok(mut guard) = state.lock() {
            guard.mappings.retain(|item| item.id != mapping.id);
            save_config(&guard);
        }
        return Err(format!("创建目录监控失败：{error}"));
    }
    if mapping.scan_existing {
        enqueue_existing_files(&app, state.inner(), &mapping);
    } else if mapping.monitor_mode == "polling" {
        seed_existing_files(state.inner(), &mapping);
    }
    emit_state(&app, state.inner());
    Ok(mapping)
}
#[tauri::command]
pub(crate) fn remove_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.watchers.remove(&id);
    guard.mappings.retain(|mapping| mapping.id != id);
    guard.queue.retain(|item| item.mapping_id != id);
    guard.waiting_files.retain(|_, item| item.mapping_id != id);
    guard.failed_uploads.retain(|_, item| item.mapping_id != id);
    let prefix = format!("{id}::");
    guard.history.retain(|key, _| !key.starts_with(&prefix));
    guard
        .pending_cloud
        .retain(|key, _| !key.starts_with(&prefix));
    guard
        .recovering_pending
        .retain(|key| !key.starts_with(&prefix));
    guard.inflight.retain(|key, _| !key.starts_with(&prefix));
    guard.paused_uploads.retain(|key| !key.starts_with(&prefix));
    guard
        .queue_pause_requests
        .retain(|key| !key.starts_with(&prefix));
    save_config(&guard);
    let db_path = guard.db_path.clone();
    drop(guard);
    remove_mapping_transient_uploads(&db_path, &id)?;
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
pub(crate) fn toggle_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "监控目录不存在".to_string())?;
        mapping.enabled = enabled;
        let copy = mapping.clone();
        if !enabled {
            guard.watchers.remove(&id);
        }
        save_config(&guard);
        copy
    };
    if enabled {
        if let Err(error) = install_watcher(state.inner(), &mapping) {
            if let Ok(mut guard) = state.lock() {
                if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                    current.enabled = false;
                    current.watch_error = Some(error.clone());
                }
                save_config(&guard);
            }
            emit_state(&app, state.inner());
            return Err(format!("启动备份任务监控失败：{error}"));
        }
        if let Ok(mut guard) = state.lock() {
            if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                current.watch_error = None;
            }
            save_config(&guard);
        }
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
pub(crate) fn update_mapping_sync_types(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    sync_types: Vec<String>,
) -> Result<(), String> {
    let selected = normalize_sync_types(&sync_types);
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "备份任务不存在".to_string())?;
        mapping.sync_types = selected.clone();
        let mapping = mapping.clone();
        guard
            .queue
            .retain(|item| item.mapping_id != id || should_sync(&item.file_path, &selected));
        save_config(&guard);
        mapping
    };
    if mapping.enabled {
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
pub(crate) fn update_mapping_monitor_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    monitor_mode: String,
) -> Result<(), String> {
    let mode = normalize_monitor_mode(&monitor_mode);
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "备份任务不存在".to_string())?;
        mapping.monitor_mode = mode;
        mapping.watch_error = None;
        let mapping = mapping.clone();
        save_config(&guard);
        mapping
    };
    if mapping.enabled {
        if let Err(error) = install_watcher(state.inner(), &mapping) {
            if let Ok(mut guard) = state.lock() {
                if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                    current.enabled = false;
                    current.watch_error = Some(error.clone());
                }
                save_config(&guard);
            }
            emit_state(&app, state.inner());
            return Err(format!("切换监控方式失败：{error}"));
        }
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}


#[tauri::command]
pub(crate) fn update_mapping_auto_share(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    auto_share: bool,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    if auto_share && (guard.hdhive_base_url.is_empty() || guard.hdhive_secret.is_empty()) {
        return Err("开启自动分享前请先配置 Hdhive 地址和密钥".to_string());
    }
    let mapping = guard
        .mappings
        .iter_mut()
        .find(|mapping| mapping.id == id)
        .ok_or_else(|| "备份任务不存在".to_string())?;
    mapping.auto_share = auto_share;
    save_config(&guard);
    drop(guard);
    emit_state(&app, state.inner());
    Ok(())
}

#[tauri::command]
pub(crate) fn update_mapping_organizer(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    organizer_mapping_id: String,
) -> Result<(), String> {
    let remote_parent_id = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == id)
            .map(|mapping| mapping.remote_parent_id.clone())
            .ok_or_else(|| "备份任务不存在".to_string())?
    };
    let organizer_mapping_id = organizer_mapping_id.trim().to_string();
    organizer::validate_backup_mapping_link(&app, &organizer_mapping_id, &remote_parent_id)?;
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let mapping = guard
        .mappings
        .iter_mut()
        .find(|mapping| mapping.id == id)
        .ok_or_else(|| "备份任务不存在".to_string())?;
    mapping.organizer_mapping_id = organizer_mapping_id;
    save_config(&guard);
    drop(guard);
    emit_state(&app, state.inner());
    Ok(())
}
