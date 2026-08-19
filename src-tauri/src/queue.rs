//! 上传队列调度、并发控制与手动上传入队。

use crate::prelude::*;

pub(crate) fn drain_flash_preflight(app: tauri::AppHandle, state: SharedState) {
    loop {
        let item = {
            let mut guard = match state.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if guard.paused
                || guard.token.is_none()
                || guard.active_uploads == 0
                || guard.active_flash_preflights >= FLASH_PREFLIGHT_CONCURRENCY
            {
                None
            } else {
                let position = guard.queue.iter().position(|candidate| {
                    let key = item_key(&candidate.mapping_id, &candidate.file_path);
                    !guard.inflight.contains_key(&key)
                        && !guard.cancelled_uploads.contains_key(&key)
                        && !guard.paused_uploads.contains(&key)
                        && !flash_preflight_cached(&guard, candidate)
                });
                position.and_then(|position| {
                    let item = guard.queue.remove(position)?;
                    let key = item_key(&item.mapping_id, &item.file_path);
                    guard.active_flash_preflights += 1;
                    guard.inflight.insert(
                        key.clone(),
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                    guard.inflight_items.insert(key, item.clone());
                    Some(item)
                })
            }
        };
        let Some(item) = item else {
            emit_state(&app, &state);
            return;
        };
        emit(
            &app,
            json!({
                "type": "file",
                "state": "preparing",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "stage": "正在后台校验秒传"
            }),
        );
        let app2 = app.clone();
        let state2 = state.clone();
        tauri::async_runtime::spawn(async move {
            let upload_key = item_key(&item.mapping_id, &item.file_path);
            let mut item = item;
            let result = match prepare_upload_item(&state2, &item).await {
                Ok(Some(ready)) => {
                    item = ready;
                    interruptible_upload_step(
                        &state2,
                        &upload_key,
                        preflight_flash_upload(&app2, &state2, &item),
                    )
                    .await
                    .and_then(|result| result)
                    .map(Some)
                }
                Ok(None) => Ok(None),
                Err(message) => Err(message),
            };
            let waiting_for_file = result.as_ref().ok().is_some_and(Option::is_none);
            let cancelled = upload_is_cancelled(&state2, &upload_key);
            let pause_interrupted = result
                .as_ref()
                .err()
                .is_some_and(|message| message == UPLOAD_PAUSED_MESSAGE);
            let auth_expired = result
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("登录态已失效"));
            let cloud_pending = state2
                .lock()
                .ok()
                .is_some_and(|guard| guard.pending_cloud.contains_key(&upload_key));
            let mut db_path = None;
            let mut requeued = false;
            let mut paused_upload = false;
            let mut individually_paused = false;
            if let Ok(mut guard) = state2.lock() {
                guard.active_flash_preflights = guard.active_flash_preflights.saturating_sub(1);
                guard.inflight.remove(&upload_key);
                guard.inflight_items.remove(&upload_key);
                db_path = Some(guard.db_path.clone());
                if auth_expired {
                    guard.token = None;
                }
                let mapping_active = item.mapping_id.starts_with("__")
                    || guard
                        .mappings
                        .iter()
                        .any(|mapping| mapping.id == item.mapping_id && mapping.enabled);
                individually_paused = guard.paused_uploads.contains(&upload_key);
                let queue_pause_requested = guard.queue_pause_requests.remove(&upload_key);
                paused_upload = !cloud_pending
                    && (pause_interrupted || individually_paused || queue_pause_requested);
                if !paused_upload {
                    guard.paused_uploads.remove(&upload_key);
                }
                if cancelled {
                    guard.pending_cloud.remove(&upload_key);
                    guard.flash_preflight_cache.remove(&upload_key);
                }
                match &result {
                    Ok(Some(FlashPreflightOutcome::Miss(_))) if mapping_active && !cancelled => {
                        let token = match result.as_ref().ok().and_then(Option::as_ref) {
                            Some(FlashPreflightOutcome::Miss(token)) => Some(token.clone()),
                            _ => None,
                        };
                        guard.flash_preflight_cache.insert(
                            upload_key.clone(),
                            FlashPreflightCache {
                                stamp: Stamp {
                                    size: item.size,
                                    modified_ms: item.modified_ms,
                                },
                                upload_token: token,
                                created_at: Instant::now(),
                            },
                        );
                        guard.queue.push_front(item.clone());
                        requeued = true;
                    }
                    Ok(Some(FlashPreflightOutcome::Skipped)) | Err(_)
                        if !cloud_pending && mapping_active && !cancelled =>
                    {
                        guard.flash_preflight_cache.insert(
                            upload_key.clone(),
                            FlashPreflightCache {
                                stamp: Stamp {
                                    size: item.size,
                                    modified_ms: item.modified_ms,
                                },
                                upload_token: None,
                                created_at: Instant::now(),
                            },
                        );
                        guard.queue.push_front(item.clone());
                        requeued = true;
                    }
                    _ => {
                        guard.flash_preflight_cache.remove(&upload_key);
                    }
                }
                if waiting_for_file && !cancelled {
                    guard.waiting_files.insert(upload_key.clone(), item.clone());
                }
            }
            if auth_expired {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = clear_persisted_access_token(path) {
                        status(&app2, "error", message);
                    }
                }
            }
            if cancelled {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = clear_cancelled_upload_artifacts(path, &item) {
                        status(&app2, "warning", message);
                    }
                }
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "cancelled",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "uploaded_bytes": 0,
                        "total_bytes": item.size,
                        "stage": "已取消"
                    }),
                );
                emit_state(&app2, &state2);
                drain_queue(app2, state2);
                return;
            }
            if paused_upload {
                let uploaded_bytes = db_path
                    .as_deref()
                    .and_then(|path| load_upload_checkpoint(path, &item).ok().flatten())
                    .map(|checkpoint| checkpoint.uploaded_bytes)
                    .unwrap_or(0);
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "paused",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "uploaded_bytes": uploaded_bytes,
                        "total_bytes": item.size,
                        "stage": if individually_paused { "已暂停" } else { "队列已暂停" }
                    }),
                );
                emit_state(&app2, &state2);
                if waiting_for_file {
                    tauri::async_runtime::spawn(requeue_busy_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
                drain_queue(app2, state2);
                return;
            }
            match result {
                Ok(Some(FlashPreflightOutcome::Accepted {
                    task_id,
                    token,
                    device_id,
                })) => {
                    let confirm_app = app2.clone();
                    let confirm_state = state2.clone();
                    let confirm_item = item.clone();
                    tauri::async_runtime::spawn(async move {
                        let confirm_key =
                            item_key(&confirm_item.mapping_id, &confirm_item.file_path);
                        let task_result = abortable_upload_step(
                            &confirm_state,
                            &confirm_key,
                            wait_upload_task(
                                &confirm_app,
                                &token,
                                &device_id,
                                &task_id,
                                &confirm_item.file_path,
                            ),
                        )
                        .await;
                        match task_result {
                            Ok(Ok(task_data)) => {
                                let outcome = UploadOutcome {
                                    task_id,
                                    remote_file_id: task_data
                                        .get("fileId")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                };
                                if let Err(message) = complete_upload_replacement(
                                    &confirm_app,
                                    &confirm_state,
                                    &token,
                                    &device_id,
                                    &confirm_item,
                                    &outcome,
                                )
                                .await
                                {
                                    status(
                                         &confirm_app,
                                         "warning",
                                         format!("新版本已入库，但安全替换尚未完成；后台将继续恢复：{message}"),
                                     );
                                    emit_state(&confirm_app, &confirm_state);
                                    return;
                                }
                                match remember_confirmed_upload(
                                    &confirm_state,
                                    &confirm_item,
                                    &outcome,
                                ) {
                                    Ok(()) => {
                                        finalize_successful_upload(
                                            &confirm_app,
                                            &confirm_state,
                                            &confirm_item,
                                            &outcome,
                                        )
                                        .await;
                                    }
                                    Err(message) => status(&confirm_app, "warning", message),
                                }
                            }
                            Ok(Err(error)) => status(
                                &confirm_app,
                                "warning",
                                format!(
                                    "秒传已完成，云端入库将在后台继续确认：{}",
                                    error.message()
                                ),
                            ),
                            Err(message) if message == UPLOAD_CANCELLED_MESSAGE => {
                                if let Ok(guard) = confirm_state.lock() {
                                    if let Err(error) = clear_cancelled_upload_artifacts(
                                        &guard.db_path,
                                        &confirm_item,
                                    ) {
                                        status(&confirm_app, "warning", error);
                                    }
                                }
                            }
                            Err(message) => status(&confirm_app, "warning", message),
                        }
                        emit_state(&confirm_app, &confirm_state);
                    });
                }
                Ok(Some(FlashPreflightOutcome::Miss(_))) => {
                    if requeued {
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": "queued",
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": "秒传未命中，等待上传通道"
                            }),
                        );
                    }
                }
                Ok(Some(FlashPreflightOutcome::Skipped)) => {
                    if requeued {
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": "queued",
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": "已有上传断点，等待上传通道"
                            }),
                        );
                    }
                }
                Ok(None) => {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "waiting-file",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": 0,
                            "total_bytes": item.size,
                            "stage": "另外的程序正在使用该文件，释放后将自动上传"
                        }),
                    );
                    tauri::async_runtime::spawn(requeue_busy_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
                Err(message) if cloud_pending => {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "processing",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": item.size,
                            "total_bytes": item.size,
                            "stage": "秒传已完成，后台将继续确认云端入库"
                        }),
                    );
                    status(&app2, "warning", message);
                }
                Err(message) => {
                    if requeued {
                        status(
                            &app2,
                            "warning",
                            format!("后台秒传预检失败，已回到上传队列：{message}"),
                        );
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": if auth_expired { "waiting-login" } else { "queued" },
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": if auth_expired { "登录态已失效，重新登录后继续" } else { "秒传预检失败，等待普通上传" }
                            }),
                        );
                    }
                }
            }
            emit_state(&app2, &state2);
            drain_queue(app2, state2);
        });
    }
}

