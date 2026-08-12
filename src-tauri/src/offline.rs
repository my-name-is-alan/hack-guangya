//! 云添加（离线下载）任务与文件名混淆恢复。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct OfflineSettings {
    pub(crate) filename_obfuscation_enabled: bool,
    pub(crate) pending_restores: i64,
}


#[derive(Debug)]
pub(crate) struct OfflineNameRestore {
    pub(crate) task_id: String,
    pub(crate) original_name: String,
    pub(crate) attempts: i64,
    pub(crate) updated_at: i64,
}

pub(crate) fn pending_offline_name_restore_count(path: &Path) -> Result<i64, String> {
    open_database(path)?
        .query_row(
            "SELECT COUNT(*) FROM offline_name_restores WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取待恢复离线任务失败：{error}"))
}

pub(crate) fn offline_filename_obfuscation_enabled(path: &Path) -> Result<bool, String> {
    Ok(load_app_state(path, "offline_filename_obfuscation")?.as_deref() == Some("true"))
}

pub(crate) fn offline_settings_for_path(path: &Path) -> Result<OfflineSettings, String> {
    Ok(OfflineSettings {
        filename_obfuscation_enabled: offline_filename_obfuscation_enabled(path)?,
        pending_restores: pending_offline_name_restore_count(path)?,
    })
}

#[tauri::command]
pub(crate) fn get_offline_settings(state: tauri::State<'_, SharedState>) -> Result<OfflineSettings, String> {
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    offline_settings_for_path(&db_path)
}

#[tauri::command]
pub(crate) fn update_offline_settings(
    state: tauri::State<'_, SharedState>,
    filename_obfuscation_enabled: bool,
) -> Result<OfflineSettings, String> {
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    save_app_state(
        &db_path,
        "offline_filename_obfuscation",
        if filename_obfuscation_enabled {
            "true"
        } else {
            "false"
        },
    )?;
    offline_settings_for_path(&db_path)
}

