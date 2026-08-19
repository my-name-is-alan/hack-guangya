//! 上传流水线：预检秒传、OSS 上传编排与收尾。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UploadReplacement {
    pub(crate) old_file_id: String,
    pub(crate) original_name: String,
    pub(crate) temporary_name: String,
    pub(crate) backup_name: String,
    pub(crate) previous_size: u64,
    pub(crate) previous_modified_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UploadItem {
    pub(crate) mapping_id: String,
    pub(crate) file_path: PathBuf,
    pub(crate) remote_parent_id: String,
    pub(crate) remote_dir: String,
    pub(crate) relative_path: String,
    pub(crate) change_kind: String,
    pub(crate) size: u64,
    pub(crate) modified_ms: u128,
    #[serde(default)]
    pub(crate) replacement: Option<UploadReplacement>,
}
#[derive(Debug, Clone)]
pub(crate) struct FsEvent {
    pub(crate) mapping_id: String,
    pub(crate) path: PathBuf,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) size: u64,
    pub(crate) modified_ms: u128,
}


#[derive(Debug, Clone)]
pub(crate) struct UploadOutcome {
    pub(crate) task_id: String,
    pub(crate) remote_file_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct FlashPreflightCache {
    pub(crate) stamp: Stamp,
    pub(crate) upload_token: Option<UploadToken>,
    pub(crate) created_at: Instant,
}

pub(crate) enum FlashPreflightOutcome {
    Accepted {
        task_id: String,
        token: String,
        device_id: String,
    },
    Miss(UploadToken),
    Skipped,
}


#[derive(Debug, Clone)]
pub(crate) struct PendingUpload {
    pub(crate) item: UploadItem,
    pub(crate) task_id: String,
}

#[derive(Debug)]
pub(crate) enum CloudTaskCheck {
    Confirmed(Value),
    Pending,
}

#[derive(Debug)]
pub(crate) enum CloudConfirmError {
    Retryable(String),
    Permanent(String),
}

impl CloudConfirmError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Retryable(message) | Self::Permanent(message) => message,
        }
    }
}


pub(crate) fn item_key(mapping_id: &str, path: &Path) -> String {
    format!("{mapping_id}::{}", path.to_string_lossy())
}
pub(crate) fn stamp_matches(item: &UploadItem, stamp: &Stamp) -> bool {
    stamp.size == item.size && stamp.modified_ms == item.modified_ms
}
pub(crate) fn flash_preflight_cached(state: &RuntimeState, item: &UploadItem) -> bool {
    state
        .flash_preflight_cache
        .get(&item_key(&item.mapping_id, &item.file_path))
        .is_some_and(|cached| stamp_matches(item, &cached.stamp))
}
pub(crate) fn take_flash_preflight_token(
    state: &SharedState,
    item: &UploadItem,
) -> Result<Option<UploadToken>, String> {
    let key = item_key(&item.mapping_id, &item.file_path);
    let cached = state
        .lock()
        .map_err(|error| error.to_string())?
        .flash_preflight_cache
        .remove(&key);
    Ok(cached.and_then(|cached| {
        (stamp_matches(item, &cached.stamp)
            && cached.created_at.elapsed()
                <= Duration::from_secs(FLASH_PREFLIGHT_TOKEN_MAX_AGE_SECS))
        .then_some(cached.upload_token)
        .flatten()
    }))
}
pub(crate) fn upload_already_scheduled(
    history: &HashMap<String, Stamp>,
    pending_cloud: &HashMap<String, Stamp>,
    inflight: &HashMap<String, Stamp>,
    queue: &VecDeque<UploadItem>,
    waiting_files: &HashMap<String, UploadItem>,
    cancelled_uploads: &HashMap<String, Stamp>,
    item: &UploadItem,
) -> bool {
    let key = item_key(&item.mapping_id, &item.file_path);
    history
        .get(&key)
        .is_some_and(|stamp| stamp_matches(item, stamp))
        || pending_cloud.contains_key(&key)
        || inflight
            .get(&key)
            .is_some_and(|stamp| stamp_matches(item, stamp))
        || queue.iter().any(|queued| {
            item_key(&queued.mapping_id, &queued.file_path) == key
                && queued.size == item.size
                && queued.modified_ms == item.modified_ms
        })
        || waiting_files.contains_key(&key)
        || cancelled_uploads
            .get(&key)
            .is_some_and(|stamp| stamp_matches(item, stamp))
}

pub(crate) fn upload_is_cancelled(state: &SharedState, key: &str) -> bool {
    state
        .lock()
        .ok()
        .is_some_and(|guard| guard.cancelled_uploads.contains_key(key))
}

pub(crate) fn upload_pause_requested(state: &SharedState, key: &str) -> bool {
    state.lock().ok().is_some_and(|guard| {
        guard.paused_uploads.contains(key) || guard.queue_pause_requests.contains(key)
    })
}

