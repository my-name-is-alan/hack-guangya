//! 下载：单流/分片下载、暂停恢复、打包任务。

use crate::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DownloadControlState {
    Running,
    Paused,
    Cancelled,
}

#[derive(Clone, Default)]
pub(crate) struct DownloadRegistry {
    pub(crate) tasks: Arc<Mutex<HashMap<String, watch::Sender<DownloadControlState>>>>,
}

pub(crate) struct DownloadRegistration {
    pub(crate) registry: DownloadRegistry,
    pub(crate) download_id: String,
}

impl Drop for DownloadRegistration {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.registry.tasks.lock() {
            tasks.remove(&self.download_id);
        }
    }
}


#[tauri::command]
pub(crate) fn pause_download(
    app: tauri::AppHandle,
    downloads: tauri::State<'_, DownloadRegistry>,
    task_id: String,
) -> Result<Value, String> {
    set_download_control(downloads.inner(), &task_id, DownloadControlState::Paused)?;
    emit(
        &app,
        json!({ "type": "download", "download_id": task_id, "state": "paused", "bytes_per_second": 0 }),
    );
    Ok(json!({ "task_id": task_id, "state": "paused" }))
}

#[tauri::command]
pub(crate) fn resume_download(
    app: tauri::AppHandle,
    downloads: tauri::State<'_, DownloadRegistry>,
    task_id: String,
) -> Result<Value, String> {
    set_download_control(downloads.inner(), &task_id, DownloadControlState::Running)?;
    emit(
        &app,
        json!({ "type": "download", "download_id": task_id, "state": "downloading", "bytes_per_second": 0 }),
    );
    Ok(json!({ "task_id": task_id, "state": "downloading" }))
}

#[tauri::command]
pub(crate) fn cancel_download(
    app: tauri::AppHandle,
    downloads: tauri::State<'_, DownloadRegistry>,
    task_id: String,
) -> Result<Value, String> {
    set_download_control(downloads.inner(), &task_id, DownloadControlState::Cancelled)?;
    emit(
        &app,
        json!({ "type": "download", "download_id": task_id, "state": "cancelled", "bytes_per_second": 0 }),
    );
    Ok(json!({ "task_id": task_id, "state": "cancelled" }))
}

#[tauri::command]
pub(crate) async fn get_received_share_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    downloads: tauri::State<'_, DownloadRegistry>,
    access_token: String,
    file_ids: Vec<String>,
    packaged: bool,
    file_name: String,
    destination_dir: String,
    download_id: String,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    if access_token.trim().is_empty() {
        return Err("分享访问令牌为空，请重新打开分享链接".into());
    }
    if !packaged && file_ids.len() != 1 {
        return Err("单文件下载只能选择一个文件".into());
    }
    let (mut download_control, _download_registration) =
        begin_download_task(downloads.inner(), &download_id)?;
    let download_task_concurrency = current_download_task_concurrency(state.inner())?;
    let (token, device_id) = auth_context(&state)?;
    wait_download_running(&mut download_control).await?;
    if !packaged {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/get_share_download_url",
            json!({ "fileId": file_ids[0], "accessToken": access_token }),
            &[205, 206, 207, 504],
        )
        .await?;
        wait_download_running(&mut download_control).await?;
        if response.code != 0 {
            return Err(format!(
                "当前分享下载受限，请到光鸭官方页面处理（业务码 {}：{}）",
                response.code, response.msg
            ));
        }
        let data = response.data.unwrap_or_else(|| json!({}));
        let download_url = data
            .get("downloadUrl")
            .or_else(|| data.get("downloadURL"))
            .or_else(|| data.get("signedURL"))
            .or_else(|| data.get("signedUrl"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "光鸭没有返回文件下载地址".to_string())?
            .to_string();
        return download_to_local(
            &app,
            &download_url,
            &file_name,
            &destination_dir,
            &download_id,
            download_task_concurrency,
            download_control,
        )
        .await;
    }
    let response = api_post(
        &token,
        &device_id,
        "/scheduler/v1/create_packaging_task",
        json!({ "fileIds": file_ids, "accessToken": access_token }),
        &[205, 206, 207, 504],
    )
    .await?;
    wait_download_running(&mut download_control).await?;
    if response.code != 0 {
        return Err(format!(
            "当前批量下载受限，请到光鸭官方页面处理（业务码 {}：{}）",
            response.code, response.msg
        ));
    }
    let task_id = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "光鸭没有返回压缩任务 ID".to_string())?
        .to_string();
    for _ in 0..600 {
        wait_download_running(&mut download_control).await?;
        let result = api_post(
            &token,
            &device_id,
            "/scheduler/v1/query_packaging_task",
            json!({ "taskId": task_id, "accessToken": access_token }),
            &[],
        )
        .await?;
        wait_download_running(&mut download_control).await?;
        let data = result.data.unwrap_or_else(|| json!({}));
        if let Some(download_url) = data
            .get("signedURL")
            .or_else(|| data.get("signedUrl"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return download_to_local(
                &app,
                download_url,
                &file_name,
                &destination_dir,
                &download_id,
                download_task_concurrency,
                download_control,
            )
            .await;
        }
        ensure_packaging_task_active(&data)?;
        await_download_operation(
            &mut download_control,
            sleep(Duration::from_secs(1)),
            Duration::from_secs(2),
            || "等待打包任务响应超时".to_string(),
        )
        .await?;
    }
    Err("光鸭打包超过 10 分钟仍未完成，请稍后重试".into())
}

