//! 统一的光鸭 API 模块。
//!
//! 分层结构：
//! - 传输层：`http_client()` 共享连接池 + `RetryPolicy` 指数退避重试，按端点幂等性
//!   决定哪些失败可以安全重试（只读接口全量重试；写接口只重试"请求未送达"类错误）；
//! - 业务层：`api_post` 解析业务码（`code`/`msg`/`data`），检测登录态过期后通过
//!   `AuthBroker` 单飞刷新令牌并重放一次原请求；
//! - 账号层：`account_post` / `account_get` 访问 account.guangyapan.com，复用同一连接池。

use crate::prelude::*;

// ---------------------------------------------------------------------------
// 传输层：共享客户端与重试策略
// ---------------------------------------------------------------------------

/// 当前生效的全局代理地址（"设置 → 网络"），空串表示直连。
/// 由应用启动与代理设置变更时写入，`http_client()` 检测到变化会重建客户端。
static GLOBAL_API_PROXY: OnceLock<Mutex<String>> = OnceLock::new();

pub(crate) fn set_global_api_proxy(proxy: &str) {
    let holder = GLOBAL_API_PROXY.get_or_init(|| Mutex::new(String::new()));
    if let Ok(mut guard) = holder.lock() {
        *guard = proxy.trim().to_string();
    }
}

fn current_global_api_proxy() -> String {
    GLOBAL_API_PROXY
        .get()
        .and_then(|holder| holder.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

fn build_http_client(proxy: &str) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(20);
    if !proxy.is_empty() {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy).map_err(|error| format!("初始化全局代理失败：{error}"))?,
        );
    }
    builder
        .build()
        .map_err(|error| format!("创建网络客户端失败：{error}"))
}

/// 全局共享的 HTTP 客户端：只配置连接超时、连接池与全局代理，总超时由每个
/// 请求单独指定。`reqwest::Client` 内部是引用计数的，按值克隆开销极小。
///
/// 旧实现的业务客户端完全忽略代理设置——用户配置代理后业务接口仍走直连，
/// 与 GCID 导出/下载/HDHive（都接了代理）行为不一致。
pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<Mutex<Option<(String, reqwest::Client)>>> = OnceLock::new();
    let proxy = current_global_api_proxy();
    let holder = CLIENT.get_or_init(|| Mutex::new(None));
    let mut guard = holder
        .lock()
        .map_err(|error| format!("获取网络客户端失败：{error}"))?;
    if let Some((cached_proxy, client)) = guard.as_ref() {
        if *cached_proxy == proxy {
            return Ok(client.clone());
        }
    }
    let client = build_http_client(&proxy)?;
    *guard = Some((proxy, client.clone()));
    Ok(client)
}

/// 兼容旧命名：业务 API 客户端即共享客户端。
pub(crate) fn business_api_client() -> Result<reqwest::Client, String> {
    http_client()
}

/// 请求幂等性分类，决定重试的激进程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Idempotency {
    /// 只读请求：网络错误、超时、429、5xx 都可以安全重试。
    Read,
    /// 写请求：只重试"服务端肯定没有开始处理"的失败（连接失败、429、503）。
    Mutation,
}

/// 根据端点路径推断幂等性。未识别的端点一律按写请求保守处理。
pub(crate) fn endpoint_idempotency(endpoint: &str) -> Idempotency {
    const READ_ONLY_MARKERS: &[&str] = &[
        "/file/get_",
        "/file/search_files",
        "/userres/v1/get_",
        "/userres/v1/check_can_flash_upload",
        "/cloudcollection/v1/list_task",
        "/cloudcollection/v1/resolve_res",
        "/developer/v1/pre_upload_status",
        "/developer/v1/upload_status",
        "/scheduler/v1/query_packaging_task",
        "/misc/v1/",
        "/assets/v1/",
        "/user/v1/",
    ];
    if READ_ONLY_MARKERS
        .iter()
        .any(|marker| endpoint.contains(marker))
    {
        Idempotency::Read
    } else {
        Idempotency::Mutation
    }
}