pub(crate) async fn wait_for_upload_cancellation(state: &SharedState, key: &str) {
    while !upload_is_cancelled(state, key) {
        sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) async fn wait_for_upload_pause(state: &SharedState, key: &str) {
    while !upload_pause_requested(state, key) {
        sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) async fn abortable_upload_step<T, F>(state: &SharedState, key: &str, future: F) -> Result<T, String>
where
    F: Future<Output = T>,
{
    tokio::select! {
        value = future => Ok(value),
        _ = wait_for_upload_cancellation(state, key) => Err(UPLOAD_CANCELLED_MESSAGE.to_string()),
    }
}

pub(crate) async fn interruptible_upload_step<T, F>(
    state: &SharedState,
    key: &str,
    future: F,
) -> Result<T, String>
where
    F: Future<Output = T>,
{
    tokio::select! {
        value = future => Ok(value),
        _ = wait_for_upload_cancellation(state, key) => Err(UPLOAD_CANCELLED_MESSAGE.to_string()),
        _ = wait_for_upload_pause(state, key) => Err(UPLOAD_PAUSED_MESSAGE.to_string()),
    }
}

#[cfg(windows)]
pub(crate) fn file_available_for_upload(path: &Path) -> Result<bool, String> {
    use std::os::windows::fs::OpenOptionsExt;

    match fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(readable_fs_path(path))
    {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Ok(false),
        Err(error) => Err(format!("读取源文件失败：{error}")),
    }
}

#[cfg(not(windows))]
pub(crate) fn file_available_for_upload(path: &Path) -> Result<bool, String> {
    fs::OpenOptions::new()
        .read(true)
        .open(readable_fs_path(path))
        .map(|_| true)
        .map_err(|error| format!("读取源文件失败：{error}"))
}

pub(crate) fn hydrate_upload_replacement(path: &Path, item: &mut UploadItem) -> Result<(), String> {
    if item.replacement.is_some()
        || item.change_kind != "changed"
        || item.mapping_id.is_empty()
        || item.mapping_id.starts_with("__")
    {
        return Ok(());
    }
    let original_name = Path::new(&item.relative_path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "无法确定安全覆盖的原文件名".to_string())?;
    let previous = open_database(path)?
        .query_row(
            "SELECT size, modified_ms, remote_file_id
             FROM uploaded_files
             WHERE mapping_id = ?1 AND file_path = ?2 AND upload_state = ?3
               AND remote_parent_id = ?4 AND remote_dir = ?5 AND relative_path = ?6",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                UPLOAD_STATE_CLOUD_CONFIRMED,
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path
            ],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取待替换云端文件失败：{error}"))?;
    let Some((previous_size, previous_modified_ms, Some(old_file_id))) = previous else {
        return Ok(());
    };
    if old_file_id.trim().is_empty() {
        return Ok(());
    }
    item.replacement = Some(UploadReplacement {
        old_file_id,
        original_name: original_name.to_string(),
        temporary_name: format!(".__gy_replace_{}", Uuid::new_v4().simple()),
        backup_name: format!(".__gy_replace_backup_{}", Uuid::new_v4().simple()),
        previous_size,
        previous_modified_ms: previous_modified_ms.parse().unwrap_or_default(),
    });
    Ok(())
}

pub(crate) fn upload_remote_name(item: &UploadItem) -> Result<String, String> {
    if let Some(replacement) = item.replacement.as_ref() {
        return Ok(replacement.temporary_name.clone());
    }
    item.file_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .ok_or_else(|| "无法读取文件名".to_string())
}

pub(crate) async fn prepare_upload_item(
    state: &SharedState,
    item: &UploadItem,
) -> Result<Option<UploadItem>, String> {
    if !file_available_for_upload(&item.file_path)? {
        return Ok(None);
    }
    let first = fs::metadata(readable_fs_path(&item.file_path))
        .map_err(|error| format!("读取源文件失败：{error}"))?;
    if !first.is_file() {
        return Err("源路径不是文件".into());
    }
    sleep(Duration::from_millis(FILE_STABILITY_WAIT_MS)).await;
    if !file_available_for_upload(&item.file_path)? {
        return Ok(None);
    }
    let second = fs::metadata(readable_fs_path(&item.file_path))
        .map_err(|error| format!("读取源文件失败：{error}"))?;
    if first.len() != second.len() || modified_ms(&first) != modified_ms(&second) {
        return Ok(None);
    }
    let mut ready = item.clone();
    ready.size = second.len();
    ready.modified_ms = modified_ms(&second);
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    hydrate_upload_replacement(&db_path, &mut ready)?;
    Ok(Some(ready))
}

pub(crate) async fn requeue_busy_upload(app: tauri::AppHandle, state: SharedState, mut item: UploadItem) {
    sleep(Duration::from_secs(FILE_BUSY_RETRY_SECS)).await;
    let metadata = fs::metadata(&item.file_path)
        .ok()
        .filter(|value| value.is_file());
    if let Some(metadata) = &metadata {
        item.size = metadata.len();
        item.modified_ms = modified_ms(metadata);
    }
    let key = item_key(&item.mapping_id, &item.file_path);
    let queued = if let Ok(mut guard) = state.lock() {
        guard.waiting_files.remove(&key);
        if metadata.is_none() {
            false
        } else if !item.mapping_id.starts_with("__")
            && !guard
                .mappings
                .iter()
                .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
        {
            false
        } else if guard.cancelled_uploads.contains_key(&key)
            || upload_already_scheduled(
                &guard.history,
                &guard.pending_cloud,
                &guard.inflight,
                &guard.queue,
                &guard.waiting_files,
                &guard.cancelled_uploads,
                &item,
            )
        {
            false
        } else {
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.failed_uploads.remove(&key);
            guard.queue.push_back(item.clone());
            true
        }
    } else {
        false
    };
    if queued {
        let paused = state
            .lock()
            .ok()
            .is_some_and(|guard| guard.paused_uploads.contains(&key));
        emit(
            &app,
            json!({
                "type": "file",
                "state": if paused { "paused" } else { "waiting-file" },
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "stage": if paused { "已暂停，可从当前断点继续" } else { "另外的程序正在使用该文件，释放后将自动上传" }
            }),
        );
        emit_state(&app, &state);
        drain_queue(app, state);
    } else {
        emit_state(&app, &state);
    }
}

pub(crate) async fn requeue_resumable_upload(app: tauri::AppHandle, state: SharedState, item: UploadItem) {
    sleep(Duration::from_secs(PENDING_UPLOAD_RETRY_SECS)).await;
    let db_path = match state.lock() {
        Ok(guard) => guard.db_path.clone(),
        Err(_) => return,
    };
    let checkpoint = match load_upload_checkpoint(&db_path, &item) {
        Ok(Some(value)) => value,
        _ => return,
    };
    let key = item_key(&item.mapping_id, &item.file_path);
    let queued = if let Ok(mut guard) = state.lock() {
        let mapping_active = item.mapping_id.starts_with("__")
            || guard
                .mappings
                .iter()
                .any(|mapping| mapping.id == item.mapping_id && mapping.enabled);
        if !mapping_active
            || guard.cancelled_uploads.contains_key(&key)
            || upload_already_scheduled(
                &guard.history,
                &guard.pending_cloud,
                &guard.inflight,
                &guard.queue,
                &guard.waiting_files,
                &guard.cancelled_uploads,
                &item,
            )
        {
            false
        } else {
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.failed_uploads.remove(&key);
            guard.queue.push_back(item.clone());
            true
        }
    } else {
        false
    };
    if queued {
        emit(
            &app,
            json!({
                "type": "file",
                "state": "queued",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": checkpoint.uploaded_bytes,
                "total_bytes": item.size,
                "stage": "上传中断，已保留断点并自动重试"
            }),
        );
        emit_state(&app, &state);
        drain_queue(app, state);
    }
}

pub(crate) async fn preflight_flash_upload(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
) -> Result<FlashPreflightOutcome, String> {
    let (token, device_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.db_path.clone(),
        )
    };
    if load_upload_checkpoint(&db_path, item)?.is_some() {
        return Ok(FlashPreflightOutcome::Skipped);
    }

    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 0,
            "uploaded_bytes": 0,
            "total_bytes": item.size,
            "bytes_per_second": 0,
            "stage": "正在后台校验秒传"
        }),
    );
    let parent_id = ensure_remote_path(
        app,
        state,
        &token,
        &device_id,
        &item.remote_parent_id,
        &item.remote_dir,
    )
    .await?;
    let name = upload_remote_name(item)?;
    let mut res = json!({ "fileSize": item.size });
    if item.size < OSS_MIB {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": 0,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "bytes_per_second": 0,
                "stage": "正在后台计算秒传 MD5"
            }),
        );
        res["md5"] = json!(calculate_file_md5(&item.file_path).await?);
    }
    let result = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_center_token",
        json!({ "capacity": 2, "name": name, "res": res, "parentId": parent_id }),
        &[156],
    )
    .await?;
    let mut instant_upload = result.code == 156;
    let mut data: UploadToken = serde_json::from_value(
        result
            .data
            .ok_or_else(|| "光鸭没有返回上传凭证".to_string())?,
    )
    .map_err(|error| format!("上传凭证格式异常：{error}"))?;

    if !instant_upload && item.size >= OSS_MIB {
        let cached_hashes = match {
            let guard = state.lock().map_err(|error| error.to_string())?;
            load_cached_file_hashes(
                &guard.db_path,
                &item.file_path,
                item.size,
                item.modified_ms,
                cache_settings(&guard),
            )
        } {
            Ok(value) => value,
            Err(error) => {
                status(app, "warning", error);
                None
            }
        };
        let hashes_result = if let Some(hashes) = cached_hashes {
            emit(
                app,
                json!({
                    "type": "progress",
                    "file_path": item.file_path.to_string_lossy(),
                    "percent": 0,
                    "uploaded_bytes": 0,
                    "total_bytes": item.size,
                    "bytes_per_second": 0,
                    "stage": "后台已复用本地秒传指纹"
                }),
            );
            Ok(hashes)
        } else {
            let result = calculate_file_flash_hashes(app, &item.file_path, item.size).await;
            if let Ok(hashes) = &result {
                let saved = {
                    let guard = state.lock().map_err(|error| error.to_string())?;
                    save_cached_file_hashes(
                        &guard.db_path,
                        &item.file_path,
                        item.size,
                        item.modified_ms,
                        hashes,
                        cache_settings(&guard),
                    )
                };
                if let Err(error) = saved {
                    status(app, "warning", error);
                }
            }
            result
        };
        match hashes_result {
            Ok(hashes) => match api_post(
                &token,
                &device_id,
                "/userres/v1/check_can_flash_upload",
                json!({
                    "taskId": data.task_id,
                    "gcid": hashes.gcid,
                    "cid": hashes.cid
                }),
                &[],
            )
            .await
            {
                Ok(check) => {
                    let check_data = check.data.unwrap_or_default();
                    instant_upload = check_data
                        .get("canFlashUpload")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if instant_upload {
                        if let Some(task_id) = check_data
                            .get("taskId")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            data.task_id = task_id.to_string();
                        }
                    }
                }
                Err(error) => status(
                    app,
                    "warning",
                    format!("后台秒传校验失败，稍后继续普通上传：{error}"),
                ),
            },
            Err(error) => status(
                app,
                "warning",
                format!("后台秒传指纹计算失败，稍后继续普通上传：{error}"),
            ),
        }
    }

    if !instant_upload {
        return Ok(FlashPreflightOutcome::Miss(data));
    }

    clear_upload_checkpoint(&db_path, item)?;
    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 100,
            "uploaded_bytes": item.size,
            "total_bytes": item.size,
            "bytes_per_second": 0,
            "stage": "已命中秒传"
        }),
    );
    let pending_outcome = UploadOutcome {
        task_id: data.task_id.clone(),
        remote_file_id: None,
    };
    remember_pending_upload(state, item, &pending_outcome)
        .map_err(|message| format!("文件已秒传，但写入本地上传记录失败：{message}"))?;
    emit(
        app,
        json!({
            "type": "file",
            "state": "processing",
            "file_path": item.file_path.to_string_lossy(),
            "mapping_id": item.mapping_id,
            "uploaded_bytes": item.size,
            "total_bytes": item.size,
            "stage": "已秒传，正在等待云端入库"
        }),
    );
    Ok(FlashPreflightOutcome::Accepted {
        task_id: data.task_id,
        token,
        device_id,
    })
}

