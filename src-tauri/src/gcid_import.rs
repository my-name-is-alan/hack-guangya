//! GCID 秒传导入：解析、入库、并发执行。

use crate::prelude::*;

#[derive(Debug, Clone)]
pub(crate) struct GcidImportFile {
    pub(crate) path: String,
    pub(crate) folder_path: String,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) gcid: String,
    pub(crate) cid: String,
    pub(crate) attempts: i64,
}

#[derive(Debug)]
pub(crate) enum GcidImportOutcome {
    Imported { task_id: String, file_id: String },
    Existing { file_id: String },
    Missed { task_id: String },
    Conflict(String),
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GcidImportSourceInfo {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) size: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct GcidImportCounts {
    pub(crate) pending: u64,
    pub(crate) processing: u64,
    pub(crate) imported: u64,
    pub(crate) existing: u64,
    pub(crate) missed: u64,
    pub(crate) conflict: u64,
    pub(crate) failed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GcidImportStatus {
    pub(crate) job_id: String,
    pub(crate) source_path: String,
    pub(crate) source_name: String,
    pub(crate) destination_parent_id: String,
    pub(crate) destination_name: String,
    pub(crate) total_files: u64,
    pub(crate) total_size: String,
    pub(crate) status: String,
    pub(crate) current_path: String,
    pub(crate) error: Option<String>,
    pub(crate) counts: GcidImportCounts,
    pub(crate) finished: u64,
    pub(crate) updated_at: i64,
}


pub(crate) fn parse_gcid_file_size(value: &Value) -> Result<u64, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .filter(|size| *size > 0)
            .ok_or_else(|| "文件大小必须是正整数".to_string()),
        Value::String(value) => value
            .parse::<u64>()
            .ok()
            .filter(|size| *size > 0)
            .ok_or_else(|| "文件大小必须是正整数字符串".to_string()),
        _ => Err("文件大小格式无效".to_string()),
    }
}

pub(crate) fn normalize_gcid_relative_path(value: &str) -> Result<String, String> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized
            .as_bytes()
            .get(1)
            .is_some_and(|value| *value == b':')
    {
        return Err(format!("不是合法的相对路径：{value}"));
    }
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!("路径包含空目录或越界片段：{value}"));
    }
    if parts.iter().any(|part| part.chars().any(char::is_control)) {
        return Err(format!("路径包含控制字符：{value}"));
    }
    Ok(parts.join("/"))
}

pub(crate) fn parse_gcid_export(raw: &[u8]) -> Result<(Vec<GcidImportFile>, u128, String), String> {
    let export: GcidExport =
        serde_json::from_slice(raw).map_err(|error| format!("JSON 格式无效：{error}"))?;
    if export.source != "guangya"
        || export.hash_type != "gcid"
        || !export.uses_gcid_in_export
        || !export.uses_cid_in_export
    {
        return Err("只支持同时包含 GCID 与 CID 的光鸭导出格式".to_string());
    }
    if export.files.is_empty() {
        return Err("导入文件不包含 files 记录".to_string());
    }
    if export
        .total_files_count
        .is_some_and(|total| total != export.files.len() as u64)
    {
        return Err(format!(
            "文件总数不一致：声明 {}，实际 {}",
            export.total_files_count.unwrap_or_default(),
            export.files.len()
        ));
    }
    let mut seen = HashSet::with_capacity(export.files.len());
    let mut total_size = 0_u128;
    let mut files = Vec::with_capacity(export.files.len());
    for (index, item) in export.files.into_iter().enumerate() {
        let relative_path = normalize_gcid_relative_path(&item.path)
            .map_err(|error| format!("第 {} 条记录：{error}", index + 1))?;
        if !seen.insert(relative_path.clone()) {
            return Err(format!("存在重复路径：{relative_path}"));
        }
        let size = parse_gcid_file_size(&item.size)
            .map_err(|error| format!("第 {} 条记录：{error}", index + 1))?;
        let gcid = item.gcid.to_ascii_uppercase();
        if gcid.len() != 40 || !gcid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("第 {} 条记录的 GCID 无效", index + 1));
        }
        let cid = item.cid.to_ascii_uppercase();
        if !valid_sha1_hex(&cid) {
            return Err(format!("第 {} 条记录的 CID 无效", index + 1));
        }
        let (folder_path, name) = relative_path
            .rsplit_once('/')
            .map(|(folder, name)| (folder.to_string(), name.to_string()))
            .unwrap_or_else(|| (String::new(), relative_path.clone()));
        total_size = total_size
            .checked_add(size as u128)
            .ok_or_else(|| "导入文件总大小溢出".to_string())?;
        files.push(GcidImportFile {
            path: relative_path,
            folder_path,
            name,
            size,
            gcid,
            cid,
            attempts: 0,
        });
    }
    if let Some(declared) = export.total_size.as_ref() {
        let declared = match declared {
            Value::Number(number) => number.as_u64().map(u128::from),
            Value::String(value) => value.parse::<u128>().ok(),
            _ => None,
        };
        if declared.is_some_and(|declared| declared != total_size) {
            return Err(format!(
                "文件总大小不一致：声明 {}，实际 {total_size}",
                declared.unwrap_or_default()
            ));
        }
    }
    Ok((files, total_size, export.common_path))
}

