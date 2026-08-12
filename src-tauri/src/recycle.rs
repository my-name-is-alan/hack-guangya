//! 回收站：列表、恢复、彻底删除与清空任务。

use crate::prelude::*;

#[derive(Default)]
pub(crate) struct RecycleBinClearFlightState {
    pub(crate) generation: u64,
    pub(crate) running: bool,
    pub(crate) result: Option<(u64, Result<Value, String>)>,
}

#[derive(Default)]
pub(crate) struct RecycleBinClearFlight {
    pub(crate) state: tokio::sync::Mutex<RecycleBinClearFlightState>,
    pub(crate) notify: Notify,
}

pub(crate) static CLEAR_RECYCLE_BIN_FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<RecycleBinClearFlight>>>> =
    OnceLock::new();

pub(crate) fn recycle_bin_clear_flight(account_scope: &str) -> Arc<RecycleBinClearFlight> {
    let flights = CLEAR_RECYCLE_BIN_FLIGHTS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut flights) = flights.lock() else {
        return Arc::new(RecycleBinClearFlight::default());
    };
    flights
        .entry(account_scope.to_string())
        .or_insert_with(|| Arc::new(RecycleBinClearFlight::default()))
        .clone()
}


pub(crate) fn recycle_file_list_request(page: Option<u64>) -> Value {
    json!({
        "page": page.unwrap_or(0),
        "pageSize": DEFAULT_API_PAGE_SIZE,
        "parentId": "",
        "dirType": 4,
        "orderBy": 12,
        "sortType": 1
    })
}