pub(crate) async fn upload_item(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
) -> Result<UploadOutcome, String> {
    let (token, device_id, multipart_part_size, db_path) = {
        let guard = state.lock().map_err(|e| e.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.multipart_part_size.clone(),
            guard.db_path.clone(),
        )
    };
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在准备云端目录" }),
    );
    let parent_id = ensure_remote_path(
        app,
        state,
        &token,
        &device_id,
        &item.remote_parent_id,
        &item.remote_dir,
    )
    .await?;
    let name = upload_remote_name(item)?;
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在申请上传凭证" }),
    );
    let mut persisted = load_upload_checkpoint(&db_path, item)?;
    let mut resumed_data = None;
    if let Some(saved) = persisted.as_ref() {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": if item.size == 0 { 0 } else { saved.uploaded_bytes.saturating_mul(100) / item.size },
                "uploaded_bytes": saved.uploaded_bytes,
                "total_bytes": item.size,
                "stage": "正在恢复上传断点"
            }),
        );
        let resume_result = api_post(
            &token,
            &device_id,
            "/userres/v1/get_res_center_resume_token",
            json!({
                "capacity": 2,
                "res": { "fileSize": item.size },
                "taskId": saved.checkpoint.task_id,
                "object": {
                    "objectPath": saved.checkpoint.object_path,
                    "provider": saved.checkpoint.provider
                }
            }),
            &[156],
        )
        .await;
        match resume_result {
            Ok(result) => {
                let mut token_data: UploadToken = serde_json::from_value(
                    result
                        .data
                        .ok_or_else(|| "光鸭没有返回续传凭证".to_string())?,
                )
                .map_err(|error| format!("续传凭证格式异常：{error}"))?;
                if token_data
                    .object_path
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.object_path = Some(saved.checkpoint.object_path.clone());
                }
                if token_data
                    .bucket_name
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.bucket_name = Some(saved.checkpoint.bucket_name.clone());
                }
                if token_data
                    .end_point
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.end_point = token_data
                        .full_end_point
                        .clone()
                        .or_else(|| Some(saved.checkpoint.end_point.clone()));
                }
                if token_data.provider.is_none() {
                    token_data.provider = saved.checkpoint.provider.clone();
                }
                resumed_data = Some(token_data);
            }
            Err(error) => {
                status(
                    app,
                    "warning",
                    format!("恢复上传断点失败，将重新创建上传任务：{error}"),
                );
                clear_upload_checkpoint(&db_path, item)?;
                persisted = None;
            }
        }
    }
    let preflight_token = if resumed_data.is_none() && persisted.is_none() {
        take_flash_preflight_token(state, item)?
    } else {
        None
    };
    let (mut data, mut instant_upload) = if let Some(data) = resumed_data {
        (data, false)
    } else if let Some(data) = preflight_token {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": 0,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "bytes_per_second": 0,
                "stage": "秒传未命中，正在进入上传通道"
            }),
        );
        (data, false)
    } else {
        let mut res = json!({ "fileSize": item.size });
        if item.size < 1024 * 1024 {
            emit(
                app,
                json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在计算秒传 MD5" }),
            );
            res["md5"] = json!(calculate_file_md5(&item.file_path).await?);
        }
        let result = api_post(
            &token,
            &device_id,
            "/userres/v1/get_res_center_token",
            json!({ "capacity": 2, "name": name, "res": res, "parentId": parent_id }),
            &[156],
        )
        .await?;
        let instant_upload = result.code == 156;
        let data: UploadToken = serde_json::from_value(
            result
                .data
                .ok_or_else(|| "光鸭没有返回上传凭证".to_string())?,
        )
        .map_err(|e| format!("上传凭证格式异常：{e}"))?;
        (data, instant_upload)
    };
    if !instant_upload && persisted.is_none() && item.size >= 1024 * 1024 {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在校验秒传" }),
        );
        let cached_hashes = match {
            let guard = state.lock().map_err(|error| error.to_string())?;
            load_cached_file_hashes(
                &guard.db_path,
                &item.file_path,
                item.size,
                item.modified_ms,
                cache_settings(&guard),
            )
        } {
            Ok(value) => value,
            Err(error) => {
                status(app, "warning", error);
                None
            }
        };
        let hashes_result = if let Some(hashes) = cached_hashes {
            emit(
                app,
                json!({
                    "type": "progress",
                    "file_path": item.file_path.to_string_lossy(),
                    "percent": 0,
                    "uploaded_bytes": 0,
                    "total_bytes": item.size,
                    "bytes_per_second": 0,
                    "stage": "已复用本地秒传指纹"
                }),
            );
            Ok(hashes)
        } else {
            let result = calculate_file_flash_hashes(app, &item.file_path, item.size).await;
            if let Ok(hashes) = &result {
                let saved = {
                    let guard = state.lock().map_err(|error| error.to_string())?;
                    save_cached_file_hashes(
                        &guard.db_path,
                        &item.file_path,
                        item.size,
                        item.modified_ms,
                        hashes,
                        cache_settings(&guard),
                    )
                };
                if let Err(error) = saved {
                    status(app, "warning", error);
                }
            }
            result
        };
        match hashes_result {
            Ok(hashes) => match api_post(
                &token,
                &device_id,
                "/userres/v1/check_can_flash_upload",
                json!({
                    "taskId": data.task_id,
                    "gcid": hashes.gcid,
                    "cid": hashes.cid
                }),
                &[],
            )
            .await
            {
                Ok(check) => {
                    let check_data = check.data.unwrap_or_default();
                    instant_upload = check_data
                        .get("canFlashUpload")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if instant_upload {
                        if let Some(task_id) = check_data
                            .get("taskId")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            data.task_id = task_id.to_string();
                        }
                    }
                }
                Err(error) => status(
                    app,
                    "warning",
                    format!("秒传校验失败，继续普通上传：{error}"),
                ),
            },
            Err(error) => status(
                app,
                "warning",
                format!("秒传指纹计算失败，继续普通上传：{error}"),
            ),
        }
    }
    if !instant_upload {
        if upload_credentials_expired(&data) {
            emit(
                app,
                json!({
                    "type": "progress",
                    "file_path": item.file_path.to_string_lossy(),
                    "uploaded_bytes": persisted.as_ref().map(|checkpoint| checkpoint.uploaded_bytes).unwrap_or(0),
                    "total_bytes": item.size,
                    "bytes_per_second": 0,
                    "stage": "上传凭证已过期，正在刷新后续传"
                }),
            );
            data = refresh_upload_token(&token, &device_id, item.size, &data).await?;
        }
        let uploaded_bytes = persisted
            .as_ref()
            .map(|checkpoint| checkpoint.uploaded_bytes)
            .unwrap_or(0);
        let resumed = persisted.is_some();
        emit(
            app,
            json!({ "type": "file", "state": "uploading", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": uploaded_bytes, "total_bytes": item.size }),
        );
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": if item.size == 0 { 0 } else { uploaded_bytes.saturating_mul(100) / item.size }, "uploaded_bytes": uploaded_bytes, "total_bytes": item.size, "stage": if resumed { "正在从断点继续上传" } else { "正在连接 OSS" } }),
        );
        let mut current_checkpoint = persisted;
        let mut credential_refreshes = 0usize;
        loop {
            match upload_oss(
                &data,
                item,
                app,
                &multipart_part_size,
                &db_path,
                current_checkpoint.clone(),
            )
            .await
            {
                Ok(()) => break,
                Err(error) if is_oss_security_token_expired(&error) && credential_refreshes < 3 => {
                    credential_refreshes += 1;
                    current_checkpoint = load_upload_checkpoint(&db_path, item)?;
                    let uploaded_bytes = current_checkpoint
                        .as_ref()
                        .map(|checkpoint| checkpoint.uploaded_bytes)
                        .unwrap_or(0);
                    emit(
                        app,
                        json!({
                            "type": "progress",
                            "file_path": item.file_path.to_string_lossy(),
                            "uploaded_bytes": uploaded_bytes,
                            "total_bytes": item.size,
                            "bytes_per_second": 0,
                            "stage": "OSS 上传凭证已过期，正在刷新后续传"
                        }),
                    );
                    data = refresh_upload_token(&token, &device_id, item.size, &data).await?;
                }
                Err(error) => return Err(error),
            }
        }
    } else {
        clear_upload_checkpoint(&db_path, item)?;
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已命中秒传" }),
        );
    }
    let pending_outcome = UploadOutcome {
        task_id: data.task_id.clone(),
        remote_file_id: None,
    };
    remember_pending_upload(state, item, &pending_outcome)
        .map_err(|message| format!("文件已上传，但写入本地上传记录失败：{message}"))?;
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已上传，正在等待云端入库" }),
    );
    emit(
        app,
        json!({ "type": "file", "state": "processing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已上传，正在等待云端入库" }),
    );
    let task_data = wait_upload_task(app, &token, &device_id, &data.task_id, &item.file_path)
        .await
        .map_err(|error| {
            format!(
                "文件已上传并已写入待确认记录，不会重复上传；后台将继续确认云端入库：{}",
                error.message()
            )
        })?;
    let outcome = UploadOutcome {
        task_id: data.task_id,
        remote_file_id: task_data
            .get("fileId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    complete_upload_replacement(app, state, &token, &device_id, item, &outcome)
        .await
        .map_err(|message| {
            if message == UPLOAD_CANCELLED_MESSAGE {
                message
            } else {
                format!("新版本已入库，但安全替换尚未完成；后台将继续恢复：{message}")
            }
        })?;
    remember_confirmed_upload(state, item, &outcome)
        .map_err(|message| format!("云端已入库，但更新本地确认状态失败：{message}"))?;
    Ok(outcome)
}

pub(crate) async fn organizer_upload_bytes(
    app: &tauri::AppHandle,
    parent_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let temp_dir =
        std::env::temp_dir().join(format!("guangya-organizer-{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&temp_dir).map_err(|error| format!("创建刮削上传临时目录失败：{error}"))?;
    let file_path = temp_dir.join(name);
    fs::write(&file_path, bytes).map_err(|error| format!("写入刮削上传临时文件失败：{error}"))?;
    let metadata =
        fs::metadata(&file_path).map_err(|error| format!("读取刮削临时文件失败：{error}"))?;
    let item = UploadItem {
        mapping_id: "__organizer__".to_string(),
        file_path: file_path.clone(),
        remote_parent_id: parent_id.to_string(),
        remote_dir: String::new(),
        relative_path: name.to_string(),
        change_kind: "added".to_string(),
        size: metadata.len(),
        modified_ms: modified_ms(&metadata),
        replacement: None,
    };
    let state = app.state::<SharedState>();
    let result = upload_item(app, state.inner(), &item).await;
    if let Ok(outcome) = &result {
        // Organizer metadata bypasses the ordinary upload queue, so it must
        // explicitly publish the terminal transfer event after cloud indexing.
        finalize_successful_upload(app, state.inner(), &item, outcome).await;
        let _ = fs::remove_file(&file_path);
        let _ = fs::remove_dir(&temp_dir);
    }
    result.and_then(|outcome| {
        outcome
            .remote_file_id
            .ok_or_else(|| "刮削文件已上传，但云端没有返回文件 ID".to_string())
    })
}

pub(crate) fn archive_candidate(base: &Path, modified_ms: u128, collision: u64) -> PathBuf {
    if collision == 0 {
        return base.to_path_buf();
    }
    let stem = base
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let suffix = if collision == 1 {
        format!("-{modified_ms}")
    } else {
        format!("-{modified_ms}-{collision}")
    };
    let extension = base
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    base.with_file_name(format!("{stem}{suffix}{extension}"))
}

pub(crate) fn remove_partial_archive(path: &Path, original_error: String) -> String {
    match fs::remove_file(path) {
        Ok(()) => original_error,
        Err(cleanup_error) if cleanup_error.kind() == io::ErrorKind::NotFound => original_error,
        Err(cleanup_error) => {
            format!("{original_error}；清理未完成的归档副本也失败：{cleanup_error}")
        }
    }
}

pub(crate) fn copy_archive_exclusive(
    source: &Path,
    destination: &Path,
    expected_size: u64,
    expected_modified_ms: u128,
) -> Result<bool, String> {
    let mut source_file = fs::File::open(source).map_err(|e| format!("打开源文件失败：{e}"))?;
    let before = source_file
        .metadata()
        .map_err(|e| format!("读取源文件元数据失败：{e}"))?;
    if before.len() != expected_size || modified_ms(&before) != expected_modified_ms {
        return Err("归档前源文件发生变化，已保留源文件".into());
    }
    let mut destination_file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(format!("创建排他归档文件失败：{error}")),
    };
    let copied = match io::copy(&mut source_file, &mut destination_file) {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("复制到归档目录失败：{error}"),
            ));
        }
    };
    if let Err(error) = destination_file.sync_all() {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            format!("同步归档文件失败：{error}"),
        ));
    }
    let destination_size = match destination_file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("核对归档文件失败：{error}"),
            ));
        }
    };
    let after = match source_file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("复制后核对源文件失败：{error}"),
            ));
        }
    };
    if copied != expected_size || destination_size != expected_size {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            format!(
                "归档复制字节数不一致（预期 {expected_size}，读取 {copied}，写入 {destination_size}），已保留源文件"
            ),
        ));
    }
    if after.len() != expected_size || modified_ms(&after) != expected_modified_ms {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            "归档复制期间源文件发生变化，已保留源文件".into(),
        ));
    }
    drop(destination_file);
    if let Err(error) = fs::remove_file(source) {
        return Err(remove_partial_archive(
            destination,
            format!("归档副本已核对，但移除源文件失败：{error}"),
        ));
    }
    Ok(true)
}