pub(crate) fn drain_queue(app: tauri::AppHandle, state: SharedState) {
    loop {
        let item = {
            let mut guard = match state.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if guard.paused
                || guard.token.is_none()
                || guard.active_uploads >= guard.upload_concurrency
            {
                None
            } else {
                let position = guard.queue.iter().position(|candidate| {
                    let key = item_key(&candidate.mapping_id, &candidate.file_path);
                    !guard.inflight.contains_key(&key)
                        && !guard.cancelled_uploads.contains_key(&key)
                        && !guard.paused_uploads.contains(&key)
                });
                let item = position.and_then(|position| guard.queue.remove(position));
                if let Some(item) = &item {
                    guard.active_uploads += 1;
                    guard.inflight.insert(
                        item_key(&item.mapping_id, &item.file_path),
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                    guard
                        .inflight_items
                        .insert(item_key(&item.mapping_id, &item.file_path), item.clone());
                }
                item
            }
        };
        let Some(item) = item else {
            emit_state(&app, &state);
            drain_flash_preflight(app, state);
            return;
        };
        emit(
            &app,
            json!({ "type": "file", "state": "preparing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": 0, "total_bytes": item.size }),
        );
        let app2 = app.clone();
        let state2 = state.clone();
        tauri::async_runtime::spawn(async move {
            let upload_key = item_key(&item.mapping_id, &item.file_path);
            let mut item = item;
            let result = match prepare_upload_item(&state2, &item).await {
                Ok(Some(ready)) => {
                    item = ready;
                    interruptible_upload_step(
                        &state2,
                        &upload_key,
                        upload_item(&app2, &state2, &item),
                    )
                    .await
                    .and_then(|result| result)
                    .map(Some)
                }
                Ok(None) => Ok(None),
                Err(message) => Err(message),
            };
            let waiting_for_file = result.as_ref().ok().is_some_and(Option::is_none);
            let cancelled = upload_is_cancelled(&state2, &upload_key);
            let pause_interrupted = result
                .as_ref()
                .err()
                .is_some_and(|message| message == UPLOAD_PAUSED_MESSAGE);
            let auth_expired = result
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("登录态已失效"));
            let outcome = result.as_ref().ok().and_then(|value| value.clone());
            let error_message = result.as_ref().err().cloned();
            let mut db_path = None;
            let mut cloud_pending = false;
            let mut paused_upload = false;
            let mut individually_paused = false;
            if let Ok(mut guard) = state2.lock() {
                guard.active_uploads = guard.active_uploads.saturating_sub(1);
                guard.inflight.remove(&upload_key);
                guard.inflight_items.remove(&upload_key);
                db_path = Some(guard.db_path.clone());
                if auth_expired {
                    guard.token = None;
                }
                cloud_pending = guard.pending_cloud.contains_key(&upload_key);
                individually_paused = guard.paused_uploads.contains(&upload_key);
                let queue_pause_requested = guard.queue_pause_requests.remove(&upload_key);
                paused_upload = !cloud_pending
                    && !cancelled
                    && (pause_interrupted
                        || (waiting_for_file && (individually_paused || queue_pause_requested)));
                if paused_upload {
                    let mapping_active = item.mapping_id.starts_with("__")
                        || guard
                            .mappings
                            .iter()
                            .any(|mapping| mapping.id == item.mapping_id && mapping.enabled);
                    if !waiting_for_file
                        && mapping_active
                        && !guard.queue.iter().any(|queued| {
                            item_key(&queued.mapping_id, &queued.file_path) == upload_key
                        })
                    {
                        guard.queue.push_front(item.clone());
                    }
                } else {
                    guard.paused_uploads.remove(&upload_key);
                }
                if cancelled {
                    guard.pending_cloud.remove(&upload_key);
                    guard.flash_preflight_cache.remove(&upload_key);
                }
                if waiting_for_file && !cancelled {
                    guard.waiting_files.insert(upload_key.clone(), item.clone());
                }
            }
            if auth_expired {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = clear_persisted_access_token(path) {
                        status(&app2, "error", message);
                    }
                }
            }
            if cancelled {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = clear_cancelled_upload_artifacts(path, &item) {
                        status(&app2, "warning", message);
                    }
                }
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "cancelled",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "uploaded_bytes": 0,
                        "total_bytes": item.size,
                        "stage": "已取消"
                    }),
                );
                emit_state(&app2, &state2);
                drain_queue(app2, state2);
                return;
            }
            if paused_upload {
                let uploaded_bytes = db_path
                    .as_deref()
                    .and_then(|path| load_upload_checkpoint(path, &item).ok().flatten())
                    .map(|checkpoint| checkpoint.uploaded_bytes)
                    .unwrap_or(0);
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "paused",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "uploaded_bytes": uploaded_bytes,
                        "total_bytes": item.size,
                        "stage": if individually_paused { "已暂停，可从当前断点继续" } else { "队列已暂停，可从当前断点继续" }
                    }),
                );
                emit_state(&app2, &state2);
                if waiting_for_file {
                    tauri::async_runtime::spawn(requeue_busy_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
                drain_queue(app2, state2);
                return;
            }
            if waiting_for_file {
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "waiting-file",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "uploaded_bytes": 0,
                        "total_bytes": item.size,
                        "stage": "另外的程序正在使用该文件，释放后将自动上传"
                    }),
                );
                tauri::async_runtime::spawn(requeue_busy_upload(
                    app2.clone(),
                    state2.clone(),
                    item.clone(),
                ));
            } else if let Some(outcome) = outcome {
                finalize_successful_upload(&app2, &state2, &item, &outcome).await;
            } else {
                let message = error_message.unwrap_or_else(|| "上传失败".into());
                if cloud_pending {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "processing",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": item.size,
                            "total_bytes": item.size,
                            "stage": "OSS 已完成，后台将继续确认云端入库"
                        }),
                    );
                    status(&app2, "warning", message);
                    emit_state(&app2, &state2);
                    drain_queue(app2, state2);
                    return;
                }
                let resumable = db_path
                    .as_deref()
                    .and_then(|path| load_upload_checkpoint(path, &item).ok().flatten());
                let auto_share_enabled = resumable.is_none()
                    && state2.lock().ok().is_some_and(|guard| {
                        guard.mappings.iter().any(|mapping| {
                            mapping.id == item.mapping_id
                                && mapping.auto_share
                                && mapping.organizer_mapping_id.is_empty()
                        })
                    });
                if auto_share_enabled {
                    if let Some(path) = db_path.as_deref() {
                        if let Err(error) = record_auto_share_failure(path, &item, &message) {
                            status(&app2, "error", error);
                        }
                    }
                }
                let uploaded_bytes = resumable
                    .as_ref()
                    .map(|checkpoint| checkpoint.uploaded_bytes)
                    .unwrap_or(0);
                if let Ok(mut guard) = state2.lock() {
                    guard
                        .failed_uploads
                        .insert(upload_key.clone(), item.clone());
                }
                emit(
                    &app2,
                    json!({ "type": "file", "state": "error", "file_path": item.file_path.to_string_lossy(), "uploaded_bytes": uploaded_bytes, "total_bytes": item.size, "error": message.clone() }),
                );
                if resumable.is_some() {
                    tauri::async_runtime::spawn(requeue_resumable_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
            }
            emit_state(&app2, &state2);
            drain_queue(app2, state2);
        });
    }
}

