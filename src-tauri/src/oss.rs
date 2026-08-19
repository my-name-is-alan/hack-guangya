//! 阿里云 OSS 签名、分片上传与上传凭据刷新。

use crate::prelude::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UploadCredentials {
    #[serde(rename = "accessKeyID")]
    pub(crate) access_key_id: String,
    #[serde(rename = "secretAccessKey", alias = "accessKeySecret")]
    pub(crate) secret_access_key: String,
    #[serde(rename = "sessionToken")]
    pub(crate) session_token: String,
    #[serde(default)]
    pub(crate) expiration: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadToken {
    pub(crate) task_id: String,
    pub(crate) object_path: Option<String>,
    pub(crate) bucket_name: Option<String>,
    pub(crate) end_point: Option<String>,
    pub(crate) full_end_point: Option<String>,
    pub(crate) creds: Option<UploadCredentials>,
    #[serde(default)]
    pub(crate) provider: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OssUploadCheckpoint {
    pub(crate) task_id: String,
    pub(crate) object_path: String,
    pub(crate) bucket_name: String,
    pub(crate) end_point: String,
    pub(crate) provider: Option<Value>,
    pub(crate) upload_id: String,
    pub(crate) part_size: u64,
    pub(crate) completed_parts: BTreeMap<u32, String>,
}


pub(crate) fn normalize_oss_endpoint(endpoint: &str, bucket: &str) -> String {
    let host = endpoint
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    host.strip_prefix(&format!("{}.", bucket.trim()))
        .unwrap_or(host)
        .to_string()
}
pub(crate) fn normalize_oss_endpoint_url(endpoint: &str, bucket: &str) -> String {
    let scheme = if endpoint.trim().starts_with("http://") {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{}", normalize_oss_endpoint(endpoint, bucket))
}
pub(crate) fn ceil_div_u64(value: u64, divisor: u64) -> u64 {
    value / divisor + u64::from(value % divisor != 0)
}
pub(crate) fn oss_part_size(size: u64) -> u64 {
    let tier_size = if size <= 100 * 1024 * 1024 {
        OSS_MIB
    } else if size <= 1024 * 1024 * 1024 {
        2 * OSS_MIB
    } else if size <= 10 * 1024 * 1024 * 1024 {
        4 * OSS_MIB
    } else {
        OSS_LARGE_FILE_PART_SIZE
    };
    let minimum_size = ceil_div_u64(size, OSS_MULTIPART_TARGET_PARTS);
    let aligned_minimum_size = ceil_div_u64(minimum_size, OSS_MIB).saturating_mul(OSS_MIB);
    tier_size.max(aligned_minimum_size)
}
pub(crate) fn configured_oss_part_size(size: u64, multipart_part_size: &str) -> u64 {
    if multipart_part_size == DEFAULT_MULTIPART_PART_SIZE {
        return oss_part_size(size);
    }
    let configured_size = match multipart_part_size {
        "4m" => 4 * OSS_MIB,
        "8m" => 8 * OSS_MIB,
        "16m" => 16 * OSS_MIB,
        _ => return oss_part_size(size),
    };
    let minimum_size = ceil_div_u64(size, OSS_MULTIPART_TARGET_PARTS);
    let aligned_minimum_size = ceil_div_u64(minimum_size, OSS_MIB).saturating_mul(OSS_MIB);
    configured_size.max(aligned_minimum_size)
}

pub(crate) fn oss_checkpoint_uploaded_bytes(checkpoint: &OssUploadCheckpoint, size: u64) -> u64 {
    checkpoint
        .completed_parts
        .keys()
        .filter_map(|part_number| {
            let offset = u64::from(part_number.saturating_sub(1)) * checkpoint.part_size;
            (offset < size).then_some(checkpoint.part_size.min(size - offset))
        })
        .sum::<u64>()
        .min(size)
}

pub(crate) fn oss_request_url(
    checkpoint: &OssUploadCheckpoint,
    query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let endpoint = normalize_oss_endpoint_url(&checkpoint.end_point, &checkpoint.bucket_name);
    let mut url =
        reqwest::Url::parse(&endpoint).map_err(|error| format!("OSS 端点无效：{error}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "OSS 端点缺少主机名".to_string())?
        .to_string();
    url.set_host(Some(&format!("{}.{}", checkpoint.bucket_name, host)))
        .map_err(|_| "OSS 存储桶地址无效".to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "OSS 对象地址无法设置路径".to_string())?;
        segments.clear();
        for segment in checkpoint.object_path.split('/') {
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
    }
    url.set_query(query);
    Ok(url)
}

pub(crate) fn oss_string_to_sign(
    method: &str,
    date: &str,
    security_token: &str,
    checkpoint: &OssUploadCheckpoint,
    query: Option<&str>,
) -> String {
    let mut resource = format!(
        "/{}/{}",
        checkpoint.bucket_name,
        checkpoint.object_path.trim_start_matches('/')
    );
    if let Some(query) = query.filter(|value| !value.is_empty()) {
        resource.push('?');
        resource.push_str(query);
    }
    format!("{method}\n\n\n{date}\nx-oss-security-token:{security_token}\n{resource}")
}

pub(crate) async fn oss_signed_request(
    client: &reqwest::Client,
    credentials: &UploadCredentials,
    checkpoint: &OssUploadCheckpoint,
    method: reqwest::Method,
    query: Option<&str>,
    body: Option<Vec<u8>>,
    app: &tauri::AppHandle,
    path: &Path,
    uploaded_bytes: u64,
    total_bytes: u64,
) -> Result<reqwest::Response, String> {
    let url = oss_request_url(checkpoint, query)?;
    for attempt in 0..=OSS_WRITE_RETRY_TIMES {
        let date = httpdate::fmt_http_date(std::time::SystemTime::now());
        let string_to_sign = oss_string_to_sign(
            method.as_str(),
            &date,
            &credentials.session_token,
            checkpoint,
            query,
        );
        let mut mac = Hmac::<Sha1>::new_from_slice(credentials.secret_access_key.as_bytes())
            .map_err(|error| format!("初始化 OSS 签名失败：{error}"))?;
        mac.update(string_to_sign.as_bytes());
        let signature = BASE64_STANDARD.encode(mac.finalize().into_bytes());
        let authorization = format!("OSS {}:{signature}", credentials.access_key_id);
        let mut request = client
            .request(method.clone(), url.clone())
            .header(DATE, &date)
            .header("x-oss-security-token", &credentials.session_token)
            .header(AUTHORIZATION, authorization);
        if let Some(content) = body.clone() {
            request = request.body(content);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status_code = response.status();
                let response_body = response.text().await.unwrap_or_default();
                let retryable = status_code.is_server_error()
                    || status_code.as_u16() == 408
                    || status_code.as_u16() == 429;
                if !retryable || attempt == OSS_WRITE_RETRY_TIMES {
                    return Err(format!(
                        "OSS 请求失败（{}）：{}",
                        status_code,
                        response_body.trim()
                    ));
                }
            }
            Err(error) if attempt == OSS_WRITE_RETRY_TIMES => {
                return Err(format!("OSS 请求失败：{error}"));
            }
            Err(_) => {}
        }
        let retry_after = Duration::from_secs((attempt as u64 + 1).min(10));
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": path.to_string_lossy(),
                "uploaded_bytes": uploaded_bytes,
                "total_bytes": total_bytes,
                "bytes_per_second": 0,
                "stage": format!(
                    "OSS 临时错误，{} 秒后进行第 {} 次重试",
                    retry_after.as_secs(),
                    attempt + 1
                )
            }),
        );
        sleep(retry_after).await;
    }
    Err("OSS 请求失败".into())
}

pub(crate) fn xml_tag_value(body: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = body.find(&start_tag)? + start_tag.len();
    let end = body[start..].find(&end_tag)? + start;
    Some(body[start..end].trim().to_string())
}

pub(crate) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn preferred_oss_endpoint(token_data: &UploadToken) -> Option<String> {
    token_data
        .full_end_point
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            token_data
                .end_point
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })
        .map(str::to_owned)
}