pub(crate) fn validate_gcid_destination(value: &str) -> Result<String, String> {
    let destination = value.trim();
    if destination.is_empty()
        || destination == "."
        || destination == ".."
        || destination.contains('/')
        || destination.contains('\\')
        || destination.chars().any(char::is_control)
    {
        return Err("目标文件夹名称不能为空，也不能包含斜杠、控制字符或越界片段".to_string());
    }
    Ok(destination.to_string())
}

pub(crate) fn gcid_import_job_id(raw: &[u8], destination_parent_id: &str, destination_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    hasher.update(b"\0");
    hasher.update(destination_parent_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(destination_name.as_bytes());
    hex::encode(hasher.finalize())[..32].to_string()
}

pub(crate) fn prepare_gcid_import_database(
    database_path: &Path,
    raw: &[u8],
    source_path: &Path,
    destination_parent_id: &str,
    destination_name: &str,
) -> Result<String, String> {
    let destination_name = validate_gcid_destination(destination_name)?;
    let (files, total_size, _) = parse_gcid_export(raw)?;
    let job_id = gcid_import_job_id(raw, destination_parent_id, &destination_name);
    let source_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("GCID 导入.json")
        .to_string();
    let source_path = source_path.to_string_lossy().to_string();
    let now = unix_timestamp();
    let mut connection = open_database(database_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始导入事务失败：{error}"))?;
    transaction
        .execute(
            "INSERT INTO gcid_import_jobs
               (job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ready', '', NULL, ?8, ?8)
             ON CONFLICT(job_id) DO UPDATE SET
               source_path = excluded.source_path,
               source_name = excluded.source_name,
               destination_parent_id = excluded.destination_parent_id,
               destination_name = excluded.destination_name,
               total_files = excluded.total_files,
               total_size = excluded.total_size,
               status = 'ready',
               current_path = '',
               error = NULL,
               updated_at = excluded.updated_at",
            params![
                job_id,
                source_path,
                source_name,
                destination_parent_id,
                destination_name,
                files.len() as i64,
                total_size.to_string(),
                now
            ],
        )
        .map_err(|error| format!("保存导入任务失败：{error}"))?;
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO gcid_import_files
                   (job_id, path, folder_path, file_name, file_size, gcid, cid,
                    status, attempts, task_id, file_id, error, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, NULL, NULL, NULL, ?8)
                 ON CONFLICT(job_id, path) DO UPDATE SET
                   folder_path = excluded.folder_path,
                   file_name = excluded.file_name,
                   file_size = excluded.file_size,
                   gcid = excluded.gcid,
                   cid = excluded.cid,
                   status = 'pending',
                   attempts = 0,
                   task_id = NULL,
                   file_id = NULL,
                   error = NULL,
                   updated_at = excluded.updated_at",
            )
            .map_err(|error| format!("准备导入记录失败：{error}"))?;
        for file in files {
            let size = i64::try_from(file.size).map_err(|_| format!("文件过大：{}", file.path))?;
            insert
                .execute(params![
                    job_id,
                    file.path,
                    file.folder_path,
                    file.name,
                    size,
                    file.gcid,
                    file.cid,
                    now
                ])
                .map_err(|error| format!("保存导入记录失败：{error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("提交导入任务失败：{error}"))?;
    Ok(job_id)
}

pub(crate) fn load_gcid_import_counts(
    connection: &Connection,
    job_id: &str,
) -> Result<GcidImportCounts, String> {
    let mut counts = GcidImportCounts::default();
    let mut statement = connection
        .prepare(
            "SELECT status, COUNT(*)
             FROM gcid_import_files
             WHERE job_id = ?1
             GROUP BY status",
        )
        .map_err(|error| format!("读取导入统计失败：{error}"))?;
    let rows = statement
        .query_map(params![job_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|error| format!("查询导入统计失败：{error}"))?;
    for row in rows {
        let (status, count) = row.map_err(|error| format!("解析导入统计失败：{error}"))?;
        match status.as_str() {
            "pending" => counts.pending = count,
            "processing" => counts.processing = count,
            "imported" => counts.imported = count,
            "existing" => counts.existing = count,
            "missed" => counts.missed = count,
            "conflict" => counts.conflict = count,
            "failed" => counts.failed = count,
            _ => {}
        }
    }
    Ok(counts)
}

pub(crate) fn gcid_import_has_retryable_work(counts: &GcidImportCounts) -> bool {
    counts.pending > 0
        || counts.processing > 0
        || counts.failed > 0
        || counts.missed > 0
        || counts.conflict > 0
}

pub(crate) fn gcid_import_is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "completed_with_errors")
}

pub(crate) fn reset_all_gcid_import_files(database_path: &Path, job_id: &str) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET status = 'pending', attempts = 0, task_id = NULL, file_id = NULL,
                 error = NULL, updated_at = ?2
             WHERE job_id = ?1",
            params![job_id, unix_timestamp()],
        )
        .map_err(|error| format!("重置导入记录失败：{error}"))?;
    Ok(())
}

pub(crate) fn reset_retryable_gcid_import_files(database_path: &Path, job_id: &str) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET status = 'pending', attempts = 0, task_id = NULL, file_id = NULL,
                 error = NULL, updated_at = ?2
             WHERE job_id = ?1
               AND status IN ('processing', 'failed', 'missed', 'conflict')",
            params![job_id, unix_timestamp()],
        )
        .map_err(|error| format!("恢复未完成导入记录失败：{error}"))?;
    Ok(())
}

