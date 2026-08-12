//! 开发者模式：凭据、所有权校验、多号秒传任务。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeveloperTarget {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) token_masked: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeveloperSettings {
    pub(crate) configured: bool,
    pub(crate) enabled: bool,
    pub(crate) requested_enabled: bool,
    pub(crate) client_id: String,
    pub(crate) client_secret_set: bool,
    pub(crate) account_id: String,
    pub(crate) current_account_id: String,
    pub(crate) account_verified: bool,
    pub(crate) account_matches_current: bool,
    pub(crate) verified_at: i64,
    pub(crate) managed_by_environment: bool,
    pub(crate) client_id_managed_by_environment: bool,
    pub(crate) client_secret_managed_by_environment: bool,
    pub(crate) targets: Vec<DeveloperTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeveloperTransferJob {
    pub(crate) id: String,
    pub(crate) target_id: String,
    pub(crate) target_name: String,
    pub(crate) file_ids: Vec<String>,
    pub(crate) file_names: Vec<String>,
    pub(crate) status: String,
    pub(crate) phase: String,
    pub(crate) pre_task_id: Option<String>,
    pub(crate) upload_task_id: Option<String>,
    pub(crate) total_count: i64,
    pub(crate) passed_count: i64,
    pub(crate) rejected_count: i64,
    pub(crate) pending_count: i64,
    pub(crate) success_count: i64,
    pub(crate) skipped_count: i64,
    pub(crate) work_total_count: i64,
    pub(crate) processed_count: i64,
    pub(crate) current_path: String,
    pub(crate) error_code: Option<i64>,
    pub(crate) message: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug)]