pub(crate) async fn enqueue_path(app: &tauri::AppHandle, state: &SharedState, event: FsEvent) {
    let mapping = state.lock().ok().and_then(|guard| {
        guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == event.mapping_id && mapping.enabled)
            .cloned()
    });
    let Some(mapping) = mapping else {
        return;
    };
    let event_paths = collect_watch_event_files(&event.path, &mapping.sync_types);
    if event_paths.is_empty() {
        return;
    }
    if event_paths.len() != 1 || event_paths.first() != Some(&event.path) {
        if let Ok(guard) = state.lock() {
            for path in event_paths {
                let _ = guard.event_tx.send(FsEvent {
                    mapping_id: mapping.id.clone(),
                    path,
                });
            }
        }
        return;
    }
    let Ok(meta) = fs::metadata(&event.path) else {
        return;
    };
    let relative = event
        .path
        .strip_prefix(&mapping.local_path)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let relative_dir = Path::new(&relative)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let remote_dir = [
        if mapping.remote_parent_id.is_empty() {
            normalize_remote_path(&mapping.remote_path)
        } else {
            String::new()
        },
        relative_dir,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("/");
    let auto_share_enabled = mapping.auto_share;
    let organizer_mapping_id = mapping.organizer_mapping_id.clone();
    let mut item = UploadItem {
        mapping_id: mapping.id,
        file_path: event.path.clone(),
        remote_parent_id: mapping.remote_parent_id,
        remote_dir,
        relative_path: relative,
        change_kind: "added".to_string(),
        size: meta.len(),
        modified_ms: modified_ms(&meta),
        replacement: None,
    };
    if let Ok(guard) = state.lock() {
        if upload_already_scheduled(
            &guard.history,
            &guard.pending_cloud,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &guard.cancelled_uploads,
            &item,
        ) {
            return;
        }
    } else {
        return;
    }
    let db_path = match state.lock() {
        Ok(guard) => guard.db_path.clone(),
        Err(_) => return,
    };
    let reused_upload = match reuse_matching_confirmed_upload(&db_path, &item) {
        Ok(value) => value,
        Err(error) => {
            status(app, "warning", error);
            None
        }
    };
    let waiting_for_login = if let Ok(mut guard) = state.lock() {
        let waiting_for_login = guard.token.is_none();
        let key = item_key(&item.mapping_id, &item.file_path);
        if upload_already_scheduled(
            &guard.history,
            &guard.pending_cloud,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &guard.cancelled_uploads,
            &item,
        ) {
            return;
        }
        guard.cancelled_uploads.remove(&key);
        guard.failed_uploads.remove(&key);
        if reused_upload.is_some() {
            guard.history.insert(
                key,
                Stamp {
                    size: item.size,
                    modified_ms: item.modified_ms,
                },
            );
            false
        } else {
            if guard.history.contains_key(&key) {
                item.change_kind = "changed".to_string();
            }
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.queue.push_back(item.clone());
            waiting_for_login
        }
    } else {
        return;
    };
    if let Some((source_mapping_id, outcome)) = reused_upload {
        if !organizer_mapping_id.is_empty() {
            if let Err(error) = organizer::notify_upload(
                app.clone(),
                organizer_mapping_id,
                outcome.remote_file_id.clone().unwrap_or_default(),
                item.relative_path.clone(),
                auto_share_enabled,
            )
            .await
            {
                status(
                    app,
                    "error",
                    format!("历史文件无需重复上传，但上传后整理排队失败；未分享 A 目录：{error}"),
                );
            }
        } else if auto_share_enabled {
            let target = auto_share_target(&item);
            let binding_reused = match target.as_ref() {
                Some(target) => reuse_auto_share_binding(
                    &db_path,
                    &item.mapping_id,
                    &source_mapping_id,
                    &target.key,
                ),
                None => Ok(false),
            };
            match binding_reused {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = schedule_auto_share(app, state, &item, &outcome).await {
                        status(
                            app,
                            "error",
                            format!("历史文件无需重复上传，但自动分享排队失败：{error}"),
                        );
                    }
                }
                Err(error) => status(app, "error", error),
            }
        }
        emit_state(app, state);
        return;
    }
    emit(
        app,
        json!({ "type": "file", "state": if waiting_for_login { "waiting-login" } else { "queued" }, "file_path": event.path.to_string_lossy() }),
    );
    emit_state(app, state);
    drain_queue(app.clone(), state.clone());
}