pub(crate) fn load_gcid_import_status(
    database_path: &Path,
    job_id: Option<&str>,
) -> Result<Option<GcidImportStatus>, String> {
    let connection = open_database(database_path)?;
    let query = if job_id.is_some() {
        "SELECT job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, updated_at
         FROM gcid_import_jobs WHERE job_id = ?1"
    } else {
        "SELECT job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, updated_at
         FROM gcid_import_jobs ORDER BY updated_at DESC LIMIT 1"
    };
    let load = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, u64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, i64>(10)?,
        ))
    };
    let record = if let Some(job_id) = job_id {
        connection
            .query_row(query, params![job_id], load)
            .optional()
    } else {
        connection.query_row(query, [], load).optional()
    }
    .map_err(|error| format!("读取导入任务失败：{error}"))?;
    let Some((
        job_id,
        source_path,
        source_name,
        destination_parent_id,
        destination_name,
        total_files,
        total_size,
        status,
        current_path,
        error,
        updated_at,
    )) = record
    else {
        return Ok(None);
    };
    let counts = load_gcid_import_counts(&connection, &job_id)?;
    let finished =
        counts.imported + counts.existing + counts.missed + counts.conflict + counts.failed;
    Ok(Some(GcidImportStatus {
        job_id,
        source_path,
        source_name,
        destination_parent_id,
        destination_name,
        total_files,
        total_size,
        status,
        current_path,
        error,
        counts,
        finished,
        updated_at,
    }))
}

