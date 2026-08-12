//! 上传持久化：历史、断点、待确认任务。

use crate::prelude::*;

#[derive(Debug, Clone)]
pub(crate) struct PersistedUploadCheckpoint {
    pub(crate) checkpoint: OssUploadCheckpoint,
    pub(crate) uploaded_bytes: u64,
}


pub(crate) fn load_upload_history(path: &Path) -> Result<HashMap<String, Stamp>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, file_path, size, modified_ms FROM uploaded_files
             WHERE upload_state = ?1",
        )
        .map_err(|e| format!("读取上传记录失败：{e}"))?;
    let rows = statement
        .query_map(params![UPLOAD_STATE_CLOUD_CONFIRMED], |row| {
            let mapping_id: String = row.get(0)?;
            let file_path: String = row.get(1)?;
            let size: u64 = row.get(2)?;
            let modified_raw: String = row.get(3)?;
            Ok((mapping_id, file_path, size, modified_raw))
        })
        .map_err(|e| format!("查询上传记录失败：{e}"))?;
    let mut history = HashMap::new();
    for row in rows {
        let (mapping_id, file_path, size, modified_raw) =
            row.map_err(|e| format!("解析上传记录失败：{e}"))?;
        let modified_ms = modified_raw.parse::<u128>().unwrap_or(0);
        history.insert(
            item_key(&mapping_id, Path::new(&file_path)),
            Stamp { size, modified_ms },
        );
    }
    Ok(history)
}

pub(crate) fn reuse_matching_confirmed_upload(
    path: &Path,
    item: &UploadItem,
) -> Result<Option<(String, UploadOutcome)>, String> {
    let connection = open_database(path)?;
    let matched = connection
        .query_row(
            "SELECT mapping_id, task_id, remote_file_id
             FROM uploaded_files
             WHERE upload_state = ?1
               AND substr(mapping_id, 1, 2) <> '__'
               AND file_path = ?2
               AND size = ?3
               AND modified_ms = ?4
               AND remote_parent_id = ?5
               AND remote_dir = ?6
               AND relative_path = ?7
             ORDER BY uploaded_at DESC
             LIMIT 1",
            params![
                UPLOAD_STATE_CLOUD_CONFIRMED,
                item.file_path.to_string_lossy().as_ref(),
                item.size,
                item.modified_ms.to_string(),
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    UploadOutcome {
                        task_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        remote_file_id: row.get(2)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|error| format!("复用历史上传记录失败：{error}"))?;
    drop(connection);
    if let Some((_, outcome)) = matched.as_ref() {
        save_upload_record(path, item, outcome, UPLOAD_STATE_CLOUD_CONFIRMED)?;
    }
    Ok(matched)
}

pub(crate) fn load_pending_uploads(path: &Path) -> Result<Vec<PendingUpload>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, file_path, size, modified_ms, task_id,
                    remote_parent_id, remote_dir, relative_path, change_kind, replacement_json
             FROM uploaded_files
             WHERE upload_state = ?1 AND task_id IS NOT NULL AND TRIM(task_id) <> ''",
        )
        .map_err(|e| format!("读取待确认上传记录失败：{e}"))?;
    let rows = statement
        .query_map(params![UPLOAD_STATE_OSS_COMPLETE], |row| {
            let modified_raw: String = row.get(3)?;
            Ok(PendingUpload {
                item: UploadItem {
                    mapping_id: row.get(0)?,
                    file_path: PathBuf::from(row.get::<_, String>(1)?),
                    size: row.get(2)?,
                    modified_ms: modified_raw.parse::<u128>().unwrap_or(0),
                    remote_parent_id: row.get(5)?,
                    remote_dir: row.get(6)?,
                    relative_path: row.get(7)?,
                    change_kind: row.get(8)?,
                    replacement: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|value| serde_json::from_str(&value).ok()),
                },
                task_id: row.get(4)?,
            })
        })
        .map_err(|e| format!("查询待确认上传记录失败：{e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析待确认上传记录失败：{e}"))
}

pub(crate) fn pending_upload_stamps(path: &Path) -> Result<HashMap<String, Stamp>, String> {
    Ok(load_pending_uploads(path)?
        .into_iter()
        .map(|pending| {
            (
                item_key(&pending.item.mapping_id, &pending.item.file_path),
                Stamp {
                    size: pending.item.size,
                    modified_ms: pending.item.modified_ms,
                },
            )
        })
        .collect())
}

pub(crate) fn clear_upload_checkpoint(path: &Path, item: &UploadItem) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM upload_checkpoints WHERE mapping_id = ?1 AND file_path = ?2",
            params![item.mapping_id, item.file_path.to_string_lossy()],
        )
        .map_err(|error| format!("清除上传断点失败：{error}"))?;
    Ok(())
}