pub(crate) struct DeveloperApiError {
    pub(crate) message: String,
    pub(crate) code: Option<i64>,
    pub(crate) retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeveloperPreAuditBatch {
    pub(crate) task_id: String,
    pub(crate) file_count: i64,
    #[serde(default)]
    pub(crate) passed_count: i64,
    #[serde(default)]
    pub(crate) rejected_count: i64,
    #[serde(default)]
    pub(crate) done: bool,
    #[serde(default)]
    pub(crate) failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeveloperPreAuditPlan {
    pub(crate) version: u8,
    pub(crate) batches: Vec<DeveloperPreAuditBatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeveloperPreAuditSummary {
    pub(crate) total_count: i64,
    pub(crate) passed_count: i64,
    pub(crate) rejected_count: i64,
    pub(crate) pending_count: i64,
    pub(crate) failed_batches: usize,
    pub(crate) done: bool,
}

impl std::fmt::Display for DeveloperApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DeveloperApiError {}


#[derive(Debug, Clone)]
pub(crate) struct DeveloperNameRestore {
    pub(crate) file_id: String,
    pub(crate) original_name: String,
}


pub(crate) fn mask_developer_value(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return String::new();
    }
    if chars.len() <= 8 {
        return "••••••••".to_string();
    }
    format!(
        "{}••••{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}

pub(crate) fn normalize_developer_setting(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{label}不能为空"));
    }
    if normalized.len() > 256 || !normalized.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(format!("{label}必须是 1 到 256 个可见 ASCII 字符"));
    }
    Ok(normalized.to_string())
}

pub(crate) fn normalize_developer_target_name(value: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err("小号名称不能为空".to_string());
    }
    if normalized.chars().count() > 64 || normalized.chars().any(char::is_control) {
        return Err("小号名称不能超过 64 个字符或包含控制字符".to_string());
    }
    Ok(normalized.to_string())
}

pub(crate) fn developer_credentials(path: &Path) -> Result<(String, String, bool, bool), String> {
    let environment_client_id = std::env::var("GUANGYA_DEVELOPER_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let environment_client_secret = std::env::var("GUANGYA_DEVELOPER_CLIENT_SECRET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let client_id_from_environment = environment_client_id.is_some();
    let client_secret_from_environment = environment_client_secret.is_some();
    let client_id = environment_client_id
        .or(load_app_state(path, "developer_client_id")?)
        .unwrap_or_default();
    let client_secret = environment_client_secret
        .or(load_app_state(path, "developer_client_secret")?)
        .unwrap_or_default();
    Ok((
        client_id,
        client_secret,
        client_id_from_environment,
        client_secret_from_environment,
    ))
}

pub(crate) fn developer_target_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeveloperTarget> {
    let token_id: String = row.get(2)?;
    Ok(DeveloperTarget {
        id: row.get(0)?,
        name: row.get(1)?,
        token_masked: mask_developer_value(&token_id),
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

pub(crate) fn load_developer_targets(path: &Path) -> Result<Vec<DeveloperTarget>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT id, name, token_id, created_at, updated_at
             FROM developer_targets ORDER BY updated_at DESC, name",
        )
        .map_err(|error| format!("读取小号 TOKEN 配置失败：{error}"))?;
    let rows = statement
        .query_map([], developer_target_from_row)
        .map_err(|error| format!("读取小号 TOKEN 配置失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析小号 TOKEN 配置失败：{error}"))?;
    Ok(rows)
}

pub(crate) fn load_developer_settings_for_account(
    path: &Path,
    current_account_id: &str,
) -> Result<DeveloperSettings, String> {
    let (client_id, client_secret, client_id_from_environment, client_secret_from_environment) =
        developer_credentials(path)?;
    let requested_enabled = load_app_state(path, "developer_mode_enabled")?.as_deref() == Some("1");
    let account_id = load_app_state(path, "developer_account_id")?.unwrap_or_default();
    let verified_client_id =
        load_app_state(path, "developer_verified_client_id")?.unwrap_or_default();
    let verified_at = load_app_state(path, "developer_account_verified_at")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let account_verified =
        !account_id.is_empty() && verified_at > 0 && verified_client_id == client_id;
    let account_matches_current =
        !current_account_id.is_empty() && account_id == current_account_id;
    let configured = !client_id.is_empty() && !client_secret.is_empty();
    Ok(DeveloperSettings {
        configured,
        enabled: requested_enabled && account_verified && account_matches_current && configured,
        requested_enabled,
        client_id,
        client_secret_set: !client_secret.is_empty(),
        account_id,
        current_account_id: current_account_id.to_string(),
        account_verified,
        account_matches_current,
        verified_at,
        managed_by_environment: client_id_from_environment || client_secret_from_environment,
        client_id_managed_by_environment: client_id_from_environment,
        client_secret_managed_by_environment: client_secret_from_environment,
        targets: load_developer_targets(path)?,
    })
}

pub(crate) fn load_developer_settings(path: &Path) -> Result<DeveloperSettings, String> {
    load_developer_settings_for_account(path, "")
}

pub(crate) fn developer_job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeveloperTransferJob> {
    let file_ids_raw: String = row.get(3)?;
    let file_names_raw: String = row.get(4)?;
    Ok(DeveloperTransferJob {
        id: row.get(0)?,
        target_id: row.get(1)?,
        target_name: row.get(2)?,
        file_ids: serde_json::from_str(&file_ids_raw).unwrap_or_default(),
        file_names: serde_json::from_str(&file_names_raw).unwrap_or_default(),
        status: row.get(5)?,
        phase: row.get(6)?,
        pre_task_id: row.get(7)?,
        upload_task_id: row.get(8)?,
        total_count: row.get(9)?,
        passed_count: row.get(10)?,
        rejected_count: row.get(11)?,
        pending_count: row.get(12)?,
        success_count: row.get(13)?,
        skipped_count: row.get(14)?,
        work_total_count: row.get(15)?,
        processed_count: row.get(16)?,
        current_path: row.get(17)?,
        error_code: row.get(18)?,
        message: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

pub(crate) const DEVELOPER_JOB_COLUMNS: &str =
    "id, target_id, target_name, file_ids_json, file_names_json, status, phase,
     pre_task_id, upload_task_id, total_count, passed_count, rejected_count,
     pending_count, success_count, skipped_count, work_total_count, processed_count,
     current_path, error_code, message, created_at, updated_at";

pub(crate) fn load_developer_transfer_job(
    path: &Path,
    job_id: &str,
) -> Result<Option<DeveloperTransferJob>, String> {
    open_database(path)?
        .query_row(
            &format!("SELECT {DEVELOPER_JOB_COLUMNS} FROM developer_transfer_jobs WHERE id = ?1"),
            params![job_id],
            developer_job_from_row,
        )
        .optional()
        .map_err(|error| format!("读取小号互传任务失败：{error}"))
}

pub(crate) fn list_developer_transfer_jobs(
    path: &Path,
    limit: Option<usize>,
) -> Result<Vec<DeveloperTransferJob>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 100) as i64;
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {DEVELOPER_JOB_COLUMNS} FROM developer_transfer_jobs
             ORDER BY created_at DESC LIMIT ?1"
        ))
        .map_err(|error| format!("读取小号互传任务失败：{error}"))?;
    let rows = statement
        .query_map(params![limit], developer_job_from_row)
        .map_err(|error| format!("读取小号互传任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析小号互传任务失败：{error}"))?;
    Ok(rows)
}

pub(crate) fn save_developer_transfer_job(path: &Path, job: &DeveloperTransferJob) -> Result<(), String> {
    open_database(path)?
        .execute(
            "UPDATE developer_transfer_jobs SET
               status = ?2, phase = ?3, pre_task_id = ?4, upload_task_id = ?5,
               total_count = ?6, passed_count = ?7, rejected_count = ?8,
               pending_count = ?9, success_count = ?10, skipped_count = ?11,
               work_total_count = ?12, processed_count = ?13, current_path = ?14,
               error_code = ?15, message = ?16, updated_at = ?17
             WHERE id = ?1",
            params![
                job.id,
                job.status,
                job.phase,
                job.pre_task_id,
                job.upload_task_id,
                job.total_count,
                job.passed_count,
                job.rejected_count,
                job.pending_count,
                job.success_count,
                job.skipped_count,
                job.work_total_count,
                job.processed_count,
                job.current_path,
                job.error_code,
                job.message,
                job.updated_at,
            ],
        )
        .map_err(|error| format!("更新小号互传任务失败：{error}"))?;
    Ok(())
}

pub(crate) fn mutate_developer_transfer_job<F>(
    path: &Path,
    job_id: &str,
    mutate: F,
) -> Result<DeveloperTransferJob, String>
where
    F: FnOnce(&mut DeveloperTransferJob),
{
    let mut job = load_developer_transfer_job(path, job_id)?
        .ok_or_else(|| "小号互传任务不存在".to_string())?;
    mutate(&mut job);
    job.updated_at = unix_timestamp();
    save_developer_transfer_job(path, &job)?;
    Ok(job)
}

pub(crate) fn update_and_emit_developer_job<F>(
    app: &tauri::AppHandle,
    path: &Path,
    job_id: &str,
    mutate: F,
) -> Result<DeveloperTransferJob, String>
where
    F: FnOnce(&mut DeveloperTransferJob),
{
    let job = mutate_developer_transfer_job(path, job_id, mutate)?;
    emit(app, json!({ "type": "developer-transfer", "job": job }));
    Ok(job)
}

pub(crate) fn developer_signature(
    client_id: &str,
    client_secret: &str,
    nonce: &str,
    timestamp: i64,
) -> String {
    let source = format!(
        "client_id={client_id}&client_secret={client_secret}&nonce={nonce}&timestamp={timestamp}"
    );
    let md5_bytes = Md5::digest(source.as_bytes());
    hex::encode(Sha512::digest(md5_bytes))
}

pub(crate) fn developer_headers(client_id: &str, client_secret: &str) -> Result<HeaderMap, String> {
    let client_id = normalize_developer_setting(client_id, "开发者 client_id")?;
    let client_secret = normalize_developer_setting(client_secret, "开发者 client_secret")?;
    let nonce = Uuid::new_v4().simple().to_string();
    let timestamp = unix_timestamp();
    let sign = developer_signature(&client_id, &client_secret, &nonce, timestamp);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "client_id",
        HeaderValue::from_str(&client_id).map_err(|error| error.to_string())?,
    );
    headers.insert(
        "nonce",
        HeaderValue::from_str(&nonce).map_err(|error| error.to_string())?,
    );
    headers.insert(
        "timestamp",
        HeaderValue::from_str(&timestamp.to_string()).map_err(|error| error.to_string())?,
    );
    headers.insert(
        "sign",
        HeaderValue::from_str(&sign).map_err(|error| error.to_string())?,
    );
    Ok(headers)
}

pub(crate) fn developer_error_message(code: i64, fallback: &str) -> String {
    match code {
        18001 => "接收 TOKEN 不存在或已删除",
        18002 => "接收 TOKEN 已绑定其他开发者账号",
        18003 => "发送账号与接收账号相同，不能互传",
        18006 => "所选文件不属于当前开发者账号",
        18007 => "小号云盘空间不足",
        18008 => "小号授权的目标目录已不存在",
        18009 => "任务不存在，或不属于当前开发者凭据",
        18010 => "操作过于频繁，请稍后重试",
        18011 => "文件尚未通过预审，暂时不能秒传",
        18012 => "一次最多互传 20 项",
        18013 => "开发者服务繁忙，请稍后重试",
        18014 => "这些文件已经传给该小号，不能重复传输",
        18020 => "开发者凭据无效或已删除",
        18021 => "开发者签名校验失败",
        18022 => "开发者签名已过期，请校准系统时间",
        18023 => "开发者请求 nonce 已被使用",
        18025 => "当前开发者凭据没有此接口权限",
        18026 => "当前开发者账号已被限制使用接口",
        _ if !fallback.trim().is_empty() => fallback,
        _ => return format!("开发者接口失败（业务码 {code}）"),
    }
    .to_string()
}

pub(crate) async fn developer_api_post(
    client_id: &str,
    client_secret: &str,
    endpoint: &str,
    body: Value,
) -> Result<Value, DeveloperApiError> {
    let client = http_client().map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: true,
    })?;
    let headers =
        developer_headers(client_id, client_secret).map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?;
    let response = client
        .post(format!("{DEVELOPER_API_BASE}{endpoint}"))
        .headers(headers)
        .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .map_err(|error| DeveloperApiError {
            message: format!("无法连接开发者接口 {endpoint}：{error}"),
            code: None,
            retryable: true,
        })?;
    let http_status = response.status().as_u16();
    let raw = response.text().await.map_err(|error| DeveloperApiError {
        message: format!("读取开发者接口 {endpoint} 响应失败：{error}"),
        code: None,
        retryable: true,
    })?;
    let payload: Value =
        serde_json::from_str(raw.trim().trim_start_matches('\u{feff}')).map_err(|error| {
            DeveloperApiError {
                message: format!("开发者接口 {endpoint} 返回了非 JSON 响应：{error}"),
                code: None,
                retryable: http_status >= 500,
            }
        })?;
    let code = payload
        .get("code")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0);
    if !(200..300).contains(&http_status) || code != 0 {
        let fallback = payload
            .get("msg")
            .or_else(|| payload.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(DeveloperApiError {
            message: developer_error_message(code, fallback),
            code: Some(code),
            retryable: http_status == 429 || http_status >= 500 || matches!(code, 18010 | 18013),
        });
    }
    Ok(payload)
}

