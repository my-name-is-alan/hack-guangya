//! 文件管理命令：列表、搜索、详情、复制/移动/删除/重命名。

use crate::prelude::*;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameRequest {
    pub(crate) file_id: String,
    pub(crate) current_name: String,
    pub(crate) new_name: String,
}


pub(crate) fn normalize_search_file_type(value: Option<&str>) -> Result<Option<String>, String> {
    let normalized = value.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "all" {
        return Ok(None);
    }
    if ["image", "video", "audio", "document", "archive", "folder"].contains(&normalized.as_str()) {
        Ok(Some(normalized))
    } else {
        Err("文件类型只支持 image、video、audio、document、archive 或 folder".to_string())
    }
}
pub(crate) fn normalize_search_extension(value: Option<&str>) -> Option<String> {
    let normalized = value
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}
pub(crate) fn cloud_item_is_folder(item: &Value) -> bool {
    item.get("resType").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.parse::<i64>().ok())
    }) == Some(2)
        || item
            .get("isFolder")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}
pub(crate) fn cloud_item_extension(item: &Value) -> String {
    let explicit = ["fileSuffix", "extension", "ext"]
        .iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if !explicit.is_empty() {
        return explicit;
    }
    ["fileName", "name"]
        .iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .unwrap_or_default()
        .to_ascii_lowercase()
}
pub(crate) fn cloud_item_matches_search_filters(
    item: &Value,
    file_type: Option<&str>,
    extension: Option<&str>,
) -> bool {
    let is_folder = cloud_item_is_folder(item);
    let item_extension = cloud_item_extension(item);
    let type_matches = match file_type {
        None => true,
        Some("folder") => is_folder,
        Some("image") => !is_folder && IMAGE_EXTENSIONS.contains(&item_extension.as_str()),
        Some("video") => !is_folder && VIDEO_EXTENSIONS.contains(&item_extension.as_str()),
        Some("audio") => !is_folder && AUDIO_EXTENSIONS.contains(&item_extension.as_str()),
        Some("document") => !is_folder && DOCUMENT_EXTENSIONS.contains(&item_extension.as_str()),
        Some("archive") => !is_folder && ARCHIVE_EXTENSIONS.contains(&item_extension.as_str()),
        Some(_) => false,
    };
    type_matches && extension.is_none_or(|expected| !is_folder && item_extension == expected)
}
pub(crate) fn cloud_search_file_type(file_type: Option<&str>, extension: Option<&str>) -> Option<u8> {
    match file_type {
        Some("image") => Some(CLOUD_FILE_TYPE_IMAGE),
        Some("video") => Some(CLOUD_FILE_TYPE_VIDEO),
        Some("audio") => Some(CLOUD_FILE_TYPE_AUDIO),
        Some("document") => Some(CLOUD_FILE_TYPE_DOCUMENT),
        Some("archive") => Some(CLOUD_FILE_TYPE_ARCHIVE),
        Some("folder") => None,
        _ => extension.and_then(|extension| {
            if IMAGE_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_IMAGE)
            } else if VIDEO_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_VIDEO)
            } else if AUDIO_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_AUDIO)
            } else if DOCUMENT_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_DOCUMENT)
            } else if ARCHIVE_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_ARCHIVE)
            } else {
                None
            }
        }),
    }
}
pub(crate) fn cloud_search_request(
    query: &str,
    file_type: Option<&str>,
    extension: Option<&str>,
    page: u64,
) -> (&'static str, Value) {
    let query = query.trim();
    if !query.is_empty() {
        return (
            "/userres/v1/file/search_files",
            json!({ "name": query, "pageSize": 100, "page": page }),
        );
    }

    let mut request = json!({
        "parentId": "*",
        "pageSize": 100,
        "page": page,
        "orderBy": 3,
        "sortType": 1,
        "resType": if file_type == Some("folder") { 2 } else { 1 }
    });
    if let Some(file_type) = cloud_search_file_type(file_type, extension) {
        request["fileTypes"] = json!([file_type]);
    }
    ("/userres/v1/file/get_file_list", request)
}