pub(crate) fn clear_recycle_bin_request() -> (&'static str, Value) {
    ("/userres/v1/file/clear_recycle_bin", json!({}))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecycleBinClearOperation {
    Unknown { updated_at: i64 },
    Task { task_id: String, updated_at: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecycleBinClearAction {
    Submit,
    ProtectUnknown,
    ResumeTask,
}

pub(crate) fn jwt_account_identity(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if header.is_empty() || payload.is_empty() || signature.is_empty() || parts.next().is_some() {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    let account_id = [
        "sub",
        "accountId",
        "account_id",
        "userId",
        "user_id",
        "uid",
        "id",
    ]
    .into_iter()
    .find_map(|key| {
        let value = value_as_id(claims.get(key));
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })?;
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(API_BASE);
    Some(format!("{issuer}\0{account_id}"))
}

pub(crate) fn stable_account_scope_from_token(token: &str) -> Option<String> {
    jwt_account_identity(token).map(|identity| {
        format!(
            "account:{}",
            hex::encode(Sha256::digest(identity.as_bytes()))
        )
    })
}

pub(crate) fn new_auth_account_scope(token: &str) -> String {
    stable_account_scope_from_token(token)
        .unwrap_or_else(|| format!("session:{}", Uuid::new_v4().simple()))
}

pub(crate) fn persist_auth_account_scope(database: &Path, account_scope: &str) -> Result<(), String> {
    open_database(database)?
        .execute(
            "UPDATE auth_session SET account_scope = ?1, updated_at = ?2 WHERE id = 1",
            params![account_scope, unix_timestamp()],
        )
        .map_err(|error| format!("保存登录账号范围失败：{error}"))?;
    Ok(())
}

pub(crate) fn ensure_auth_account_scope(database: &Path, session: &mut AuthSession) -> Result<(), String> {
    if session.access_token.is_none() || session.account_scope.is_some() {
        return Ok(());
    }
    let account_scope = new_auth_account_scope(session.access_token.as_deref().unwrap_or_default());
    persist_auth_account_scope(database, &account_scope)?;
    session.account_scope = Some(account_scope);
    Ok(())
}

pub(crate) fn load_recycle_bin_clear_operation(
    database: &Path,
    account_scope: &str,
) -> Result<Option<RecycleBinClearOperation>, String> {
    open_database(database)?
        .query_row(
            "SELECT state, task_id, updated_at
             FROM recycle_bin_clear_operations WHERE account_scope = ?1",
            params![account_scope],
            |row| {
                let state: String = row.get(0)?;
                let task_id: Option<String> = row.get(1)?;
                let updated_at: i64 = row.get(2)?;
                Ok((state, task_id, updated_at))
            },
        )
        .optional()
        .map_err(|error| format!("读取清空回收站任务状态失败：{error}"))?
        .map(|(state, task_id, updated_at)| match state.as_str() {
            "unknown" => Ok(RecycleBinClearOperation::Unknown { updated_at }),
            "task" => task_id
                .filter(|value| !value.trim().is_empty())
                .map(|task_id| RecycleBinClearOperation::Task {
                    task_id,
                    updated_at,
                })
                .ok_or_else(|| "清空回收站任务状态损坏：缺少 taskId".to_string()),
            _ => Err(format!("清空回收站任务状态损坏：{state}")),
        })
        .transpose()
}

pub(crate) fn save_recycle_bin_clear_operation(
    database: &Path,
    account_scope: &str,
    operation: &RecycleBinClearOperation,
) -> Result<(), String> {
    let (state, task_id, updated_at) = match operation {
        RecycleBinClearOperation::Unknown { updated_at } => ("unknown", None, *updated_at),
        RecycleBinClearOperation::Task {
            task_id,
            updated_at,
        } => ("task", Some(task_id.as_str()), *updated_at),
    };
    open_database(database)?
        .execute(
            "INSERT INTO recycle_bin_clear_operations
               (account_scope, state, task_id, last_error, created_at, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?4)
             ON CONFLICT(account_scope) DO UPDATE SET
               state = excluded.state,
               task_id = excluded.task_id,
               last_error = NULL,
               updated_at = excluded.updated_at",
            params![account_scope, state, task_id, updated_at],
        )
        .map_err(|error| format!("保存清空回收站任务状态失败：{error}"))?;
    Ok(())
}

pub(crate) fn clear_recycle_bin_operation(database: &Path, account_scope: &str) -> Result<(), String> {
    open_database(database)?
        .execute(
            "DELETE FROM recycle_bin_clear_operations WHERE account_scope = ?1",
            params![account_scope],
        )
        .map_err(|error| format!("清理清空回收站任务状态失败：{error}"))?;
    Ok(())
}

pub(crate) fn plan_recycle_bin_clear(
    operation: Option<&RecycleBinClearOperation>,
    force_retry: bool,
) -> RecycleBinClearAction {
    match operation {
        None => RecycleBinClearAction::Submit,
        Some(RecycleBinClearOperation::Task { .. }) => RecycleBinClearAction::ResumeTask,
        Some(RecycleBinClearOperation::Unknown { .. }) if force_retry => {
            RecycleBinClearAction::Submit
        }
        Some(RecycleBinClearOperation::Unknown { .. }) => RecycleBinClearAction::ProtectUnknown,
    }
}

pub(crate) fn recycle_bin_clear_pending(state: &str, task_id: Option<&str>, message: String) -> Value {
    json!({
        "completed": false,
        "pending": true,
        "state": state,
        "taskId": task_id,
        "message": message,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RecycleBinTaskStatus {
    Pending,
    Succeeded,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialRecycleBinClearResponseClass {
    Accepted,
    Ambiguous,
    DefinitiveRejection,
}

pub(crate) fn transient_recycle_bin_clear_business_code(code: i64) -> bool {
    matches!(code, 100 | 101 | 102 | 103 | 408 | 429 | 18010 | 18013)
}

pub(crate) fn classify_initial_recycle_bin_clear_response(
    http_status: u16,
    business_code: i64,
) -> InitialRecycleBinClearResponseClass {
    if business_auth_expired(http_status, business_code) {
        return InitialRecycleBinClearResponseClass::DefinitiveRejection;
    }
    if (200..300).contains(&http_status) && business_code == 0 {
        return InitialRecycleBinClearResponseClass::Accepted;
    }
    if matches!(http_status, 408 | 425 | 429)
        || http_status >= 500
        || transient_recycle_bin_clear_business_code(business_code)
    {
        return InitialRecycleBinClearResponseClass::Ambiguous;
    }
    if (400..500).contains(&http_status)
        || ((200..300).contains(&http_status) && business_code != 0)
    {
        return InitialRecycleBinClearResponseClass::DefinitiveRejection;
    }
    InitialRecycleBinClearResponseClass::Ambiguous
}

pub(crate) fn initial_recycle_bin_clear_error(http_status: u16, response: &ApiResponse) -> String {
    if business_auth_expired(http_status, response.code) {
        "登录态已失效，请重新打开官方登录页".to_string()
    } else if response.msg.trim().is_empty() {
        format!("光鸭接口失败：HTTP {http_status}/{}", response.code)
    } else {
        response.msg.clone()
    }
}

pub(crate) fn classify_recycle_bin_task_status(data: &Value) -> RecycleBinTaskStatus {
    let status_code = data.get("status").and_then(Value::as_i64).unwrap_or(0);
    let detail = data.get("detail").cloned().unwrap_or_default();
    let detail_code = detail.get("code").and_then(Value::as_i64).unwrap_or(0);
    let message = || {
        detail
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("清空回收站失败")
            .to_string()
    };
    if [2, 3].contains(&status_code) && detail_code != 0 {
        RecycleBinTaskStatus::Failed(message())
    } else if status_code == 2 {
        RecycleBinTaskStatus::Succeeded
    } else if status_code == 3 {
        RecycleBinTaskStatus::Failed(message())
    } else {
        RecycleBinTaskStatus::Pending
    }
}

pub(crate) async fn wait_recycle_bin_clear_task(
    token: &str,
    device_id: &str,
    task_id: &str,
    deadline: Instant,
) -> Result<RecycleBinTaskStatus, String> {
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let response = tokio::time::timeout(
            remaining,
            api_post(
                token,
                device_id,
                "/userres/v1/get_task_status",
                json!({ "taskId": task_id }),
                &[],
            ),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) if error.contains("登录态已失效") => return Err(error),
            Ok(Err(_)) | Err(_) => {
                if Instant::now() >= deadline {
                    break;
                }
                sleep(
                    Duration::from_secs(CLEAR_RECYCLE_BIN_POLL_INTERVAL_SECS)
                        .min(deadline.saturating_duration_since(Instant::now())),
                )
                .await;
                continue;
            }
        };
        let data = response.data.unwrap_or_default();
        let status = classify_recycle_bin_task_status(&data);
        if !matches!(status, RecycleBinTaskStatus::Pending) {
            return Ok(status);
        }
        sleep(
            Duration::from_secs(CLEAR_RECYCLE_BIN_POLL_INTERVAL_SECS)
                .min(deadline.saturating_duration_since(Instant::now())),
        )
        .await;
    }
    Ok(RecycleBinTaskStatus::Pending)
}

pub(crate) async fn run_recycle_bin_clear_singleflight<F, Fut>(
    account_scope: &str,
    operation: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let flight = recycle_bin_clear_flight(account_scope);
    let generation = {
        let mut state = flight.state.lock().await;
        if state.running {
            state.generation
        } else {
            state.running = true;
            state.generation = state.generation.wrapping_add(1);
            state.result = None;
            let generation = state.generation;
            drop(state);
            let result = operation().await;
            let mut state = flight.state.lock().await;
            state.running = false;
            state.result = Some((generation, result.clone()));
            drop(state);
            flight.notify.notify_waiters();
            return result;
        }
    };

    loop {
        let notified = flight.notify.notified();
        if let Some(result) = {
            let state = flight.state.lock().await;
            state
                .result
                .as_ref()
                .filter(|(result_generation, _)| *result_generation == generation)
                .map(|(_, result)| result.clone())
        } {
            return result;
        }
        notified.await;
    }
}


#[tauri::command]
pub(crate) async fn list_recycle_files(
    state: tauri::State<'_, SharedState>,
    page: Option<u64>,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/get_file_list",
        recycle_file_list_request(page),
        &[],
    )
    .await?;
    Ok(response
        .data
        .unwrap_or_else(|| json!({ "list": [], "total": 0 })))
}


pub(crate) async fn clear_recycle_bin_inner(
    token: &str,
    device_id: &str,
    database: &Path,
    account_scope: &str,
    force_retry: bool,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(CLEAR_RECYCLE_BIN_DEADLINE_SECS);

    let mut operation = load_recycle_bin_clear_operation(database, account_scope)?;
    match plan_recycle_bin_clear(operation.as_ref(), force_retry) {
        RecycleBinClearAction::ProtectUnknown => {
            return Ok(recycle_bin_clear_pending(
                "unknown",
                None,
                "上次清空请求的结果无法确认，程序不会自动重发；请先刷新回收站确认，只有显式强制重试才会重新提交"
                    .to_string(),
            ));
        }
        RecycleBinClearAction::Submit if operation.is_some() => {
            clear_recycle_bin_operation(database, account_scope)?;
            operation = None;
        }
        RecycleBinClearAction::Submit | RecycleBinClearAction::ResumeTask => {}
    }

    if operation.is_none() {
        save_recycle_bin_clear_operation(
            database,
            account_scope,
            &RecycleBinClearOperation::Unknown {
                updated_at: unix_timestamp(),
            },
        )?;
        let (endpoint, request) = clear_recycle_bin_request();
        let response = match tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            api_post_response(token, device_id, endpoint, request),
        )
        .await
        {
            Ok(Ok((http_status, response))) => {
                match classify_initial_recycle_bin_clear_response(http_status, response.code) {
                    InitialRecycleBinClearResponseClass::Accepted => response,
                    InitialRecycleBinClearResponseClass::Ambiguous => {
                        return Ok(recycle_bin_clear_pending(
                            "unknown",
                            None,
                            "云端返回了暂时无法确定执行结果的响应，程序不会自动重发；请先刷新回收站确认，只有显式强制重试才会重新提交"
                                .to_string(),
                        ));
                    }
                    InitialRecycleBinClearResponseClass::DefinitiveRejection => {
                        clear_recycle_bin_operation(database, account_scope)?;
                        return Err(initial_recycle_bin_clear_error(http_status, &response));
                    }
                }
            }
            Ok(Err(_)) | Err(_) => {
                return Ok(recycle_bin_clear_pending(
                    "unknown",
                    None,
                    "清空请求结果无法确认，程序不会自动重发；请先刷新回收站确认，只有显式强制重试才会重新提交"
                        .to_string(),
                ));
            }
        };
        let data = response.data.unwrap_or_else(|| json!({}));
        let Some(task_id) = operation_task_id(&data) else {
            clear_recycle_bin_operation(database, account_scope)?;
            return Ok(json!({
                "completed": true,
                "pending": false,
                "state": "completed",
                "data": data,
            }));
        };
        let task = RecycleBinClearOperation::Task {
            task_id,
            updated_at: unix_timestamp(),
        };
        save_recycle_bin_clear_operation(database, account_scope, &task)?;
        operation = Some(task);
    }

    let RecycleBinClearOperation::Task { task_id, .. } = operation.expect("operation created")
    else {
        unreachable!("unknown operations return before task polling")
    };
    match wait_recycle_bin_clear_task(token, device_id, &task_id, deadline).await? {
        RecycleBinTaskStatus::Succeeded => {
            clear_recycle_bin_operation(database, account_scope)?;
            Ok(json!({
                "completed": true,
                "pending": false,
                "state": "completed",
                "taskId": task_id,
            }))
        }
        RecycleBinTaskStatus::Failed(message) => {
            clear_recycle_bin_operation(database, account_scope)?;
            Err(message)
        }
        RecycleBinTaskStatus::Pending => Ok(recycle_bin_clear_pending(
            "pending",
            Some(&task_id),
            "云端仍在清空回收站，再次点击将继续查询同一个任务，不会重复提交".to_string(),
        )),
    }
}

/// 通知前端回收站内容已变化（删除/恢复/彻底删除/清空后都会触发）。
///
/// 回收站不属于普通目录树，`cloud-directory-invalidated` 覆盖不到它；
/// 没有这个事件时，已打开的回收站面板会一直显示旧列表。
pub(crate) fn publish_recycle_bin_changed(app: &tauri::AppHandle, source: &str) {
    emit(
        app,
        json!({ "type": "cloud-recycle-bin-changed", "source": source }),
    );
}

#[tauri::command(rename_all = "snake_case")]
pub(crate) async fn clear_recycle_bin(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    force_retry: Option<bool>,
) -> Result<Value, String> {
    let (token, device_id, database, account_scope) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.db_path.clone(),
            guard
                .auth_account_scope
                .clone()
                .ok_or_else(|| "登录会话缺少账号范围，请重新登录".to_string())?,
        )
    };
    let force_retry = force_retry.unwrap_or(false);
    let result = run_recycle_bin_clear_singleflight(&account_scope, || async {
        clear_recycle_bin_inner(&token, &device_id, &database, &account_scope, force_retry).await
    })
    .await?;
    if result
        .get("completed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        publish_recycle_bin_changed(&app, "clear-recycle-bin");
    }
    Ok(result)
}