pub(crate) fn hydrate_pending_item(state: &SharedState, pending: &PendingUpload) -> UploadItem {
    let mut item = pending.item.clone();
    if item.mapping_id.starts_with("__") {
        return item;
    }
    let mapping = state.lock().ok().and_then(|guard| {
        guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == item.mapping_id)
            .cloned()
    });
    let Some(mapping) = mapping else {
        return item;
    };
    let relative = item
        .file_path
        .strip_prefix(&mapping.local_path)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| item.relative_path.clone());
    let relative_dir = Path::new(&relative)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    item.remote_parent_id = mapping.remote_parent_id.clone();
    item.remote_dir = [
        if mapping.remote_parent_id.is_empty() {
            normalize_remote_path(&mapping.remote_path)
        } else {
            String::new()
        },
        relative_dir,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("/");
    item.relative_path = relative;
    item
}

pub(crate) fn queue_rejected_pending_upload(state: &SharedState, pending: &PendingUpload) -> bool {
    let mut item = hydrate_pending_item(state, pending);
    let key = item_key(&item.mapping_id, &item.file_path);
    if let Ok(mut guard) = state.lock() {
        guard.pending_cloud.remove(&key);
    } else {
        return false;
    }
    let Ok(metadata) = fs::metadata(&item.file_path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    item.size = metadata.len();
    item.modified_ms = modified_ms(&metadata);
    let Ok(mut guard) = state.lock() else {
        return false;
    };
    if !item.mapping_id.starts_with("__")
        && !guard
            .mappings
            .iter()
            .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
    {
        return false;
    }
    if guard.cancelled_uploads.contains_key(&key)
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
        return false;
    }
    guard
        .queue
        .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
    guard.queue.push_back(item);
    true
}

