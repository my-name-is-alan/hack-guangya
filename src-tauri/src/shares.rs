//! 分享：创建、管理、直链、接收分享与收藏。

use crate::prelude::*;

pub(crate) fn parse_guangya_share_link(value: &str) -> Result<(String, String), String> {
    let text = value.trim();
    let candidate = text
        .split_whitespace()
        .find(|part| part.contains("guangyapan.com/s/"))
        .unwrap_or(text)
        .trim_matches(|character| "\"'<>，。；;".contains(character));
    let parsed = reqwest::Url::parse(candidate).map_err(|_| "请输入完整的光鸭分享链接")?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "guangyapan.com" && !host.ends_with(".guangyapan.com") {
        return Err("只支持 guangyapan.com 的分享链接".into());
    }
    let parts = parsed
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let share_id = parts
        .windows(2)
        .find(|parts| parts[0].eq_ignore_ascii_case("s"))
        .map(|parts| parts[1])
        .unwrap_or_default();
    if share_id.is_empty()
        || !share_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ['_', '-'].contains(&character))
    {
        return Err("光鸭分享链接中缺少有效的 share_id".into());
    }
    let code = parsed
        .query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("code"))
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    Ok((share_id.to_string(), code))
}


pub(crate) async fn fetch_received_share_files(
    token: &str,
    device_id: &str,
    access_token: &str,
    parent_id: &str,
) -> Result<Value, String> {
    if access_token.trim().is_empty() {
        return Err("分享访问令牌为空，请重新打开分享链接".into());
    }
    let mut items = Vec::new();
    let mut cursor = None;
    let mut total = 0_u64;
    for _ in 0..100 {
        let mut body = json!({
            "pageSize": 100,
            "accessToken": access_token,
            "orderBy": 0,
            "sortType": 0,
            "parentId": parent_id,
        });
        if let Some(value) = cursor {
            body["cursor"] = json!(value);
        }
        let response = api_post(
            token,
            device_id,
            "/userres/v1/get_share_page_files_list",
            body,
            &[],
        )
        .await?;
        let data = response.data.unwrap_or_else(|| json!({}));
        total = total.max(data.get("total").and_then(Value::as_u64).unwrap_or(0));
        let page = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_len = page.len();
        items.extend(page);
        let has_more = data
            .get("hasMore")
            .and_then(Value::as_bool)
            .unwrap_or(page_len == 100 && (total == 0 || items.len() < total as usize));
        if !has_more || page_len == 0 || (total > 0 && items.len() >= total as usize) {
            break;
        }
        let next_cursor = data
            .get("cursor")
            .and_then(Value::as_i64)
            .unwrap_or(items.len() as i64);
        if cursor == Some(next_cursor) {
            break;
        }
        cursor = Some(next_cursor);
    }
    total = total.max(items.len() as u64);
    Ok(json!({ "list": items, "total": total, "parentId": parent_id }))
}

pub(crate) async fn fetch_all_shares(token: &str, device_id: &str) -> Result<Value, String> {
    let mut items = Vec::new();
    let mut total = 0_u64;
    for page in 0..100 {
        let response = api_post(
            token,
            device_id,
            "/userres/v1/get_share_list",
            json!({ "page": page, "pageSize": 100, "orderType": 1, "sortType": 1 }),
            &[],
        )
        .await?;
        let data = response.data.unwrap_or_else(|| json!({}));
        total = total.max(data.get("total").and_then(Value::as_u64).unwrap_or(0));
        let current = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_len = current.len();
        items.extend(current);
        if page_len == 0 || page_len < 100 || (total > 0 && items.len() >= total as usize) {
            break;
        }
    }
    total = total.max(items.len() as u64);
    Ok(json!({ "list": items, "total": total }))
}

pub(crate) fn value_as_id(value: Option<&Value>) -> String {
    value
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|number| number.to_string()))
                .or_else(|| value.as_i64().map(|number| number.to_string()))
        })
        .unwrap_or_default()
}