#[tauri::command]
pub(crate) async fn get_cloud_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    downloads: tauri::State<'_, DownloadRegistry>,
    file_ids: Vec<String>,
    packaged: bool,
    file_name: String,
    destination_dir: String,
    download_id: String,
) -> Result<Value, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    if !packaged && file_ids.len() != 1 {
        return Err("单文件下载只能选择一个文件".into());
    }
    let (mut download_control, _download_registration) =
        begin_download_task(downloads.inner(), &download_id)?;
    let download_task_concurrency = current_download_task_concurrency(state.inner())?;
    let (token, device_id) = auth_context(&state)?;
    wait_download_running(&mut download_control).await?;
    if !packaged {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/get_res_download_url",
            json!({ "fileId": file_ids[0] }),
            &[],
        )
        .await?;
        wait_download_running(&mut download_control).await?;
        let data = response.data.unwrap_or_else(|| json!({}));
        let download_url = data
            .get("signedURL")
            .or_else(|| data.get("signedUrl"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "光鸭没有返回文件下载地址".to_string())?
            .to_string();
        return download_to_local(
            &app,
            &download_url,
            &file_name,
            &destination_dir,
            &download_id,
            download_task_concurrency,
            download_control,
        )
        .await;
    }
    let response = api_post(
        &token,
        &device_id,
        "/scheduler/v1/create_packaging_task",
        json!({ "fileIds": file_ids }),
        &[205, 206, 207, 504],
    )
    .await?;
    wait_download_running(&mut download_control).await?;
    if response.code != 0 {
        return Err(format!(
            "当前批量下载受限（业务码 {}：{}）",
            response.code, response.msg
        ));
    }
    let task_id = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "光鸭没有返回压缩任务 ID".to_string())?
        .to_string();
    for _ in 0..600 {
        wait_download_running(&mut download_control).await?;
        let result = api_post(
            &token,
            &device_id,
            "/scheduler/v1/query_packaging_task",
            json!({ "taskId": task_id }),
            &[],
        )
        .await?;
        wait_download_running(&mut download_control).await?;
        let data = result.data.unwrap_or_else(|| json!({}));
        if let Some(download_url) = data
            .get("signedURL")
            .or_else(|| data.get("signedUrl"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return download_to_local(
                &app,
                download_url,
                &file_name,
                &destination_dir,
                &download_id,
                download_task_concurrency,
                download_control,
            )
            .await;
        }
        ensure_packaging_task_active(&data)?;
        await_download_operation(
            &mut download_control,
            sleep(Duration::from_secs(1)),
            Duration::from_secs(2),
            || "等待打包任务响应超时".to_string(),
        )
        .await?;
    }
    Err("光鸭打包超过 10 分钟仍未完成，请稍后重试".into())
}