pub(crate) async fn recover_pending_upload(app: tauri::AppHandle, state: SharedState, pending: PendingUpload) {
    let key = item_key(&pending.item.mapping_id, &pending.item.file_path);
    let (token, device_id, db_path) = match state.lock() {
        Ok(guard) => {
            let Some(token) = guard.token.clone() else {
                drop(guard);
                if let Ok(mut guard) = state.lock() {
                    guard.recovering_pending.remove(&key);
                }
                return;
            };
            (token, guard.device_id.clone(), guard.db_path.clone())
        }
        Err(_) => return,
    };
    let task_result = abortable_upload_step(
        &state,
        &key,
        check_upload_task(&token, &device_id, &pending.task_id),
    )
    .await;
    let task_result = match task_result {
        Ok(result) => result,
        Err(message) if message == UPLOAD_CANCELLED_MESSAGE => {
            if let Err(error) = clear_cancelled_upload_artifacts(&db_path, &pending.item) {
                status(&app, "warning", error);
            }
            if let Ok(mut guard) = state.lock() {
                guard.recovering_pending.remove(&key);
                guard.pending_cloud.remove(&key);
            }
            emit_state(&app, &state);
            return;
        }
        Err(message) => {
            status(&app, "warning", message);
            return;
        }
    };
    match task_result {
        Ok(CloudTaskCheck::Confirmed(task_data)) => {
            let item = hydrate_pending_item(&state, &pending);
            let outcome = UploadOutcome {
                task_id: pending.task_id.clone(),
                remote_file_id: task_data
                    .get("fileId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            if let Err(message) =
                complete_upload_replacement(&app, &state, &token, &device_id, &item, &outcome).await
            {
                status(
                    &app,
                    "warning",
                    format!("新版本已入库，但安全替换尚未完成；后台将继续恢复：{message}"),
                );
                if let Ok(mut guard) = state.lock() {
                    guard.recovering_pending.remove(&key);
                }
                emit_state(&app, &state);
                return;
            }
            match remember_confirmed_upload(&state, &item, &outcome) {
                Ok(()) => {
                    finalize_successful_upload(&app, &state, &item, &outcome).await;
                }
                Err(message) => status(&app, "warning", message),
            }
        }
        Ok(CloudTaskCheck::Pending) => {}
        Err(CloudConfirmError::Retryable(message)) => {
            if message.contains("登录态已失效") {
                if let Ok(mut guard) = state.lock() {
                    guard.token = None;
                }
                if let Err(error) = clear_persisted_access_token(&db_path) {
                    status(&app, "error", error);
                }
            } else {
                status(
                    &app,
                    "warning",
                    format!("待确认上传将在后台重试：{message}"),
                );
            }
        }
        Err(CloudConfirmError::Permanent(message)) => {
            match delete_pending_upload(&db_path, &pending) {
                Ok(true) => {
                    let queued = queue_rejected_pending_upload(&state, &pending);
                    status(
                        &app,
                        "error",
                        if queued {
                            format!("云端明确拒绝入库，已清理待确认记录并重新排队：{message}")
                        } else {
                            format!("云端明确拒绝入库，已清理待确认记录；源文件当前无法重新排队：{message}")
                        },
                    );
                    if queued {
                        drain_queue(app.clone(), state.clone());
                    }
                }
                Ok(false) => {}
                Err(error) => status(&app, "error", error),
            }
        }
    }
    if let Ok(mut guard) = state.lock() {
        guard.recovering_pending.remove(&key);
    }
    emit_state(&app, &state);
}

pub(crate) async fn pending_upload_recovery_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(PENDING_UPLOAD_RETRY_SECS)).await;
        let (db_path, can_recover) = match state.lock() {
            Ok(guard) => (guard.db_path.clone(), guard.token.is_some()),
            Err(_) => continue,
        };
        if !can_recover {
            continue;
        }
        let pending_uploads = match load_pending_uploads(&db_path) {
            Ok(pending) => pending,
            Err(message) => {
                status(&app, "error", message);
                continue;
            }
        };
        for pending in pending_uploads {
            let key = item_key(&pending.item.mapping_id, &pending.item.file_path);
            let should_start = state.lock().ok().is_some_and(|mut guard| {
                !guard.inflight.contains_key(&key) && guard.recovering_pending.insert(key.clone())
            });
            if should_start {
                tauri::async_runtime::spawn(recover_pending_upload(
                    app.clone(),
                    state.clone(),
                    pending,
                ));
            }
        }
    }
}