pub(crate) fn emit_gcid_import_status(app: &tauri::AppHandle, database_path: &Path, job_id: &str) {
    if let Ok(Some(import_status)) = load_gcid_import_status(database_path, Some(job_id)) {
        emit(
            app,
            json!({ "type": "gcid-import", "status": import_status }),
        );
    }
}

pub(crate) fn claim_gcid_import_file(
    database_path: &Path,
    job_id: &str,
) -> Result<Option<GcidImportFile>, String> {
    let mut connection = open_database(database_path)?;
    // A deferred transaction lets every worker read the same pending row and
    // then makes all but one fail while upgrading to a writer. Reserving the
    // write lock before the SELECT keeps claiming serialized while the cloud
    // work itself still runs at the configured concurrency.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| format!("开始领取导入记录失败：{error}"))?;
    let record = transaction
        .query_row(
            "SELECT path, folder_path, file_name, file_size, gcid, cid, attempts
             FROM gcid_import_files
             WHERE job_id = ?1 AND status = 'pending'
             ORDER BY path
             LIMIT 1",
            params![job_id],
            |row| {
                Ok(GcidImportFile {
                    path: row.get(0)?,
                    folder_path: row.get(1)?,
                    name: row.get(2)?,
                    size: row.get(3)?,
                    gcid: row.get(4)?,
                    cid: row.get(5)?,
                    attempts: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("领取导入记录失败：{error}"))?;
    if let Some(record) = record.as_ref() {
        let changed = transaction
            .execute(
                "UPDATE gcid_import_files
                 SET status = 'processing', error = NULL, updated_at = ?3
                 WHERE job_id = ?1 AND path = ?2 AND status = 'pending'",
                params![job_id, record.path, unix_timestamp()],
            )
            .map_err(|error| format!("锁定导入记录失败：{error}"))?;
        if changed == 0 {
            transaction
                .rollback()
                .map_err(|error| format!("回滚导入记录失败：{error}"))?;
            return claim_gcid_import_file(database_path, job_id);
        }
        transaction
            .execute(
                "UPDATE gcid_import_jobs
                 SET current_path = ?2, updated_at = ?3
                 WHERE job_id = ?1",
                params![job_id, record.path, unix_timestamp()],
            )
            .map_err(|error| format!("更新当前导入文件失败：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交导入记录失败：{error}"))?;
    Ok(record)
}

pub(crate) fn update_gcid_import_attempt(
    database_path: &Path,
    job_id: &str,
    path: &str,
    attempt: i64,
    error: Option<&str>,
) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET attempts = ?3, error = ?4, updated_at = ?5
             WHERE job_id = ?1 AND path = ?2",
            params![job_id, path, attempt, error, unix_timestamp()],
        )
        .map_err(|error| format!("更新导入重试状态失败：{error}"))?;
    Ok(())
}

pub(crate) fn finish_gcid_import_file(
    database_path: &Path,
    job_id: &str,
    path: &str,
    status: &str,
    task_id: Option<&str>,
    file_id: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET status = ?3, task_id = ?4, file_id = ?5, error = ?6, updated_at = ?7
             WHERE job_id = ?1 AND path = ?2",
            params![
                job_id,
                path,
                status,
                task_id,
                file_id,
                error,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存导入结果失败：{error}"))?;
    Ok(())
}


pub(crate) async fn wait_gcid_import_task(
    token: &str,
    device_id: &str,
    task_id: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(CLOUD_CONFIRM_TIMEOUT_SECS);
    let mut attempt = 0_u64;
    while Instant::now() < deadline {
        match check_upload_task(token, device_id, task_id).await {
            Ok(CloudTaskCheck::Confirmed(data)) => {
                return data
                    .get("fileId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| "云端入库完成但没有返回文件 ID".to_string());
            }
            Ok(CloudTaskCheck::Pending) => {}
            Err(CloudConfirmError::Retryable(message)) => {
                if message.contains("登录态已失效") {
                    return Err(message);
                }
            }
            Err(CloudConfirmError::Permanent(message)) => return Err(message),
        }
        attempt += 1;
        let wait = Duration::from_millis((500 * attempt.div_ceil(5)).clamp(500, 5_000));
        sleep(wait.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    Err(format!(
        "云端入库超过 {CLOUD_CONFIRM_TIMEOUT_SECS} 秒仍未完成"
    ))
}

pub(crate) async fn process_gcid_import_file(
    app: &tauri::AppHandle,
    state: &SharedState,
    destination_parent_id: &str,
    destination_name: &str,
    record: &GcidImportFile,
) -> Result<GcidImportOutcome, String> {
    let (token, device_id) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
        )
    };
    let remote_path = if record.folder_path.is_empty() {
        destination_name.to_string()
    } else {
        format!("{destination_name}/{}", record.folder_path)
    };
    let parent_id = ensure_remote_path(
        app,
        state,
        &token,
        &device_id,
        destination_parent_id,
        &remote_path,
    )
    .await?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_center_token",
        json!({
            "capacity": 2,
            "name": record.name,
            "res": { "fileSize": record.size },
            "parentId": parent_id
        }),
        &[156, 160],
    )
    .await?;
    if response.code == 160 {
        return match find_remote_file(&token, &device_id, &parent_id, &record.name).await? {
            Some((file_id, file_size, 1)) if file_size == record.size => {
                Ok(GcidImportOutcome::Existing { file_id })
            }
            Some((_, file_size, 1)) => Ok(GcidImportOutcome::Conflict(format!(
                "同名文件大小不一致：云端 {file_size}，导入 {}",
                record.size
            ))),
            Some(_) => Ok(GcidImportOutcome::Conflict("同名项是文件夹".to_string())),
            None => Err("光鸭返回名称冲突，但未找到同名文件".to_string()),
        };
    }
    let mut task_id = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "光鸭没有返回上传任务 ID".to_string())?;
    let mut instant = response.code == 156;
    if !instant {
        let flash = api_post(
            &token,
            &device_id,
            "/userres/v1/check_can_flash_upload",
            json!({ "taskId": task_id, "gcid": record.gcid, "cid": record.cid }),
            &[112],
        )
        .await?;
        if flash.code == 112 {
            return Ok(GcidImportOutcome::Missed { task_id });
        }
        let data = flash.data.unwrap_or_default();
        instant = data
            .get("canFlashUpload")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(next_task_id) = data
            .get("taskId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            task_id = next_task_id.to_string();
        }
    }
    if !instant {
        return Ok(GcidImportOutcome::Missed { task_id });
    }
    let file_id = wait_gcid_import_task(&token, &device_id, &task_id).await?;
    Ok(GcidImportOutcome::Imported { task_id, file_id })
}

pub(crate) async fn gcid_import_worker(
    app: tauri::AppHandle,
    state: SharedState,
    database_path: PathBuf,
    job_id: String,
    destination_parent_id: String,
    destination_name: String,
    completed_since_emit: Arc<AtomicUsize>,
) {
    loop {
        let record = match claim_gcid_import_file(&database_path, &job_id) {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(error) => {
                status(&app, "error", error);
                break;
            }
        };
        let first_attempt = (record.attempts + 1).clamp(1, MAX_GCID_IMPORT_ATTEMPTS);
        let mut terminal = false;
        for attempt in first_attempt..=MAX_GCID_IMPORT_ATTEMPTS {
            let _ =
                update_gcid_import_attempt(&database_path, &job_id, &record.path, attempt, None);
            match process_gcid_import_file(
                &app,
                &state,
                &destination_parent_id,
                &destination_name,
                &record,
            )
            .await
            {
                Ok(GcidImportOutcome::Imported { task_id, file_id }) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "imported",
                        Some(&task_id),
                        Some(&file_id),
                        None,
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Existing { file_id }) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "existing",
                        None,
                        Some(&file_id),
                        None,
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Missed { task_id }) => {
                    let message = "光鸭未命中该 GCID，且没有本地源文件可普通上传";
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "missed",
                        Some(&task_id),
                        None,
                        Some(message),
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Conflict(error)) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "conflict",
                        None,
                        None,
                        Some(&error),
                    );
                    terminal = true;
                    break;
                }
                Err(error) if attempt < MAX_GCID_IMPORT_ATTEMPTS => {
                    let _ = update_gcid_import_attempt(
                        &database_path,
                        &job_id,
                        &record.path,
                        attempt,
                        Some(&error),
                    );
                    sleep(Duration::from_secs((attempt as u64).clamp(1, 5))).await;
                }
                Err(error) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "failed",
                        None,
                        None,
                        Some(&error),
                    );
                    status(
                        &app,
                        "error",
                        format!("GCID 导入失败：{}：{error}", record.path),
                    );
                    terminal = true;
                    break;
                }
            }
        }
        if !terminal {
            let _ = finish_gcid_import_file(
                &database_path,
                &job_id,
                &record.path,
                "failed",
                None,
                None,
                Some("达到最大重试次数"),
            );
        }
        let completed = completed_since_emit.fetch_add(1, Ordering::Relaxed) + 1;
        if completed % 50 == 0 {
            emit_gcid_import_status(&app, &database_path, &job_id);
        }
    }
}