pub(crate) fn ensure_packaging_task_active(data: &Value) -> Result<(), String> {
    let status = data
        .get("status")
        .or_else(|| data.get("state"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let failed = matches!(
        status.as_str(),
        "failed" | "failure" | "error" | "cancelled" | "canceled" | "expired"
    );
    let error_code = data
        .get("errorCode")
        .or_else(|| data.get("error_code"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or(0);
    if !failed && error_code == 0 {
        return Ok(());
    }
    Err(data
        .get("message")
        .or_else(|| data.get("msg"))
        .or_else(|| data.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("光鸭文件打包失败")
        .to_string())
}

pub(crate) fn safe_download_name(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches([' ', '.']).trim();
    if cleaned.is_empty() {
        "光鸭下载".to_string()
    } else {
        cleaned.to_string()
    }
}

pub(crate) fn available_download_path(directory: &Path, file_name: &str) -> PathBuf {
    let requested = Path::new(file_name);
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("光鸭下载");
    let extension = requested.extension().and_then(|value| value.to_str());
    let first = directory.join(file_name);
    if !first.exists() {
        return first;
    }
    for index in 1..10_000 {
        let candidate_name = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem} ({index}).{extension}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = directory.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}", Uuid::new_v4()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DownloadByteRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParsedContentRange {
    pub(crate) start: u64,
    pub(crate) end: u64,
    pub(crate) total: u64,
}

pub(crate) fn parse_content_range(value: &str) -> Option<ParsedContentRange> {
    let value = value.trim().strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let parsed = ParsedContentRange {
        start: start.parse().ok()?,
        end: end.parse().ok()?,
        total: total.parse().ok()?,
    };
    (parsed.start <= parsed.end && parsed.end < parsed.total).then_some(parsed)
}

pub(crate) fn response_content_range(response: &reqwest::Response) -> Option<ParsedContentRange> {
    response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_range)
}

pub(crate) fn response_total_bytes(response: &reqwest::Response) -> Option<u64> {
    response_content_range(response)
        .map(|value| value.total)
        .or_else(|| response.content_length().filter(|value| *value > 0))
}

pub(crate) fn configured_download_connections(download_task_concurrency: usize) -> usize {
    let task_concurrency = download_task_concurrency.clamp(1, MAX_TRANSFER_CONCURRENCY);
    (DOWNLOAD_MAX_HTTP_CONNECTIONS / task_concurrency).clamp(2, DOWNLOAD_MAX_CONNECTIONS_PER_FILE)
}

pub(crate) fn begin_download_task(
    registry: &DownloadRegistry,
    download_id: &str,
) -> Result<(watch::Receiver<DownloadControlState>, DownloadRegistration), String> {
    let download_id = download_id.trim();
    if download_id.is_empty() {
        return Err("下载任务 ID 为空".into());
    }
    if download_id.len() > MAX_API_ID_LENGTH {
        return Err("下载任务 ID 过长".into());
    }
    let mut tasks = registry
        .tasks
        .lock()
        .map_err(|_| "下载任务控制器不可用".to_string())?;
    if tasks.contains_key(download_id) {
        return Err("同一个下载任务正在运行".into());
    }
    let (sender, receiver) = watch::channel(DownloadControlState::Running);
    tasks.insert(download_id.to_string(), sender);
    Ok((
        receiver,
        DownloadRegistration {
            registry: registry.clone(),
            download_id: download_id.to_string(),
        },
    ))
}

pub(crate) fn set_download_control(
    registry: &DownloadRegistry,
    download_id: &str,
    state: DownloadControlState,
) -> Result<(), String> {
    let sender = registry
        .tasks
        .lock()
        .map_err(|_| "下载任务控制器不可用".to_string())?
        .get(download_id.trim())
        .cloned()
        .ok_or_else(|| "下载任务不存在或已经结束".to_string())?;
    if *sender.borrow() == DownloadControlState::Cancelled {
        return Err("下载任务已经取消".into());
    }
    sender.send_replace(state);
    Ok(())
}

pub(crate) fn download_is_cancelled(control: &watch::Receiver<DownloadControlState>) -> bool {
    *control.borrow() == DownloadControlState::Cancelled
}

pub(crate) async fn wait_download_running(
    control: &mut watch::Receiver<DownloadControlState>,
) -> Result<(), String> {
    loop {
        let state = *control.borrow_and_update();
        match state {
            DownloadControlState::Running => return Ok(()),
            DownloadControlState::Cancelled => return Err("下载已取消".into()),
            DownloadControlState::Paused => control
                .changed()
                .await
                .map_err(|_| "下载任务控制器已经关闭".to_string())?,
        }
    }
}

pub(crate) async fn await_download_operation<T>(
    control: &mut watch::Receiver<DownloadControlState>,
    operation: impl Future<Output = T>,
    timeout: Duration,
    timeout_error: impl Fn() -> String,
) -> Result<T, String> {
    tokio::pin!(operation);
    loop {
        wait_download_running(control).await?;
        let idle_timeout = sleep(timeout);
        tokio::pin!(idle_timeout);
        tokio::select! {
            result = &mut operation => return Ok(result),
            _ = &mut idle_timeout => return Err(timeout_error()),
            changed = control.changed() => {
                changed.map_err(|_| "下载任务控制器已经关闭".to_string())?;
            }
        }
    }
}

pub(crate) fn download_http_semaphore() -> Arc<Semaphore> {
    static LIMITER: OnceLock<Arc<Semaphore>> = OnceLock::new();
    LIMITER
        .get_or_init(|| Arc::new(Semaphore::new(DOWNLOAD_MAX_HTTP_CONNECTIONS)))
        .clone()
}

pub(crate) fn current_download_task_concurrency(state: &SharedState) -> Result<usize, String> {
    state
        .lock()
        .map(|guard| guard.download_concurrency)
        .map_err(|_| "读取下载并发设置失败".to_string())
}

pub(crate) fn download_byte_ranges(total_bytes: u64, connections: usize) -> Vec<DownloadByteRange> {
    if total_bytes == 0 {
        return Vec::new();
    }
    let target_chunks = connections.max(1).saturating_mul(4) as u64;
    let balanced = total_bytes / target_chunks + u64::from(total_bytes % target_chunks != 0);
    let chunk_size = balanced.clamp(DOWNLOAD_RANGE_MIN_BYTES, DOWNLOAD_RANGE_MAX_BYTES);
    let mut ranges = Vec::new();
    let mut start = 0_u64;
    while start < total_bytes {
        let end = start
            .saturating_add(chunk_size.saturating_sub(1))
            .min(total_bytes - 1);
        ranges.push(DownloadByteRange { start, end });
        start = end.saturating_add(1);
    }
    ranges
}

pub(crate) async fn probe_download(
    client: &reqwest::Client,
    download_url: &str,
    control: &mut watch::Receiver<DownloadControlState>,
) -> Result<(Option<u64>, bool), String> {
    wait_download_running(control).await?;
    let Ok(_permit) = download_http_semaphore().acquire_owned().await else {
        return Ok((None, false));
    };
    let response = match await_download_operation(
        control,
        client.get(download_url).header(RANGE, "bytes=0-0").send(),
        Duration::from_secs(DOWNLOAD_PROBE_TIMEOUT_SECS),
        || "探测下载分片能力超时".to_string(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return Ok((None, false)),
        Err(error) if download_is_cancelled(control) => return Err(error),
        Err(_) => return Ok((None, false)),
    };
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        if let Some(range) = response_content_range(&response) {
            return Ok((Some(range.total), range.start == 0 && range.end == 0));
        }
    }
    Ok((response_total_bytes(&response), false))
}

pub(crate) async fn download_range_to_file(
    client: &reqwest::Client,
    download_url: &str,
    partial: &Path,
    range: DownloadByteRange,
    total_bytes: u64,
    progress: &AtomicU64,
    mut control: watch::Receiver<DownloadControlState>,
) -> Result<(), String> {
    wait_download_running(&mut control).await?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(partial)
        .await
        .map_err(|error| format!("打开分片临时文件失败：{error}"))?;
    file.seek(SeekFrom::Start(range.start))
        .await
        .map_err(|error| format!("定位下载分片失败：{error}"))?;
    let mut cursor = range.start;
    let mut last_error = "分片数据提前结束".to_string();
    for attempt in 1..=DOWNLOAD_RANGE_ATTEMPTS {
        wait_download_running(&mut control).await?;
        if cursor > range.end {
            break;
        }
        let permit = download_http_semaphore()
            .acquire_owned()
            .await
            .map_err(|_| "下载连接调度器已关闭".to_string())?;
        let requested_range = format!("bytes={cursor}-{}", range.end);
        let mut response = match await_download_operation(
            &mut control,
            client
                .get(download_url)
                .header(RANGE, requested_range)
                .send(),
            Duration::from_secs(DOWNLOAD_READ_IDLE_TIMEOUT_SECS),
            || {
                format!(
                    "连接分片服务器超过 {} 秒无响应",
                    DOWNLOAD_READ_IDLE_TIMEOUT_SECS
                )
            },
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                last_error = format!("连接分片服务器失败：{error}");
                continue;
            }
            Err(error) => {
                if download_is_cancelled(&control) {
                    return Err(error);
                }
                last_error = error;
                continue;
            }
        };
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!(
                "下载服务器拒绝分片请求（HTTP {}）",
                response.status()
            ));
        }
        let content_range = response_content_range(&response)
            .ok_or_else(|| "下载分片响应缺少有效 Content-Range".to_string())?;
        if content_range.start != cursor
            || content_range.end != range.end
            || content_range.total != total_bytes
        {
            return Err(format!(
                "下载分片范围不一致（期望 {cursor}-{} / {total_bytes}，实际 {}-{} / {}）",
                range.end, content_range.start, content_range.end, content_range.total
            ));
        }
        loop {
            let next = await_download_operation(
                &mut control,
                response.chunk(),
                Duration::from_secs(DOWNLOAD_READ_IDLE_TIMEOUT_SECS),
                || {
                    format!(
                        "下载分片超过 {} 秒没有新数据",
                        DOWNLOAD_READ_IDLE_TIMEOUT_SECS
                    )
                },
            )
            .await;
            let chunk = match next {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) => {
                    last_error = "分片数据提前结束".to_string();
                    break;
                }
                Ok(Err(error)) => {
                    last_error = format!("读取下载分片失败：{error}");
                    break;
                }
                Err(error) => {
                    if download_is_cancelled(&control) {
                        return Err(error);
                    }
                    last_error = error;
                    break;
                }
            };
            let remaining = range.end.saturating_sub(cursor).saturating_add(1);
            if chunk.len() as u64 > remaining {
                return Err("下载服务器返回了超出请求范围的数据".into());
            }
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("写入下载分片失败：{error}"))?;
            cursor = cursor.saturating_add(chunk.len() as u64);
            progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            if cursor > range.end {
                break;
            }
        }
        if cursor > range.end {
            drop(permit);
            break;
        }
        if attempt == DOWNLOAD_RANGE_ATTEMPTS {
            return Err(format!(
                "下载分片 {}-{} 重试 {} 次仍失败：{last_error}",
                range.start, range.end, DOWNLOAD_RANGE_ATTEMPTS
            ));
        }
        drop(permit);
        // 指数退避：CDN 限流时立即连打 3 次只会快速耗尽重试机会。
        sleep(Duration::from_millis(400_u64 << (attempt - 1).min(4))).await;
    }
    if cursor <= range.end {
        return Err(format!(
            "下载分片 {}-{} 未完成：{last_error}",
            range.start, range.end
        ));
    }
    file.flush()
        .await
        .map_err(|error| format!("刷新下载分片失败：{error}"))?;
    Ok(())
}