const MAX_REPORTED_SCAN_SKIPS: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct QueueUploadPathsResult {
    pub queued: usize,
    pub skipped: usize,
    pub skips: Vec<UploadScanSkip>,
}

pub(crate) fn collect_manual_uploads(
    path: &Path,
    remote_prefix: &str,
    files: &mut Vec<(PathBuf, String)>,
    skips: &mut Vec<UploadScanSkip>,
    visited: &mut HashSet<PathIdentity>,
) {
    let Some((kind, readable)) = inspect_local_entry(path, skips) else {
        return;
    };
    match kind {
        LocalEntryKind::File => {
            if ignored(path) {
                skips.push(UploadScanSkip::new(path, "临时或下载中的文件"));
                return;
            }
            files.push((
                user_visible_path(path),
                normalize_remote_path(remote_prefix),
            ));
        }
        LocalEntryKind::Directory => {
            if !visited.insert(path_identity(&readable)) {
                skips.push(UploadScanSkip::new(path, "检测到循环链接，已跳过"));
                return;
            }
            let Some(folder_name) = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .filter(|value| !value.is_empty())
            else {
                skips.push(UploadScanSkip::new(path, "无法读取目录名"));
                return;
            };
            let folder_prefix = [remote_prefix, folder_name.as_str()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            let entries = match fs::read_dir(&readable) {
                Ok(entries) => entries,
                Err(error) => {
                    skips.push(UploadScanSkip::new(
                        path,
                        format!("读取目录失败：{error}"),
                    ));
                    return;
                }
            };
            for entry in entries {
                match entry {
                    Ok(entry) => collect_manual_uploads(
                        &entry.path(),
                        &folder_prefix,
                        files,
                        skips,
                        visited,
                    ),
                    Err(error) => skips.push(UploadScanSkip::new(
                        path,
                        format!("读取目录项失败：{error}"),
                    )),
                }
            }
        }
    }
}


#[tauri::command]
pub(crate) fn select_upload_files() -> Vec<String> {
    rfd::FileDialog::new()
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

#[tauri::command]
pub(crate) fn select_upload_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub(crate) fn queue_upload_paths(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    paths: Vec<String>,
    parent_id: String,
) -> Result<QueueUploadPathsResult, String> {
    if paths.is_empty() {
        return Err("没有选择需要上传的文件".into());
    }
    if state.lock().map_err(|e| e.to_string())?.token.is_none() {
        return Err("请先登录光鸭云盘".into());
    }
    let mut files = Vec::new();
    let mut skips = Vec::new();
    let mut visited = HashSet::new();
    for input in paths {
        let path = PathBuf::from(input);
        if !path.exists() && !extended_length_path(&path).exists() {
            return Err(format!("本地路径不存在：{}", path.display()));
        }
        collect_manual_uploads(&path, "", &mut files, &mut skips, &mut visited);
    }
    if files.is_empty() {
        if skips.is_empty() {
            return Err("选中的路径中没有可上传文件".into());
        }
        let preview = skips
            .iter()
            .take(3)
            .map(|skip| format!("{}：{}", skip.path, skip.reason))
            .collect::<Vec<_>>()
            .join("；");
        return Err(format!(
            "选中的路径中没有可上传文件；已跳过 {} 个路径。{preview}",
            skips.len()
        ));
    }
    let mut count = 0usize;
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        for (path, remote_dir) in files {
            let metadata = fs::metadata(readable_fs_path(&path)).map_err(|e| e.to_string())?;
            let item = UploadItem {
                mapping_id: "__manual__".into(),
                file_path: path,
                remote_parent_id: parent_id.clone(),
                remote_dir,
                relative_path: String::new(),
                change_kind: "added".to_string(),
                size: metadata.len(),
                modified_ms: modified_ms(&metadata),
                replacement: None,
            };
            let key = item_key(&item.mapping_id, &item.file_path);
            guard.cancelled_uploads.remove(&key);
            guard.failed_uploads.remove(&key);
            if guard
                .inflight
                .get(&key)
                .is_some_and(|stamp| stamp_matches(&item, stamp))
                || guard.queue.iter().any(|queued| {
                    item_key(&queued.mapping_id, &queued.file_path) == key
                        && queued.size == item.size
                        && queued.modified_ms == item.modified_ms
                })
            {
                continue;
            }
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.queue.push_back(item);
            count += 1;
        }
    }
    let skipped = skips.len();
    if skipped > 0 {
        let preview = skips
            .iter()
            .take(3)
            .map(|skip| format!("{}：{}", skip.path, skip.reason))
            .collect::<Vec<_>>()
            .join("；");
        status(
            &app,
            "warning",
            format!("扫描时跳过 {skipped} 个路径，未加入上传队列。{preview}"),
        );
    }
    if count == 0 {
        return Ok(QueueUploadPathsResult {
            queued: 0,
            skipped,
            skips: skips.into_iter().take(MAX_REPORTED_SCAN_SKIPS).collect(),
        });
    }
    status(&app, "info", format!("已加入上传队列：{count} 个文件"));
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(QueueUploadPathsResult {
        queued: count,
        skipped,
        skips: skips.into_iter().take(MAX_REPORTED_SCAN_SKIPS).collect(),
    })
}


#[tauri::command]
pub(crate) fn pause_queue(app: tauri::AppHandle, state: tauri::State<'_, SharedState>) {
    if let Ok(mut guard) = state.lock() {
        guard.paused = true;
        let active_keys = guard.inflight_items.keys().cloned().collect::<Vec<_>>();
        guard.queue_pause_requests.extend(active_keys);
    }
    emit_state(&app, state.inner());
}

#[tauri::command]
pub(crate) fn pause_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_path: String,
    mapping_id: Option<String>,
) -> Result<bool, String> {
    let target_path = file_path.trim();
    if target_path.is_empty() {
        return Err("缺少要暂停的上传路径".into());
    }
    let target_mapping = mapping_id.unwrap_or_default();
    let (db_path, matched, inflight_keys) = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        let matches = |item: &UploadItem| {
            item.file_path.to_string_lossy() == target_path
                && (target_mapping.is_empty() || item.mapping_id == target_mapping)
        };
        let mut matched = HashMap::<String, UploadItem>::new();
        for item in guard
            .queue
            .iter()
            .chain(guard.waiting_files.values())
            .chain(guard.inflight_items.values())
        {
            if matches(item) {
                matched.insert(item_key(&item.mapping_id, &item.file_path), item.clone());
            }
        }
        if matched.is_empty() {
            return Ok(false);
        }
        let inflight_keys = matched
            .keys()
            .filter(|key| guard.inflight_items.contains_key(*key))
            .cloned()
            .collect::<HashSet<_>>();
        guard.paused_uploads.extend(matched.keys().cloned());
        (guard.db_path.clone(), matched, inflight_keys)
    };
    for (key, item) in &matched {
        let uploaded_bytes = load_upload_checkpoint(&db_path, item)?
            .map(|checkpoint| checkpoint.uploaded_bytes)
            .unwrap_or(0);
        let inflight = inflight_keys.contains(key);
        emit(
            &app,
            json!({
                "type": "file",
                "state": if inflight { "pausing" } else { "paused" },
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": uploaded_bytes,
                "total_bytes": item.size,
                "stage": if inflight { "正在暂停并保存上传断点" } else { "已暂停，可从当前断点继续" }
            }),
        );
    }
    emit_state(&app, state.inner());
    Ok(true)
}