pub(crate) async fn developer_post_with_retry(
    client_id: &str,
    client_secret: &str,
    endpoint: &str,
    body: Value,
    retries: usize,
) -> Result<Value, DeveloperApiError> {
    for attempt in 0..=retries {
        match developer_api_post(client_id, client_secret, endpoint, body.clone()).await {
            Ok(payload) => return Ok(payload),
            Err(error) if error.retryable && attempt < retries => {
                let delay = if error.code == Some(18010) {
                    60
                } else {
                    2 * (attempt as u64 + 1)
                };
                sleep(Duration::from_secs(delay.min(60))).await;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

pub(crate) fn developer_task_id(payload: &Value) -> Option<String> {
    payload
        .get("data")
        .and_then(|data| data.get("task_id").or_else(|| data.get("taskId")))
        .or_else(|| payload.get("task_id"))
        .or_else(|| payload.get("taskId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn developer_count(data: &Value, snake: &str, camel: &str) -> Option<i64> {
    data.get(snake)
        .or_else(|| data.get(camel))
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
}

pub(crate) fn apply_developer_counts(job: &mut DeveloperTransferJob, data: &Value) {
    if let Some(value) = developer_count(data, "total_count", "totalCount") {
        job.total_count = value.max(job.total_count);
    }
    if let Some(value) = developer_count(data, "passed_count", "passedCount") {
        job.passed_count = value;
    }
    if let Some(value) = developer_count(data, "rejected_count", "rejectedCount") {
        job.rejected_count = value;
    }
    if let Some(value) = developer_count(data, "pending_count", "pendingCount") {
        job.pending_count = value;
    }
    if let Some(value) = developer_count(data, "success_count", "successCount")
        .or_else(|| developer_count(data, "use_count", "useCount"))
    {
        job.success_count = value;
    }
    if let Some(value) = developer_count(data, "skipped_count", "skippedCount") {
        job.skipped_count = value;
    }
}

pub(crate) fn developer_pre_audit_plan(task_state: &str, fallback_file_count: i64) -> DeveloperPreAuditPlan {
    let raw = task_state.trim();
    if let Ok(plan) = serde_json::from_str::<DeveloperPreAuditPlan>(raw) {
        if plan.version == 2 {
            return plan;
        }
    }
    DeveloperPreAuditPlan {
        version: 1,
        batches: vec![DeveloperPreAuditBatch {
            task_id: raw.to_string(),
            file_count: fallback_file_count.max(0),
            passed_count: 0,
            rejected_count: 0,
            done: false,
            failed: false,
        }],
    }
}

pub(crate) fn encode_developer_pre_audit_plan(plan: &DeveloperPreAuditPlan) -> Result<String, String> {
    serde_json::to_string(&DeveloperPreAuditPlan {
        version: 2,
        batches: plan.batches.clone(),
    })
    .map_err(|error| format!("保存分批预审状态失败：{error}"))
}

pub(crate) fn summarize_developer_pre_audit_plan(plan: &DeveloperPreAuditPlan) -> DeveloperPreAuditSummary {
    let mut summary = DeveloperPreAuditSummary {
        total_count: 0,
        passed_count: 0,
        rejected_count: 0,
        pending_count: 0,
        failed_batches: 0,
        done: !plan.batches.is_empty(),
    };
    for batch in &plan.batches {
        let passed = batch.passed_count.max(0);
        let rejected = batch.rejected_count.max(0);
        summary.total_count += batch.file_count.max(passed + rejected).max(0);
        summary.passed_count += passed;
        summary.rejected_count += rejected;
        summary.failed_batches += usize::from(batch.failed);
        summary.done &= batch.done;
    }
    summary.pending_count =
        (summary.total_count - summary.passed_count - summary.rejected_count).max(0);
    summary
}


pub(crate) fn account_id_from_profile(payload: &Value) -> Option<String> {
    let data = payload.get("data").unwrap_or(payload);
    let profile = data
        .get("user")
        .or_else(|| data.get("profile"))
        .unwrap_or(data);
    ["sub", "userId", "user_id", "id"]
        .into_iter()
        .find_map(|key| match profile.get(key) {
            Some(Value::String(value)) if !value.trim().is_empty() => {
                Some(value.trim().to_string())
            }
            Some(Value::Number(value)) => Some(value.to_string()),
            _ => None,
        })
}

pub(crate) async fn current_developer_account_id(token: &str, device_id: &str) -> Result<String, String> {
    let profile = account_get(token, device_id, "/v1/user/me").await?;
    account_id_from_profile(&profile)
        .ok_or_else(|| "当前登录态没有返回可识别的账号 ID，无法绑定开发者模式".to_string())
}

pub(crate) fn developer_mode_requested(path: &Path) -> Result<bool, String> {
    Ok(load_app_state(path, "developer_mode_enabled")?.as_deref() == Some("1"))
}

pub(crate) async fn verify_developer_account_ownership(
    state: &tauri::State<'_, SharedState>,
    probe_file_id: Option<&str>,
) -> Result<(String, String), String> {
    let (token, device_id) = auth_context(state)?;
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let current_account_id = current_developer_account_id(&token, &device_id).await?;
    let probe_file_id = if let Some(value) = probe_file_id.filter(|value| !value.trim().is_empty())
    {
        let file_id = normalize_api_id(value, "账号校验文件 ID")?;
        api_post(
            &token,
            &device_id,
            "/userres/v1/file/get_file_detail",
            json!({ "fileId": file_id }),
            &[],
        )
        .await?;
        file_id
    } else {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/file/get_file_list",
            json!({
                "parentId": "",
                "page": 0,
                "pageSize": 1,
                "dirType": 0,
                "orderBy": 0,
                "sortType": 0
            }),
            &[],
        )
        .await?;
        response
            .data
            .as_ref()
            .and_then(|data| data.get("list"))
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(|item| item.get("fileId"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                "当前账号没有可用于所有权校验的文件或目录，请先在根目录创建一个文件夹后重试"
                    .to_string()
            })?
    };
    let (client_id, client_secret, _, _) = developer_credentials(&database_path)?;
    if client_id.is_empty() || client_secret.is_empty() {
        return Err("请先填写开发者 client_id 和 client_secret".to_string());
    }
    developer_post_with_retry(
        &client_id,
        &client_secret,
        "/userres/v1/file/get_file_detail",
        json!({ "fileId": probe_file_id }),
        0,
    )
    .await
    .map_err(|error| error.message)?;
    Ok((current_account_id, probe_file_id))
}

pub(crate) async fn ensure_developer_mode_for_current_account(
    state: &tauri::State<'_, SharedState>,
    probe_file_id: Option<&str>,
) -> Result<(String, String, String), String> {
    let (token, device_id) = auth_context(state)?;
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    if !developer_mode_requested(&database_path)? {
        return Err("请先在“设置 → 账号”中开启开发者模式".to_string());
    }
    let bound_account_id =
        load_app_state(&database_path, "developer_account_id")?.unwrap_or_default();
    let verified_client_id =
        load_app_state(&database_path, "developer_verified_client_id")?.unwrap_or_default();
    let verified_at = load_app_state(&database_path, "developer_account_verified_at")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if bound_account_id.is_empty() || verified_at <= 0 {
        return Err("开发者凭据尚未通过当前账号所有权校验".to_string());
    }
    let current_account_id = current_developer_account_id(&token, &device_id).await?;
    if current_account_id != bound_account_id {
        return Err(
            "开发者模式绑定的账号与当前登录账号不一致，请切回原账号或重新验证凭据".to_string(),
        );
    }
    let (client_id, client_secret, _, _) = developer_credentials(&database_path)?;
    if client_id.is_empty() || client_secret.is_empty() {
        return Err("请先填写开发者 client_id 和 client_secret".to_string());
    }
    if verified_client_id != client_id {
        return Err("开发者 client_id 已变化，请重新验证当前账号".to_string());
    }
    if let Some(value) = probe_file_id.filter(|value| !value.trim().is_empty()) {
        let file_id = normalize_api_id(value, "账号校验文件 ID")?;
        developer_post_with_retry(
            &client_id,
            &client_secret,
            "/userres/v1/file/get_file_detail",
            json!({ "fileId": file_id }),
            0,
        )
        .await
        .map_err(|error| error.message)?;
    }
    Ok((client_id, client_secret, current_account_id))
}

pub(crate) async fn developer_file_read_fallback(
    state: &tauri::State<'_, SharedState>,
    endpoint: &str,
    body: Value,
    primary_error: String,
) -> Result<Value, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    if !developer_mode_requested(&database_path)? {
        return Err(primary_error);
    }
    let (client_id, client_secret, _) = ensure_developer_mode_for_current_account(state, None)
        .await
        .map_err(|fallback| {
            format!("主接口读取失败：{primary_error}；开发者接口兜底失败：{fallback}")
        })?;
    // 兜底路径本身也是只读接口，允许有界重试：主接口已经失败，兜底再因
    // 一次瞬时抖动放弃就等于双路都废。
    let payload = developer_post_with_retry(&client_id, &client_secret, endpoint, body, 2)
        .await
        .map_err(|error| {
            format!(
                "主接口读取失败：{primary_error}；开发者接口兜底失败：{}",
                error.message
            )
        })?;
    Ok(payload.get("data").cloned().unwrap_or_else(|| json!({})))
}


pub(crate) async fn rename_developer_name_with_retry(
    token: &str,
    device_id: &str,
    file_id: &str,
    new_name: &str,
) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 0..DEVELOPER_NAME_RENAME_ATTEMPTS {
        match rename_remote(token, device_id, file_id, new_name).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = error;
                if cloud_selection_entry_detail(token, device_id, file_id, new_name)
                    .await
                    .is_ok_and(|entry| entry.name == new_name)
                {
                    return Ok(());
                }
            }
        }
        if attempt + 1 < DEVELOPER_NAME_RENAME_ATTEMPTS {
            let delay = match attempt {
                0 => 400,
                1 => 800,
                2 => 1_600,
                _ => 3_000,
            };
            sleep(Duration::from_millis(delay)).await;
        }
    }
    Err(format!(
        "{last_error}（文件名改写已重试 {DEVELOPER_NAME_RENAME_ATTEMPTS} 次）"
    ))
}


#[tauri::command]
pub(crate) async fn get_developer_settings(
    state: tauri::State<'_, SharedState>,
) -> Result<DeveloperSettings, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    // Settings are local data and must remain visible even when the cloud
    // session has expired. Transfer actions still require a live session and
    // perform the account-bound checks in their own handlers.
    let current_account_id = match auth_context(&state) {
        Ok((token, device_id)) => current_developer_account_id(&token, &device_id)
            .await
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    load_developer_settings_for_account(&database_path, &current_account_id)
}

#[tauri::command]
pub(crate) fn update_developer_credentials(
    state: tauri::State<'_, SharedState>,
    client_id: String,
    client_secret: Option<String>,
    clear: Option<bool>,
) -> Result<DeveloperSettings, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let (current_id, current_secret, id_from_environment, secret_from_environment) =
        developer_credentials(&database_path)?;
    if clear.unwrap_or(false) {
        if id_from_environment || secret_from_environment {
            return Err("开发者凭据由环境变量托管，不能在页面中清除".to_string());
        }
        save_app_state(&database_path, "developer_client_id", "")?;
        save_app_state(&database_path, "developer_client_secret", "")?;
        save_app_state(&database_path, "developer_mode_enabled", "0")?;
        save_app_state(&database_path, "developer_account_id", "")?;
        save_app_state(&database_path, "developer_verified_client_id", "")?;
        save_app_state(&database_path, "developer_account_verified_at", "0")?;
        return load_developer_settings(&database_path);
    }
    let next_id = normalize_developer_setting(
        if client_id.trim().is_empty() {
            &current_id
        } else {
            &client_id
        },
        "开发者 client_id",
    )?;
    let requested_secret = client_secret.unwrap_or_default();
    let next_secret = if requested_secret.trim().is_empty() {
        current_secret.clone()
    } else {
        normalize_developer_setting(&requested_secret, "开发者 client_secret")?
    };
    if next_secret.is_empty() {
        return Err("首次配置时必须填写开发者 client_secret".to_string());
    }
    if id_from_environment && next_id != current_id {
        return Err("client_id 由 GUANGYA_DEVELOPER_CLIENT_ID 托管".to_string());
    }
    if secret_from_environment && !requested_secret.trim().is_empty() {
        return Err("client_secret 由 GUANGYA_DEVELOPER_CLIENT_SECRET 托管".to_string());
    }
    let credentials_changed = next_id != current_id || next_secret != current_secret;
    if !id_from_environment {
        save_app_state(&database_path, "developer_client_id", &next_id)?;
    }
    if !secret_from_environment {
        save_app_state(&database_path, "developer_client_secret", &next_secret)?;
    }
    if credentials_changed {
        save_app_state(&database_path, "developer_mode_enabled", "0")?;
        save_app_state(&database_path, "developer_account_id", "")?;
        save_app_state(&database_path, "developer_verified_client_id", "")?;
        save_app_state(&database_path, "developer_account_verified_at", "0")?;
    }
    load_developer_settings(&database_path)
}