pub(crate) async fn run_gcid_import(
    app: tauri::AppHandle,
    state: SharedState,
    database_path: PathBuf,
    job_id: String,
    destination_parent_id: String,
    destination_name: String,
    concurrency: usize,
) {
    let completed_since_emit = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        workers.push(tauri::async_runtime::spawn(gcid_import_worker(
            app.clone(),
            state.clone(),
            database_path.clone(),
            job_id.clone(),
            destination_parent_id.clone(),
            destination_name.clone(),
            completed_since_emit.clone(),
        )));
    }
    for worker in workers {
        let _ = worker.await;
    }
    let final_status = load_gcid_import_status(&database_path, Some(&job_id))
        .ok()
        .flatten();
    let (status_value, error_value) = match final_status.as_ref() {
        Some(result) if result.counts.pending > 0 || result.counts.processing > 0 => {
            ("paused", Some("仍有未处理记录，可点击继续导入".to_string()))
        }
        Some(result)
            if result.counts.failed > 0
                || result.counts.missed > 0
                || result.counts.conflict > 0 =>
        {
            (
                "completed_with_errors",
                Some("导入完成，但存在异常记录".to_string()),
            )
        }
        Some(_) => ("completed", None),
        None => ("failed", Some("无法读取导入任务状态".to_string())),
    };
    if let Ok(connection) = open_database(&database_path) {
        let _ = connection.execute(
            "UPDATE gcid_import_jobs
             SET status = ?2, current_path = '', error = ?3, updated_at = ?4
             WHERE job_id = ?1",
            params![job_id, status_value, error_value, unix_timestamp()],
        );
    }
    if let Ok(mut guard) = state.lock() {
        guard.gcid_import_running.remove(&job_id);
    }
    publish_all_cloud_directories_changed(&app, &state, "gcid-import");
    emit_gcid_import_status(&app, &database_path, &job_id);
    if status_value == "completed" {
        status(&app, "success", "GCID JSON 秒传导入完成");
    } else {
        status(&app, "warning", "GCID JSON 秒传导入结束，请查看导入统计");
    }
}