pub(crate) fn paginate_filtered_search_results(
    matches: Vec<Value>,
    page: u64,
    page_size: usize,
    remote_exhausted: bool,
) -> (Vec<Value>, u64) {
    let offset = usize::try_from(page)
        .unwrap_or(usize::MAX)
        .saturating_mul(page_size);
    let visible_total = if remote_exhausted {
        matches.len()
    } else {
        matches
            .len()
            .min(offset.saturating_add(page_size).saturating_add(1))
    };
    let list = matches.into_iter().skip(offset).take(page_size).collect();
    (list, u64::try_from(visible_total).unwrap_or(u64::MAX))
}


pub(crate) fn create_folder_request(
    parent_id: &str,
    dir_name: &str,
    fail_if_name_exist: Option<bool>,
) -> Result<Value, String> {
    let parent_id = normalize_parent_id(parent_id)?;
    let dir_name = normalize_remote_name(dir_name)?;
    let mut request = json!({ "parentId": parent_id, "dirName": dir_name });
    if let Some(fail_if_name_exist) = fail_if_name_exist {
        request
            .as_object_mut()
            .expect("create folder request must be an object")
            .insert("failIfNameExist".to_string(), json!(fail_if_name_exist));
    }
    Ok(request)
}

pub(crate) fn file_detail_request(file_id: &str) -> Result<Value, String> {
    Ok(json!({ "fileId": normalize_api_id(file_id, "文件 ID")? }))
}

pub(crate) fn normalize_file_type_filter(values: Option<&[i64]>) -> Result<Option<Vec<i64>>, String> {
    let Some(values) = values else {
        return Ok(None);
    };
    if values.len() > 12 || values.iter().any(|value| !(0..=11).contains(value)) {
        return Err("文件类型只能包含 0–11".into());
    }
    let mut seen = HashSet::new();
    let values = values
        .iter()
        .copied()
        .filter(|value| seen.insert(*value))
        .collect::<Vec<_>>();
    Ok((!values.is_empty()).then_some(values))
}

pub(crate) fn recent_actions_request(
    cursor: Option<&str>,
    page_size: Option<u64>,
    file_types: Option<&[i64]>,
    exclude_file_types: Option<&[i64]>,
) -> Result<Value, String> {
    let mut request = json!({
        "cursor": normalize_api_cursor(cursor)?.unwrap_or_default(),
        "pageSize": normalize_api_page_size(page_size, DEFAULT_RECENT_PAGE_SIZE)?
    });
    if let Some(file_types) = normalize_file_type_filter(file_types)? {
        request
            .as_object_mut()
            .expect("recent actions request must be an object")
            .insert("fileTypes".to_string(), json!(file_types));
    }
    if let Some(file_types) = normalize_file_type_filter(exclude_file_types)? {
        request
            .as_object_mut()
            .expect("recent actions request must be an object")
            .insert("excludeFileTypes".to_string(), json!(file_types));
    }
    Ok(request)
}


#[tauri::command]
pub(crate) async fn create_folder(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    parent_id: String,
    dir_name: String,
    fail_if_name_exist: Option<bool>,
) -> Result<Value, String> {
    let request = create_folder_request(&parent_id, &dir_name, fail_if_name_exist)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/create_dir",
        request,
        &[],
    )
    .await?;
    let data = finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        &app,
        state.inner(),
        [parent_id],
        &[],
        false,
        "desktop-create-folder",
    );
    Ok(data)
}

#[tauri::command]
pub(crate) async fn get_file_detail(
    state: tauri::State<'_, SharedState>,
    file_id: String,
) -> Result<Value, String> {
    let request = file_detail_request(&file_id)?;
    let (token, device_id) = auth_context(&state)?;
    let primary = api_post(
        &token,
        &device_id,
        "/userres/v1/file/get_file_detail",
        request.clone(),
        &[],
    )
    .await;
    match primary {
        Ok(response) => response
            .data
            .ok_or_else(|| "光鸭没有返回文件详情".to_string()),
        Err(primary_error) => {
            developer_file_read_fallback(
                &state,
                "/userres/v1/file/get_file_detail",
                request,
                primary_error,
            )
            .await
        }
    }
}

