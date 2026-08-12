//! SQLite 打开/建表、设备 ID、通用 app_state 与文件指纹缓存。

use crate::prelude::*;

pub(crate) fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建本地数据目录失败：{e}"))?;
    }
    let connection = Connection::open(path).map_err(|e| format!("打开 SQLite 失败：{e}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 SQLite 等待时间失败：{e}"))?;
    Ok(connection)
}

pub(crate) fn init_database(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS auth_session (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               access_token TEXT,
               refresh_token TEXT,
               account_scope TEXT,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS uploaded_files (
               mapping_id TEXT NOT NULL,
               file_path TEXT NOT NULL,
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               task_id TEXT,
               remote_file_id TEXT,
               upload_state TEXT NOT NULL DEFAULT 'cloud_confirmed',
               remote_parent_id TEXT NOT NULL DEFAULT '',
               remote_dir TEXT NOT NULL DEFAULT '',
               relative_path TEXT NOT NULL DEFAULT '',
               change_kind TEXT NOT NULL DEFAULT 'added',
               replacement_json TEXT,
               uploaded_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, file_path)
             );
             CREATE TABLE IF NOT EXISTS upload_checkpoints (
               mapping_id TEXT NOT NULL,
               file_path TEXT NOT NULL,
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               item_json TEXT NOT NULL,
               checkpoint_json TEXT NOT NULL,
               uploaded_bytes INTEGER NOT NULL DEFAULT 0,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, file_path)
             );
             CREATE TABLE IF NOT EXISTS app_state (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS file_fingerprints (
               file_path TEXT NOT NULL,
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               gcid TEXT NOT NULL,
               cid TEXT NOT NULL DEFAULT '',
               computed_at INTEGER NOT NULL,
               PRIMARY KEY (file_path, size, modified_ms)
             );
             CREATE TABLE IF NOT EXISTS gcid_export_snapshots (
               account_scope TEXT NOT NULL,
               selection_key TEXT NOT NULL,
               root_signatures_json TEXT NOT NULL,
               export_json TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               last_used_at INTEGER NOT NULL,
               PRIMARY KEY (account_scope, selection_key)
             );
             CREATE TABLE IF NOT EXISTS gcid_export_file_hashes (
               account_scope TEXT NOT NULL,
               file_id TEXT NOT NULL,
               file_size TEXT NOT NULL,
               gcid TEXT NOT NULL,
               cid TEXT NOT NULL,
               last_used_at INTEGER NOT NULL,
               PRIMARY KEY (account_scope, file_id, file_size, gcid)
             );
             CREATE TABLE IF NOT EXISTS auto_share_targets (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               target_type TEXT NOT NULL,
               remote_target_id TEXT NOT NULL,
               title TEXT NOT NULL,
               share_id TEXT NOT NULL,
               share_url TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key)
             );
             CREATE TABLE IF NOT EXISTS auto_share_events (
               event_id TEXT PRIMARY KEY,
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               share_url TEXT,
               status TEXT NOT NULL,
               action TEXT,
               error_code TEXT,
               message TEXT,
               resource_url TEXT,
               notification_status TEXT,
               payload TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS auto_share_pending (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               target_type TEXT NOT NULL,
               title TEXT NOT NULL,
               remote_target_id TEXT NOT NULL,
               added_paths TEXT NOT NULL,
               changed_paths TEXT NOT NULL,
               event_id TEXT NOT NULL,
               retry_count INTEGER NOT NULL DEFAULT 0,
               due_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key)
             );
             CREATE TABLE IF NOT EXISTS auto_share_failures (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               relative_path TEXT NOT NULL,
               error TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key, relative_path)
             );
             CREATE TABLE IF NOT EXISTS gcid_import_jobs (
               job_id TEXT PRIMARY KEY,
               source_path TEXT NOT NULL,
               source_name TEXT NOT NULL,
               destination_parent_id TEXT NOT NULL,
               destination_name TEXT NOT NULL,
               total_files INTEGER NOT NULL,
               total_size TEXT NOT NULL,
               status TEXT NOT NULL,
               current_path TEXT NOT NULL DEFAULT '',
               error TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS gcid_import_files (
               job_id TEXT NOT NULL,
               path TEXT NOT NULL,
               folder_path TEXT NOT NULL,
               file_name TEXT NOT NULL,
               file_size INTEGER NOT NULL,
               gcid TEXT NOT NULL,
               cid TEXT NOT NULL DEFAULT '',
               status TEXT NOT NULL DEFAULT 'pending',
               attempts INTEGER NOT NULL DEFAULT 0,
               task_id TEXT,
               file_id TEXT,
               error TEXT,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (job_id, path)
             );
             CREATE TABLE IF NOT EXISTS developer_targets (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               token_id TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS developer_transfer_jobs (
               id TEXT PRIMARY KEY,
               target_id TEXT NOT NULL,
               target_name TEXT NOT NULL,
               file_ids_json TEXT NOT NULL,
               file_names_json TEXT NOT NULL,
               status TEXT NOT NULL,
               phase TEXT NOT NULL,
               pre_task_id TEXT,
               upload_task_id TEXT,
               total_count INTEGER NOT NULL DEFAULT 0,
               passed_count INTEGER NOT NULL DEFAULT 0,
               rejected_count INTEGER NOT NULL DEFAULT 0,
               pending_count INTEGER NOT NULL DEFAULT 0,
               success_count INTEGER NOT NULL DEFAULT 0,
               skipped_count INTEGER NOT NULL DEFAULT 0,
               work_total_count INTEGER NOT NULL DEFAULT 0,
               processed_count INTEGER NOT NULL DEFAULT 0,
               current_path TEXT NOT NULL DEFAULT '',
               error_code INTEGER,
               message TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS developer_transfer_name_restores (
               job_id TEXT NOT NULL,
               file_id TEXT NOT NULL,
               original_name TEXT NOT NULL,
               temporary_name TEXT NOT NULL,
               status TEXT NOT NULL DEFAULT 'active',
               last_error TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (job_id, file_id)
             );
             CREATE TABLE IF NOT EXISTS offline_name_restores (
               task_id TEXT PRIMARY KEY,
               original_name TEXT NOT NULL,
               temporary_name TEXT NOT NULL,
               file_id TEXT,
               status TEXT NOT NULL DEFAULT 'pending',
               attempts INTEGER NOT NULL DEFAULT 0,
               last_error TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS recycle_bin_clear_operations (
               account_scope TEXT PRIMARY KEY,
               state TEXT NOT NULL,
               task_id TEXT,
               last_error TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| format!("初始化 SQLite 失败：{e}"))?;
    connection
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS gcid_import_files_status
               ON gcid_import_files(job_id, status, path);
             CREATE INDEX IF NOT EXISTS developer_transfer_jobs_status
               ON developer_transfer_jobs(status, updated_at);
             CREATE INDEX IF NOT EXISTS developer_transfer_name_restores_status
               ON developer_transfer_name_restores(status, file_id, updated_at);
             CREATE INDEX IF NOT EXISTS offline_name_restores_status
               ON offline_name_restores(status, updated_at);
             CREATE INDEX IF NOT EXISTS recycle_bin_clear_operations_updated
               ON recycle_bin_clear_operations(updated_at);
             UPDATE gcid_import_files
               SET status = 'pending', error = '应用上次退出，已等待继续'
               WHERE status = 'processing';
             UPDATE gcid_import_jobs
               SET status = 'paused', error = '应用上次退出，点击继续导入'
               WHERE status IN ('preparing', 'running');",
        )
        .map_err(|e| format!("初始化 GCID 导入状态失败：{e}"))?;
    let _ = connection.execute(
        "ALTER TABLE auto_share_events ADD COLUMN notification_status TEXT",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE auto_share_events ADD COLUMN error_code TEXT",
        [],
    );
    for migration in [
        "ALTER TABLE auth_session ADD COLUMN account_scope TEXT",
        "ALTER TABLE uploaded_files ADD COLUMN upload_state TEXT NOT NULL DEFAULT 'cloud_confirmed'",
        "ALTER TABLE uploaded_files ADD COLUMN remote_parent_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN remote_dir TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN relative_path TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN change_kind TEXT NOT NULL DEFAULT 'added'",
        "ALTER TABLE uploaded_files ADD COLUMN replacement_json TEXT",
        "ALTER TABLE file_fingerprints ADD COLUMN cid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE gcid_import_files ADD COLUMN cid TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE developer_transfer_jobs ADD COLUMN work_total_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE developer_transfer_jobs ADD COLUMN processed_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE developer_transfer_jobs ADD COLUMN current_path TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(migration, []);
    }
    connection
        .execute(
            "UPDATE uploaded_files
             SET upload_state = CASE
               WHEN task_id IS NOT NULL AND TRIM(task_id) <> ''
                 AND (remote_file_id IS NULL OR TRIM(remote_file_id) = '')
               THEN ?1 ELSE ?2 END
             WHERE upload_state IS NULL OR upload_state = ''
                OR upload_state NOT IN (?1, ?2)
                OR (upload_state = ?2 AND task_id IS NOT NULL AND TRIM(task_id) <> ''
                    AND (remote_file_id IS NULL OR TRIM(remote_file_id) = ''))",
            params![UPLOAD_STATE_OSS_COMPLETE, UPLOAD_STATE_CLOUD_CONFIRMED],
        )
        .map_err(|e| format!("迁移上传状态失败：{e}"))?;
    Ok(())
}

pub(crate) fn valid_sha1_hex(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn load_cached_file_hashes(
    database: &Path,
    file_path: &Path,
    size: u64,
    modified_ms: u128,
    settings: CacheSettings,
) -> Result<Option<FileHashes>, String> {
    if !settings.enabled {
        return Ok(None);
    }
    let size = i64::try_from(size).map_err(|_| "文件过大，无法缓存秒传指纹".to_string())?;
    let connection = open_database(database)?;
    let hashes = connection
        .query_row(
            "SELECT gcid, cid FROM file_fingerprints
             WHERE file_path = ?1 AND size = ?2 AND modified_ms = ?3",
            params![
                file_path.to_string_lossy().as_ref(),
                size,
                modified_ms.to_string()
            ],
            |row| {
                Ok(FileHashes {
                    gcid: row.get(0)?,
                    cid: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取秒传指纹缓存失败：{error}"))?;
    Ok(hashes.filter(|value| valid_sha1_hex(&value.gcid) && valid_sha1_hex(&value.cid)))
}

pub(crate) fn save_cached_file_hashes(
    database: &Path,
    file_path: &Path,
    size: u64,
    modified_ms: u128,
    hashes: &FileHashes,
    settings: CacheSettings,
) -> Result<(), String> {
    if !settings.enabled {
        return Ok(());
    }
    let size = i64::try_from(size).map_err(|_| "文件过大，无法缓存秒传指纹".to_string())?;
    let connection = open_database(database)?;
    let file_path = file_path.to_string_lossy();
    let modified_ms = modified_ms.to_string();
    connection
        .execute(
            "DELETE FROM file_fingerprints
             WHERE file_path = ?1 AND (size <> ?2 OR modified_ms <> ?3)",
            params![file_path.as_ref(), size, modified_ms],
        )
        .map_err(|error| format!("清理旧秒传指纹失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO file_fingerprints
               (file_path, size, modified_ms, gcid, cid, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(file_path, size, modified_ms)
             DO UPDATE SET gcid = excluded.gcid, cid = excluded.cid,
                           computed_at = excluded.computed_at",
            params![
                file_path.as_ref(),
                size,
                modified_ms,
                hashes.gcid,
                hashes.cid,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存秒传指纹缓存失败：{error}"))?;
    trim_file_fingerprint_cache(database, settings.max_entries)?;
    Ok(())
}


pub(crate) fn load_or_create_device_id(path: &Path) -> Result<String, String> {
    let connection = open_database(path)?;
    let current = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'device_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("读取设备 ID 失败：{e}"))?;
    let value = current
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase().replace('-', ""))
        .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    connection
        .execute(
            "INSERT INTO app_state (key, value, updated_at) VALUES ('device_id', ?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![value, unix_timestamp()],
        )
        .map_err(|e| format!("保存设备 ID 失败：{e}"))?;
    Ok(value)
}

pub(crate) fn load_app_state(path: &Path, key: &str) -> Result<Option<String>, String> {
    open_database(path)?
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取本地设置失败：{error}"))
}

pub(crate) fn save_app_state(path: &Path, key: &str, value: &str) -> Result<(), String> {
    open_database(path)?
        .execute(
            "INSERT INTO app_state (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
            params![key, value, unix_timestamp()],
        )
        .map_err(|error| format!("保存本地设置失败：{error}"))?;
    Ok(())
}
