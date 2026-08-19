//! 本地目录监控（文件事件/轮询）与事件循环。

use crate::prelude::*;

pub(crate) fn should_sync(path: &Path, sync_types: &[String]) -> bool {
    let extension = file_extension(path);
    !extension.is_empty()
        && normalize_sync_types(sync_types)
            .iter()
            .any(|value| value == &extension)
}

pub(crate) fn ignored(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_lowercase();
    name.starts_with("~$")
        || [
            ".tmp",
            ".part",
            ".crdownload",
            ".download",
            ".swp",
            ".ds_store",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}
pub(crate) fn modified_ms(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

pub(crate) fn install_watcher(state: &SharedState, mapping: &Mapping) -> Result<(), String> {
    if !Path::new(&mapping.local_path).is_dir() {
        return Err("监控目录不存在或无法访问".to_string());
    }
    if let Ok(mut guard) = state.lock() {
        guard.watchers.remove(&mapping.id);
    }
    if mapping.monitor_mode == "polling" {
        return Ok(());
    }
    let tx = state.lock().map_err(|e| e.to_string())?.event_tx.clone();
    let mapping_id = mapping.id.clone();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for path in event.paths {
                        let _ = tx.send(FsEvent {
                            mapping_id: mapping_id.clone(),
                            path,
                        });
                    }
                }
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| e.to_string())?;
    watcher
        .watch(Path::new(&mapping.local_path), RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.watchers.remove(&mapping.id);
    guard.watchers.insert(mapping.id.clone(), watcher);
    Ok(())
}

pub(crate) fn collect_existing_files(root: &Path, sync_types: &[String], files: &mut Vec<PathBuf>) {
    collect_existing_files_with_skips(root, sync_types, files, &mut Vec::new());
}

pub(crate) fn collect_existing_files_with_skips(
    root: &Path,
    sync_types: &[String],
    files: &mut Vec<PathBuf>,
    skips: &mut Vec<UploadScanSkip>,
) {
    let mut visited = HashSet::new();
    collect_existing_files_inner(root, sync_types, files, skips, &mut visited);
}

fn collect_existing_files_inner(
    root: &Path,
    sync_types: &[String],
    files: &mut Vec<PathBuf>,
    skips: &mut Vec<UploadScanSkip>,
    visited: &mut HashSet<PathIdentity>,
) {
    let Some((kind, readable)) = inspect_local_entry(root, skips) else {
        return;
    };
    match kind {
        LocalEntryKind::File => {
            if !ignored(root) && should_sync(root, sync_types) {
                files.push(user_visible_path(root));
            }
        }
        LocalEntryKind::Directory => {
            if !visited.insert(path_identity(&readable)) {
                skips.push(UploadScanSkip::new(root, "检测到循环链接，已跳过"));
                return;
            }
            let entries = match fs::read_dir(&readable) {
                Ok(entries) => entries,
                Err(error) => {
                    skips.push(UploadScanSkip::new(
                        root,
                        format!("读取目录失败：{error}"),
                    ));
                    return;
                }
            };
            for entry in entries {
                match entry {
                    Ok(entry) => collect_existing_files_inner(
                        &entry.path(),
                        sync_types,
                        files,
                        skips,
                        visited,
                    ),
                    Err(error) => skips.push(UploadScanSkip::new(
                        root,
                        format!("读取目录项失败：{error}"),
                    )),
                }
            }
        }
    }
}

pub(crate) fn collect_watch_event_files(path: &Path, sync_types: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_existing_files(path, sync_types, &mut files);
    files
}

pub(crate) fn enqueue_existing_files(app: &tauri::AppHandle, state: &SharedState, mapping: &Mapping) {
    if !mapping.scan_existing {
        return;
    }
    let mut files = Vec::new();
    let mut skips = Vec::new();
    collect_existing_files_with_skips(
        Path::new(&mapping.local_path),
        &mapping.sync_types,
        &mut files,
        &mut skips,
    );
    emit(
        app,
        json!({ "type": "status", "level": "info", "message": format!("正在扫描已有文件：{} 个", files.len()) }),
    );
    if !skips.is_empty() {
        let preview = skips
            .iter()
            .take(3)
            .map(|skip| format!("{}：{}", skip.path, skip.reason))
            .collect::<Vec<_>>()
            .join("；");
        emit(
            app,
            json!({
                "type": "status",
                "level": "warning",
                "message": format!("扫描已有文件时跳过 {} 个路径。{preview}", skips.len())
            }),
        );
    }
    if let Ok(guard) = state.lock() {
        for path in files {
            let _ = guard.event_tx.send(FsEvent {
                mapping_id: mapping.id.clone(),
                path,
            });
        }
    }
    emit_state(app, state);
}

pub(crate) fn seed_existing_files(state: &SharedState, mapping: &Mapping) {
    let mut files = Vec::new();
    collect_existing_files(
        Path::new(&mapping.local_path),
        &mapping.sync_types,
        &mut files,
    );
    if let Ok(mut guard) = state.lock() {
        for path in files {
            if let Ok(metadata) = fs::metadata(&path) {
                guard.history.insert(
                    item_key(&mapping.id, &path),
                    Stamp {
                        size: metadata.len(),
                        modified_ms: modified_ms(&metadata),
                    },
                );
            }
        }
    }
}


pub(crate) async fn polling_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        let mappings = state
            .lock()
            .map(|guard| {
                guard
                    .mappings
                    .iter()
                    .filter(|mapping| mapping.enabled && mapping.monitor_mode == "polling")
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for mapping in mappings {
            let mut files = Vec::new();
            collect_existing_files(
                Path::new(&mapping.local_path),
                &mapping.sync_types,
                &mut files,
            );
            for path in files {
                enqueue_path(
                    &app,
                    &state,
                    FsEvent {
                        mapping_id: mapping.id.clone(),
                        path,
                    },
                )
                .await;
            }
        }
    }
}


pub(crate) async fn event_loop(app: tauri::AppHandle, state: SharedState, mut rx: UnboundedReceiver<FsEvent>) {
    while let Some(event) = rx.recv().await {
        let app = app.clone();
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            sleep(Duration::from_millis(900)).await;
            enqueue_path(&app, &state, event).await;
        });
    }
}
