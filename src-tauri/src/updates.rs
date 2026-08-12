//! 应用版本信息与自动更新。

use crate::prelude::*;

#[derive(Default)]
pub(crate) struct PendingAppUpdate(pub(crate) Mutex<Option<Update>>);


#[derive(Debug, Clone, Serialize)]
pub(crate) struct AppVersionInfo {
    pub(crate) version: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AppUpdateMetadata {
    pub(crate) version: String,
    pub(crate) current_version: String,
    pub(crate) notes: String,
    published_at: Option<String>,
}


#[tauri::command]
pub(crate) fn get_app_version() -> AppVersionInfo {
    AppVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
pub(crate) async fn fetch_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    pending: tauri::State<'_, PendingAppUpdate>,
) -> Result<Option<AppUpdateMetadata>, String> {
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let global_proxy = load_global_network_proxy(&db_path)?;
    let mut updater = app.updater_builder().timeout(Duration::from_secs(30));
    if !global_proxy.trim().is_empty() {
        let normalized = normalize_proxy_value(Some(global_proxy), "全局代理")?;
        let url = reqwest::Url::parse(&normalized)
            .map_err(|error| format!("GitHub 代理地址格式不正确：{error}"))?;
        updater = updater.proxy(url);
    }
    let update = updater
        .build()
        .map_err(|error| format!("初始化更新检查失败：{error}"))?
        .check()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?;

    let metadata = update.as_ref().map(|item| AppUpdateMetadata {
        version: item.version.clone(),
        current_version: item.current_version.clone(),
        notes: item.body.clone().unwrap_or_default(),
        published_at: item.date.map(|date| date.to_string()),
    });
    *pending
        .0
        .lock()
        .map_err(|_| "更新状态锁已损坏".to_string())? = update;
    Ok(metadata)
}

#[tauri::command]
pub(crate) async fn install_app_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingAppUpdate>,
) -> Result<(), String> {
    let update = pending
        .0
        .lock()
        .map_err(|_| "更新状态锁已损坏".to_string())?
        .take()
        .ok_or_else(|| "没有待安装的更新，请先检查更新".to_string())?;

    let version = update.version.clone();
    let received = Arc::new(AtomicU64::new(0));
    let progress_app = app.clone();
    let progress_received = received.clone();
    let finished_app = app.clone();
    let started_payload = json!({
        "type": "app-update",
        "event": "started",
        "version": version,
    });
    let _ = app.emit("sync-event", started_payload);

    update
        .download_and_install(
            move |chunk_length, content_length| {
                let downloaded = progress_received
                    .fetch_add(chunk_length as u64, Ordering::Relaxed)
                    + chunk_length as u64;
                let _ = progress_app.emit(
                    "sync-event",
                    json!({
                        "type": "app-update",
                        "event": "progress",
                        "downloaded": downloaded,
                        "total": content_length,
                    }),
                );
            },
            move || {
                let _ = finished_app.emit(
                    "sync-event",
                    json!({
                        "type": "app-update",
                        "event": "downloaded",
                    }),
                );
            },
        )
        .await
        .map_err(|error| format!("下载或安装更新失败：{error}"))?;

    Ok(())
}