pub(crate) async fn download_parallel_ranges(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    download_url: &str,
    partial: &Path,
    total_bytes: u64,
    connections: usize,
    download_id: &str,
    actual_name: &str,
    mut control: watch::Receiver<DownloadControlState>,
) -> Result<u64, String> {
    wait_download_running(&mut control).await?;
    let file = tokio::fs::File::create(partial)
        .await
        .map_err(|error| format!("无法创建临时下载文件 {}：{error}", partial.display()))?;
    file.set_len(total_bytes)
        .await
        .map_err(|error| format!("无法预分配下载文件空间：{error}"))?;
    drop(file);

    let progress = Arc::new(AtomicU64::new(0));
    let ranges = download_byte_ranges(total_bytes, connections);
    let task_control = control.clone();
    let task_progress = progress.clone();
    let tasks = stream::iter(ranges.into_iter().map(move |range| {
        let client = client.clone();
        let download_url = download_url.to_string();
        let partial = partial.to_path_buf();
        let progress = task_progress.clone();
        let control = task_control.clone();
        async move {
            download_range_to_file(
                &client,
                &download_url,
                &partial,
                range,
                total_bytes,
                &progress,
                control,
            )
            .await
        }
    }))
    .buffer_unordered(connections)
    .try_collect::<Vec<()>>();
    tokio::pin!(tasks);

    let mut last_emit = Instant::now();
    let mut last_emit_bytes = 0_u64;
    let result = loop {
        wait_download_running(&mut control).await?;
        tokio::select! {
            result = &mut tasks => break result.map(|_| ()),
            changed = control.changed() => {
                changed.map_err(|_| "下载任务控制器已经关闭".to_string())?;
                last_emit = Instant::now();
                last_emit_bytes = progress.load(Ordering::Relaxed);
            }
            _ = sleep(Duration::from_millis(400)) => {
                let downloaded_bytes = progress.load(Ordering::Relaxed);
                let elapsed = last_emit.elapsed().as_secs_f64();
                let bytes_per_second = if elapsed > 0.0 {
                    ((downloaded_bytes.saturating_sub(last_emit_bytes)) as f64 / elapsed) as u64
                } else {
                    0
                };
                emit(
                    app,
                    json!({
                        "type": "download",
                        "download_id": download_id,
                        "state": "downloading",
                        "file_name": actual_name,
                        "downloaded_bytes": downloaded_bytes,
                        "total_bytes": total_bytes,
                        "percent": (downloaded_bytes.saturating_mul(100) / total_bytes).min(99),
                        "bytes_per_second": bytes_per_second,
                        "segmented": true,
                        "connections": connections
                    }),
                );
                last_emit = Instant::now();
                last_emit_bytes = downloaded_bytes;
            }
        }
    };
    result?;
    let downloaded_bytes = progress.load(Ordering::Relaxed);
    if downloaded_bytes != total_bytes {
        return Err(format!(
            "并发分片下载不完整：应为 {total_bytes} 字节，实际 {downloaded_bytes} 字节"
        ));
    }
    Ok(downloaded_bytes)
}