/// 指数退避重试策略。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    /// 总尝试次数（含首次请求）。
    pub(crate) attempts: u32,
    pub(crate) base_delay_ms: u64,
    pub(crate) max_delay_ms: u64,
}

impl RetryPolicy {
    pub(crate) const READ: Self = Self {
        attempts: 4,
        base_delay_ms: 300,
        max_delay_ms: 5_000,
    };
    pub(crate) const MUTATION: Self = Self {
        attempts: 3,
        base_delay_ms: 500,
        max_delay_ms: 5_000,
    };

    pub(crate) fn for_idempotency(idempotency: Idempotency) -> Self {
        match idempotency {
            Idempotency::Read => Self::READ,
            Idempotency::Mutation => Self::MUTATION,
        }
    }

    /// 第 `attempt` 次失败后的退避时间（0 起），带 ±25% 抖动防止惊群。
    pub(crate) fn backoff(&self, attempt: u32) -> Duration {
        let exp = self
            .base_delay_ms
            .saturating_mul(1_u64 << attempt.min(16))
            .min(self.max_delay_ms);
        let jitter_range = exp / 4;
        let jitter = if jitter_range == 0 {
            0
        } else {
            // 用时间戳低位做轻量抖动，避免引入随机数依赖。
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.subsec_nanos() as u64)
                .unwrap_or(0);
            seed % (jitter_range * 2)
        };
        Duration::from_millis(exp.saturating_sub(jitter_range).saturating_add(jitter))
    }
}

/// 传输层失败分类。
enum TransportError {
    /// 请求可能根本没有送达（连接失败）——对任何请求都可以重试。
    NotDelivered(String),
    /// 请求可能已被服务端处理（超时、读取失败）——只有只读请求可以重试。
    MaybeDelivered(String),
    /// 服务端明确拒绝且暂时性（429/503），可重试；附带建议等待时间。
    Rejected { message: String, retry_after: Option<Duration> },
    /// 不可重试的失败。
    Permanent(String),
}

impl TransportError {
    fn message(self) -> String {
        match self {
            Self::NotDelivered(message)
            | Self::MaybeDelivered(message)
            | Self::Rejected { message, .. }
            | Self::Permanent(message) => message,
        }
    }

    fn retryable(&self, idempotency: Idempotency) -> bool {
        match self {
            Self::NotDelivered(_) | Self::Rejected { .. } => true,
            Self::MaybeDelivered(_) => idempotency == Idempotency::Read,
            Self::Permanent(_) => false,
        }
    }