#[tauri::command]
pub(crate) fn select_gcid_import_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("光鸭 GCID JSON", &["json"])
        .pick_file()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
pub(crate) async fn stage_gcid_import_text(
    state: tauri::State<'_, SharedState>,
    content: String,
) -> Result<GcidImportSourceInfo, String> {
    if content.trim().is_empty() {
        return Err("请先粘贴 JSON 内容".to_string());
    }
    let size = content.len() as u64;
    if size > MAX_GCID_IMPORT_BYTES {
        return Err(format!(
            "粘贴内容超过 {} MB，请改用文件导入",
            MAX_GCID_IMPORT_BYTES / 1024 / 1024
        ));
    }
    let hash = hex::encode(Sha256::digest(content.as_bytes()));
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let directory = database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("imports");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("创建导入暂存目录失败：{error}"))?;
    let file_name = format!("粘贴导入-{}.json", &hash[..12]);
    let file_path = directory.join(&file_name);
    tokio::fs::write(&file_path, content.as_bytes())
        .await
        .map_err(|error| format!("写入粘贴 JSON 文件失败：{error}"))?;
    Ok(GcidImportSourceInfo {
        path: file_path.to_string_lossy().to_string(),
        name: file_name,
        size,
    })
}

#[tauri::command]
pub(crate) async fn prepare_gcid_import(
    state: tauri::State<'_, SharedState>,
    source_path: String,
    destination_parent_id: String,
    destination_name: String,
) -> Result<GcidImportStatus, String> {
    let source_path = PathBuf::from(source_path);
    let metadata = tokio::fs::metadata(&source_path)
        .await
        .map_err(|error| format!("读取 JSON 文件失败：{error}"))?;
    if !metadata.is_file() {
        return Err("导入来源不是文件".to_string());
    }
    if metadata.len() > MAX_GCID_IMPORT_BYTES {
        return Err(format!(
            "JSON 文件超过 {} MB，拒绝载入",
            MAX_GCID_IMPORT_BYTES / 1024 / 1024
        ));
    }
    let raw = tokio::fs::read(&source_path)
        .await
        .map_err(|error| format!("读取 JSON 文件失败：{error}"))?;
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let job_id = prepare_gcid_import_database(
        &database_path,
        &raw,
        &source_path,
        &destination_parent_id,
        &destination_name,
    )?;
    // A target can be deleted or renamed by this app, another client, or the
    // official web UI. Re-imports must resolve fresh directory IDs.
    invalidate_remote_directory_cache(state.inner());
    load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "创建导入任务后无法读取状态".to_string())
}