#[tauri::command]
pub(crate) async fn list_recent_actions(
    state: tauri::State<'_, SharedState>,
    cursor: Option<String>,
    page_size: Option<u64>,
    file_types: Option<Vec<i64>>,
    exclude_file_types: Option<Vec<i64>>,
) -> Result<Value, String> {
    let request = recent_actions_request(
        cursor.as_deref(),
        page_size,
        file_types.as_deref(),
        exclude_file_types.as_deref(),
    )?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_user_action",
        request,
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({ "list": [] })))
}

#[tauri::command]
pub(crate) async fn list_files(
    state: tauri::State<'_, SharedState>,
    parent_id: String,
    page: u64,
    folders_only: Option<bool>,
    force_refresh: Option<bool>,
) -> Result<Value, String> {
    if force_refresh.unwrap_or(false) {
        invalidate_remote_directory_cache(state.inner());
        webdav::invalidate_directory_cache(&parent_id);
    }
    let (token, device_id) = auth_context(&state)?;
    let request = file_list_request(&parent_id, page, folders_only.unwrap_or(false));
    let primary = tokio::time::timeout(
        Duration::from_secs(FILE_LIST_REQUEST_TIMEOUT_SECS),
        api_post(
            &token,
            &device_id,
            "/userres/v1/file/get_file_list",
            request.clone(),
            &[],
        ),
    )
    .await;
    let data = match primary {
        Ok(Ok(response)) => response
            .data
            .unwrap_or_else(|| json!({ "list": [], "total": 0 })),
        Ok(Err(primary_error)) => {
            developer_file_read_fallback(
                &state,
                "/userres/v1/file/get_file_list",
                request,
                primary_error,
            )
            .await?
        }
        Err(_) => {
            developer_file_read_fallback(
                &state,
                "/userres/v1/file/get_file_list",
                request,
                "文件目录加载超过 12 秒，请重试".to_string(),
            )
            .await?
        }
    };
    reconcile_remote_directory_cache_page(state.inner(), &parent_id, page, &data);
    // UI 刚从上游读取了这个目录：完整快照直接覆写挂载端缓存，分页快照只
    // 标脏。不再使用 invalidate（它会递增 generation，打断并发 PROPFIND
    // 并把该目录踢出后台预热，详见 webdav::mark_stale 的说明）。
    webdav::refresh_directory_cache_from_listing(&parent_id, page, &data);
    Ok(data)
}

pub(crate) fn file_list_request(parent_id: &str, page: u64, folders_only: bool) -> Value {
    let mut request = json!({
        "page": page,
        "pageSize": 100,
        "parentId": parent_id,
        "orderBy": 0,
        "sortType": 0
    });
    if folders_only {
        request
            .as_object_mut()
            .expect("file list request must be an object")
            .insert("resType".to_string(), json!(2));
    }
    request
}