pub(crate) fn archive_file_without_overwrite(
    source: &Path,
    requested_destination: &Path,
    expected_size: u64,
    expected_modified_ms: u128,
) -> Result<PathBuf, String> {
    for collision in 0..u64::MAX {
        let destination = archive_candidate(requested_destination, expected_modified_ms, collision);
        match fs::hard_link(source, &destination) {
            Ok(()) => {
                let source_metadata = fs::metadata(source).map_err(|e| {
                    remove_partial_archive(&destination, format!("核对源文件失败：{e}"))
                })?;
                let destination_metadata = fs::metadata(&destination).map_err(|e| {
                    remove_partial_archive(&destination, format!("核对归档文件失败：{e}"))
                })?;
                if source_metadata.len() != expected_size
                    || modified_ms(&source_metadata) != expected_modified_ms
                    || destination_metadata.len() != expected_size
                {
                    return Err(remove_partial_archive(
                        &destination,
                        "归档期间源文件发生变化，已保留源文件".into(),
                    ));
                }
                if let Err(error) = fs::remove_file(source) {
                    return Err(remove_partial_archive(
                        &destination,
                        format!("创建归档链接后移除源文件失败：{error}"),
                    ));
                }
                return Ok(destination);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => match copy_archive_exclusive(
                source,
                &destination,
                expected_size,
                expected_modified_ms,
            )? {
                true => return Ok(destination),
                false => continue,
            },
        }
    }
    Err("无法生成唯一的归档文件名".into())
}

pub(crate) fn apply_source_policy(state: &SharedState, item: &UploadItem) -> Result<Option<String>, String> {
    if item.mapping_id.starts_with("__") {
        return Ok(None);
    }
    let mapping = state
        .lock()
        .map_err(|e| e.to_string())?
        .mappings
        .iter()
        .find(|mapping| mapping.id == item.mapping_id)
        .cloned()
        .ok_or_else(|| "备份任务已被移除，源文件保持不变".to_string())?;
    if mapping.source_policy == "keep" {
        return Ok(None);
    }
    let metadata = fs::metadata(&item.file_path).map_err(|e| format!("读取源文件失败：{e}"))?;
    if metadata.len() != item.size || modified_ms(&metadata) != item.modified_ms {
        return Err("上传期间源文件发生变化，已保留源文件且不会执行上传后策略".into());
    }
    if mapping.source_policy == "delete" {
        fs::remove_file(&item.file_path).map_err(|e| format!("删除源文件失败：{e}"))?;
        return Ok(Some("已按任务策略删除源文件".into()));
    }
    if mapping.source_policy != "archive" {
        return Err(format!("未知的源文件策略：{}", mapping.source_policy));
    }
    let archive_root = mapping
        .archive_path
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "归档策略没有配置归档目录".to_string())?;
    let source_root = PathBuf::from(&mapping.local_path);
    let archive_root = PathBuf::from(archive_root);
    if archive_root.starts_with(&source_root) {
        return Err("归档目录不能位于被监控目录内部".into());
    }
    let relative = item
        .file_path
        .strip_prefix(&source_root)
        .map_err(|_| "无法计算源文件的相对路径".to_string())?;
    let requested_destination = archive_root.join(relative);
    if let Some(parent) = requested_destination.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建归档目录失败：{e}"))?;
    }
    let destination = archive_file_without_overwrite(
        &item.file_path,
        &requested_destination,
        item.size,
        item.modified_ms,
    )?;
    Ok(Some(format!("已移动到归档目录：{}", destination.display())))
}