#[tauri::command]
pub(crate) fn resume_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_path: String,
    mapping_id: Option<String>,
) -> Result<bool, String> {
    let target_path = file_path.trim();
    if target_path.is_empty() {
        return Err("缺少要继续的上传路径".into());
    }
    let target_mapping = mapping_id.unwrap_or_default();
    let (matched, waiting_keys, waiting_for_login) = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        if guard.paused {
            return Err("上传队列已暂停，请先恢复队列".into());
        }
        let matches = |item: &UploadItem| {
            item.file_path.to_string_lossy() == target_path
                && (target_mapping.is_empty() || item.mapping_id == target_mapping)
        };
        let mut matched = HashMap::<String, UploadItem>::new();
        for item in guard
            .queue
            .iter()
            .chain(guard.waiting_files.values())
            .chain(guard.inflight_items.values())
        {
            let key = item_key(&item.mapping_id, &item.file_path);
            if guard.paused_uploads.contains(&key) && matches(item) {
                matched.insert(key, item.clone());
            }
        }
        if matched.is_empty() {
            return Ok(false);
        }
        if matched
            .keys()
            .any(|key| guard.inflight_items.contains_key(key))
        {
            return Err("上传任务正在进入暂停状态，请稍后继续".into());
        }
        let waiting_keys = matched
            .keys()
            .filter(|key| guard.waiting_files.contains_key(*key))
            .cloned()
            .collect::<HashSet<_>>();
        for key in matched.keys() {
            guard.paused_uploads.remove(key);
        }
        (matched, waiting_keys, guard.token.is_none())
    };
    for (key, item) in &matched {
        let (state_name, stage) = if waiting_keys.contains(key) {
            ("waiting-file", "另外的程序正在使用该文件，释放后将自动上传")
        } else if waiting_for_login {
            ("waiting-login", "等待登录后继续上传")
        } else {
            ("queued", "已恢复，等待上传通道")
        };
        emit(
            &app,
            json!({
                "type": "file",
                "state": state_name,
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "stage": stage
            }),
        );
    }
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(true)
}