#[tauri::command]
pub(crate) async fn search_files(
    state: tauri::State<'_, SharedState>,
    query: String,
    file_type: Option<String>,
    extension: Option<String>,
    page: Option<u64>,
) -> Result<Value, String> {
    let file_type = normalize_search_file_type(file_type.as_deref())?;
    let extension = normalize_search_extension(extension.as_deref());
    let (token, device_id) = auth_context(&state)?;
    let page = page.unwrap_or(0);
    const PAGE_SIZE: usize = 100;
    let has_local_filter = file_type.is_some() || extension.is_some();
    if !has_local_filter {
        let (endpoint, request) = cloud_search_request(&query, None, None, page);
        let response = api_post(&token, &device_id, endpoint, request, &[]).await?;
        let data = response
            .data
            .unwrap_or_else(|| json!({ "list": [], "total": 0 }));
        let total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Ok(json!({
            "remote_count": list.len(),
            "remote_total": total,
            "list": list,
            "total": total,
            "page": page,
            "page_size": PAGE_SIZE,
        }));
    }

    let required_matches = usize::try_from(page)
        .unwrap_or(usize::MAX)
        .saturating_mul(PAGE_SIZE)
        .saturating_add(PAGE_SIZE)
        .saturating_add(1);
    let mut remote_page = 0_u64;
    let mut remote_total = 0_u64;
    let mut remote_count = 0_u64;
    let mut matches = Vec::new();
    let remote_exhausted = loop {
        let (endpoint, request) = cloud_search_request(
            &query,
            file_type.as_deref(),
            extension.as_deref(),
            remote_page,
        );
        let response = api_post(&token, &device_id, endpoint, request, &[]).await?;
        let data = response
            .data
            .unwrap_or_else(|| json!({ "list": [], "total": 0 }));
        let source_total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        remote_total = remote_total.max(source_total);
        let source_list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let source_count = source_list.len();
        remote_count = remote_count.saturating_add(source_count as u64);
        matches.extend(source_list.into_iter().filter(|item| {
            cloud_item_matches_search_filters(item, file_type.as_deref(), extension.as_deref())
        }));
        let exhausted =
            source_count < PAGE_SIZE || (remote_total > 0 && remote_count >= remote_total);
        if matches.len() >= required_matches || exhausted {
            break exhausted;
        }
        remote_page = remote_page.saturating_add(1);
    };
    let (list, total) =
        paginate_filtered_search_results(matches, page, PAGE_SIZE, remote_exhausted);
    Ok(json!({
        "list": list,
        "total": total,
        "remote_total": remote_total,
        "remote_count": remote_count,
        "page": page,
        "page_size": PAGE_SIZE
    }))
}


pub(crate) async fn rename_remote(
    token: &str,
    device_id: &str,
    file_id: &str,
    new_name: &str,
) -> Result<(), String> {
    api_post(
        token,
        device_id,
        "/userres/v1/file/rename",
        json!({ "fileId": file_id, "newName": new_name }),
        &[],
    )
    .await?;
    Ok(())
}


pub(crate) fn normalize_api_id(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label}不能为空"));
    }
    if value.chars().count() > MAX_API_ID_LENGTH || value.chars().any(char::is_control) {
        return Err(format!("{label}格式无效"));
    }
    Ok(value.to_string())
}

pub(crate) fn normalize_parent_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().count() > MAX_API_ID_LENGTH || value.chars().any(char::is_control) {
        return Err("父目录 ID 格式无效".into());
    }
    Ok(value.to_string())
}

pub(crate) fn normalize_id_list(values: &[String], label: &str) -> Result<Vec<String>, String> {
    if values.is_empty() {
        return Err(format!("请至少选择一个{label}"));
    }
    if values.len() > MAX_API_ID_BATCH {
        return Err(format!("单次最多操作 {MAX_API_ID_BATCH} 个{label}"));
    }
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = normalize_api_id(value, &format!("{label} ID"))?;
        if seen.insert(value.clone()) {
            normalized.push(value);
        }
    }
    if normalized.is_empty() {
        return Err(format!("请至少选择一个{label}"));
    }
    Ok(normalized)
}

pub(crate) fn normalize_api_cursor(cursor: Option<&str>) -> Result<Option<String>, String> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    if cursor.chars().count() > MAX_API_CURSOR_LENGTH || cursor.chars().any(char::is_control) {
        return Err("分页游标格式无效".into());
    }
    Ok(Some(cursor.to_string()))
}

pub(crate) fn normalize_api_page_size(page_size: Option<u64>, default: u64) -> Result<u64, String> {
    let page_size = page_size.unwrap_or(default);
    if !(1..=MAX_API_PAGE_SIZE).contains(&page_size) {
        return Err(format!("每页数量必须在 1–{MAX_API_PAGE_SIZE} 之间"));
    }
    Ok(page_size)
}

pub(crate) fn normalize_remote_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("文件夹名称不能为空".into());
    }
    if value.chars().count() > MAX_REMOTE_NAME_LENGTH
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|character| character.is_control() || "\\/:*?\"<>|".contains(character))
    {
        return Err("文件夹名称格式无效".into());
    }
    Ok(value.to_string())
}