#[tauri::command]
pub(crate) fn get_gcid_import_status(
    state: tauri::State<'_, SharedState>,
    job_id: Option<String>,
) -> Result<Option<GcidImportStatus>, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    load_gcid_import_status(&database_path, job_id.as_deref())
}

#[tauri::command]
pub(crate) fn start_gcid_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    job_id: String,
    concurrency: usize,
) -> Result<GcidImportStatus, String> {
    if !(1..=MAX_GCID_IMPORT_CONCURRENCY).contains(&concurrency) {
        return Err(format!(
            "秒传导入并发数必须在 1–{MAX_GCID_IMPORT_CONCURRENCY} 之间"
        ));
    }
    let (database_path, has_token) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (guard.db_path.clone(), guard.token.is_some())
    };
    if !has_token {
        return Err("请先登录光鸭云盘".to_string());
    }
    let current = load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "导入任务不存在，请重新选择 JSON".to_string())?;
    let reimport_all = gcid_import_is_terminal(&current.status);
    if !reimport_all && !gcid_import_has_retryable_work(&current.counts) {
        return Ok(current);
    }
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.gcid_import_running.insert(job_id.clone()) {
            return Err("这个导入任务已经在运行".to_string());
        }
    }
    let reset_result = if reimport_all {
        reset_all_gcid_import_files(&database_path, &job_id)
    } else {
        reset_retryable_gcid_import_files(&database_path, &job_id)
    };
    if let Err(error) = reset_result {
        if let Ok(mut guard) = state.lock() {
            guard.gcid_import_running.remove(&job_id);
        }
        return Err(error);
    }
    let connection = match open_database(&database_path) {
        Ok(connection) => connection,
        Err(error) => {
            if let Ok(mut guard) = state.lock() {
                guard.gcid_import_running.remove(&job_id);
            }
            return Err(error);
        }
    };
    if let Err(error) = connection.execute(
        "UPDATE gcid_import_jobs
         SET status = 'running', error = NULL, updated_at = ?2
         WHERE job_id = ?1",
        params![job_id, unix_timestamp()],
    ) {
        if let Ok(mut guard) = state.lock() {
            guard.gcid_import_running.remove(&job_id);
        }
        return Err(format!("启动导入任务失败：{error}"));
    }
    let running = load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "启动后无法读取导入任务".to_string())?;
    let destination_parent_id = running.destination_parent_id.clone();
    let destination_name = running.destination_name.clone();
    emit_gcid_import_status(&app, &database_path, &job_id);
    tauri::async_runtime::spawn(run_gcid_import(
        app,
        state.inner().clone(),
        database_path,
        job_id,
        destination_parent_id,
        destination_name,
        concurrency,
    ));
    Ok(running)
}