pub(crate) fn load_upload_checkpoint(
    path: &Path,
    item: &UploadItem,
) -> Result<Option<PersistedUploadCheckpoint>, String> {
    let connection = open_database(path)?;
    let row = connection
        .query_row(
            "SELECT size, modified_ms, checkpoint_json, uploaded_bytes
             FROM upload_checkpoints
             WHERE mapping_id = ?1 AND file_path = ?2",
            params![item.mapping_id, item.file_path.to_string_lossy()],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取上传断点失败：{error}"))?;
    let Some((size, modified_ms, checkpoint_json, uploaded_bytes)) = row else {
        return Ok(None);
    };
    if size != item.size || modified_ms != item.modified_ms.to_string() {
        clear_upload_checkpoint(path, item)?;
        return Ok(None);
    }
    let checkpoint = match serde_json::from_str::<OssUploadCheckpoint>(&checkpoint_json) {
        Ok(value) => value,
        Err(_) => {
            clear_upload_checkpoint(path, item)?;
            return Ok(None);
        }
    };
    Ok(Some(PersistedUploadCheckpoint {
        checkpoint,
        uploaded_bytes: uploaded_bytes.min(item.size),
    }))
}

pub(crate) fn save_upload_checkpoint(
    path: &Path,
    item: &UploadItem,
    checkpoint: &OssUploadCheckpoint,
    uploaded_bytes: u64,
) -> Result<(), String> {
    let connection = open_database(path)?;
    let item_json =
        serde_json::to_string(item).map_err(|error| format!("序列化上传任务失败：{error}"))?;
    let checkpoint_json = serde_json::to_string(checkpoint)
        .map_err(|error| format!("序列化上传断点失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO upload_checkpoints
               (mapping_id, file_path, size, modified_ms, item_json, checkpoint_json,
                uploaded_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(mapping_id, file_path) DO UPDATE SET
               size = excluded.size,
               modified_ms = excluded.modified_ms,
               item_json = excluded.item_json,
               checkpoint_json = excluded.checkpoint_json,
               uploaded_bytes = excluded.uploaded_bytes,
               updated_at = excluded.updated_at",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.size,
                item.modified_ms.to_string(),
                item_json,
                checkpoint_json,
                uploaded_bytes.min(item.size),
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存上传断点失败：{error}"))?;
    Ok(())
}

pub(crate) fn load_resumable_uploads(path: &Path) -> Result<VecDeque<UploadItem>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, file_path, size, modified_ms, item_json
             FROM upload_checkpoints ORDER BY updated_at",
        )
        .map_err(|error| format!("读取待续传任务失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| format!("查询待续传任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析待续传任务失败：{error}"))?;
    drop(statement);
    let mut restored = VecDeque::new();
    for (mapping_id, file_path, size, modified_ms_value, item_json) in rows {
        let parsed = serde_json::from_str::<UploadItem>(&item_json).ok();
        let valid = parsed.as_ref().is_some_and(|item| {
            item.mapping_id == mapping_id
                && item.file_path == PathBuf::from(&file_path)
                && item.size == size
                && item.modified_ms.to_string() == modified_ms_value
                && fs::metadata(&item.file_path).ok().is_some_and(|metadata| {
                    metadata.is_file()
                        && metadata.len() == size
                        && modified_ms(&metadata) == item.modified_ms
                })
        });
        if valid {
            restored.push_back(parsed.expect("checked resumable upload item"));
        } else {
            connection
                .execute(
                    "DELETE FROM upload_checkpoints WHERE mapping_id = ?1 AND file_path = ?2",
                    params![mapping_id, file_path],
                )
                .map_err(|error| format!("清理失效上传断点失败：{error}"))?;
        }
    }
    Ok(restored)
}

pub(crate) fn save_upload_record(
    path: &Path,
    item: &UploadItem,
    outcome: &UploadOutcome,
    upload_state: &str,
) -> Result<(), String> {
    let connection = open_database(path)?;
    let replacement_json = item
        .replacement
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| format!("序列化安全替换状态失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO uploaded_files
               (mapping_id, file_path, size, modified_ms, task_id, remote_file_id,
                upload_state, remote_parent_id, remote_dir, relative_path, change_kind,
                replacement_json, uploaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(mapping_id, file_path) DO UPDATE SET
               size = excluded.size,
               modified_ms = excluded.modified_ms,
               task_id = excluded.task_id,
               remote_file_id = excluded.remote_file_id,
               upload_state = excluded.upload_state,
               remote_parent_id = excluded.remote_parent_id,
               remote_dir = excluded.remote_dir,
               relative_path = excluded.relative_path,
               change_kind = excluded.change_kind,
               replacement_json = excluded.replacement_json,
               uploaded_at = excluded.uploaded_at",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.size,
                item.modified_ms.to_string(),
                outcome.task_id,
                outcome.remote_file_id,
                upload_state,
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path,
                item.change_kind,
                replacement_json,
                unix_timestamp()
            ],
        )
        .map_err(|e| format!("保存上传记录失败：{e}"))?;
    Ok(())
}

pub(crate) fn remember_pending_upload(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let database = state.lock().map_err(|e| e.to_string())?.db_path.clone();
    save_upload_record(&database, item, outcome, UPLOAD_STATE_OSS_COMPLETE)?;
    state
        .lock()
        .map_err(|e| e.to_string())?
        .pending_cloud
        .insert(
            item_key(&item.mapping_id, &item.file_path),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
    Ok(())
}

pub(crate) fn confirm_pending_record(
    database: &Path,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<bool, String> {
    open_database(database)?
        .execute(
            "UPDATE uploaded_files SET remote_file_id = ?1, upload_state = ?2,
                    remote_parent_id = ?3, remote_dir = ?4, relative_path = ?5,
                    change_kind = ?6, replacement_json = NULL, uploaded_at = ?7
             WHERE mapping_id = ?8 AND file_path = ?9 AND task_id = ?10
               AND upload_state = ?11",
            params![
                outcome.remote_file_id,
                UPLOAD_STATE_CLOUD_CONFIRMED,
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path,
                item.change_kind,
                unix_timestamp(),
                item.mapping_id,
                item.file_path.to_string_lossy(),
                outcome.task_id,
                UPLOAD_STATE_OSS_COMPLETE
            ],
        )
        .map(|changed| changed > 0)
        .map_err(|e| format!("更新云端确认状态失败：{e}"))
}

pub(crate) fn remember_confirmed_upload(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let database = state.lock().map_err(|e| e.to_string())?.db_path.clone();
    if !confirm_pending_record(&database, item, outcome)? {
        return Err("待确认上传记录已被移除或已由其他任务更新".into());
    }
    let key = item_key(&item.mapping_id, &item.file_path);
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.pending_cloud.remove(&key);
    guard.history.insert(
        key,
        Stamp {
            size: item.size,
            modified_ms: item.modified_ms,
        },
    );
    Ok(())
}

pub(crate) fn delete_pending_upload(path: &Path, pending: &PendingUpload) -> Result<bool, String> {
    let connection = open_database(path)?;
    let changed = if let Some(replacement) = pending.item.replacement.as_ref() {
        connection.execute(
            "UPDATE uploaded_files
             SET size = ?1, modified_ms = ?2, task_id = NULL, remote_file_id = ?3,
                 upload_state = ?4, replacement_json = NULL, uploaded_at = ?5
             WHERE mapping_id = ?6 AND file_path = ?7 AND task_id = ?8 AND upload_state = ?9",
            params![
                replacement.previous_size,
                replacement.previous_modified_ms.to_string(),
                replacement.old_file_id,
                UPLOAD_STATE_CLOUD_CONFIRMED,
                unix_timestamp(),
                pending.item.mapping_id,
                pending.item.file_path.to_string_lossy(),
                pending.task_id,
                UPLOAD_STATE_OSS_COMPLETE
            ],
        )
    } else {
        connection.execute(
            "DELETE FROM uploaded_files
             WHERE mapping_id = ?1 AND file_path = ?2 AND task_id = ?3 AND upload_state = ?4",
            params![
                pending.item.mapping_id,
                pending.item.file_path.to_string_lossy(),
                pending.task_id,
                UPLOAD_STATE_OSS_COMPLETE
            ],
        )
    };
    changed
        .map(|changed| changed > 0)
        .map_err(|e| format!("清理待确认上传记录失败：{e}"))
}

pub(crate) fn clear_cancelled_upload_artifacts(path: &Path, item: &UploadItem) -> Result<(), String> {
    clear_upload_checkpoint(path, item)?;
    let key = item_key(&item.mapping_id, &item.file_path);
    for pending in load_pending_uploads(path)? {
        if item_key(&pending.item.mapping_id, &pending.item.file_path) == key {
            delete_pending_upload(path, &pending)?;
        }
    }
    Ok(())
}

pub(crate) fn remove_mapping_transient_uploads(path: &Path, mapping_id: &str) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM uploaded_files
             WHERE mapping_id = ?1 AND upload_state <> ?2",
            params![mapping_id, UPLOAD_STATE_CLOUD_CONFIRMED],
        )
        .map_err(|e| format!("清理任务待确认上传记录失败：{e}"))?;
    Ok(())
}