pub(crate) async fn download_to_local(
    app: &tauri::AppHandle,
    download_url: &str,
    requested_name: &str,
    destination_dir: &str,
    download_id: &str,
    download_task_concurrency: usize,
    mut control: watch::Receiver<DownloadControlState>,
) -> Result<Value, String> {
    wait_download_running(&mut control).await?;
    if destination_dir.trim().is_empty() {
        return Err("请先选择下载保存目录".into());
    }
    if download_id.trim().is_empty() {
        return Err("下载任务 ID 为空".into());
    }
    let directory = PathBuf::from(destination_dir.trim());
    let metadata = tokio::fs::metadata(&directory)
        .await
        .map_err(|error| format!("无法访问下载目录 {}：{error}", directory.display()))?;
    if !metadata.is_dir() {
        return Err(format!("下载位置不是文件夹：{}", directory.display()));
    }
    let file_name = safe_download_name(requested_name);
    let target = available_download_path(&directory, &file_name);
    let actual_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("光鸭下载")
        .to_string();
    let partial = directory.join(format!(".{actual_name}.{}.part", Uuid::new_v4()));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("创建下载客户端失败：{error}"))?;
    let (probed_total_bytes, supports_ranges) =
        probe_download(&client, download_url, &mut control).await?;
    let configured_connections = configured_download_connections(download_task_concurrency);
    let segmented = supports_ranges
        && configured_connections > 1
        && probed_total_bytes.is_some_and(|total| total >= DOWNLOAD_PARALLEL_MIN_BYTES);
    let connections = if segmented { configured_connections } else { 1 };
    emit(
        app,
        json!({
            "type": "download",
            "download_id": download_id,
            "state": "downloading",
            "file_name": actual_name,
            "downloaded_bytes": 0,
            "total_bytes": probed_total_bytes,
            "percent": probed_total_bytes.map(|_| 0),
            "bytes_per_second": 0,
            "segmented": segmented,
            "connections": connections
        }),
    );
    let result: Result<(u64, Option<u64>, bool, usize), String> = async {
        if segmented {
            let total_bytes = probed_total_bytes.expect("segmented downloads require a known size");
            match download_parallel_ranges(
                app,
                &client,
                download_url,
                &partial,
                total_bytes,
                connections,
                download_id,
                &actual_name,
                control.clone(),
            )
            .await
            {
                Ok(downloaded_bytes) => {
                    return Ok((downloaded_bytes, Some(total_bytes), true, connections));
                }
                Err(error) => {
                    let _ = tokio::fs::remove_file(&partial).await;
                    if download_is_cancelled(&control) {
                        return Err(error);
                    }
                    emit(
                        app,
                        json!({
                            "type": "download",
                            "download_id": download_id,
                            "state": "downloading",
                            "file_name": actual_name,
                            "downloaded_bytes": 0,
                            "total_bytes": total_bytes,
                            "percent": 0,
                            "bytes_per_second": 0,
                            "segmented": false,
                            "connections": 1,
                            "fallback_reason": error
                        }),
                    );
                }
            }
        }

        let permit = download_http_semaphore()
            .acquire_owned()
            .await
            .map_err(|_| "下载连接调度器已关闭".to_string())?;
        wait_download_running(&mut control).await?;
        let mut response = await_download_operation(
            &mut control,
            client.get(download_url).send(),
            Duration::from_secs(DOWNLOAD_READ_IDLE_TIMEOUT_SECS),
            || {
                format!(
                    "连接光鸭下载服务器超过 {} 秒无响应",
                    DOWNLOAD_READ_IDLE_TIMEOUT_SECS
                )
            },
        )
        .await?
        .map_err(|error| format!("连接光鸭下载服务器失败：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("光鸭文件下载失败（HTTP {}）", response.status()));
        }
        let total_bytes = response_total_bytes(&response).or(probed_total_bytes);
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|error| format!("无法创建临时下载文件 {}：{error}", partial.display()))?;
        let mut downloaded_bytes = 0_u64;
        let mut last_emit = Instant::now();
        let mut last_emit_bytes = 0_u64;
        loop {
            let chunk = await_download_operation(
                &mut control,
                response.chunk(),
                Duration::from_secs(DOWNLOAD_READ_IDLE_TIMEOUT_SECS),
                || format!("下载超过 {} 秒没有新数据", DOWNLOAD_READ_IDLE_TIMEOUT_SECS),
            )
            .await?
            .map_err(|error| format!("读取光鸭下载数据失败：{error}"))?;
            let Some(chunk) = chunk else { break };
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("写入下载文件失败：{error}"))?;
            downloaded_bytes += chunk.len() as u64;
            if last_emit.elapsed() >= Duration::from_millis(400) {
                let elapsed = last_emit.elapsed().as_secs_f64();
                let bytes_per_second = if elapsed > 0.0 {
                    ((downloaded_bytes - last_emit_bytes) as f64 / elapsed) as u64
                } else {
                    0
                };
                let percent = total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| (downloaded_bytes.saturating_mul(100) / total).min(99));
                emit(
                    app,
                    json!({
                        "type": "download",
                        "download_id": download_id,
                        "state": "downloading",
                        "file_name": actual_name,
                        "downloaded_bytes": downloaded_bytes,
                        "total_bytes": total_bytes,
                        "percent": percent,
                        "bytes_per_second": bytes_per_second,
                        "segmented": false,
                        "connections": 1
                    }),
                );
                last_emit = Instant::now();
                last_emit_bytes = downloaded_bytes;
            }
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新下载文件失败：{error}"))?;
        drop(file);
        drop(permit);
        if let Some(total_bytes) = total_bytes {
            if downloaded_bytes != total_bytes {
                return Err(format!(
                    "下载数据不完整：应为 {total_bytes} 字节，实际 {downloaded_bytes} 字节"
                ));
            }
        }
        Ok((downloaded_bytes, total_bytes, false, 1))
    }
    .await;
    let (downloaded_bytes, total_bytes, completed_segmented, completed_connections) = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = tokio::fs::remove_file(&partial).await;
            if download_is_cancelled(&control) {
                emit(
                    app,
                    json!({ "type": "download", "download_id": download_id, "state": "cancelled", "bytes_per_second": 0 }),
                );
            } else {
                emit(
                    app,
                    json!({ "type": "download", "download_id": download_id, "state": "error", "error": error }),
                );
            }
            return Err(error);
        }
    };
    if let Err(error) = wait_download_running(&mut control).await {
        let _ = tokio::fs::remove_file(&partial).await;
        emit(
            app,
            json!({ "type": "download", "download_id": download_id, "state": "cancelled", "bytes_per_second": 0 }),
        );
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&partial, &target).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!("完成下载文件失败：{error}"));
    }
    let file_path = target.to_string_lossy().to_string();
    emit(
        app,
        json!({
            "type": "download",
            "download_id": download_id,
            "state": "done",
            "file_name": actual_name,
            "file_path": file_path,
            "downloaded_bytes": downloaded_bytes,
            "total_bytes": total_bytes,
            "percent": 100,
            "bytes_per_second": 0,
            "segmented": completed_segmented,
            "connections": completed_connections
        }),
    );
    Ok(json!({
        "file_path": file_path,
        "file_name": actual_name,
        "bytes": downloaded_bytes
    }))
}