pub(crate) fn source_changed_since_upload(item: &UploadItem) -> bool {
    fs::metadata(&item.file_path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .is_some_and(|metadata| {
            metadata.len() != item.size || modified_ms(&metadata) != item.modified_ms
        })
}

pub(crate) fn resubmit_source_if_changed(state: &SharedState, item: &UploadItem) -> bool {
    if item.mapping_id.starts_with("__") {
        return false;
    }
    if source_changed_since_upload(item) {
        if let Ok(guard) = state.lock() {
            return guard
                .event_tx
                .send(FsEvent {
                    mapping_id: item.mapping_id.clone(),
                    path: item.file_path.clone(),
                })
                .is_ok();
        }
    }
    false
}

pub(crate) async fn finalize_successful_upload(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) {
    if let Some(parent_id) = cached_remote_path_id(state, &item.remote_parent_id, &item.remote_dir)
    {
        publish_directory_contents_changed(app, [parent_id], "upload-confirmed");
    } else {
        publish_all_cloud_directories_changed(app, state, "upload-confirmed");
    }
    let key = item_key(&item.mapping_id, &item.file_path);
    if upload_is_cancelled(state, &key) {
        if let Ok(guard) = state.lock() {
            if let Err(message) = clear_cancelled_upload_artifacts(&guard.db_path, item) {
                status(app, "warning", message);
            }
        }
        return;
    }
    if resubmit_source_if_changed(state, item) {
        emit(
            app,
            json!({
                "type": "file",
                "state": "waiting-file",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": 0,
                "total_bytes": fs::metadata(&item.file_path).map(|metadata| metadata.len()).unwrap_or(item.size),
                "stage": "检测到源文件仍在写入，等待完整后重新上传"
            }),
        );
        return;
    }
    let db_path = state.lock().ok().map(|guard| guard.db_path.clone());
    if let Some(path) = db_path.as_deref() {
        if let Err(message) = clear_auto_share_failure(path, item) {
            status(app, "error", message);
        }
    }
    let organizer_flow = state.lock().ok().and_then(|guard| {
        guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == item.mapping_id)
            .filter(|mapping| !mapping.organizer_mapping_id.is_empty())
            .map(|mapping| (mapping.organizer_mapping_id.clone(), mapping.auto_share))
    });
    if let Some((organizer_mapping_id, share_after)) = organizer_flow {
        let remote_file_id = outcome.remote_file_id.clone().unwrap_or_default();
        if let Err(message) = organizer::notify_upload(
            app.clone(),
            organizer_mapping_id,
            remote_file_id,
            item.relative_path.clone(),
            share_after,
        )
        .await
        {
            status(
                app,
                "error",
                format!("文件已上传，但上传后整理排队失败；为避免分享 A 目录，未执行原自动分享：{message}"),
            );
        }
    } else if let Err(message) = schedule_auto_share(app, state, item, outcome).await {
        status(
            app,
            "error",
            format!("文件已上传，但自动分享排队失败：{message}"),
        );
    }
    match apply_source_policy(state, item) {
        Ok(Some(message)) => status(app, "success", message),
        Ok(None) => {}
        Err(message) => {
            status(app, "error", message);
            resubmit_source_if_changed(state, item);
        }
    }
    emit(
        app,
        json!({
            "type": "file",
            "state": "done",
            "file_path": item.file_path.to_string_lossy(),
            "mapping_id": item.mapping_id,
            "uploaded_bytes": item.size,
            "total_bytes": item.size
        }),
    );
}
