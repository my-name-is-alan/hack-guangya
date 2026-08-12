//! 云端异步任务轮询（上传入库、文件操作任务）。

use crate::prelude::*;

pub(crate) fn classify_upload_task_response(
    http_status: u16,
    result: ApiResponse,
) -> Result<CloudTaskCheck, CloudConfirmError> {
    let ApiResponse { code, msg, data } = result;
    if business_auth_expired(http_status, code) {
        return Err(CloudConfirmError::Retryable(
            "登录态已失效，请重新打开官方登录页".to_string(),
        ));
    }

    let error_message = || {
        let detail = if msg.trim().is_empty() {
            format!("业务码 {code}")
        } else {
            format!("{msg}（业务码 {code}）")
        };
        format!("云端入库查询失败：HTTP {http_status}，{detail}")
    };
    if !(200..300).contains(&http_status) {
        let message = error_message();
        return if http_status >= 500 || matches!(http_status, 408 | 429) {
            Err(CloudConfirmError::Retryable(message))
        } else {
            Err(CloudConfirmError::Permanent(message))
        };
    }

    match code {
        147 => Ok(CloudTaskCheck::Pending),
        0 => data
            .filter(|data| {
                data.get("fileId")
                    .and_then(Value::as_str)
                    .is_some_and(|file_id| !file_id.trim().is_empty())
            })
            .map(CloudTaskCheck::Confirmed)
            .ok_or_else(|| {
                CloudConfirmError::Permanent(
                    "云端入库成功响应缺少有效的 fileId，已停止轮询".to_string(),
                )
            }),
        _ => Err(CloudConfirmError::Permanent(error_message())),
    }
}

pub(crate) async fn check_upload_task(
    token: &str,
    device_id: &str,
    task_id: &str,
) -> Result<CloudTaskCheck, CloudConfirmError> {
    match api_post_response(
        token,
        device_id,
        "/userres/v1/file/get_info_by_task_id",
        json!({ "taskId": task_id }),
    )
    .await
    {
        Ok((http_status, result)) => classify_upload_task_response(http_status, result),
        Err(BusinessRequestError::InvalidResponse {
            http_status: 401, ..
        }) => Err(CloudConfirmError::Retryable(
            "登录态已失效，请重新打开官方登录页".to_string(),
        )),
        Err(BusinessRequestError::InvalidResponse {
            http_status,
            message,
        }) if http_status >= 500 || matches!(http_status, 408 | 429) => {
            Err(CloudConfirmError::Retryable(message))
        }
        Err(BusinessRequestError::InvalidResponse { message, .. }) => {
            Err(CloudConfirmError::Permanent(message))
        }
        Err(BusinessRequestError::Request(message)) => Err(CloudConfirmError::Retryable(message)),
    }
}

pub(crate) async fn wait_upload_task(
    app: &tauri::AppHandle,
    token: &str,
    device_id: &str,
    task_id: &str,
    file_path: &Path,
) -> Result<Value, CloudConfirmError> {
    let deadline = Instant::now() + Duration::from_secs(CLOUD_CONFIRM_TIMEOUT_SECS);
    let mut attempt = 0_u64;
    while Instant::now() < deadline {
        match check_upload_task(token, device_id, task_id).await {
            Ok(CloudTaskCheck::Confirmed(data)) => return Ok(data),
            Ok(CloudTaskCheck::Pending) => {}
            Err(CloudConfirmError::Retryable(message)) if message.contains("登录态已失效") => {
                return Err(CloudConfirmError::Retryable(message));
            }
            Err(CloudConfirmError::Retryable(_)) => {}
            Err(error @ CloudConfirmError::Permanent(_)) => return Err(error),
        }
        attempt += 1;
        emit(
            app,
            json!({ "type": "progress", "file_path": file_path.to_string_lossy(), "percent": 100, "stage": "文件已上传，云端正在入库" }),
        );
        let delay = Duration::from_secs(attempt.div_ceil(5).clamp(1, 5));
        sleep(delay.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    Err(CloudConfirmError::Retryable(format!(
        "云端入库超过 {CLOUD_CONFIRM_TIMEOUT_SECS} 秒仍未完成，请稍后刷新云盘确认"
    )))
}

pub(crate) async fn wait_operation_task(token: &str, device_id: &str, task_id: &str) -> Result<(), String> {
    // 单次查询失败不代表云端任务失败：任务仍在服务端执行，轮询接口的瞬时
    // 5xx/网络抖动应继续等待，只有连续多次失败才放弃（此前一次失败就把
    // 整个移动/删除操作报错，实际操作却已在云端完成）。
    let mut consecutive_failures = 0_u32;
    for _ in 0..90 {
        let result = match api_post(
            token,
            device_id,
            "/userres/v1/get_task_status",
            json!({ "taskId": task_id }),
            &[],
        )
        .await
        {
            Ok(result) => {
                consecutive_failures = 0;
                result
            }
            Err(error) => {
                consecutive_failures += 1;
                if consecutive_failures >= 5 || error.contains("登录态已失效") {
                    return Err(error);
                }
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let data = result.data.unwrap_or_default();
        let status_code = data.get("status").and_then(Value::as_i64).unwrap_or(0);
        let detail = data.get("detail").cloned().unwrap_or_default();
        let detail_code = detail.get("code").and_then(Value::as_i64).unwrap_or(0);
        if [2, 3].contains(&status_code) && detail_code != 0 {
            return Err(detail
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("文件操作失败")
                .to_string());
        }
        if status_code == 2 {
            return Ok(());
        }
        if status_code == 3 {
            return Err(detail
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("文件操作失败")
                .to_string());
        }
        sleep(Duration::from_secs(1)).await;
    }
    Err("文件操作长时间没有完成，请稍后刷新网盘".into())
}


pub(crate) fn operation_task_id(data: &Value) -> Option<String> {
    let value = data
        .get("taskId")
        .or_else(|| data.get("taskID"))
        .unwrap_or(data);
    let task_id = value_as_id(Some(value));
    (!task_id.trim().is_empty()).then(|| task_id.trim().to_string())
}

pub(crate) async fn finish_operation_response(
    token: &str,
    device_id: &str,
    response: ApiResponse,
) -> Result<Value, String> {
    let data = response.data.unwrap_or_else(|| json!({}));
    if let Some(task_id) = operation_task_id(&data) {
        wait_operation_task(token, device_id, &task_id).await?;
    }
    Ok(data)
}