    fn suggested_delay(&self) -> Option<Duration> {
        match self {
            Self::Rejected { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

fn classify_send_error(endpoint: &str, error: &reqwest::Error) -> TransportError {
    let message = format!("无法连接光鸭接口 {endpoint}：网络异常，请稍后重试（{error}）");
    if error.is_connect() {
        TransportError::NotDelivered(message)
    } else {
        TransportError::MaybeDelivered(message)
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds.min(30)))
}

/// 带重试的业务请求：POST JSON 到 API_BASE，返回 (HTTP 状态, 原始响应文本)。
async fn send_business_request(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: &Value,
    idempotency: Idempotency,
) -> Result<(u16, String), TransportError> {
    let client = http_client().map_err(TransportError::Permanent)?;
    let policy = RetryPolicy::for_idempotency(idempotency);
    let mut last_error: Option<TransportError> = None;
    for attempt in 0..policy.attempts {
        if attempt > 0 {
            let delay = last_error
                .as_ref()
                .and_then(TransportError::suggested_delay)
                .unwrap_or_else(|| policy.backoff(attempt - 1));
            sleep(delay).await;
        }
        let headers =
            business_api_headers(token, device_id).map_err(TransportError::Permanent)?;
        let outcome = async {
            let response = client
                .post(format!("{API_BASE}{endpoint}"))
                .headers(headers)
                .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
                .json(body)
                .send()
                .await
                .map_err(|error| classify_send_error(endpoint, &error))?;
            let http_status = response.status().as_u16();
            match http_status {
                429 | 503 => {
                    let retry_after = parse_retry_after(response.headers());
                    return Err(TransportError::Rejected {
                        message: format!(
                            "光鸭接口 {endpoint} 暂时不可用（HTTP {http_status}），请稍后重试"
                        ),
                        retry_after,
                    });
                }
                500..=599 => {
                    let raw = response.text().await.unwrap_or_default();
                    return Err(TransportError::MaybeDelivered(format!(
                        "光鸭接口 {endpoint} 服务端错误（HTTP {http_status}）：{}",
                        raw.trim().chars().take(200).collect::<String>()
                    )));
                }
                _ => {}
            }
            let raw = response.text().await.map_err(|error| {
                TransportError::MaybeDelivered(format!(
                    "读取光鸭接口 {endpoint} 响应失败：{error}"
                ))
            })?;
            Ok((http_status, raw))
        }
        .await;
        match outcome {
            Ok(result) => return Ok(result),
            Err(error) => {
                let final_attempt = attempt + 1 >= policy.attempts;
                if final_attempt || !error.retryable(idempotency) {
                    return Err(error);
                }
                last_error = Some(error);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| TransportError::Permanent("光鸭接口请求失败".to_string())))
}

// ---------------------------------------------------------------------------
// 登录态协调：过期后单飞刷新并重放
// ---------------------------------------------------------------------------

struct AuthBroker {
    app: tauri::AppHandle,
    state: SharedState,
    refresh_gate: tokio::sync::Mutex<()>,
}

static AUTH_BROKER: OnceLock<AuthBroker> = OnceLock::new();

/// 在应用启动时安装登录态协调器；之后 `api_post` 遇到令牌过期会自动刷新并重放。
pub(crate) fn install_auth_broker(app: tauri::AppHandle, state: SharedState) {
    let _ = AUTH_BROKER.set(AuthBroker {
        app,
        state,
        refresh_gate: tokio::sync::Mutex::new(()),
    });
}

/// 尝试获取一个比 `stale_token` 更新的可用令牌。
///
/// - 若其它请求已经完成刷新（当前令牌 != 过期令牌），直接复用；
/// - 否则以单飞方式调用刷新流程；
/// - 返回 `None` 表示无法恢复登录态（调用方应把过期错误抛给用户）。
pub(crate) async fn fresh_business_token(stale_token: &str) -> Option<String> {
    let broker = AUTH_BROKER.get()?;
    let _permit = broker.refresh_gate.lock().await;
    let current = broker
        .state
        .lock()
        .ok()
        .and_then(|guard| guard.token.clone());
    if let Some(current) = current {
        if current != stale_token {
            return Some(current);
        }
    }
    match refresh_saved_session(broker.app.clone(), broker.state.clone()).await {
        Ok(true) => broker
            .state
            .lock()
            .ok()
            .and_then(|guard| guard.token.clone()),
        Ok(false) => None,
        Err(_) => None,
    }
}

const AUTH_EXPIRED_MESSAGE: &str = "登录态已失效，请重新打开官方登录页";

// ---------------------------------------------------------------------------
// 业务层：code/msg/data 协议
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct ApiResponse {
    #[serde(default)]
    pub(crate) code: i64,
    #[serde(default)]
    pub(crate) msg: String,
    pub(crate) data: Option<Value>,
}

#[derive(Debug)]
pub(crate) enum BusinessRequestError {
    Request(String),
    InvalidResponse { http_status: u16, message: String },
}

impl BusinessRequestError {
    fn into_message(self) -> String {
        match self {
            Self::Request(message) | Self::InvalidResponse { message, .. } => message,
        }
    }
}

pub(crate) fn business_api_headers(token: &str, device_id: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| error.to_string())?,
    );
    headers.insert("dt", HeaderValue::from_static(API_DEVICE_TYPE));
    headers.insert("av", HeaderValue::from_static(API_APP_VERSION));
    headers.insert("vc", HeaderValue::from_static(API_VERSION_CODE));
    headers.insert("x-client-id", HeaderValue::from_static(OAUTH_CLIENT_ID));
    headers.insert(
        "x-device-id",
        HeaderValue::from_str(device_id).map_err(|error| error.to_string())?,
    );
    headers.insert("user-agent", HeaderValue::from_static(API_USER_AGENT));
    // Retain the legacy alias alongside the canonical x-device-id header.
    headers.insert(
        "did",
        HeaderValue::from_str(device_id).map_err(|error| error.to_string())?,
    );
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = Uuid::new_v4().simple().to_string()[..16].to_string();
    headers.insert(
        "traceparent",
        HeaderValue::from_str(&format!("00-{trace_id}-{span_id}-01"))
            .map_err(|error| error.to_string())?,
    );
    Ok(headers)
}

pub(crate) fn business_auth_expired(http_status: u16, code: i64) -> bool {
    http_status == 401 || matches!(code, 110 | 117 | 118)
}

pub(crate) async fn api_post_response(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: Value,
) -> Result<(u16, ApiResponse), BusinessRequestError> {
    api_post_response_with(token, device_id, endpoint, &body).await
}

async fn api_post_response_with(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: &Value,
) -> Result<(u16, ApiResponse), BusinessRequestError> {
    let idempotency = endpoint_idempotency(endpoint);
    let (http_status, raw) =
        send_business_request(token, device_id, endpoint, body, idempotency)
            .await
            .map_err(|error| BusinessRequestError::Request(error.message()))?;
    let payload = parse_api_response(&raw, http_status, endpoint).map_err(|message| {
        BusinessRequestError::InvalidResponse {
            http_status,
            message,
        }
    })?;
    Ok((http_status, payload))
}

/// 业务 POST：自动重试传输层瞬时失败；登录态过期时刷新令牌并重放一次。
///
/// `allowed` 中的业务码不视为错误（由调用方自行处理）。
pub(crate) async fn api_post(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: Value,
    allowed: &[i64],
) -> Result<ApiResponse, String> {
    let mut active_token = token.to_string();
    for replay in 0..2 {
        let outcome =
            api_post_response_with(&active_token, device_id, endpoint, &body).await;
        let auth_expired = match &outcome {
            Ok((http_status, payload)) => business_auth_expired(*http_status, payload.code),
            Err(BusinessRequestError::InvalidResponse { http_status: 401, .. }) => true,
            Err(_) => false,
        };
        if auth_expired {
            if replay == 0 {
                if let Some(fresh) = fresh_business_token(&active_token).await {
                    active_token = fresh;
                    continue;
                }
            }
            return Err(AUTH_EXPIRED_MESSAGE.into());
        }
        let (http_status, payload) = match outcome {
            Ok(response) => response,
            Err(error) => return Err(error.into_message()),
        };
        return finish_api_post(endpoint, &body, http_status, payload, allowed);
    }
    Err(AUTH_EXPIRED_MESSAGE.into())
}

fn finish_api_post(
    endpoint: &str,
    body: &Value,
    http_status: u16,
    payload: ApiResponse,
    allowed: &[i64],
) -> Result<ApiResponse, String> {
    if !(200..300).contains(&http_status) || (payload.code != 0 && !allowed.contains(&payload.code))
    {
        let message = if payload.msg.is_empty() {
            format!("光鸭接口失败：HTTP {http_status}/{}", payload.code)
        } else {
            payload.msg.clone()
        };
        // 分享接口失败时附带请求参数，便于排查官方风控拦截。
        if endpoint == "/userres/v1/share_file" {
            let request_preview = serde_json::to_string(body)
                .unwrap_or_else(|_| "<无法序列化分享参数>".to_string());
            return Err(format!(
                "{message}（HTTP {http_status}，业务码 {}；请求参数：{request_preview}）",
                payload.code
            ));
        }
        return Err(message);
    }
    Ok(payload)
}

pub(crate) fn parse_api_response(raw: &str, status: u16, endpoint: &str) -> Result<ApiResponse, String> {
    let trimmed = raw.trim().trim_start_matches('\u{feff}');
    let value: Value = serde_json::from_str(trimmed).map_err(|error| {
        let preview = trimmed.chars().take(240).collect::<String>();
        format!("光鸭接口 {endpoint} 返回了非 JSON 响应（HTTP {status}）：{preview}（{error}）")
    })?;
    let msg = value
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let has_code = value.get("code").is_some_and(|value| !value.is_null());
    let code = match value.get("code").filter(|value| !value.is_null()) {
        Some(value) => value
            .as_i64()
            .or_else(|| value.as_str()?.parse().ok())
            .ok_or_else(|| format!("光鸭接口 {endpoint} 返回了无效业务码"))?,
        None => 0,
    };
    let normalized_msg = msg.trim().to_ascii_lowercase();
    if code == 0
        && ((!normalized_msg.is_empty() && !matches!(normalized_msg.as_str(), "success" | "ok"))
            || (!has_code && normalized_msg.is_empty() && value.get("data").is_none()))
    {
        return Err(format!(
            "光鸭接口 {endpoint} 返回了未标明成功状态的响应（HTTP {status}）"
        ));
    }
    Ok(ApiResponse {
        code,
        msg,
        data: value.get("data").cloned(),
    })
}

// ---------------------------------------------------------------------------
// 账号层：account.guangyapan.com
// ---------------------------------------------------------------------------

pub(crate) fn account_api_headers(device_id: &str, token: Option<&str>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers.insert("x-client-id", HeaderValue::from_static(OAUTH_CLIENT_ID));
    headers.insert(
        "x-device-id",
        HeaderValue::from_str(device_id).map_err(|error| error.to_string())?,
    );
    headers.insert(
        "x-client-version",
        HeaderValue::from_static(API_APP_VERSION),
    );
    headers.insert("x-sdk-version", HeaderValue::from_static("9.0.2"));
    headers.insert("x-protocol-version", HeaderValue::from_static("301"));
    headers.insert("accept-language", HeaderValue::from_static("zh-CN"));
    headers.insert("user-agent", HeaderValue::from_static(API_USER_AGENT));
    if let Some(token) = token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| error.to_string())?,
        );
    }
    Ok(headers)
}

