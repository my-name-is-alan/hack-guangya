//! 上传后自动分享与 HDHive 通知。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct HdhivePublicConfig {
    pub(crate) enabled: bool,
    pub(crate) configured: bool,
    pub(crate) base_url: String,
    pub(crate) instance_id: String,
}


#[derive(Debug, Clone, Serialize)]
pub(crate) struct AutoShareReceipt {
    pub(crate) event_id: String,
    pub(crate) mapping_id: String,
    pub(crate) target_key: String,
    pub(crate) share_url: Option<String>,
    pub(crate) status: String,
    pub(crate) action: Option<String>,
    pub(crate) error_code: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) resource_url: Option<String>,
    pub(crate) notification_status: Option<String>,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct AutoShareTarget {
    pub(crate) key: String,
    pub(crate) target_type: String,
    pub(crate) title: String,
    pub(crate) relative_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingAutoShare {
    pub(crate) mapping_id: String,
    pub(crate) target_key: String,
    pub(crate) target_type: String,
    pub(crate) title: String,
    pub(crate) remote_target_id: String,
    pub(crate) added: HashSet<String>,
    pub(crate) changed: HashSet<String>,
    pub(crate) event_id: String,
    pub(crate) retry_count: i64,
}


pub(crate) fn default_hdhive_enabled() -> bool {
    true
}
pub(crate) fn parse_hdhive_enabled(value: Option<&str>) -> bool {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("false" | "0" | "off" | "disabled") => false,
        Some("true" | "1" | "on" | "enabled") => true,
        _ => default_hdhive_enabled(),
    }
}
pub(crate) fn hdhive_allowed_hosts() -> HashSet<String> {
    std::env::var("HDHIVE_ALLOWED_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}
pub(crate) fn normalize_hdhive_base_url_with_allowed_hosts(
    value: &str,
    allowed_hosts: &HashSet<String>,
) -> Result<String, String> {
    let input = value.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    let mut parsed = reqwest::Url::parse(input)
        .map_err(|_| "Hdhive 地址必须是完整的 HTTP(S) URL".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Hdhive 地址必须使用 HTTP 或 HTTPS".to_string());
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Hdhive 地址不能包含账号、查询参数或片段".to_string());
    }
    let raw_hostname = parsed
        .host_str()
        .ok_or_else(|| "Hdhive 地址必须包含主机名".to_string())?;
    let hostname = raw_hostname.to_ascii_lowercase();
    let host = parsed
        .port()
        .map(|port| format!("{hostname}:{port}"))
        .unwrap_or_else(|| hostname.clone());
    if !allowed_hosts.is_empty()
        && !allowed_hosts.contains(&host)
        && !allowed_hosts.contains(&hostname)
    {
        return Err("Hdhive 地址不在 HDHIVE_ALLOWED_HOSTS 允许列表中".to_string());
    }
    let normalized_path = parsed.path().trim_end_matches('/').to_string();
    parsed.set_path(&normalized_path);
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}
pub(crate) fn normalize_hdhive_base_url(value: &str) -> Result<String, String> {
    normalize_hdhive_base_url_with_allowed_hosts(value, &hdhive_allowed_hosts())
}
pub(crate) fn build_hdhive_target_url(
    base_url: &str,
    path_segments: &[&str],
) -> Result<(reqwest::Url, String), String> {
    if path_segments.is_empty()
        || path_segments.iter().any(|segment| {
            segment.is_empty()
                || *segment == "."
                || *segment == ".."
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err("Hdhive 请求路径无效".to_string());
    }
    let mut target = reqwest::Url::parse(base_url)
        .map_err(|_| "Hdhive 地址必须是完整的 HTTP(S) URL".to_string())?;
    target
        .path_segments_mut()
        .map_err(|_| "Hdhive 地址不能作为 API 基地址".to_string())?
        .pop_if_empty()
        .extend(path_segments.iter().copied());
    target.set_query(None);
    target.set_fragment(None);
    Ok((target, format!("/{}", path_segments.join("/"))))
}

pub(crate) fn load_auto_share_receipts(path: &Path) -> Result<Vec<AutoShareReceipt>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT event_id, mapping_id, target_key, share_url, status, action, error_code, message, resource_url, notification_status, updated_at
             FROM auto_share_events ORDER BY updated_at DESC LIMIT 50",
        )
        .map_err(|error| format!("读取自动分享回执失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(AutoShareReceipt {
                event_id: row.get(0)?,
                mapping_id: row.get(1)?,
                target_key: row.get(2)?,
                share_url: row.get(3)?,
                status: row.get(4)?,
                action: row.get(5)?,
                error_code: row.get(6)?,
                message: row.get(7)?,
                resource_url: row.get(8)?,
                notification_status: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|error| format!("读取自动分享回执失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析自动分享回执失败：{error}"))?;
    Ok(rows)
}

pub(crate) fn auto_share_target(item: &UploadItem) -> Option<AutoShareTarget> {
    if item.mapping_id.starts_with("__") {
        return None;
    }
    let parts = normalize_remote_path(&item.relative_path)
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let title = parts.first()?.clone();
    Some(AutoShareTarget {
        key: title.clone(),
        target_type: if parts.len() == 1 { "file" } else { "folder" }.to_string(),
        title,
        relative_path: parts.join("/"),
    })
}

pub(crate) fn reuse_auto_share_binding(
    path: &Path,
    current_mapping_id: &str,
    source_mapping_id: &str,
    target_key: &str,
) -> Result<bool, String> {
    let connection = open_database(path)?;
    let stored = connection
        .query_row(
            "SELECT target_type, remote_target_id, title, share_id, share_url
             FROM auto_share_targets
             WHERE target_key = ?1
               AND mapping_id IN (?2, ?3)
             ORDER BY CASE WHEN mapping_id = ?2 THEN 0 ELSE 1 END, updated_at DESC
             LIMIT 1",
            params![target_key, current_mapping_id, source_mapping_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取历史分享绑定失败：{error}"))?;
    let Some((target_type, remote_target_id, title, share_id, share_url)) = stored else {
        return Ok(false);
    };
    connection
        .execute(
            "INSERT INTO auto_share_targets
               (mapping_id, target_key, target_type, remote_target_id, title, share_id, share_url, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(mapping_id, target_key) DO UPDATE SET
               target_type = excluded.target_type,
               remote_target_id = excluded.remote_target_id,
               title = excluded.title,
               share_id = excluded.share_id,
               share_url = excluded.share_url,
               updated_at = excluded.updated_at",
            params![
                current_mapping_id,
                target_key,
                target_type,
                remote_target_id,
                title,
                share_id,
                share_url,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("迁移历史分享绑定失败：{error}"))?;
    Ok(true)
}

pub(crate) fn target_has_work(state: &RuntimeState, mapping_id: &str, target_key: &str) -> bool {
    state
        .queue
        .iter()
        .chain(state.inflight_items.values())
        .chain(state.waiting_files.values())
        .any(|item| {
            item.mapping_id == mapping_id
                && auto_share_target(item).is_some_and(|target| target.key == target_key)
        })
}

pub(crate) fn target_has_pending_cloud(
    database: &Path,
    mapping_id: &str,
    target_key: &str,
) -> Result<bool, String> {
    Ok(load_pending_uploads(database)?.iter().any(|pending| {
        pending.item.mapping_id == mapping_id
            && auto_share_target(&pending.item).is_some_and(|target| target.key == target_key)
    }))
}

pub(crate) fn save_auto_share_event(
    path: &Path,
    event_id: &str,
    mapping_id: &str,
    target_key: &str,
    share_url: Option<&str>,
    status: &str,
    action: Option<&str>,
    message: Option<&str>,
    resource_url: Option<&str>,
    payload: &Value,
) -> Result<(), String> {
    open_database(path)?
        .execute(
            "INSERT INTO auto_share_events
               (event_id, mapping_id, target_key, share_url, status, action, message, resource_url, payload, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(event_id) DO UPDATE SET share_url=excluded.share_url, status=excluded.status,
               action=excluded.action, error_code=NULL, message=excluded.message, resource_url=excluded.resource_url,
               payload=excluded.payload, updated_at=excluded.updated_at",
            params![
                event_id,
                mapping_id,
                target_key,
                share_url,
                status,
                action,
                message,
                resource_url,
                payload.to_string(),
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存自动分享回执失败：{error}"))?;
    Ok(())
}

pub(crate) fn record_auto_share_failure(path: &Path, item: &UploadItem, message: &str) -> Result<(), String> {
    let Some(target) = auto_share_target(item) else {
        return Ok(());
    };
    open_database(path)?
        .execute(
            "INSERT INTO auto_share_failures (mapping_id, target_key, relative_path, error, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(mapping_id, target_key, relative_path) DO UPDATE SET error=excluded.error, updated_at=excluded.updated_at",
            params![item.mapping_id, target.key, target.relative_path, message, unix_timestamp()],
        )
        .map_err(|error| format!("记录自动分享上传失败状态失败：{error}"))?;
    Ok(())
}

pub(crate) fn clear_auto_share_failure(path: &Path, item: &UploadItem) -> Result<(), String> {
    let Some(target) = auto_share_target(item) else {
        return Ok(());
    };
    open_database(path)?
        .execute(
            "DELETE FROM auto_share_failures WHERE mapping_id=?1 AND target_key=?2 AND relative_path=?3",
            params![item.mapping_id, target.key, target.relative_path],
        )
        .map_err(|error| format!("清理自动分享上传失败状态失败：{error}"))?;
    Ok(())
}


pub(crate) fn load_due_auto_shares(path: &Path) -> Result<Vec<PendingAutoShare>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, target_key, target_type, title, remote_target_id,
                    added_paths, changed_paths, event_id, retry_count
             FROM auto_share_pending WHERE due_at <= ?1 ORDER BY due_at LIMIT 20",
        )
        .map_err(|error| format!("读取待分享任务失败：{error}"))?;
    let rows = statement
        .query_map(params![unix_timestamp()], |row| {
            let added_raw: String = row.get(5)?;
            let changed_raw: String = row.get(6)?;
            Ok(PendingAutoShare {
                mapping_id: row.get(0)?,
                target_key: row.get(1)?,
                target_type: row.get(2)?,
                title: row.get(3)?,
                remote_target_id: row.get(4)?,
                added: serde_json::from_str::<Vec<String>>(&added_raw)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                changed: serde_json::from_str::<Vec<String>>(&changed_raw)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                event_id: row.get(7)?,
                retry_count: row.get(8)?,
            })
        })
        .map_err(|error| format!("读取待分享任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析待分享任务失败：{error}"))?;
    Ok(rows)
}

pub(crate) fn reschedule_auto_share(
    path: &Path,
    pending: &PendingAutoShare,
    delay_secs: i64,
) -> Result<(), String> {
    open_database(path)?
        .execute(
            "UPDATE auto_share_pending SET retry_count=?1, due_at=?2, updated_at=?3
             WHERE mapping_id=?4 AND target_key=?5",
            params![
                pending.retry_count,
                unix_timestamp() + delay_secs,
                unix_timestamp(),
                pending.mapping_id,
                pending.target_key
            ],
        )
        .map_err(|error| format!("更新待分享任务失败：{error}"))?;
    Ok(())
}

pub(crate) fn persist_pending_auto_share(path: &Path, pending: &PendingAutoShare) -> Result<(), String> {
    let mut added = pending.added.iter().cloned().collect::<Vec<_>>();
    let mut changed = pending.changed.iter().cloned().collect::<Vec<_>>();
    added.sort();
    changed.sort();
    open_database(path)?
        .execute(
            "INSERT INTO auto_share_pending
               (mapping_id, target_key, target_type, title, remote_target_id, added_paths, changed_paths, event_id, retry_count, due_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(mapping_id, target_key) DO UPDATE SET target_type=excluded.target_type,
               title=excluded.title, remote_target_id=excluded.remote_target_id,
               added_paths=excluded.added_paths, changed_paths=excluded.changed_paths,
               retry_count=0, due_at=excluded.due_at, updated_at=excluded.updated_at",
            params![
                pending.mapping_id,
                pending.target_key,
                pending.target_type,
                pending.title,
                pending.remote_target_id,
                serde_json::to_string(&added).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&changed).unwrap_or_else(|_| "[]".to_string()),
                pending.event_id,
                pending.retry_count,
                unix_timestamp() + AUTO_SHARE_QUIET_SECS,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存待分享任务失败：{error}"))?;
    Ok(())
}

pub(crate) fn delete_pending_auto_share(
    path: &Path,
    mapping_id: &str,
    target_key: &str,
) -> Result<(), String> {
    open_database(path)?
        .execute(
            "DELETE FROM auto_share_pending WHERE mapping_id=?1 AND target_key=?2",
            params![mapping_id, target_key],
        )
        .map_err(|error| format!("清理待分享任务失败：{error}"))?;
    Ok(())
}

pub(crate) fn share_id_from_url(value: &str) -> String {
    value
        .split("/s/")
        .nth(1)
        .unwrap_or_default()
        .split(['?', '#', '/'])
        .next()
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn share_id_for_hdhive(data: &Value, share_url: &str) -> String {
    let url_share_id = share_id_from_url(share_url);
    if !url_share_id.is_empty() {
        return url_share_id;
    }
    ["shareCode", "share_code", "shareId", "shareID", "share_id"]
        .iter()
        .find_map(|key| {
            let value = data.get(key)?;
            value
                .as_str()
                .map(str::to_owned)
                .or_else(|| value.as_u64().map(|number| number.to_string()))
        })
        .unwrap_or_default()
}

pub(crate) const DEFAULT_SHARE_TEMPLATE: &str =
    "光鸭云盘用户给你分享了{{filename}}，点击链接或复制整段内容，打开「光鸭APP」即可获取。\n链接：{{link}}";

pub(crate) fn normalize_share_access(
    share_type: Option<u8>,
    code: Option<&str>,
    auto_fill_code: Option<bool>,
) -> Result<(u8, String, bool), String> {
    let share_type = share_type.unwrap_or(0);
    if !matches!(share_type, 0..=2) {
        return Err("访问码类型无效".into());
    }
    let code = code.unwrap_or_default().trim();
    if share_type == 2
        && (code.chars().count() != 4 || !code.chars().all(|value| value.is_ascii_alphanumeric()))
    {
        return Err("固定访问码必须是 4 位英文或数字".into());
    }
    Ok((
        share_type,
        if share_type == 2 {
            code.to_string()
        } else {
            String::new()
        },
        share_type != 0 && auto_fill_code.unwrap_or(false),
    ))
}

pub(crate) fn share_file_payload(
    file_ids: &[String],
    title: &str,
    share_type: u8,
    code: &str,
    auto_fill_code: bool,
) -> Value {
    let title = title.trim();
    let title = if title.is_empty() {
        "云盘分享"
    } else {
        title
    };
    json!({
        "fileIds": file_ids,
        "title": title,
        "validateDuration": 0,
        "shareType": share_type,
        "code": code,
        "autoFillCode": auto_fill_code,
        // 光鸭网页版的普通分享会同时提交下载限制和分享文案模板。
        "trafficLimit": "0",
        "maxRestoreCount": 0,
        "downloadType": 1,
        "shareTemplate": DEFAULT_SHARE_TEMPLATE
    })
}

pub(crate) fn manual_share_event_payload(
    event_id: &str,
    file_ids: &[String],
    title: &str,
    target_type: &str,
    share_id: &str,
    share_url: &str,
    intent: &str,
) -> Value {
    json!({
        "event_id": event_id,
        "mapping_id": "__manual__",
        "target_key": title,
        "target_type": if target_type == "folder" { "folder" } else { "file" },
        "remote_target_id": file_ids.first().cloned().unwrap_or_default(),
        "share_id": share_id,
        "share_url": share_url,
        "title": title,
        "intent": if intent == "update" { "update" } else { "new" },
        "change_hint": { "added": [], "changed": [], "removed": [] }
    })
}

pub(crate) fn hdhive_signature(secret: &str, method: &str, path: &str, body: &str, timestamp: &str) -> String {
    let body_hash = hex::encode(Sha256::digest(body.as_bytes()));
    let canonical = format!(
        "{timestamp}\n{}\n{path}\n{body_hash}",
        method.to_uppercase()
    );
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts all key sizes");
    mac.update(canonical.as_bytes());
    format!("v1={}", hex::encode(mac.finalize().into_bytes()))
}

pub(crate) async fn hdhive_request(
    base_url: &str,
    secret: &str,
    instance_id: &str,
    proxy: &str,
    method: reqwest::Method,
    path_segments: &[&str],
    body: Option<&Value>,
) -> Result<Value, String> {
    if base_url.is_empty() || secret.is_empty() {
        return Err("尚未配置 Hdhive 接入地址和密钥".to_string());
    }
    let normalized_base_url = normalize_hdhive_base_url(base_url)?;
    let (target_url, signature_path) =
        build_hdhive_target_url(&normalized_base_url, path_segments)?;
    let body_text = body.map(Value::to_string).unwrap_or_default();
    let timestamp = unix_timestamp().to_string();
    let mut client_builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS));
    if !proxy.trim().is_empty() {
        client_builder = client_builder.proxy(
            reqwest::Proxy::all(proxy.trim())
                .map_err(|error| format!("初始化 Hdhive 代理失败：{error}"))?,
        );
    }
    let client = client_builder
        .build()
        .map_err(|error| format!("创建 Hdhive 客户端失败：{error}"))?;
    let response = client
        .request(method.clone(), target_url)
        .header(CONTENT_TYPE, "application/json")
        .header("X-GuangYa-Instance-Id", instance_id)
        .header("X-GuangYa-Timestamp", &timestamp)
        .header(
            "X-GuangYa-Signature",
            hdhive_signature(
                secret,
                method.as_str(),
                &signature_path,
                &body_text,
                &timestamp,
            ),
        )
        .body(body_text)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("连接 Hdhive 失败：{error}"))?;
    let status_code = response.status();
    let raw = response
        .text()
        .await
        .map_err(|error| format!("读取 Hdhive 响应失败：{error}"))?;
    let payload: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("Hdhive 返回非 JSON 响应（HTTP {status_code}）：{error}"))?;
    if !status_code.is_success() {
        return Err(payload
            .get("description")
            .or_else(|| payload.get("message"))
            .or_else(|| payload.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("Hdhive 请求失败")
            .to_string());
    }
    Ok(payload.get("data").cloned().unwrap_or(payload))
}

pub(crate) async fn schedule_auto_share(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let Some(target) = auto_share_target(item) else {
        return Ok(());
    };
    let (mapping, token, device_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.hdhive_enabled {
            return Ok(());
        }
        let Some(mapping) = guard
            .mappings
            .iter()
            .find(|entry| entry.id == item.mapping_id)
            .cloned()
        else {
            return Ok(());
        };
        if !mapping.auto_share {
            return Ok(());
        }
        (
            mapping,
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.db_path.clone(),
        )
    };
    let remote_target_id = if target.target_type == "file" {
        outcome
            .remote_file_id
            .clone()
            .ok_or_else(|| "云端没有返回文件 ID，无法自动分享".to_string())?
    } else {
        let remote_path = [
            if mapping.remote_parent_id.is_empty() {
                normalize_remote_path(&mapping.remote_path)
            } else {
                String::new()
            },
            target.key.clone(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
        ensure_remote_path(
            app,
            state,
            &token,
            &device_id,
            &mapping.remote_parent_id,
            &remote_path,
        )
        .await?
    };
    let connection = open_database(&db_path)?;
    let existing = connection
        .query_row(
            "SELECT added_paths, changed_paths, event_id FROM auto_share_pending WHERE mapping_id=?1 AND target_key=?2",
            params![item.mapping_id, target.key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|error| format!("读取待分享聚合失败：{error}"))?;
    drop(connection);
    let mut pending = PendingAutoShare {
        mapping_id: item.mapping_id.clone(),
        target_key: target.key,
        target_type: target.target_type,
        title: target.title,
        remote_target_id,
        added: HashSet::new(),
        changed: HashSet::new(),
        event_id: Uuid::new_v4().to_string(),
        retry_count: 0,
    };
    if let Some((added, changed, event_id)) = existing {
        pending.added = serde_json::from_str::<Vec<String>>(&added)
            .unwrap_or_default()
            .into_iter()
            .collect();
        pending.changed = serde_json::from_str::<Vec<String>>(&changed)
            .unwrap_or_default()
            .into_iter()
            .collect();
        pending.event_id = event_id;
    }
    if item.change_kind == "changed" {
        pending.changed.insert(target.relative_path);
    } else {
        pending.added.insert(target.relative_path);
    }
    persist_pending_auto_share(&db_path, &pending)
}

pub(crate) async fn poll_hdhive_receipt(
    app: tauri::AppHandle,
    state: SharedState,
    pending: PendingAutoShare,
    share_url: String,
    payload: Value,
) {
    for attempt in 0..60_u64 {
        sleep(Duration::from_secs((2 + attempt / 2).min(10))).await;
        let (base_url, secret, instance_id, db_path) = match state.lock() {
            Ok(guard) if guard.hdhive_enabled => (
                guard.hdhive_base_url.clone(),
                guard.hdhive_secret.clone(),
                guard.hdhive_instance_id.clone(),
                guard.db_path.clone(),
            ),
            Ok(_) => return,
            Err(_) => return,
        };
        let proxy = match load_global_network_proxy(&db_path) {
            Ok(value) => value,
            Err(_) => return,
        };
        match hdhive_request(
            &base_url,
            &secret,
            &instance_id,
            &proxy,
            reqwest::Method::GET,
            &[
                "api",
                "integrations",
                "guangya-sync",
                "events",
                pending.event_id.as_str(),
            ],
            None,
        )
        .await
        {
            Ok(result) => {
                let current_status = result
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("processing");
                let action = result.get("action").and_then(Value::as_str);
                let notification_status = result.get("notification_status").and_then(Value::as_str);
                let error_message = result
                    .get("error_message")
                    .and_then(Value::as_str)
                    .filter(|message| !message.trim().is_empty());
                let message = error_message.map(str::to_owned).unwrap_or_else(|| {
                    let outcome = match current_status {
                        "completed" => match action {
                            Some("created") => "影巢投稿完成",
                            Some("updated") => "影巢内容更新完成",
                            Some("no_change") => "影巢确认内容没有变化",
                            Some("baseline_initialized") => "影巢已建立内容基线",
                            _ => "影巢处理完成",
                        },
                        "needs_review" => "影巢需要人工补充信息",
                        "failed" => "影巢处理失败，请重试",
                        "accepted" => "影巢已接收，等待处理",
                        _ => "影巢正在解析并投稿",
                    };
                    if current_status == "completed" && notification_status == Some("sent") {
                        format!("{outcome}，消息已推送")
                    } else {
                        outcome.to_string()
                    }
                });
                let resource_url = result.get("resource_url").and_then(Value::as_str);
                let _ = save_auto_share_event(
                    &db_path,
                    &pending.event_id,
                    &pending.mapping_id,
                    &pending.target_key,
                    Some(&share_url),
                    current_status,
                    action,
                    Some(&message),
                    resource_url,
                    &payload,
                );
                let _ = open_database(&db_path).and_then(|connection| {
                    connection
                        .execute(
                            "UPDATE auto_share_events SET notification_status=?1, error_code=?2, updated_at=?3 WHERE event_id=?4",
                            params![
                                notification_status,
                                result.get("error_code").and_then(Value::as_str),
                                unix_timestamp(),
                                pending.event_id
                            ],
                        )
                        .map(|_| ())
                        .map_err(|error| format!("保存通知回执失败：{error}"))
                });
                emit_state(&app, &state);
                if ["completed", "needs_review", "failed"].contains(&current_status) {
                    return;
                }
            }
            Err(error) if attempt == 59 => {
                let _ = save_auto_share_event(
                    &db_path,
                    &pending.event_id,
                    &pending.mapping_id,
                    &pending.target_key,
                    Some(&share_url),
                    "failed",
                    None,
                    Some(&format!("查询 Hdhive 回执失败：{error}")),
                    None,
                    &payload,
                );
                emit_state(&app, &state);
            }
            Err(_) => {}
        }
    }
}

pub(crate) async fn process_auto_share(
    app: tauri::AppHandle,
    state: SharedState,
    pending: PendingAutoShare,
) -> Result<(), String> {
    let (enabled, mapping, token, device_id, db_path, base_url, secret, instance_id, has_work) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard.hdhive_enabled,
            guard
                .mappings
                .iter()
                .find(|mapping| mapping.id == pending.mapping_id)
                .cloned(),
            guard.token.clone(),
            guard.device_id.clone(),
            guard.db_path.clone(),
            guard.hdhive_base_url.clone(),
            guard.hdhive_secret.clone(),
            guard.hdhive_instance_id.clone(),
            target_has_work(&guard, &pending.mapping_id, &pending.target_key),
        )
    };
    if !enabled {
        return Ok(());
    }
    let Some(mapping) = mapping else {
        return delete_pending_auto_share(&db_path, &pending.mapping_id, &pending.target_key);
    };
    if !mapping.auto_share {
        return delete_pending_auto_share(&db_path, &pending.mapping_id, &pending.target_key);
    }
    if has_work || target_has_pending_cloud(&db_path, &pending.mapping_id, &pending.target_key)? {
        return reschedule_auto_share(&db_path, &pending, AUTO_SHARE_QUIET_SECS);
    }
    let failure_exists = open_database(&db_path)?
        .query_row(
            "SELECT 1 FROM auto_share_failures WHERE mapping_id=?1 AND target_key=?2 LIMIT 1",
            params![pending.mapping_id, pending.target_key],
            |_| Ok(true),
        )
        .optional()
        .map_err(|error| format!("读取上传失败状态失败：{error}"))?
        .unwrap_or(false);
    if failure_exists {
        let payload = json!({ "target_key": pending.target_key });
        save_auto_share_event(
            &db_path,
            &pending.event_id,
            &pending.mapping_id,
            &pending.target_key,
            None,
            "waiting_upload",
            None,
            Some("同一分享目标仍有上传失败文件，已暂停分享"),
            None,
            &payload,
        )?;
        emit_state(&app, &state);
        return reschedule_auto_share(&db_path, &pending, 60);
    }
    let token = token.ok_or_else(|| "尚未登录光鸭云盘".to_string())?;
    let stored = open_database(&db_path)?
        .query_row(
            "SELECT remote_target_id, share_id, share_url FROM auto_share_targets WHERE mapping_id=?1 AND target_key=?2",
            params![pending.mapping_id, pending.target_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|error| format!("读取分享绑定失败：{error}"))?;
    let mut share_id = stored
        .as_ref()
        .map(|value| value.1.clone())
        .unwrap_or_default();
    let mut share_url = stored
        .as_ref()
        .map(|value| value.2.clone())
        .unwrap_or_default();
    let stored_url_share_id = share_id_from_url(&share_url);
    if !stored_url_share_id.is_empty() {
        share_id = stored_url_share_id;
    }
    let mut intent = "update";
    if stored
        .as_ref()
        .is_none_or(|value| value.0 != pending.remote_target_id || value.2.is_empty())
    {
        let existing = find_existing_share_for_files(
            &token,
            &device_id,
            std::slice::from_ref(&pending.remote_target_id),
        )
        .await?;
        let reused_existing = existing.is_some();
        let data = if let Some(existing) = existing {
            existing
        } else {
            api_post(
                &token,
                &device_id,
                "/userres/v1/share_file",
                share_file_payload(
                    std::slice::from_ref(&pending.remote_target_id),
                    &pending.title,
                    0,
                    "",
                    false,
                ),
                &[],
            )
            .await?
            .data
            .unwrap_or_default()
        };
        share_url = ["shareUrl", "shareURL", "share_url", "url"]
            .iter()
            .find_map(|key| data.get(key).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        share_id = share_id_for_hdhive(&data, &share_url);
        if share_url.is_empty() || share_id.is_empty() {
            return Err("光鸭没有返回完整分享链接".to_string());
        }
        intent = if reused_existing || stored.as_ref().is_some_and(|value| value.1 == share_id) {
            "update"
        } else {
            "new"
        };
        open_database(&db_path)?
            .execute(
                "INSERT INTO auto_share_targets
                   (mapping_id, target_key, target_type, remote_target_id, title, share_id, share_url, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(mapping_id, target_key) DO UPDATE SET target_type=excluded.target_type,
                   remote_target_id=excluded.remote_target_id, title=excluded.title,
                   share_id=excluded.share_id, share_url=excluded.share_url, updated_at=excluded.updated_at",
                params![
                    pending.mapping_id,
                    pending.target_key,
                    pending.target_type,
                    pending.remote_target_id,
                    pending.title,
                    share_id,
                    share_url,
                    unix_timestamp()
                ],
            )
            .map_err(|error| format!("保存分享绑定失败：{error}"))?;
        status(
            &app,
            "success",
            if reused_existing {
                format!("已复用光鸭已有分享：{}", pending.title)
            } else {
                format!("光鸭分享成功：{}", pending.title)
            },
        );
    }
    let mut added = pending.added.iter().cloned().collect::<Vec<_>>();
    let mut changed = pending.changed.iter().cloned().collect::<Vec<_>>();
    added.sort();
    changed.sort();
    let payload = json!({
        "event_id": pending.event_id,
        "mapping_id": pending.mapping_id,
        "target_key": pending.target_key,
        "target_type": pending.target_type,
        "remote_target_id": pending.remote_target_id,
        "share_id": share_id,
        "share_url": share_url,
        "title": pending.title,
        "intent": intent,
        "change_hint": { "added": added, "changed": changed, "removed": [] }
    });
    if !state
        .lock()
        .map_err(|error| error.to_string())?
        .hdhive_enabled
    {
        return Ok(());
    }
    save_auto_share_event(
        &db_path,
        &pending.event_id,
        &pending.mapping_id,
        &pending.target_key,
        Some(&share_url),
        "sending",
        None,
        Some("光鸭分享成功，正在通知 Hdhive"),
        None,
        &payload,
    )?;
    emit_state(&app, &state);
    let proxy = load_global_network_proxy(&db_path)?;
    let accepted = hdhive_request(
        &base_url,
        &secret,
        &instance_id,
        &proxy,
        reqwest::Method::POST,
        &["api", "integrations", "guangya-sync", "events"],
        Some(&payload),
    )
    .await?;
    let accepted_status = accepted
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("accepted");
    save_auto_share_event(
        &db_path,
        &pending.event_id,
        &pending.mapping_id,
        &pending.target_key,
        Some(&share_url),
        accepted_status,
        None,
        Some("Hdhive 已接收"),
        None,
        &payload,
    )?;
    delete_pending_auto_share(&db_path, &pending.mapping_id, &pending.target_key)?;
    emit_state(&app, &state);
    tauri::async_runtime::spawn(poll_hdhive_receipt(app, state, pending, share_url, payload));
    Ok(())
}

pub(crate) async fn auto_share_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(2)).await;
        let (db_path, configured) = match state.lock() {
            Ok(guard) => (
                guard.db_path.clone(),
                guard.hdhive_enabled
                    && !guard.hdhive_base_url.is_empty()
                    && !guard.hdhive_secret.is_empty(),
            ),
            Err(_) => continue,
        };
        if !configured {
            continue;
        }
        let pending_items = match load_due_auto_shares(&db_path) {
            Ok(items) => items,
            Err(error) => {
                status(&app, "error", error);
                continue;
            }
        };
        for pending in pending_items {
            let processing_key = format!("{}::{}", pending.mapping_id, pending.target_key);
            let should_start = state.lock().ok().is_some_and(|mut guard| {
                guard.auto_share_processing.insert(processing_key.clone())
            });
            if !should_start {
                continue;
            }
            let worker_app = app.clone();
            let worker_state = state.clone();
            let worker_db_path = db_path.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) =
                    process_auto_share(worker_app.clone(), worker_state.clone(), pending.clone())
                        .await
                {
                    let mut retry = pending;
                    retry.retry_count += 1;
                    let delay = (30_i64.saturating_mul(
                        2_i64.saturating_pow((retry.retry_count - 1).clamp(0, 6) as u32),
                    ))
                    .min(1_800);
                    let payload = json!({ "target_key": retry.target_key });
                    let _ = save_auto_share_event(
                        &worker_db_path,
                        &retry.event_id,
                        &retry.mapping_id,
                        &retry.target_key,
                        None,
                        "failed",
                        None,
                        Some(&error),
                        None,
                        &payload,
                    );
                    let _ = reschedule_auto_share(&worker_db_path, &retry, delay);
                    status(
                        &worker_app,
                        "error",
                        format!("自动分享失败，稍后重试：{error}"),
                    );
                    emit_state(&worker_app, &worker_state);
                }
                if let Ok(mut guard) = worker_state.lock() {
                    guard.auto_share_processing.remove(&processing_key);
                }
            });
        }
    }
}


#[tauri::command]
pub(crate) fn update_hdhive_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    base_url: String,
    secret: Option<String>,
    enabled: Option<bool>,
) -> Result<HdhivePublicConfig, String> {
    let normalized = normalize_hdhive_base_url(&base_url)?;
    let (db_path, secret_value, result) = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard.hdhive_base_url = normalized;
        if let Some(value) = secret.filter(|value| !value.trim().is_empty()) {
            guard.hdhive_secret = value.trim().to_string();
        }
        if let Some(enabled) = enabled {
            guard.hdhive_enabled = enabled;
        }
        let result = HdhivePublicConfig {
            enabled: guard.hdhive_enabled,
            configured: !guard.hdhive_base_url.is_empty() && !guard.hdhive_secret.is_empty(),
            base_url: guard.hdhive_base_url.clone(),
            instance_id: guard.hdhive_instance_id.clone(),
        };
        (guard.db_path.clone(), guard.hdhive_secret.clone(), result)
    };
    save_app_state(&db_path, "hdhive_base_url", &result.base_url)?;
    save_app_state(&db_path, "hdhive_secret", &secret_value)?;
    save_app_state(&db_path, "hdhive_enabled", &result.enabled.to_string())?;
    emit_state(&app, state.inner());
    Ok(result)
}


#[tauri::command]
pub(crate) async fn backfill_auto_shares(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<usize, String> {
    let (mapping, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.hdhive_enabled {
            return Err("HDHive 已关闭，请先在设置中开启".to_string());
        }
        let mapping = guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == id)
            .cloned()
            .ok_or_else(|| "备份任务不存在".to_string())?;
        if !mapping.auto_share {
            return Err("请先开启该任务的自动分享".to_string());
        }
        if !mapping.organizer_mapping_id.is_empty() {
            return Err(
                "该任务已启用上传后整理；请扫描对应整理 A 目录，由整理完成后的 B 目录重新分享"
                    .to_string(),
            );
        }
        (mapping, guard.db_path.clone())
    };
    let rows = {
        let connection = open_database(&db_path)?;
        let mut statement = connection
            .prepare(
                "SELECT file_path, size, modified_ms, remote_file_id FROM uploaded_files
                 WHERE mapping_id=?1 AND upload_state=?2
                   AND remote_file_id IS NOT NULL AND remote_file_id <> ''",
            )
            .map_err(|error| format!("读取已有上传记录失败：{error}"))?;
        let rows = statement
            .query_map(params![id, UPLOAD_STATE_CLOUD_CONFIRMED], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("读取已有上传记录失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("解析已有上传记录失败：{error}"))?;
        rows
    };
    let mut scheduled = 0;
    for (file_path, size, modified_raw, remote_file_id) in rows {
        let file_path = PathBuf::from(file_path);
        let Ok(relative) = file_path.strip_prefix(&mapping.local_path) else {
            continue;
        };
        let relative_path = relative.to_string_lossy().replace('\\', "/");
        if relative_path.is_empty() || relative_path.starts_with("../") {
            continue;
        }
        let item = UploadItem {
            mapping_id: mapping.id.clone(),
            file_path,
            remote_parent_id: mapping.remote_parent_id.clone(),
            remote_dir: String::new(),
            relative_path,
            change_kind: "added".to_string(),
            size,
            modified_ms: modified_raw.parse().unwrap_or_default(),
            replacement: None,
        };
        let outcome = UploadOutcome {
            task_id: String::new(),
            remote_file_id: Some(remote_file_id),
        };
        schedule_auto_share(&app, state.inner(), &item, &outcome).await?;
        scheduled += 1;
    }
    status(
        &app,
        "info",
        format!("已补建 {scheduled} 条已有上传记录，30 秒静默后处理"),
    );
    emit_state(&app, state.inner());
    Ok(scheduled)
}

#[tauri::command]
pub(crate) async fn retry_auto_share_event(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    event_id: String,
    tmdb_id: Option<String>,
    media_type: Option<String>,
) -> Result<Value, String> {
    let (base_url, secret, instance_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.hdhive_enabled {
            return Err("HDHive 已关闭，请先在设置中开启".to_string());
        }
        (
            guard.hdhive_base_url.clone(),
            guard.hdhive_secret.clone(),
            guard.hdhive_instance_id.clone(),
            guard.db_path.clone(),
        )
    };
    let (mapping_id, target_key, share_url, status_value, payload_raw) = open_database(&db_path)?
        .query_row(
            "SELECT mapping_id, target_key, share_url, status, payload FROM auto_share_events WHERE event_id=?1",
            params![event_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
        )
        .optional()
        .map_err(|error| format!("读取自动分享回执失败：{error}"))?
        .ok_or_else(|| "自动分享回执不存在".to_string())?;
    let retry_body = match tmdb_id.filter(|value| !value.trim().is_empty()) {
        Some(tmdb_id) => json!({
            "tmdb_id": tmdb_id,
            "media_type": media_type.unwrap_or_else(|| "tv".to_string())
        }),
        None => json!({}),
    };
    let proxy = load_global_network_proxy(&db_path)?;
    let mut payload = serde_json::from_str::<Value>(&payload_raw).unwrap_or_default();
    if status_value == "delivery_failed" {
        let normalized_share_id = payload
            .get("share_url")
            .and_then(Value::as_str)
            .map(share_id_from_url)
            .unwrap_or_default();
        if !normalized_share_id.is_empty() {
            if let Some(object) = payload.as_object_mut() {
                object.insert("share_id".to_string(), json!(normalized_share_id));
            }
        }
    }
    let (result, receipt_message) = if status_value == "delivery_failed" {
        (
            hdhive_request(
                &base_url,
                &secret,
                &instance_id,
                &proxy,
                reqwest::Method::POST,
                &["api", "integrations", "guangya-sync", "events"],
                Some(&payload),
            )
            .await?,
            "Hdhive 已重新接收投稿事件",
        )
    } else {
        (
            hdhive_request(
                &base_url,
                &secret,
                &instance_id,
                &proxy,
                reqwest::Method::POST,
                &[
                    "api",
                    "integrations",
                    "guangya-sync",
                    "events",
                    event_id.as_str(),
                    "retry",
                ],
                Some(&retry_body),
            )
            .await?,
            "Hdhive 已重新接收",
        )
    };
    save_auto_share_event(
        &db_path,
        &event_id,
        &mapping_id,
        &target_key,
        share_url.as_deref(),
        result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("accepted"),
        result.get("action").and_then(Value::as_str),
        Some(receipt_message),
        result.get("resource_url").and_then(Value::as_str),
        &payload,
    )?;
    let pending = PendingAutoShare {
        mapping_id,
        target_key,
        target_type: payload
            .get("target_type")
            .and_then(Value::as_str)
            .unwrap_or("folder")
            .to_string(),
        title: payload
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        remote_target_id: payload
            .get("remote_target_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        added: HashSet::new(),
        changed: HashSet::new(),
        event_id,
        retry_count: 0,
    };
    tauri::async_runtime::spawn(poll_hdhive_receipt(
        app.clone(),
        state.inner().clone(),
        pending,
        share_url.unwrap_or_default(),
        payload,
    ));
    emit_state(&app, state.inner());
    Ok(result)
}