pub(crate) fn upload_credentials_expired(token_data: &UploadToken) -> bool {
    let Some(expiration) = token_data
        .creds
        .as_ref()
        .and_then(|credentials| credentials.expiration.as_deref())
    else {
        return true;
    };
    match time::OffsetDateTime::parse(expiration, &time::format_description::well_known::Rfc3339) {
        Ok(value) => value <= time::OffsetDateTime::now_utc(),
        Err(_) => true,
    }
}

pub(crate) fn is_oss_security_token_expired(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("securitytokenexpired")
}

pub(crate) async fn refresh_upload_token(
    token: &str,
    device_id: &str,
    file_size: u64,
    current: &UploadToken,
) -> Result<UploadToken, String> {
    let object_path = current
        .object_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "上传凭证缺少 OSS 对象路径，无法刷新".to_string())?;
    let response = api_post(
        token,
        device_id,
        "/userres/v1/get_res_center_resume_token",
        json!({
            "capacity": 2,
            "res": { "fileSize": file_size },
            "taskId": current.task_id,
            "object": {
                "objectPath": object_path,
                "provider": current.provider
            }
        }),
        &[156],
    )
    .await?;
    let mut refreshed: UploadToken = serde_json::from_value(
        response
            .data
            .ok_or_else(|| "光鸭没有返回续传凭证".to_string())?,
    )
    .map_err(|error| format!("续传凭证格式异常：{error}"))?;
    if refreshed
        .object_path
        .as_deref()
        .unwrap_or_default()
        .is_empty()
    {
        refreshed.object_path = current.object_path.clone();
    }
    if refreshed
        .bucket_name
        .as_deref()
        .unwrap_or_default()
        .is_empty()
    {
        refreshed.bucket_name = current.bucket_name.clone();
    }
    if refreshed
        .end_point
        .as_deref()
        .unwrap_or_default()
        .is_empty()
        && refreshed
            .full_end_point
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    {
        refreshed.end_point = current.end_point.clone();
        refreshed.full_end_point = current.full_end_point.clone();
    }
    if refreshed.provider.is_none() {
        refreshed.provider = current.provider.clone();
    }
    if refreshed.creds.is_none() {
        return Err("光鸭续传接口没有返回新的 OSS 临时凭证".into());
    }
    Ok(refreshed)
}