pub(crate) async fn find_existing_share_for_files(
    token: &str,
    device_id: &str,
    file_ids: &[String],
) -> Result<Option<Value>, String> {
    let mut expected = file_ids.to_vec();
    expected.sort();
    expected.dedup();
    let shares = fetch_all_shares(token, device_id).await?;
    let items = shares
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        if item
            .get("shareStatus")
            .and_then(Value::as_i64)
            .is_some_and(|status| status != 1)
        {
            continue;
        }
        let share_url = item
            .get("shareUrl")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let share_id = {
            let from_url = share_id_from_url(share_url);
            if from_url.is_empty() {
                value_as_id(item.get("shareId"))
            } else {
                from_url
            }
        };
        if share_id.is_empty() {
            continue;
        }
        let code = item.get("code").and_then(Value::as_str).unwrap_or_default();
        let Ok(access) = api_post(
            token,
            device_id,
            "/userres/v1/get_share_access_token",
            json!({ "shareId": share_id, "code": code }),
            &[],
        )
        .await
        else {
            continue;
        };
        let access_token = access
            .data
            .as_ref()
            .and_then(|data| data.get("accessToken"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if access_token.is_empty() {
            continue;
        }
        let Ok(files) = fetch_received_share_files(token, device_id, access_token, "").await else {
            continue;
        };
        let mut actual = files
            .get("list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|file| value_as_id(file.get("fileId")))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        actual.sort();
        actual.dedup();
        if actual == expected {
            return Ok(Some(item));
        }
    }
    Ok(None)
}

pub(crate) fn normalize_share_traffic_limit(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty()
                || value.len() > 32
                || !value.chars().all(|value| value.is_ascii_digit())
            {
                return Err("分享流量限制必须是非负整数".into());
            }
            let value = value
                .parse::<u64>()
                .map_err(|_| "分享流量限制必须是非负整数".to_string())?;
            if value > MAX_SHARE_TRAFFIC_BYTES {
                return Err("分享流量限制最大为 1024 TB".into());
            }
            Ok(value.to_string())
        }
        Value::Number(value) => {
            let value = value
                .as_u64()
                .ok_or_else(|| "分享流量限制必须是非负整数".to_string())?;
            if value > MAX_SHARE_TRAFFIC_BYTES {
                return Err("分享流量限制最大为 1024 TB".into());
            }
            Ok(value.to_string())
        }
        _ => Err("分享流量限制必须是数字或十进制字符串".into()),
    }
}

pub(crate) fn update_share_request(
    id: &str,
    validate_duration: i64,
    download_type: i64,
    traffic_limit: &Value,
) -> Result<Value, String> {
    let id = normalize_api_id(id, "分享 ID")?;
    if !matches!(validate_duration, 0 | 86_400 | 604_800 | 2_592_000) {
        return Err("分享有效期必须是永久、1 天、7 天或 30 天".into());
    }
    if !matches!(download_type, 0 | 1) {
        return Err("分享下载类型必须是 0 或 1".into());
    }
    Ok(json!({
        "id": id,
        "validateDuration": validate_duration,
        "downloadType": download_type,
        "trafficLimit": normalize_share_traffic_limit(traffic_limit)?
    }))
}

pub(crate) fn direct_link_file_request(file_id: &str) -> Result<Value, String> {
    Ok(json!({ "fileId": normalize_api_id(file_id, "文件 ID")? }))
}

pub(crate) fn get_direct_link_request(file_id: &str, short_link: bool) -> Result<Value, String> {
    Ok(json!({
        "fileId": normalize_api_id(file_id, "文件 ID")?,
        "shortLink": short_link
    }))
}

pub(crate) fn delete_shares_request(ids: &[String]) -> Result<Value, String> {
    Ok(json!({ "ids": normalize_id_list(ids, "分享")? }))
}

#[tauri::command]
pub(crate) async fn list_shares(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    fetch_all_shares(&token, &device_id).await
}