#[tauri::command]
pub(crate) fn upsert_developer_target(
    state: tauri::State<'_, SharedState>,
    id: Option<String>,
    name: String,
    token_id: Option<String>,
) -> Result<DeveloperTarget, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let id = match id.filter(|value| !value.trim().is_empty()) {
        Some(value) => normalize_api_id(&value, "小号配置 ID")?,
        None => Uuid::new_v4().to_string(),
    };
    let name = normalize_developer_target_name(&name)?;
    let connection = open_database(&database_path)?;
    let existing = connection
        .query_row(
            "SELECT token_id, created_at FROM developer_targets WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取小号 TOKEN 配置失败：{error}"))?;
    let requested_token = token_id.unwrap_or_default();
    let token_id = if requested_token.trim().is_empty() {
        existing
            .as_ref()
            .map(|(token, _)| token.clone())
            .unwrap_or_default()
    } else {
        normalize_developer_setting(&requested_token, "接收 TOKEN")?
    };
    if token_id.is_empty() {
        return Err("首次添加小号时必须填写接收 TOKEN".to_string());
    }
    let now = unix_timestamp();
    let created_at = existing.map(|(_, created_at)| created_at).unwrap_or(now);
    connection
        .execute(
            "INSERT INTO developer_targets (id, name, token_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name, token_id = excluded.token_id, updated_at = excluded.updated_at",
            params![id, name, token_id, created_at, now],
        )
        .map_err(|error| format!("保存小号 TOKEN 配置失败：{error}"))?;
    connection
        .query_row(
            "SELECT id, name, token_id, created_at, updated_at FROM developer_targets WHERE id = ?1",
            params![id],
            developer_target_from_row,
        )
        .map_err(|error| format!("读取保存后的小号 TOKEN 配置失败：{error}"))
}