pub(crate) fn offline_resolved_name(data: &Value) -> String {
    let info = data
        .get("urlResInfo")
        .or_else(|| data.get("btResInfo"))
        .or_else(|| data.get("emuleResInfo"))
        .or_else(|| data.get("resourceInfo"))
        .unwrap_or(data);
    ["fileName", "name", "title"]
        .into_iter()
        .find_map(|key| {
            info.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn ed2k_file_name(source: &str) -> String {
    if !source.to_ascii_lowercase().starts_with("ed2k://|file|") {
        return String::new();
    }
    let encoded = source
        .split('|')
        .nth(2)
        .unwrap_or_default()
        .replace('+', "%20");
    percent_decode_str(&encoded)
        .decode_utf8_lossy()
        .trim()
        .to_string()
}

pub(crate) fn magnet_display_name(source: &str) -> String {
    if !source.to_ascii_lowercase().starts_with("magnet:?") {
        return String::new();
    }
    source
        .split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .find_map(|parameter| {
            let (key, value) = parameter.split_once('=').unwrap_or((parameter, ""));
            let key = percent_decode_str(key).decode_utf8_lossy();
            key.eq_ignore_ascii_case("dn").then(|| {
                percent_decode_str(&value.replace('+', "%20"))
                    .decode_utf8_lossy()
                    .trim()
                    .to_string()
            })
        })
        .unwrap_or_default()
}

pub(crate) fn offline_source_name(source: &str) -> String {
    let magnet_name = magnet_display_name(source);
    if magnet_name.is_empty() {
        ed2k_file_name(source)
    } else {
        magnet_name
    }
}

pub(crate) fn offline_temporary_name(original_name: &str, source: &str) -> String {
    let extension = if source.to_ascii_lowercase().starts_with("ed2k:") {
        Path::new(original_name)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 16
                    && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .map(|value| format!(".{value}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "gy_{}{}",
        &Uuid::new_v4().simple().to_string()[..20],
        extension
    )
}

pub(crate) fn protected_offline_source(source: &str, temporary_name: &str) -> String {
    if source.to_ascii_lowercase().starts_with("magnet:?") {
        let Some((base, query)) = source.split_once('?') else {
            return source.to_string();
        };
        let parameters = query
            .split('&')
            .filter(|parameter| {
                let key = parameter.split_once('=').map_or(*parameter, |(key, _)| key);
                !percent_decode_str(key)
                    .decode_utf8_lossy()
                    .eq_ignore_ascii_case("dn")
            })
            .collect::<Vec<_>>();
        return if parameters.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{}", parameters.join("&"))
        };
    }
    if source.to_ascii_lowercase().starts_with("ed2k://|file|") {
        let mut parts = source.split('|').collect::<Vec<_>>();
        if parts.len() > 4 {
            parts[2] = temporary_name;
        }
        return parts.join("|");
    }
    source.to_string()
}

pub(crate) fn save_offline_name_restore(
    path: &Path,
    task_id: &str,
    original_name: &str,
    temporary_name: &str,
) -> Result<(), String> {
    let now = unix_timestamp();
    open_database(path)?
        .execute(
            "INSERT INTO offline_name_restores
               (task_id, original_name, temporary_name, status, attempts, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', 0, ?4, ?4)
             ON CONFLICT(task_id) DO UPDATE SET
               original_name = excluded.original_name,
               temporary_name = excluded.temporary_name,
               file_id = NULL,
               status = 'pending',
               attempts = 0,
               last_error = NULL,
               updated_at = excluded.updated_at",
            params![task_id, original_name, temporary_name, now],
        )
        .map_err(|error| format!("保存离线文件名恢复任务失败：{error}"))?;
    Ok(())
}

pub(crate) fn remove_offline_name_restores(path: &Path, task_ids: &[String]) -> Result<(), String> {
    let mut connection = open_database(path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始清理离线恢复任务失败：{error}"))?;
    {
        let mut statement = transaction
            .prepare("DELETE FROM offline_name_restores WHERE task_id = ?1")
            .map_err(|error| format!("准备清理离线恢复任务失败：{error}"))?;
        for task_id in task_ids {
            statement
                .execute(params![task_id])
                .map_err(|error| format!("清理离线恢复任务失败：{error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("提交离线恢复任务清理失败：{error}"))
}

pub(crate) fn annotate_offline_name_restores(path: &Path, data: &mut Value) -> Result<(), String> {
    let list_key = if data.get("list").and_then(Value::as_array).is_some() {
        "list"
    } else {
        "taskList"
    };
    let Some(list) = data.get_mut(list_key).and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare("SELECT original_name, status, last_error FROM offline_name_restores WHERE task_id = ?1")
        .map_err(|error| format!("准备读取离线恢复状态失败：{error}"))?;
    for task in list {
        let task_id = task
            .get("taskId")
            .or_else(|| task.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if task_id.is_empty() {
            continue;
        }
        let restore = statement
            .query_row(params![task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })
            .optional()
            .map_err(|error| format!("读取离线恢复状态失败：{error}"))?;
        let Some((original_name, restore_status, last_error)) = restore else {
            continue;
        };
        let Some(object) = task.as_object_mut() else {
            continue;
        };
        let public_status = if restore_status == "completed" {
            "restored"
        } else if last_error.is_empty() {
            "pending"
        } else {
            "failed"
        };
        object.insert("nameRestoreStatus".to_string(), json!(public_status));
        object.insert("nameRestoreError".to_string(), json!(last_error));
        object.insert("originalName".to_string(), json!(original_name));
        if restore_status == "completed" {
            object.insert("fileName".to_string(), json!(original_name));
        }
    }
    Ok(())
}

pub(crate) fn offline_task_status(task: &Value) -> Option<i64> {
    task.get("status")
        .or_else(|| task.get("taskStatus"))
        .or_else(|| task.get("state"))
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

pub(crate) async fn reconcile_offline_name_restores(
    state: &SharedState,
    supplied_data: Option<Value>,
) -> Result<Value, String> {
    let (token, device_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        let Some(token) = guard.token.clone() else {
            return Ok(supplied_data.unwrap_or_else(|| json!({})));
        };
        (token, guard.device_id.clone(), guard.db_path.clone())
    };
    if pending_offline_name_restore_count(&db_path)? == 0 {
        let mut data = supplied_data.unwrap_or_else(|| json!({}));
        annotate_offline_name_restores(&db_path, &mut data)?;
        return Ok(data);
    }
    if OFFLINE_RESTORE_RECONCILING
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        let mut data = supplied_data.unwrap_or_else(|| json!({}));
        annotate_offline_name_restores(&db_path, &mut data)?;
        return Ok(data);
    }
    let result = async {
        let mut data = match supplied_data {
            Some(data) => data,
            None => api_post(
                &token,
                &device_id,
                "/cloudcollection/v1/list_task",
                json!({ "cursor": "", "pageSize": DEFAULT_API_PAGE_SIZE }),
                &[],
            )
            .await?
            .data
            .unwrap_or_else(|| json!({ "list": [] })),
        };
        let tasks = data
            .get("list")
            .and_then(Value::as_array)
            .or_else(|| data.get("taskList").and_then(Value::as_array))
            .into_iter()
            .flatten()
            .filter_map(|task| {
                let task_id = task
                    .get("taskId")
                    .or_else(|| task.get("id"))
                    .and_then(Value::as_str)?
                    .to_string();
                Some((task_id, task.clone()))
            })
            .collect::<HashMap<_, _>>();
        let pending = {
            let connection = open_database(&db_path)?;
            let mut statement = connection
                .prepare("SELECT task_id, original_name, attempts, updated_at FROM offline_name_restores WHERE status = 'pending'")
                .map_err(|error| format!("准备读取离线恢复任务失败：{error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok(OfflineNameRestore {
                        task_id: row.get(0)?,
                        original_name: row.get(1)?,
                        attempts: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                })
                .map_err(|error| format!("读取离线恢复任务失败：{error}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("解析离线恢复任务失败：{error}"))?
        };
        let now = unix_timestamp();
        for restore in pending {
            let Some(task) = tasks.get(&restore.task_id) else {
                continue;
            };
            if offline_task_status(task) != Some(2) {
                continue;
            }
            let file_id = task.get("fileId").and_then(Value::as_str).unwrap_or_default();
            if file_id.is_empty() {
                continue;
            }
            let current_name = task.get("fileName").and_then(Value::as_str).unwrap_or_default();
            let restore_result = if current_name == restore.original_name {
                Ok(())
            } else if restore.attempts > 0 && now - restore.updated_at < OFFLINE_RESTORE_RETRY_SECS {
                continue;
            } else {
                rename_remote(&token, &device_id, file_id, &restore.original_name).await
            };
            let connection = open_database(&db_path)?;
            match restore_result {
                Ok(()) => {
                    connection
                        .execute(
                            "UPDATE offline_name_restores SET file_id = ?1, status = 'completed', last_error = NULL, updated_at = ?2 WHERE task_id = ?3",
                            params![file_id, now, restore.task_id],
                        )
                        .map_err(|error| format!("完成离线文件名恢复任务失败：{error}"))?;
                }
                Err(error) => {
                    connection
                        .execute(
                            "UPDATE offline_name_restores SET file_id = ?1, attempts = attempts + 1, last_error = ?2, updated_at = ?3 WHERE task_id = ?4",
                            params![file_id, error.chars().take(500).collect::<String>(), now, restore.task_id],
                        )
                        .map_err(|database_error| format!("记录离线文件名恢复失败：{database_error}"))?;
                }
            }
        }
        open_database(&db_path)?
            .execute(
                "DELETE FROM offline_name_restores WHERE status = 'completed' AND updated_at < ?1",
                params![now - 30 * 86_400],
            )
            .map_err(|error| format!("清理旧离线恢复记录失败：{error}"))?;
        annotate_offline_name_restores(&db_path, &mut data)?;
        Ok(data)
    }
    .await;
    OFFLINE_RESTORE_RECONCILING.store(0, Ordering::Release);
    result
}

pub(crate) async fn offline_name_restore_loop(state: SharedState) {
    loop {
        sleep(Duration::from_secs(OFFLINE_RESTORE_POLL_SECS)).await;
        let _ = reconcile_offline_name_restores(&state, None).await;
    }
}

pub(crate) fn normalize_offline_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("请输入离线下载地址".into());
    }
    if url.len() > MAX_OFFLINE_URL_LENGTH || url.chars().any(char::is_control) {
        return Err("离线下载地址格式无效".into());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let parsed = reqwest::Url::parse(url).map_err(|_| "离线下载地址格式无效")?;
        if parsed.host_str().is_none() {
            return Err("离线下载地址缺少主机名".into());
        }
    } else if lower.starts_with("magnet:") {
        if url.len() <= "magnet:".len() {
            return Err("磁力链接格式无效".into());
        }
    } else if lower.starts_with("ed2k://") {
        if url.len() <= "ed2k://".len() {
            return Err("电驴链接格式无效".into());
        }
    } else {
        return Err("仅支持 HTTP、HTTPS、磁力或 ED2K 离线地址".into());
    }
    Ok(url.to_string())
}

pub(crate) fn normalize_offline_file_indexes(
    file_indexes: Option<&[u64]>,
) -> Result<Option<Vec<u64>>, String> {
    let Some(file_indexes) = file_indexes else {
        return Ok(None);
    };
    if file_indexes.is_empty() {
        return Err("已提供 fileIndexes 时请至少选择一个资源文件".into());
    }
    if file_indexes.len() > MAX_OFFLINE_FILE_INDEXES {
        return Err(format!(
            "单次最多选择 {MAX_OFFLINE_FILE_INDEXES} 个资源文件"
        ));
    }
    let mut seen = HashSet::new();
    Ok(Some(
        file_indexes
            .iter()
            .copied()
            .filter(|index| seen.insert(*index))
            .collect(),
    ))
}

pub(crate) fn offline_resolve_request(url: &str) -> Result<Value, String> {
    Ok(json!({ "url": normalize_offline_url(url)? }))
}

pub(crate) fn offline_task_request(
    url: &str,
    parent_id: &str,
    new_name: &str,
    file_indexes: Option<&[u64]>,
) -> Result<Value, String> {
    let url = normalize_offline_url(url)?;
    let parent_id = normalize_parent_id(parent_id)?;
    let mut request = json!({ "url": url, "parentId": parent_id });
    if let Some(name) = (!new_name.trim().is_empty()).then(|| new_name.trim()) {
        if name.chars().count() > MAX_REMOTE_NAME_LENGTH
            || name
                .chars()
                .any(|character| character.is_control() || "\\/:*?\"<>|".contains(character))
        {
            return Err("离线任务名称格式无效".into());
        }
        request
            .as_object_mut()
            .expect("offline task request must be an object")
            .insert("newName".to_string(), json!(name));
    }
    if let Some(file_indexes) = normalize_offline_file_indexes(file_indexes)? {
        if !url.to_ascii_lowercase().starts_with("magnet:") {
            return Err("只有磁力任务支持 fileIndexes".into());
        }
        request
            .as_object_mut()
            .expect("offline task request must be an object")
            .insert("fileIndexes".to_string(), json!(file_indexes));
    }
    Ok(request)
}

pub(crate) fn offline_task_list_request(
    page: Option<u64>,
    cursor: Option<&str>,
    page_size: Option<u64>,
    status: Option<&[i64]>,
) -> Result<Value, String> {
    let cursor = normalize_api_cursor(cursor)?;
    if cursor.as_deref().unwrap_or_default().is_empty() && page.is_some_and(|page| page > 0) {
        return Err("离线任务列表使用 cursor 翻页，不支持 page > 0".into());
    }
    let mut request = json!({
        "cursor": cursor.unwrap_or_default(),
        "pageSize": normalize_api_page_size(page_size, DEFAULT_API_PAGE_SIZE)?
    });
    let object = request
        .as_object_mut()
        .expect("offline task list request must be an object");
    if let Some(statuses) = status {
        if statuses.len() > 6 || statuses.iter().any(|status| !(0..=5).contains(status)) {
            return Err("离线任务状态只能包含 0–5".into());
        }
        let mut seen = HashSet::new();
        let statuses = statuses
            .iter()
            .copied()
            .filter(|status| seen.insert(*status))
            .collect::<Vec<_>>();
        object.insert("status".to_string(), json!(statuses));
    }
    Ok(request)
}

pub(crate) fn offline_task_ids_request(task_ids: &[String]) -> Result<Value, String> {
    Ok(json!({
        "taskIds": normalize_id_list(task_ids, "离线任务")?
    }))
}

#[tauri::command]
pub(crate) async fn create_offline_task(
    state: tauri::State<'_, SharedState>,
    url: String,
    parent_id: String,
    new_name: Option<String>,
    restore_name: Option<String>,
    file_indexes: Option<Vec<u64>>,
) -> Result<Value, String> {
    let source = normalize_offline_url(&url)?;
    let (token, device_id) = auth_context(&state)?;
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let should_obfuscate = offline_filename_obfuscation_enabled(&db_path)?
        && (source.to_ascii_lowercase().starts_with("magnet:")
            || source.to_ascii_lowercase().starts_with("ed2k://"));
    let requested_name = new_name.as_deref().unwrap_or_default().trim();
    let mut original_name = requested_name.to_string();
    if should_obfuscate && original_name.is_empty() {
        original_name = restore_name
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if should_obfuscate && original_name.is_empty() {
        original_name = offline_source_name(&source);
    }
    if should_obfuscate && original_name.is_empty() {
        let resolved = api_post(
            &token,
            &device_id,
            "/cloudcollection/v1/resolve_res",
            offline_resolve_request(&source)?,
            &[],
        )
        .await?
        .data
        .unwrap_or_else(|| json!({}));
        original_name = offline_resolved_name(&resolved);
    }
    if should_obfuscate {
        original_name = normalize_remote_name(&original_name)
            .map_err(|_| "待恢复文件名格式无效".to_string())?;
    }
    let temporary_name = should_obfuscate
        .then(|| offline_temporary_name(&original_name, &source))
        .unwrap_or_default();
    let submitted_source = if should_obfuscate {
        protected_offline_source(&source, &temporary_name)
    } else {
        source.clone()
    };
    let request = offline_task_request(
        &submitted_source,
        &parent_id,
        if should_obfuscate {
            &temporary_name
        } else {
            requested_name
        },
        file_indexes.as_deref(),
    )?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v1/create_task",
        request,
        &[],
    )
    .await?;
    let mut data = response
        .data
        .ok_or_else(|| "光鸭没有返回离线任务".to_string())?;
    if should_obfuscate {
        let task_id = operation_task_id(&data).ok_or_else(|| {
            "离线任务已提交，但光鸭没有返回 taskId，无法自动恢复文件名".to_string()
        })?;
        save_offline_name_restore(&db_path, &task_id, &original_name, &temporary_name)?;
        if let Some(object) = data.as_object_mut() {
            object.insert("nameRestoreStatus".to_string(), json!("pending"));
            object.insert("originalName".to_string(), json!(original_name));
        }
    }
    Ok(data)
}

#[tauri::command]
pub(crate) async fn list_offline_tasks(
    state: tauri::State<'_, SharedState>,
    page: Option<u64>,
    cursor: Option<String>,
    page_size: Option<u64>,
    status: Option<Vec<i64>>,
) -> Result<Value, String> {
    let request = offline_task_list_request(page, cursor.as_deref(), page_size, status.as_deref())?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v1/list_task",
        request,
        &[],
    )
    .await?;
    reconcile_offline_name_restores(
        state.inner(),
        Some(response.data.unwrap_or_else(|| json!({ "list": [] }))),
    )
    .await
}

#[tauri::command]
pub(crate) async fn resolve_offline_resource(
    state: tauri::State<'_, SharedState>,
    url: String,
) -> Result<Value, String> {
    let request = offline_resolve_request(&url)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v1/resolve_res",
        request,
        &[],
    )
    .await?;
    response
        .data
        .ok_or_else(|| "光鸭没有返回离线资源解析结果".to_string())
}

pub(crate) async fn delete_offline_task_records(
    state: &tauri::State<'_, SharedState>,
    task_ids: &[String],
) -> Result<Value, String> {
    let normalized_task_ids = normalize_id_list(task_ids, "离线任务")?;
    let request = json!({ "taskIds": &normalized_task_ids });
    let (token, device_id) = auth_context(state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v2/delete_task",
        request,
        &[],
    )
    .await?;
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    remove_offline_name_restores(&db_path, &normalized_task_ids)?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn delete_offline_tasks(
    state: tauri::State<'_, SharedState>,
    task_ids: Vec<String>,
) -> Result<Value, String> {
    delete_offline_task_records(&state, &task_ids).await
}

#[tauri::command]
pub(crate) async fn cancel_offline_tasks(
    state: tauri::State<'_, SharedState>,
    task_ids: Vec<String>,
) -> Result<Value, String> {
    // The official PC client uses v2/delete_task for both cancelling active
    // tasks and removing completed task records.
    delete_offline_task_records(&state, &task_ids).await
}

#[tauri::command]
pub(crate) async fn retry_offline_tasks(
    state: tauri::State<'_, SharedState>,
    task_ids: Vec<String>,
) -> Result<Value, String> {
    let request = offline_task_ids_request(&task_ids)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v2/retry_task",
        request,
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn get_offline_statistics(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/nd.bizcloudcollection.s/v1/get_task_statistics",
        json!({}),
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}