#[tauri::command]
pub(crate) async fn delete_shares(
    state: tauri::State<'_, SharedState>,
    ids: Vec<String>,
) -> Result<Value, String> {
    let request = delete_shares_request(&ids)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(&token, &device_id, "/userres/v1/delete_share", request, &[]).await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn update_share(
    state: tauri::State<'_, SharedState>,
    id: String,
    validate_duration: i64,
    download_type: i64,
    traffic_limit: Value,
) -> Result<Value, String> {
    let request = update_share_request(&id, validate_duration, download_type, &traffic_limit)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(&token, &device_id, "/userres/v1/update_share", request, &[]).await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn delete_invalid_shares(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/delete_invalid_share",
        json!({}),
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn set_direct_link(
    state: tauri::State<'_, SharedState>,
    file_id: String,
) -> Result<Value, String> {
    let request = direct_link_file_request(&file_id)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/set_direct_link",
        request,
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn unset_direct_link(
    state: tauri::State<'_, SharedState>,
    file_id: String,
) -> Result<Value, String> {
    let request = direct_link_file_request(&file_id)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/unset_direct_link",
        request,
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn get_direct_link(
    state: tauri::State<'_, SharedState>,
    file_id: String,
    short_link: Option<bool>,
) -> Result<Value, String> {
    let request = get_direct_link_request(&file_id, short_link.unwrap_or(false))?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_direct_link",
        request,
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn open_received_share(
    state: tauri::State<'_, SharedState>,
    url: String,
) -> Result<Value, String> {
    let (share_id, code) = parse_guangya_share_link(&url)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_share_access_token",
        json!({ "shareId": share_id, "code": code }),
        &[],
    )
    .await?;
    let access_token = response
        .data
        .as_ref()
        .and_then(|data| data.get("accessToken"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "光鸭没有返回分享访问令牌".to_string())?
        .to_string();
    let files = fetch_received_share_files(&token, &device_id, &access_token, "").await?;
    Ok(json!({
        "share_id": share_id,
        "code": code,
        "access_token": access_token,
        "files": files,
    }))
}

#[tauri::command]
pub(crate) async fn list_received_share_files(
    state: tauri::State<'_, SharedState>,
    access_token: String,
    parent_id: String,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    fetch_received_share_files(&token, &device_id, &access_token, &parent_id).await
}

#[tauri::command]
pub(crate) async fn restore_received_share(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    access_token: String,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let parent_id = normalize_parent_id(&parent_id)?;
    if access_token.trim().is_empty() {
        return Err("分享访问令牌为空，请重新打开分享链接".into());
    }
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/restore_share",
        json!({ "accessToken": access_token, "fileIds": file_ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    let data = response.data.unwrap_or_else(|| json!({}));
    if let Some(task_id) = data.get("taskId").and_then(Value::as_str) {
        wait_operation_task(&token, &device_id, task_id).await?;
    }
    publish_cloud_mutation(
        &app,
        state.inner(),
        [parent_id],
        &[],
        false,
        "desktop-restore-share",
    );
    Ok(data)
}


#[tauri::command]
pub(crate) async fn create_share(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    title: String,
    target_type: Option<String>,
    share_type: Option<u8>,
    code: Option<String>,
    auto_fill_code: Option<bool>,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let title = title.trim();
    let title = if title.is_empty() {
        "云盘分享".to_string()
    } else {
        title.to_string()
    };
    let (token, device_id) = auth_context(&state)?;
    let (share_type, code, auto_fill_code) =
        normalize_share_access(share_type, code.as_deref(), auto_fill_code)?;
    // 光鸭分享不是不可变快照，而是依赖当前云端资源关系；移动、删除
    // 或覆盖后旧链接可能失效。因此手动分享始终创建当前资源的新链接。
    let reused_existing = false;
    let mut data = api_post(
        &token,
        &device_id,
        "/userres/v1/share_file",
        share_file_payload(&file_ids, &title, share_type, &code, auto_fill_code),
        &[],
    )
    .await?
    .data
    .ok_or_else(|| "光鸭没有返回分享信息".to_string())?;
    let share_url = ["shareUrl", "shareURL", "share_url", "url"]
        .iter()
        .find_map(|key| data.get(key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let share_id = share_id_for_hdhive(&data, &share_url);
    if share_url.is_empty() || share_id.is_empty() {
        return Err("光鸭没有返回完整分享链接".to_string());
    }

    let event_id = Uuid::new_v4().to_string();
    let target_type = target_type
        .as_deref()
        .filter(|value| *value == "folder")
        .unwrap_or("file")
        .to_string();
    let payload = manual_share_event_payload(
        &event_id,
        &file_ids,
        &title,
        &target_type,
        &share_id,
        &share_url,
        if reused_existing { "update" } else { "new" },
    );
    let (hdhive_enabled, base_url, secret, instance_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard.hdhive_enabled,
            guard.hdhive_base_url.clone(),
            guard.hdhive_secret.clone(),
            guard.hdhive_instance_id.clone(),
            guard.db_path.clone(),
        )
    };
    let proxy = load_global_network_proxy(&db_path)?;
    let mapping_id = "__manual__";
    if hdhive_enabled {
        let _ = save_auto_share_event(
            &db_path,
            &event_id,
            mapping_id,
            &title,
            Some(&share_url),
            "sending",
            None,
            Some(if reused_existing {
                "已复用光鸭分享，正在提交影巢更新"
            } else {
                "光鸭分享成功，正在提交影巢"
            }),
            None,
            &payload,
        );
    }
    let (hdhive_status, hdhive_message) = if !hdhive_enabled {
        (
            "disabled".to_string(),
            "HDHive 已关闭，仅创建光鸭分享".to_string(),
        )
    } else {
        match hdhive_request(
            &base_url,
            &secret,
            &instance_id,
            &proxy,
            reqwest::Method::POST,
            &["api", "integrations", "guangya-sync", "events"],
            Some(&payload),
        )
        .await
        {
            Ok(accepted) => {
                let hdhive_status = accepted
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("accepted")
                    .to_string();
                let hdhive_message = if reused_existing {
                    "影巢已接收，正在更新备注".to_string()
                } else {
                    "影巢已接收，正在解析并投稿".to_string()
                };
                let _ = save_auto_share_event(
                    &db_path,
                    &event_id,
                    mapping_id,
                    &title,
                    Some(&share_url),
                    &hdhive_status,
                    None,
                    Some(&hdhive_message),
                    None,
                    &payload,
                );
                let pending = PendingAutoShare {
                    mapping_id: mapping_id.to_string(),
                    target_key: title.clone(),
                    target_type,
                    title: title.clone(),
                    remote_target_id: file_ids[0].clone(),
                    added: HashSet::new(),
                    changed: HashSet::new(),
                    event_id: event_id.clone(),
                    retry_count: 0,
                };
                tauri::async_runtime::spawn(poll_hdhive_receipt(
                    app.clone(),
                    state.inner().clone(),
                    pending,
                    share_url.clone(),
                    payload.clone(),
                ));
                (hdhive_status, hdhive_message)
            }
            Err(error) => {
                let hdhive_status = "delivery_failed".to_string();
                let hdhive_message = format!("光鸭分享成功，但提交影巢失败：{error}");
                let _ = save_auto_share_event(
                    &db_path,
                    &event_id,
                    mapping_id,
                    &title,
                    Some(&share_url),
                    &hdhive_status,
                    None,
                    Some(&hdhive_message),
                    None,
                    &payload,
                );
                (hdhive_status, hdhive_message)
            }
        }
    };
    emit_state(&app, state.inner());
    if let Some(object) = data.as_object_mut() {
        object.insert("reused_existing".to_string(), json!(reused_existing));
        object.insert("share_id".to_string(), json!(share_id));
        object.insert("share_url".to_string(), json!(share_url));
        object.insert("hdhive_event_id".to_string(), json!(event_id));
        object.insert("hdhive_status".to_string(), json!(hdhive_status));
        object.insert("hdhive_message".to_string(), json!(hdhive_message));
    }
    Ok(data)
}


#[tauri::command]
pub(crate) fn save_share_link(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    label: String,
    url: String,
) -> Result<SavedShare, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("分享链接必须以 http:// 或 https:// 开头".into());
    }
    let saved = SavedShare {
        id: Uuid::new_v4().to_string(),
        label: if label.trim().is_empty() {
            "未命名分享".into()
        } else {
            label.trim().to_string()
        },
        url,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0),
    };
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.saved_shares.insert(0, saved.clone());
        save_config(&guard);
    }
    emit_state(&app, state.inner());
    Ok(saved)
}

#[tauri::command]
pub(crate) fn remove_share_link(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.saved_shares.retain(|item| item.id != id);
    save_config(&guard);
    drop(guard);
    emit_state(&app, state.inner());
    Ok(())
}