#[tauri::command]
pub(crate) fn delete_developer_target(
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<Value, String> {
    let id = normalize_api_id(&id, "小号配置 ID")?;
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let connection = open_database(&database_path)?;
    let active = connection
        .query_row(
            "SELECT 1 FROM developer_transfer_jobs
             WHERE target_id = ?1 AND status IN ('queued', 'direct', 'auditing', 'copying', 'running')
             LIMIT 1",
            params![id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| format!("检查小号互传任务失败：{error}"))?;
    if active.is_some() {
        return Err("这个小号仍有进行中的互传任务，暂时不能删除".to_string());
    }
    let changed = connection
        .execute("DELETE FROM developer_targets WHERE id = ?1", params![id])
        .map_err(|error| format!("删除小号 TOKEN 配置失败：{error}"))?;
    if changed == 0 {
        return Err("小号配置不存在".to_string());
    }
    Ok(json!({}))
}

#[tauri::command]
pub(crate) fn list_developer_transfers(
    state: tauri::State<'_, SharedState>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    Ok(json!({ "list": list_developer_transfer_jobs(&database_path, limit)? }))
}

pub(crate) fn developer_name_mutation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(crate) async fn release_developer_name_obfuscation(
    app: &tauri::AppHandle,
    database_path: &Path,
    token: &str,
    device_id: &str,
    job_id: &str,
) -> Result<(usize, usize, usize), String> {
    let _mutation_guard = developer_name_mutation_lock().lock().await;
    let (restorable, deferred) = {
        let connection = open_database(database_path)?;
        let rows = {
            let mut statement = connection
                .prepare(
                    "SELECT file_id, original_name
                     FROM developer_transfer_name_restores
                     WHERE job_id = ?1 AND status IN ('active', 'released', 'restore_failed')",
                )
                .map_err(|error| format!("读取待恢复预审文件名失败：{error}"))?;
            let mapped = statement
                .query_map(params![job_id], |row| {
                    Ok(DeveloperNameRestore {
                        file_id: row.get(0)?,
                        original_name: row.get(1)?,
                    })
                })
                .map_err(|error| format!("读取待恢复预审文件名失败：{error}"))?;
            let rows = mapped
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("解析待恢复预审文件名失败：{error}"))?;
            rows
        };
        if rows.is_empty() {
            return Ok((0, 0, 0));
        }
        connection
            .execute(
                "UPDATE developer_transfer_name_restores
                 SET status = 'released', updated_at = ?2
                 WHERE job_id = ?1 AND status = 'active'",
                params![job_id, unix_timestamp()],
            )
            .map_err(|error| format!("释放预审文件名记录失败：{error}"))?;
        let mut restorable = Vec::new();
        let mut deferred = 0_usize;
        for row in rows {
            let active = connection
                .query_row(
                    "SELECT COUNT(*) FROM developer_transfer_name_restores
                     WHERE file_id = ?1 AND status = 'active'",
                    params![row.file_id],
                    |result| result.get::<_, i64>(0),
                )
                .map_err(|error| format!("检查并发预审文件名失败：{error}"))?;
            if active > 0 {
                deferred += 1;
            } else {
                restorable.push(row);
            }
        }
        (restorable, deferred)
    };
    let previous_job = load_developer_transfer_job(database_path, job_id)?;
    let previous_phase = previous_job
        .as_ref()
        .map(|job| job.phase.clone())
        .unwrap_or_else(|| "completed".to_string());
    let previous_message = previous_job.and_then(|job| job.message);
    let total = restorable.len().saturating_add(deferred);
    let first_path = restorable
        .first()
        .map(|row| row.original_name.clone())
        .unwrap_or_default();
    update_and_emit_developer_job(app, database_path, job_id, |job| {
        job.phase = "restoring".to_string();
        job.work_total_count = total as i64;
        job.processed_count = deferred as i64;
        job.current_path = first_path.clone();
        job.message = Some(format!("正在恢复源文件名 {deferred}/{total}"));
    })?;
    let mut restores = stream::iter(restorable.into_iter())
        .map(|row| {
            let token = token.to_string();
            let device_id = device_id.to_string();
            async move {
                let current_name = cloud_selection_entry_detail(
                    &token,
                    &device_id,
                    &row.file_id,
                    &row.original_name,
                )
                .await
                .ok()
                .map(|entry| entry.name)
                .unwrap_or_default();
                let result = if current_name == row.original_name {
                    Ok(())
                } else {
                    rename_developer_name_with_retry(
                        &token,
                        &device_id,
                        &row.file_id,
                        &row.original_name,
                    )
                    .await
                };
                (row, result)
            }
        })
        .buffer_unordered(DEVELOPER_NAME_RENAME_CONCURRENCY);
    let mut completed = deferred;
    let mut outcomes = Vec::new();
    while let Some((row, outcome)) = restores.next().await {
        completed += 1;
        let path = row.original_name.clone();
        update_and_emit_developer_job(app, database_path, job_id, |job| {
            job.processed_count = completed as i64;
            job.current_path = path.clone();
            job.message = Some(format!("正在恢复源文件名 {completed}/{total}"));
        })?;
        outcomes.push((row, outcome));
    }
    let connection = open_database(database_path)?;
    let mut restored = 0_usize;
    let mut failed = 0_usize;
    for (row, outcome) in outcomes {
        match outcome {
            Ok(()) => {
                restored += 1;
                connection
                    .execute(
                        "UPDATE developer_transfer_name_restores
                         SET status = 'completed', last_error = NULL, updated_at = ?2
                         WHERE file_id = ?1 AND status <> 'active'",
                        params![row.file_id, unix_timestamp()],
                    )
                    .map_err(|error| format!("完成预审文件名恢复失败：{error}"))?;
            }
            Err(error) => {
                failed += 1;
                connection
                    .execute(
                        "UPDATE developer_transfer_name_restores
                         SET status = 'restore_failed', last_error = ?2, updated_at = ?3
                         WHERE file_id = ?1 AND status <> 'active'",
                        params![
                            row.file_id,
                            error.chars().take(500).collect::<String>(),
                            unix_timestamp()
                        ],
                    )
                    .map_err(|database_error| {
                        format!("保存预审文件名恢复错误失败：{database_error}")
                    })?;
            }
        }
    }
    connection
        .execute(
            "DELETE FROM developer_transfer_name_restores
             WHERE status = 'completed' AND updated_at < ?1",
            params![unix_timestamp() - 30 * 86_400],
        )
        .map_err(|error| format!("清理预审文件名记录失败：{error}"))?;
    update_and_emit_developer_job(app, database_path, job_id, |job| {
        job.phase = previous_phase.clone();
        job.processed_count = total as i64;
        job.current_path.clear();
        job.message = previous_message.clone();
    })?;
    Ok((restored, deferred, failed))
}

#[tauri::command]
pub(crate) async fn test_developer_credentials(
    state: tauri::State<'_, SharedState>,
    probe_file_id: Option<String>,
) -> Result<Value, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let (account_id, _) =
        verify_developer_account_ownership(&state, probe_file_id.as_deref()).await?;
    let verified_at = unix_timestamp();
    save_app_state(&database_path, "developer_account_id", &account_id)?;
    let (client_id, _, _, _) = developer_credentials(&database_path)?;
    save_app_state(&database_path, "developer_verified_client_id", &client_id)?;
    save_app_state(
        &database_path,
        "developer_account_verified_at",
        &verified_at.to_string(),
    )?;
    save_app_state(&database_path, "developer_mode_enabled", "0")?;
    Ok(json!({
        "ok": true,
        "account_id": account_id,
        "settings": load_developer_settings_for_account(&database_path, &account_id)?
    }))
}

#[tauri::command]
pub(crate) async fn update_developer_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    enabled: bool,
) -> Result<DeveloperSettings, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    if !enabled {
        save_app_state(&database_path, "developer_mode_enabled", "0")?;
        let current_account_id = match auth_context(&state) {
            Ok((token, device_id)) => current_developer_account_id(&token, &device_id)
                .await
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        return load_developer_settings_for_account(&database_path, &current_account_id);
    }
    let (token, device_id) = auth_context(&state)?;
    let current_account_id = current_developer_account_id(&token, &device_id).await?;
    let settings = load_developer_settings_for_account(&database_path, &current_account_id)?;
    if !settings.configured {
        return Err("请先填写开发者 client_id 和 client_secret".to_string());
    }
    if !settings.account_verified {
        return Err("请先验证 client_id 确实属于当前账号".to_string());
    }
    if !settings.account_matches_current {
        return Err("这套开发者凭据绑定的不是当前登录账号，请重新配置并验证".to_string());
    }
    save_app_state(&database_path, "developer_mode_enabled", "1")?;
    resume_developer_transfer_jobs(app, state.inner().clone())?;
    load_developer_settings_for_account(&database_path, &current_account_id)
}