pub(crate) async fn upload_oss(
    token_data: &UploadToken,
    item: &UploadItem,
    app: &tauri::AppHandle,
    multipart_part_size: &str,
    db_path: &Path,
    persisted: Option<PersistedUploadCheckpoint>,
) -> Result<(), String> {
    let credentials = token_data
        .creds
        .as_ref()
        .ok_or_else(|| "光鸭没有返回 OSS 临时凭证".to_string())?;
    let size = fs::metadata(readable_fs_path(&item.file_path))
        .map_err(|error| error.to_string())?
        .len();
    let resumed = persisted.is_some();
    let mut checkpoint = if let Some(saved) = persisted {
        saved.checkpoint
    } else {
        let object_path = token_data
            .object_path
            .as_deref()
            .ok_or_else(|| "光鸭没有返回 OSS 对象路径".to_string())?
            .trim_start_matches('/')
            .to_string();
        if object_path.is_empty() {
            return Err("光鸭返回的 OSS 对象路径无效".into());
        }
        OssUploadCheckpoint {
            task_id: token_data.task_id.clone(),
            object_path,
            bucket_name: token_data
                .bucket_name
                .clone()
                .ok_or_else(|| "光鸭没有返回 OSS 存储桶".to_string())?,
            end_point: preferred_oss_endpoint(token_data)
                .ok_or_else(|| "光鸭没有返回 OSS 端点".to_string())?,
            provider: token_data.provider.clone(),
            upload_id: String::new(),
            part_size: configured_oss_part_size(size, multipart_part_size),
            completed_parts: BTreeMap::new(),
        }
    };
    if checkpoint.part_size == 0 {
        return Err("OSS 分片大小无效".into());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(OSS_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("初始化 OSS 客户端失败：{error}"))?;

    if size == 0 {
        oss_signed_request(
            &client,
            credentials,
            &checkpoint,
            reqwest::Method::PUT,
            None,
            Some(Vec::new()),
            app,
            &item.file_path,
            0,
            0,
        )
        .await?;
        clear_upload_checkpoint(db_path, item)?;
        return Ok(());
    }

    if checkpoint.upload_id.is_empty() {
        let response = oss_signed_request(
            &client,
            credentials,
            &checkpoint,
            reqwest::Method::POST,
            Some("uploads"),
            None,
            app,
            &item.file_path,
            0,
            size,
        )
        .await?;
        let response_body = response
            .text()
            .await
            .map_err(|error| format!("读取 OSS 分片任务失败：{error}"))?;
        checkpoint.upload_id = xml_tag_value(&response_body, "UploadId")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "OSS 没有返回分片任务 ID".to_string())?;
        save_upload_checkpoint(db_path, item, &checkpoint, 0)?;
    }

    let total_parts = ceil_div_u64(size, checkpoint.part_size);
    if total_parts > 10_000 || total_parts > u64::from(u32::MAX) {
        return Err("文件分片数量超过 OSS 限制".into());
    }
    let upload_started_at = std::time::Instant::now();
    let uploaded_at_start = oss_checkpoint_uploaded_bytes(&checkpoint, size);
    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": uploaded_at_start.saturating_mul(100) / size,
            "uploaded_bytes": uploaded_at_start,
            "total_bytes": size,
            "bytes_per_second": 0,
            "stage": if resumed { "正在从断点继续上传" } else { "正在上传" }
        }),
    );
    let request_checkpoint = checkpoint.clone();
    let pending_parts = (1..=total_parts as u32)
        .filter(|part| !checkpoint.completed_parts.contains_key(part))
        .collect::<Vec<_>>();
    let mut part_uploads = stream::iter(pending_parts)
        .map(|part| {
            let client = &client;
            let request_checkpoint = &request_checkpoint;
            let file_path = readable_fs_path(&item.file_path);
            async move {
                let offset = u64::from(part - 1) * request_checkpoint.part_size;
                let length = request_checkpoint.part_size.min(size - offset);
                let mut file = tokio::fs::File::open(&file_path)
                    .await
                    .map_err(|error| format!("打开上传文件失败：{error}"))?;
                file.seek(SeekFrom::Start(offset))
                    .await
                    .map_err(|error| format!("定位上传分片失败：{error}"))?;
                let mut buffer = vec![
                    0_u8;
                    usize::try_from(length).map_err(|_| {
                        "当前平台无法分配 OSS 分片缓冲区".to_string()
                    })?
                ];
                file.read_exact(&mut buffer)
                    .await
                    .map_err(|error| format!("读取上传分片失败：{error}"))?;
                let query = format!(
                    "partNumber={part}&uploadId={}",
                    request_checkpoint.upload_id
                );
                let response = oss_signed_request(
                    client,
                    credentials,
                    request_checkpoint,
                    reqwest::Method::PUT,
                    Some(&query),
                    Some(buffer),
                    app,
                    &file_path,
                    uploaded_at_start,
                    size,
                )
                .await?;
                let etag = response
                    .headers()
                    .get(ETAG)
                    .and_then(|value| value.to_str().ok())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "OSS 分片响应缺少 ETag".to_string())?
                    .to_string();
                Ok::<_, String>((part, etag))
            }
        })
        .buffer_unordered(3);
    while let Some(result) = part_uploads.next().await {
        let (part, etag) = result?;
        checkpoint.completed_parts.insert(part, etag);
        let uploaded = oss_checkpoint_uploaded_bytes(&checkpoint, size);
        save_upload_checkpoint(db_path, item, &checkpoint, uploaded)?;
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": uploaded.saturating_mul(100) / size,
                "uploaded_bytes": uploaded,
                "total_bytes": size,
                "bytes_per_second": uploaded.saturating_sub(uploaded_at_start) as f64
                    / upload_started_at.elapsed().as_secs_f64().max(0.001),
                "stage": if resumed { "正在断点续传" } else { "正在上传" }
            }),
        );
    }

    let parts_xml = checkpoint
        .completed_parts
        .iter()
        .map(|(part, etag)| {
            format!(
                "<Part><PartNumber>{part}</PartNumber><ETag>{}</ETag></Part>",
                xml_escape(etag)
            )
        })
        .collect::<String>();
    let complete_body =
        format!("<CompleteMultipartUpload>{parts_xml}</CompleteMultipartUpload>").into_bytes();
    let complete_query = format!("uploadId={}", checkpoint.upload_id);
    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 100,
            "uploaded_bytes": size,
            "total_bytes": size,
            "bytes_per_second": 0,
            "stage": "正在提交 OSS"
        }),
    );
    oss_signed_request(
        &client,
        credentials,
        &checkpoint,
        reqwest::Method::POST,
        Some(&complete_query),
        Some(complete_body),
        app,
        &item.file_path,
        size,
        size,
    )
    .await?;
    clear_upload_checkpoint(db_path, item)?;
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": size, "total_bytes": size, "bytes_per_second": 0, "stage": "OSS 上传完成" }),
    );
    Ok(())
}