#[tauri::command]
pub(crate) fn cancel_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_path: String,
    mapping_id: Option<String>,
) -> Result<bool, String> {
    let target_path = file_path.trim();
    if target_path.is_empty() {
        return Err("缺少要取消的上传路径".into());
    }
    let target_mapping = mapping_id.unwrap_or_default();
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let pending = load_pending_uploads(&db_path)?;
    let matches = |item: &UploadItem| {
        item.file_path.to_string_lossy() == target_path
            && (target_mapping.is_empty() || item.mapping_id == target_mapping)
    };
    let mut matched = HashMap::<String, UploadItem>::new();
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        for item in guard
            .queue
            .iter()
            .chain(guard.waiting_files.values())
            .chain(guard.inflight_items.values())
            .chain(guard.failed_uploads.values())
        {
            if matches(item) {
                matched.insert(item_key(&item.mapping_id, &item.file_path), item.clone());
            }
        }
        for value in &pending {
            if matches(&value.item) {
                matched.insert(
                    item_key(&value.item.mapping_id, &value.item.file_path),
                    value.item.clone(),
                );
            }
        }
        if matched
            .keys()
            .any(|key| guard.active_upload_replacements.contains(key))
        {
            return Err("新版本已经入库，正在安全替换旧文件，此阶段不能取消".into());
        }
        for (key, item) in &matched {
            guard.cancelled_uploads.insert(
                key.clone(),
                Stamp {
                    size: item.size,
                    modified_ms: item.modified_ms,
                },
            );
            guard.waiting_files.remove(key);
            guard.failed_uploads.remove(key);
            guard.flash_preflight_cache.remove(key);
            guard.pending_cloud.remove(key);
            guard.recovering_pending.remove(key);
            guard.paused_uploads.remove(key);
            guard.queue_pause_requests.remove(key);
        }
        guard
            .queue
            .retain(|item| !matched.contains_key(&item_key(&item.mapping_id, &item.file_path)));
    }
    if matched.is_empty() {
        return Ok(false);
    }
    for item in matched.values() {
        clear_cancelled_upload_artifacts(&db_path, item)?;
        emit(
            &app,
            json!({
                "type": "file",
                "state": "cancelled",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "stage": "已取消"
            }),
        );
    }
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(true)
}

#[tauri::command]
pub(crate) fn retry_upload(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_path: String,
    mapping_id: Option<String>,
) -> Result<bool, String> {
    let target_path = file_path.trim();
    if target_path.is_empty() {
        return Err("缺少要重试的上传路径".into());
    }
    let target_mapping = mapping_id.unwrap_or_default();
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let matched = guard
        .failed_uploads
        .iter()
        .find(|(_, item)| {
            item.file_path.to_string_lossy() == target_path
                && (target_mapping.is_empty() || item.mapping_id == target_mapping)
        })
        .map(|(key, item)| (key.clone(), item.clone()));
    let Some((key, mut item)) = matched else {
        return Err("失败的上传任务已失效，请重新选择文件上传".into());
    };
    let metadata = fs::metadata(&item.file_path)
        .map_err(|_| "本地源文件已不存在，请重新选择文件上传".to_string())?;
    if !metadata.is_file() {
        return Err("本地源文件已不存在，请重新选择文件上传".into());
    }
    item.size = metadata.len();
    item.modified_ms = modified_ms(&metadata);
    guard.failed_uploads.remove(&key);
    guard.cancelled_uploads.remove(&key);
    guard.paused_uploads.remove(&key);
    guard.queue_pause_requests.remove(&key);
    guard.waiting_files.remove(&key);
    guard.flash_preflight_cache.remove(&key);
    guard
        .queue
        .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
    guard.queue.push_back(item.clone());
    let waiting_for_login = guard.token.is_none();
    drop(guard);
    emit(
        &app,
        json!({
            "type": "file",
            "state": if waiting_for_login { "waiting-login" } else { "queued" },
            "file_path": item.file_path.to_string_lossy(),
            "mapping_id": item.mapping_id,
            "uploaded_bytes": 0,
            "total_bytes": item.size,
            "stage": if waiting_for_login { "等待登录后重试" } else { "已重新加入上传队列" }
        }),
    );
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(true)
}


#[tauri::command]
pub(crate) async fn resume_queue(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        guard.paused = false;
    }
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(())
}