pub(crate) async fn finish_developer_upload(
    app: &tauri::AppHandle,
    database_path: &Path,
    client_id: &str,
    client_secret: &str,
    job_id: &str,
    task_id: &str,
) -> Result<DeveloperTransferJob, DeveloperApiError> {
    update_and_emit_developer_job(app, database_path, job_id, |job| {
        job.status = "running".to_string();
        job.phase = "upload".to_string();
        job.upload_task_id = Some(task_id.to_string());
        job.work_total_count = job.total_count;
        job.processed_count = (job.success_count + job.skipped_count).min(job.total_count);
        job.current_path.clear();
        job.message = Some("小号正在接收文件".to_string());
    })
    .map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    for _ in 0..400 {
        let payload = developer_post_with_retry(
            client_id,
            client_secret,
            "/developer/v1/upload_status",
            json!({ "task_id": task_id }),
            2,
        )
        .await?;
        let data = payload.get("data").cloned().unwrap_or_else(|| json!({}));
        let status_value = data
            .get("status")
            .or_else(|| payload.get("status"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let reported_success = developer_count(&data, "success_count", "successCount")
            .or_else(|| developer_count(&data, "use_count", "useCount"))
            .unwrap_or(0);
        let completed =
            status_value == "success" || (status_value == "failed" && reported_success > 0);
        let job = update_and_emit_developer_job(app, database_path, job_id, |job| {
            apply_developer_counts(job, &data);
            job.status = if completed { "success" } else { "running" }.to_string();
            job.phase = if completed { "completed" } else { "upload" }.to_string();
            job.work_total_count = job.total_count;
            job.processed_count = if completed {
                job.total_count
            } else {
                (job.success_count + job.skipped_count).min(job.total_count)
            };
            job.current_path.clear();
            job.error_code = None;
            job.message = Some(if completed && job.rejected_count > 0 {
                format!(
                    "已秒传 {} 个，{} 个未通过预审",
                    job.success_count.max(job.passed_count),
                    job.rejected_count
                )
            } else if completed {
                "文件已秒传到小号授权目录".to_string()
            } else {
                "小号正在接收文件".to_string()
            });
        })
        .map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?;
        if completed {
            return Ok(job);
        }
        if status_value == "failed" {
            let message = data
                .get("message")
                .or_else(|| data.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("小号秒传任务失败")
                .to_string();
            return Err(DeveloperApiError {
                message,
                code: None,
                retryable: false,
            });
        }
        sleep(Duration::from_millis(1_500)).await;
    }
    Err(DeveloperApiError {
        message: "小号秒传任务长时间未完成，请稍后在任务记录中重试".to_string(),
        code: None,
        retryable: false,
    })
}

pub(crate) async fn submit_developer_upload(
    app: &tauri::AppHandle,
    database_path: &Path,
    client_id: &str,
    client_secret: &str,
    job: &DeveloperTransferJob,
    target_token: &str,
) -> Result<DeveloperTransferJob, DeveloperApiError> {
    update_and_emit_developer_job(app, database_path, &job.id, |job| {
        job.status = "copying".to_string();
        job.phase = "upload".to_string();
        job.work_total_count = job.total_count;
        job.processed_count = 0;
        job.current_path.clear();
        job.message = Some("正在提交小号秒传".to_string());
    })
    .map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    let payload = developer_post_with_retry(
        client_id,
        client_secret,
        "/developer/v1/upload_by_fileid",
        json!({ "token_id": target_token, "file_ids": job.file_ids }),
        2,
    )
    .await?;
    let task_id = developer_task_id(&payload).ok_or_else(|| DeveloperApiError {
        message: "开发者接口没有返回秒传任务 ID".to_string(),
        code: None,
        retryable: false,
    })?;
    finish_developer_upload(
        app,
        database_path,
        client_id,
        client_secret,
        &job.id,
        &task_id,
    )
    .await
}

pub(crate) async fn finish_developer_pre_audit(
    app: &tauri::AppHandle,
    database_path: &Path,
    client_id: &str,
    client_secret: &str,
    job: &DeveloperTransferJob,
    target_token: &str,
    task_state: &str,
) -> Result<DeveloperTransferJob, DeveloperApiError> {
    let mut plan = developer_pre_audit_plan(task_state, job.total_count);
    let encoded = encode_developer_pre_audit_plan(&plan).map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    update_and_emit_developer_job(app, database_path, &job.id, |job| {
        job.status = "auditing".to_string();
        job.phase = "pre_upload".to_string();
        job.pre_task_id = Some(encoded.clone());
        job.work_total_count = job.total_count;
        job.processed_count = (job.passed_count + job.rejected_count).min(job.total_count);
        job.current_path.clear();
        job.message = Some("文件正在预审，通过后会自动秒传".to_string());
    })
    .map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    for _ in 0..7_200 {
        for index in 0..plan.batches.len() {
            if plan.batches[index].done {
                continue;
            }
            let task_id = plan.batches[index].task_id.clone();
            match developer_post_with_retry(
                client_id,
                client_secret,
                "/developer/v1/pre_upload_status",
                json!({ "task_id": task_id }),
                2,
            )
            .await
            {
                Ok(payload) => {
                    let data = payload.get("data").cloned().unwrap_or_else(|| json!({}));
                    let audit_status = data
                        .get("status")
                        .or_else(|| payload.get("status"))
                        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
                        .unwrap_or(0);
                    let batch = &mut plan.batches[index];
                    let passed = developer_count(&data, "passed_count", "passedCount")
                        .unwrap_or(batch.passed_count)
                        .max(0);
                    let rejected = developer_count(&data, "rejected_count", "rejectedCount")
                        .unwrap_or(batch.rejected_count)
                        .max(0);
                    let total = developer_count(&data, "total_count", "totalCount")
                        .unwrap_or(batch.file_count)
                        .max(passed + rejected)
                        .max(0);
                    batch.file_count = total;
                    batch.passed_count = passed;
                    batch.rejected_count = rejected;
                    if audit_status == 4 {
                        batch.rejected_count = batch
                            .rejected_count
                            .max(batch.file_count - batch.passed_count);
                        batch.done = true;
                        batch.failed = true;
                    } else if audit_status == 3 {
                        batch.done = true;
                    }
                }
                Err(_) => {
                    let batch = &mut plan.batches[index];
                    batch.rejected_count = batch
                        .rejected_count
                        .max(batch.file_count - batch.passed_count);
                    batch.done = true;
                    batch.failed = true;
                }
            }
        }
        let summary = summarize_developer_pre_audit_plan(&plan);
        let encoded =
            encode_developer_pre_audit_plan(&plan).map_err(|message| DeveloperApiError {
                message,
                code: None,
                retryable: false,
            })?;
        let suffix = if summary.failed_batches > 0 {
            format!("；{} 个预审批次失败，已跳过", summary.failed_batches)
        } else {
            String::new()
        };
        update_and_emit_developer_job(app, database_path, &job.id, |job| {
            job.status = "auditing".to_string();
            job.phase = "pre_upload".to_string();
            job.pre_task_id = Some(encoded.clone());
            job.total_count = summary.total_count;
            job.passed_count = summary.passed_count;
            job.rejected_count = summary.rejected_count;
            job.pending_count = summary.pending_count;
            job.work_total_count = summary.total_count;
            job.processed_count =
                (summary.passed_count + summary.rejected_count).min(summary.total_count);
            job.current_path.clear();
            job.message = Some(format!("文件正在预审，通过后会自动秒传{suffix}"));
        })
        .map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?;
        if summary.done {
            let current = load_developer_transfer_job(database_path, &job.id)
                .map_err(|message| DeveloperApiError {
                    message,
                    code: None,
                    retryable: false,
                })?
                .ok_or_else(|| DeveloperApiError {
                    message: "预审完成后无法读取小号互传任务".to_string(),
                    code: None,
                    retryable: false,
                })?;
            return match submit_developer_upload(
                app,
                database_path,
                client_id,
                client_secret,
                &current,
                target_token,
            )
            .await
            {
                Err(error) if error.code == Some(18014) => {
                    update_and_emit_developer_job(app, database_path, &job.id, |job| {
                        job.status = "success".to_string();
                        job.phase = "completed".to_string();
                        job.skipped_count = job.skipped_count.max(job.passed_count);
                        job.work_total_count = job.total_count;
                        job.processed_count = job.total_count;
                        job.current_path.clear();
                        job.error_code = None;
                        job.message = Some(format!(
                            "通过的 {} 个文件此前已传给该小号；{} 个未通过预审",
                            job.passed_count, job.rejected_count
                        ));
                    })
                    .map_err(|message| DeveloperApiError {
                        message,
                        code: None,
                        retryable: false,
                    })
                }
                Err(error) if error.code == Some(18011) => Err(DeveloperApiError {
                    message: if current.passed_count > 0 {
                        format!(
                            "预审显示通过 {} 个，但平台正式秒传时未返回可上传文件",
                            current.passed_count
                        )
                    } else {
                        format!(
                            "预审完成：{} 个文件均未通过，未开始秒传",
                            current.rejected_count
                        )
                    },
                    code: Some(18011),
                    retryable: false,
                }),
                result => result,
            };
        }
        sleep(Duration::from_secs(3)).await;
    }
    Err(DeveloperApiError {
        message: "文件预审超过 6 小时仍未完成".to_string(),
        code: None,
        retryable: false,
    })
}

pub(crate) async fn start_developer_pre_audit(
    app: &tauri::AppHandle,
    database_path: &Path,
    client_id: &str,
    client_secret: &str,
    job: &DeveloperTransferJob,
    target_token: &str,
    cloud_token: &str,
    device_id: &str,
) -> Result<DeveloperTransferJob, DeveloperApiError> {
    let (files, _, _) = collect_cloud_selection_entries(
        cloud_token,
        device_id,
        &job.file_ids,
        &job.file_names,
        false,
        None,
        None,
    )
    .await
    .map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    if files.is_empty() {
        return Err(DeveloperApiError {
            message: "所选内容中没有可预审的文件".to_string(),
            code: None,
            retryable: false,
        });
    }
    let total = files.len();
    let first_path = files
        .first()
        .map(|entry| entry.path.clone())
        .unwrap_or_default();
    update_and_emit_developer_job(app, database_path, &job.id, |job| {
        job.status = "auditing".to_string();
        job.phase = "pre_upload".to_string();
        job.total_count = total as i64;
        job.passed_count = 0;
        job.rejected_count = 0;
        job.pending_count = total as i64;
        job.work_total_count = total as i64;
        job.processed_count = 0;
        job.current_path = first_path.clone();
        job.message = Some("正在按原文件分批提交预审".to_string());
    })
    .map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;

    let mut plan = DeveloperPreAuditPlan {
        version: 2,
        batches: Vec::new(),
    };
    let mut submitted = 0_usize;
    for entries in files.chunks(DEVELOPER_PRE_AUDIT_BATCH_SIZE) {
        let file_ids = entries
            .iter()
            .map(|entry| entry.file_id.clone())
            .collect::<Vec<_>>();
        let batch = match developer_post_with_retry(
            client_id,
            client_secret,
            "/developer/v1/pre_upload",
            json!({ "token_id": target_token, "file_ids": file_ids }),
            2,
        )
        .await
        {
            Ok(payload) => match developer_task_id(&payload) {
                Some(task_id) => DeveloperPreAuditBatch {
                    task_id,
                    file_count: entries.len() as i64,
                    passed_count: 0,
                    rejected_count: 0,
                    done: false,
                    failed: false,
                },
                None => DeveloperPreAuditBatch {
                    task_id: String::new(),
                    file_count: entries.len() as i64,
                    passed_count: 0,
                    rejected_count: entries.len() as i64,
                    done: true,
                    failed: true,
                },
            },
            Err(_) => DeveloperPreAuditBatch {
                task_id: String::new(),
                file_count: entries.len() as i64,
                passed_count: 0,
                rejected_count: entries.len() as i64,
                done: true,
                failed: true,
            },
        };
        plan.batches.push(batch);
        submitted += entries.len();
        let current_path = files
            .get(submitted.min(total.saturating_sub(1)))
            .map(|entry| entry.path.clone())
            .unwrap_or_default();
        let encoded =
            encode_developer_pre_audit_plan(&plan).map_err(|message| DeveloperApiError {
                message,
                code: None,
                retryable: false,
            })?;
        update_and_emit_developer_job(app, database_path, &job.id, |job| {
            job.pre_task_id = Some(encoded.clone());
            job.current_path = current_path.clone();
            job.message = Some(format!("正在按原文件分批提交预审 {submitted}/{total}"));
        })
        .map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?;
    }
    let prepared = load_developer_transfer_job(database_path, &job.id)
        .map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?
        .ok_or_else(|| DeveloperApiError {
            message: "分批预审提交后无法读取小号互传任务".to_string(),
            code: None,
            retryable: false,
        })?;
    let encoded = encode_developer_pre_audit_plan(&plan).map_err(|message| DeveloperApiError {
        message,
        code: None,
        retryable: false,
    })?;
    finish_developer_pre_audit(
        app,
        database_path,
        client_id,
        client_secret,
        &prepared,
        target_token,
        &encoded,
    )
    .await
}

pub(crate) async fn run_developer_transfer_job(app: tauri::AppHandle, state: SharedState, job_id: String) {
    let database_path = match state.lock() {
        Ok(guard) => guard.db_path.clone(),
        Err(error) => {
            status(&app, "error", format!("读取小号互传状态失败：{error}"));
            return;
        }
    };
    let result = async {
        let mut job = load_developer_transfer_job(&database_path, &job_id)
            .map_err(|message| DeveloperApiError {
                message,
                code: None,
                retryable: false,
            })?
            .ok_or_else(|| DeveloperApiError {
                message: "小号互传任务不存在".to_string(),
                code: None,
                retryable: false,
            })?;
        if matches!(job.status.as_str(), "success" | "failed") {
            return Ok(job);
        }
        let (client_id, client_secret, _, _) =
            developer_credentials(&database_path).map_err(|message| DeveloperApiError {
                message,
                code: None,
                retryable: false,
            })?;
        if client_id.is_empty() || client_secret.is_empty() {
            return Err(DeveloperApiError {
                message: "请先在设置中填写开发者 client_id 和 client_secret".to_string(),
                code: None,
                retryable: false,
            });
        }
        let target_token = open_database(&database_path)
            .and_then(|connection| {
                connection
                    .query_row(
                        "SELECT token_id FROM developer_targets WHERE id = ?1",
                        params![job.target_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("读取小号接收 TOKEN 失败：{error}"))
            })
            .map_err(|message| DeveloperApiError {
                message,
                code: None,
                retryable: false,
            })?
            .ok_or_else(|| DeveloperApiError {
                message: "小号接收 TOKEN 配置已不存在".to_string(),
                code: None,
                retryable: false,
            })?;
        if let Some(task_id) = job.upload_task_id.clone() {
            return finish_developer_upload(
                &app,
                &database_path,
                &client_id,
                &client_secret,
                &job.id,
                &task_id,
            )
            .await;
        }
        if let Some(task_id) = job.pre_task_id.clone() {
            return finish_developer_pre_audit(
                &app,
                &database_path,
                &client_id,
                &client_secret,
                &job,
                &target_token,
                &task_id,
            )
            .await;
        }
        job = update_and_emit_developer_job(&app, &database_path, &job.id, |job| {
            job.status = "direct".to_string();
            job.phase = "direct".to_string();
            job.work_total_count = job.total_count;
            job.processed_count = 0;
            job.current_path.clear();
            job.message = Some("正在尝试直接秒传".to_string());
        })
        .map_err(|message| DeveloperApiError {
            message,
            code: None,
            retryable: false,
        })?;
        match submit_developer_upload(
            &app,
            &database_path,
            &client_id,
            &client_secret,
            &job,
            &target_token,
        )
        .await
        {
            Ok(job) => Ok(job),
            Err(error) if error.code == Some(18014) => {
                update_and_emit_developer_job(&app, &database_path, &job.id, |current| {
                    current.status = "success".to_string();
                    current.phase = "completed".to_string();
                    current.skipped_count = current.total_count;
                    current.work_total_count = current.total_count;
                    current.processed_count = current.total_count;
                    current.current_path.clear();
                    current.error_code = None;
                    current.message = Some("这些文件此前已传给该小号，无需重复传输".to_string());
                })
                .map_err(|message| DeveloperApiError {
                    message,
                    code: None,
                    retryable: false,
                })
            }
            Err(error) if error.code == Some(18011) => {
                let (cloud_token, device_id) = {
                    let guard = state.lock().map_err(|lock_error| DeveloperApiError {
                        message: format!("读取登录态失败：{lock_error}"),
                        code: None,
                        retryable: false,
                    })?;
                    let token = guard.token.clone().ok_or_else(|| DeveloperApiError {
                        message: "登录态不可用，无法展开原文件进行预审".to_string(),
                        code: None,
                        retryable: false,
                    })?;
                    (token, guard.device_id.clone())
                };
                start_developer_pre_audit(
                    &app,
                    &database_path,
                    &client_id,
                    &client_secret,
                    &job,
                    &target_token,
                    &cloud_token,
                    &device_id,
                )
                .await
            }
            Err(error) => Err(error),
        }
    }
    .await;
    if let Err(ref error) = result {
        let _ = update_and_emit_developer_job(&app, &database_path, &job_id, |job| {
            job.status = "failed".to_string();
            job.phase = "failed".to_string();
            job.current_path.clear();
            job.error_code = error.code;
            job.message = Some(error.message.clone());
        });
    }
    let has_restores = open_database(&database_path)
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT 1 FROM developer_transfer_name_restores
                     WHERE job_id = ?1 AND status IN ('active', 'released', 'restore_failed')
                     LIMIT 1",
                    params![job_id],
                    |_| Ok(true),
                )
                .optional()
                .map(|value| value.unwrap_or(false))
                .map_err(|error| format!("检查待恢复预审文件名失败：{error}"))
        })
        .unwrap_or(false);
    if has_restores {
        let auth = state.lock().ok().and_then(|guard| {
            guard
                .token
                .clone()
                .map(|token| (token, guard.device_id.clone()))
        });
        let restore_result = match auth {
            Some((token, device_id)) => {
                release_developer_name_obfuscation(
                    &app,
                    &database_path,
                    &token,
                    &device_id,
                    &job_id,
                )
                .await
            }
            None => Err("登录态不可用，源文件名将在下次登录后继续恢复".to_string()),
        };
        match restore_result {
            Ok((_restored, _, failed)) if failed > 0 => {
                let _ = update_and_emit_developer_job(&app, &database_path, &job_id, |job| {
                    job.message = Some(format!(
                        "{}；{failed} 个源文件名恢复失败，请稍后重试",
                        job.message
                            .clone()
                            .unwrap_or_else(|| "小号互传已结束".to_string())
                    ));
                });
            }
            Ok((restored, _, 0)) if restored > 0 => {
                let _ = update_and_emit_developer_job(&app, &database_path, &job_id, |job| {
                    job.message = Some(format!(
                        "{}，源文件名已恢复",
                        job.message
                            .clone()
                            .unwrap_or_else(|| "小号互传已结束".to_string())
                    ));
                });
            }
            Err(error) => {
                let _ = update_and_emit_developer_job(&app, &database_path, &job_id, |job| {
                    job.message = Some(format!(
                        "{}；{error}",
                        job.message
                            .clone()
                            .unwrap_or_else(|| "小号互传已结束".to_string())
                    ));
                });
            }
            _ => {}
        }
    }
    if let Ok(mut guard) = state.lock() {
        guard.developer_transfer_running.remove(&job_id);
    }
}

pub(crate) fn spawn_developer_transfer_job(app: tauri::AppHandle, state: SharedState, job_id: String) {
    let should_spawn = state
        .lock()
        .map(|mut guard| guard.developer_transfer_running.insert(job_id.clone()))
        .unwrap_or(false);
    if should_spawn {
        tauri::async_runtime::spawn(run_developer_transfer_job(app, state, job_id));
    }
}

pub(crate) fn resume_developer_transfer_jobs(app: tauri::AppHandle, state: SharedState) -> Result<(), String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    if !developer_mode_requested(&database_path)? {
        return Ok(());
    }
    let bound_account_id =
        load_app_state(&database_path, "developer_account_id")?.unwrap_or_default();
    let verified_client_id =
        load_app_state(&database_path, "developer_verified_client_id")?.unwrap_or_default();
    let verified_at = load_app_state(&database_path, "developer_account_verified_at")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let (client_id, _, _, _) = developer_credentials(&database_path)?;
    if bound_account_id.is_empty() || verified_at <= 0 || verified_client_id != client_id {
        return Ok(());
    }
    let connection = open_database(&database_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM developer_transfer_jobs
             WHERE status IN ('queued', 'direct', 'auditing', 'copying', 'running')
             ORDER BY created_at",
        )
        .map_err(|error| format!("读取未完成小号互传任务失败：{error}"))?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("读取未完成小号互传任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析未完成小号互传任务失败：{error}"))?;
    drop(statement);
    let mut restore_statement = connection
        .prepare(
            "SELECT DISTINCT restores.job_id
             FROM developer_transfer_name_restores AS restores
             JOIN developer_transfer_jobs AS jobs ON jobs.id = restores.job_id
             WHERE restores.status IN ('active', 'released', 'restore_failed')
               AND jobs.status IN ('success', 'failed')",
        )
        .map_err(|error| format!("读取待恢复预审文件名失败：{error}"))?;
    let restore_ids = restore_statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("读取待恢复预审文件名失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析待恢复预审文件名失败：{error}"))?;
    drop(restore_statement);
    drop(connection);
    let auth = state.lock().ok().and_then(|guard| {
        guard
            .token
            .clone()
            .map(|token| (token, guard.device_id.clone()))
    });
    if let Some((token, device_id)) = auth {
        for restore_id in restore_ids {
            let app = app.clone();
            let database_path = database_path.clone();
            let token = token.clone();
            let device_id = device_id.clone();
            tauri::async_runtime::spawn(async move {
                let _ = release_developer_name_obfuscation(
                    &app,
                    &database_path,
                    &token,
                    &device_id,
                    &restore_id,
                )
                .await;
            });
        }
    }
    for id in ids {
        spawn_developer_transfer_job(app.clone(), state.clone(), id);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn start_developer_transfer(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    target_id: String,
    file_ids: Vec<String>,
    file_names: Option<Vec<String>>,
) -> Result<DeveloperTransferJob, String> {
    let target_id = normalize_api_id(&target_id, "小号配置 ID")?;
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    if file_ids.len() > 20 {
        return Err("开发者接口一次最多互传 20 项".to_string());
    }
    ensure_developer_mode_for_current_account(&state, file_ids.first().map(String::as_str)).await?;
    let normalized_names = file_names
        .unwrap_or_default()
        .into_iter()
        .take(file_ids.len())
        .map(|value| value.chars().take(255).collect::<String>())
        .collect::<Vec<_>>();
    let mut pairs = file_ids
        .into_iter()
        .enumerate()
        .map(|(index, file_id)| {
            (
                file_id,
                normalized_names.get(index).cloned().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    let file_ids = pairs
        .iter()
        .map(|(file_id, _)| file_id.clone())
        .collect::<Vec<_>>();
    let file_names = pairs
        .into_iter()
        .map(|(_, file_name)| file_name)
        .collect::<Vec<_>>();
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let (client_id, client_secret, _, _) = developer_credentials(&database_path)?;
    if client_id.is_empty() || client_secret.is_empty() {
        return Err("请先在设置中填写开发者 client_id 和 client_secret".to_string());
    }
    let connection = open_database(&database_path)?;
    let target_name = connection
        .query_row(
            "SELECT name FROM developer_targets WHERE id = ?1",
            params![target_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取小号 TOKEN 配置失败：{error}"))?
        .ok_or_else(|| "请选择有效的小号接收 TOKEN".to_string())?;
    let file_ids_json = serde_json::to_string(&file_ids).map_err(|error| error.to_string())?;
    let duplicate = connection
        .query_row(
            &format!(
                "SELECT {DEVELOPER_JOB_COLUMNS} FROM developer_transfer_jobs
                 WHERE target_id = ?1 AND file_ids_json = ?2
                   AND status IN ('queued', 'direct', 'auditing', 'copying', 'running')
                 ORDER BY created_at DESC LIMIT 1"
            ),
            params![target_id, file_ids_json],
            developer_job_from_row,
        )
        .optional()
        .map_err(|error| format!("检查重复小号互传任务失败：{error}"))?;
    if let Some(job) = duplicate {
        return Ok(job);
    }
    let id = Uuid::new_v4().to_string();
    let now = unix_timestamp();
    connection
        .execute(
            "INSERT INTO developer_transfer_jobs
               (id, target_id, target_name, file_ids_json, file_names_json,
                status, phase, total_count, work_total_count, message, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'direct', 'direct', ?6, ?6, ?7, ?8, ?8)",
            params![
                id,
                target_id,
                target_name,
                file_ids_json,
                serde_json::to_string(&file_names).map_err(|error| error.to_string())?,
                file_ids.len() as i64,
                "正在并发启动小号秒传",
                now,
            ],
        )
        .map_err(|error| format!("创建小号互传任务失败：{error}"))?;
    let job = load_developer_transfer_job(&database_path, &id)?
        .ok_or_else(|| "创建后无法读取小号互传任务".to_string())?;
    emit(&app, json!({ "type": "developer-transfer", "job": job }));
    spawn_developer_transfer_job(app, state.inner().clone(), id);
    Ok(job)
}