#[tauri::command]
pub(crate) async fn copy_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let parent_id = normalize_parent_id(&parent_id)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/copy_file",
        json!({ "fileIds": file_ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    let result = finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(&app, state.inner(), [parent_id], &[], false, "desktop-copy");
    Ok(result)
}

#[tauri::command]
pub(crate) async fn move_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let parent_id = normalize_parent_id(&parent_id)?;
    let affected_ids = file_ids.clone();
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/move_file",
        json!({ "fileIds": file_ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    let result = finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        &app,
        state.inner(),
        [parent_id],
        &affected_ids,
        true,
        "desktop-move",
    );
    Ok(result)
}

#[tauri::command]
pub(crate) async fn delete_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let affected_ids = file_ids.clone();
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": file_ids }),
        &[],
    )
    .await?;
    let result = finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        &app,
        state.inner(),
        Vec::new(),
        &affected_ids,
        true,
        "desktop-delete",
    );
    publish_recycle_bin_changed(&app, "desktop-delete");
    Ok(result)
}

#[tauri::command]
pub(crate) async fn restore_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/recycle_file",
        json!({ "fileIds": file_ids }),
        &[],
    )
    .await?;
    let result = finish_operation_response(&token, &device_id, response).await?;
    publish_all_cloud_directories_changed(&app, state.inner(), "desktop-restore");
    publish_recycle_bin_changed(&app, "desktop-restore");
    Ok(result)
}

#[tauri::command]
pub(crate) async fn permanently_delete_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let affected_ids = file_ids.clone();
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": file_ids }),
        &[],
    )
    .await?;
    let result = finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        &app,
        state.inner(),
        Vec::new(),
        &affected_ids,
        true,
        "desktop-permanent-delete",
    );
    publish_recycle_bin_changed(&app, "desktop-permanent-delete");
    Ok(result)
}


#[tauri::command]
pub(crate) async fn batch_rename_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    renames: Vec<RenameRequest>,
) -> Result<Value, String> {
    let mut seen = HashSet::new();
    let renames = renames
        .into_iter()
        .filter(|item| item.current_name != item.new_name)
        .collect::<Vec<_>>();
    if renames.is_empty() {
        return Err("没有需要重命名的项目".into());
    }
    let affected_ids = renames
        .iter()
        .map(|item| item.file_id.clone())
        .collect::<Vec<_>>();
    for item in &renames {
        let name = item.new_name.trim();
        if name.is_empty() || name.chars().any(|value| "\\/:*?\"<>|".contains(value)) {
            return Err(format!("无效的文件名：{}", item.new_name));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("存在重复目标名称：{name}"));
        }
    }
    let (token, device_id) = auth_context(&state)?;
    let staged = renames
        .iter()
        .enumerate()
        .map(|(index, item)| {
            (
                item.clone(),
                format!(".__gy_tmp_{}_{}", Uuid::new_v4().simple(), index),
            )
        })
        .collect::<Vec<_>>();
    let mut staged_count = 0usize;
    for (item, temporary) in &staged {
        if let Err(error) = rename_remote(&token, &device_id, &item.file_id, temporary).await {
            for (rollback, _) in staged[..staged_count].iter().rev() {
                let _ = rename_remote(
                    &token,
                    &device_id,
                    &rollback.file_id,
                    &rollback.current_name,
                )
                .await;
            }
            return Err(format!("暂存重命名失败（{}）：{error}", item.current_name));
        }
        staged_count += 1;
    }
    for (index, (item, _)) in staged.iter().enumerate() {
        if let Err(error) = rename_remote(&token, &device_id, &item.file_id, &item.new_name).await {
            for (rollback, _) in staged[..index].iter().rev() {
                let _ = rename_remote(
                    &token,
                    &device_id,
                    &rollback.file_id,
                    &rollback.current_name,
                )
                .await;
            }
            for (rollback, _) in staged[index..].iter().rev() {
                let _ = rename_remote(
                    &token,
                    &device_id,
                    &rollback.file_id,
                    &rollback.current_name,
                )
                .await;
            }
            return Err(format!("目标重命名失败（{}）：{error}", item.new_name));
        }
    }
    publish_cloud_mutation(
        &app,
        state.inner(),
        Vec::new(),
        &affected_ids,
        true,
        "desktop-rename",
    );
    Ok(json!({ "renamed": staged.len() }))
}