pub(crate) async fn account_post(
    device_id: &str,
    endpoint: &str,
    body: Value,
) -> Result<(u16, Value), String> {
    account_post_with_captcha(device_id, endpoint, body, None).await
}

/// 账号接口 POST。登录/授权语义敏感，只重试"连接失败"这类肯定未送达的错误。
pub(crate) async fn account_post_with_captcha(
    device_id: &str,
    endpoint: &str,
    body: Value,
    captcha_token: Option<&str>,
) -> Result<(u16, Value), String> {
    let client = http_client()?;
    let policy = RetryPolicy::MUTATION;
    let mut last_error = String::new();
    for attempt in 0..policy.attempts {
        if attempt > 0 {
            sleep(policy.backoff(attempt - 1)).await;
        }
        let headers = account_api_headers(device_id, None)?;
        let mut request = client
            .post(format!("{ACCOUNT_BASE}{endpoint}"))
            .headers(headers)
            .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
            .json(&body);
        if let Some(captcha_token) = captcha_token.filter(|value| !value.trim().is_empty()) {
            request = request.header("x-captcha-token", captcha_token.trim());
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_connect() && attempt + 1 < policy.attempts => {
                last_error = format!("无法连接账号接口 {endpoint}：{error}");
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let status = response.status().as_u16();
        let raw = response.text().await.map_err(|e| e.to_string())?;
        let payload = if raw.trim().is_empty() && (200..300).contains(&status) {
            json!({})
        } else {
            serde_json::from_str(raw.trim().trim_start_matches('\u{feff}')).map_err(|error| {
                format!("账号接口 {endpoint} 返回了非 JSON 响应（HTTP {status}）：{error}")
            })?
        };
        return Ok((status, payload));
    }
    Err(last_error)
}

pub(crate) fn account_payload_value<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload
        .get(key)
        .or_else(|| payload.get("data").and_then(|data| data.get(key)))
}

pub(crate) fn account_payload_string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = account_payload_value(payload, key)?;
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_i64().map(|value| value.to_string()))
            .filter(|value| !value.trim().is_empty())
    })
}

pub(crate) fn account_payload_bool(payload: &Value, key: &str) -> Option<bool> {
    let value = account_payload_value(payload, key)?;
    value.as_bool().or_else(|| {
        value.as_i64().map(|value| value != 0).or_else(|| {
            match value.as_str()?.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None,
            }
        })
    })
}

pub(crate) fn account_error_message(payload: &Value, fallback: &str) -> String {
    account_payload_string(
        payload,
        &[
            "error_description",
            "description",
            "message",
            "msg",
            "error",
        ],
    )
    .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn payload_mentions_captcha(payload: &Value) -> bool {
    let serialized = payload.to_string().to_ascii_lowercase();
    serialized.contains("captcha")
        || serialized.contains("人机验证")
        || serialized.contains("安全验证")
}

pub(crate) fn flatten_account_payload(payload: &Value) -> serde_json::Map<String, Value> {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    if let Some(data) = payload.get("data").and_then(Value::as_object) {
        for (key, value) in data {
            object.insert(key.clone(), value.clone());
        }
    }
    object.remove("data");
    object
}

pub(crate) fn captcha_challenge_response(payload: &Value, force: bool) -> Option<Value> {
    let url = account_payload_string(payload, &["captcha_url", "captchaUrl", "url"]);
    let captcha_token = account_payload_string(payload, &["captcha_token", "captchaToken"]);
    let explicitly_required = account_payload_bool(payload, "captcha_required")
        .or_else(|| account_payload_bool(payload, "captchaRequired"))
        .unwrap_or(false);
    if !force && url.is_none() && !explicitly_required {
        return None;
    }
    let mut object = flatten_account_payload(payload);
    object.insert("captcha_required".to_string(), json!(true));
    object.insert("authenticated".to_string(), json!(false));
    if let Some(url) = url {
        object.insert("captcha_url".to_string(), json!(url));
    }
    if let Some(captcha_token) = captcha_token {
        object.insert("captcha_token".to_string(), json!(captcha_token));
    }
    Some(Value::Object(object))
}

/// 账号接口 GET：只读，可安全全量重试。
pub(crate) async fn account_get(token: &str, device_id: &str, endpoint: &str) -> Result<Value, String> {
    let client = http_client()?;
    let policy = RetryPolicy::READ;
    let mut last_error = String::new();
    for attempt in 0..policy.attempts {
        if attempt > 0 {
            sleep(policy.backoff(attempt - 1)).await;
        }
        let headers = account_api_headers(device_id, Some(token))?;
        let response = match client
            .get(format!("{ACCOUNT_BASE}{endpoint}"))
            .headers(headers)
            .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("账号接口 {endpoint} 请求失败：{error}");
                continue;
            }
        };
        let status = response.status().as_u16();
        if matches!(status, 429 | 500..=599) && attempt + 1 < policy.attempts {
            last_error = format!("账号接口 {endpoint} 暂时不可用（HTTP {status}）");
            continue;
        }
        let raw = response.text().await.map_err(|e| e.to_string())?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "账号接口 {endpoint} 请求失败（HTTP {status}）：{}",
                raw.trim()
            ));
        }
        if raw.trim().is_empty() {
            return Ok(json!({}));
        }
        return serde_json::from_str(raw.trim().trim_start_matches('\u{feff}'))
            .map_err(|error| format!("账号接口 {endpoint} 返回了非 JSON 响应：{error}"));
    }
    Err(last_error)
}
