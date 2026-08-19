//! Telegram Bot 通知与交互渠道（桌面端）。
//!
//! 两种接入模式（都以 bot 身份运行，Telegram 协议限制只有 bot 能发 inline keyboard）：
//! - `bot_api`：HTTPS 调 Bot API（官方 api.telegram.org 或自建反代），getUpdates 长轮询，
//!   走全局代理（HTTP/SOCKS5 均可）。
//! - `mtproto`：api_id + api_hash + bot token，经 grammers 以 MTProto 协议直连数据中心，
//!   仅支持 SOCKS5 代理；会话持久化为应用数据目录下的 telegram.session.json。
//!
//! 出站通知：整理完成（入库）、识别失败（带重新整理 keyboard）、光鸭登录失效（带扫码按钮）、
//! Emby webhook（入库/播放/登录，接收端点挂在 Emby 网关端口）。入站命令：/status /jobs /logs
//! /update /login /help 与 `re <任务ID> tmdbid=…` 重新整理指令。

use crate::organizer::OrganizerJobInput;
use crate::prelude::*;
use grammers_client::{
    client::UpdatesConfiguration,
    message::{Button, InputMessage, ReplyMarkup},
    sender::{ConnectionParams, SenderPool},
    session::{
        types::{DcOption, PeerId, PeerInfo, PeerRef, UpdateState, UpdatesState},
        BoxFuture as GrammersBoxFuture, Session as GrammersSession, SessionData,
    },
    tl,
    update::Update as GrammersUpdate,
    Client as GrammersClient,
};

const DEFAULT_BOT_API_BASE: &str = "https://api.telegram.org";
const LOG_CAPACITY: usize = 500;
const NOTIFY_CATEGORIES: &[&str] = &["organize", "review", "auth", "emby_new", "emby_play", "emby_login"];
const BOT_COMMANDS: &[(&str, &str)] = &[
    ("status", "系统状态总览"),
    ("jobs", "最近整理任务"),
    ("logs", "最新运行日志（默认 50 条）"),
    ("update", "检查更新"),
    ("login", "获取光鸭扫码登录二维码"),
    ("help", "帮助与命令说明"),
];

// ---------------------------------------------------------------------------
// 全局运行状态
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct TelegramLogEntry {
    pub(crate) time_ms: u64,
    pub(crate) level: String,
    pub(crate) message: String,
}

#[derive(Debug, Default)]
struct TelegramStatusInfo {
    connected: bool,
    bot_username: String,
    last_error: Option<String>,
}

static LOG_BUFFER: OnceLock<Mutex<VecDeque<TelegramLogEntry>>> = OnceLock::new();
static STATUS_INFO: OnceLock<Mutex<TelegramStatusInfo>> = OnceLock::new();
static EVENT_TX: OnceLock<UnboundedSender<TelegramEvent>> = OnceLock::new();
static AUTH_EXPIRED_NOTIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_LOGGED_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LOGIN_FLOW_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

pub(crate) fn record_log(level: &str, message: &str) {
    let buffer = LOG_BUFFER.get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)));
    if let Ok(mut guard) = buffer.lock() {
        if guard.len() >= LOG_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(TelegramLogEntry {
            time_ms: now_ms(),
            level: level.to_string(),
            message: message.chars().take(2000).collect(),
        });
    }
}

pub(crate) fn recent_logs(limit: usize) -> Vec<TelegramLogEntry> {
    let count = limit.clamp(1, LOG_CAPACITY);
    LOG_BUFFER
        .get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)))
        .lock()
        .map(|guard| guard.iter().rev().take(count).rev().cloned().collect())
        .unwrap_or_default()
}

fn set_status_info(connected: bool, username: &str, error: Option<String>) {
    let info = STATUS_INFO.get_or_init(|| Mutex::new(TelegramStatusInfo::default()));
    if let Ok(mut guard) = info.lock() {
        guard.connected = connected;
        guard.bot_username = username.to_string();
        guard.last_error = error;
    }
}

fn status_info() -> (bool, String, Option<String>) {
    STATUS_INFO
        .get_or_init(|| Mutex::new(TelegramStatusInfo::default()))
        .lock()
        .map(|guard| (guard.connected, guard.bot_username.clone(), guard.last_error.clone()))
        .unwrap_or((false, String::new(), None))
}

fn send_event(event: TelegramEvent) {
    if let Some(tx) = EVENT_TX.get() {
        let _ = tx.send(event);
    }
}

/// 由 state.rs / organizer.rs 的 emit 管道调用，观察全部 sync-event。
pub(crate) fn observe_event(payload: &Value) {
    let Some(kind) = payload.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "status" => {
            let level = payload.get("level").and_then(Value::as_str).unwrap_or("info");
            let message = payload.get("message").and_then(Value::as_str).unwrap_or("");
            record_log(level, message);
        }
        "state" => {
            let logged_in = payload
                .pointer("/state/logged_in")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let previous = LAST_LOGGED_IN.swap(logged_in, Ordering::Relaxed);
            if logged_in && !previous {
                AUTH_EXPIRED_NOTIFIED.store(false, Ordering::Relaxed);
            }
        }
        "organizer" => {
            let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
            if event == "mapping-error" {
                if let Some(message) = payload.get("message").and_then(Value::as_str) {
                    record_log("warning", &format!("[整理] 监控异常：{message}"));
                }
                return;
            }
            if event != "job-updated" {
                return;
            }
            let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
            if let Some(message) = payload.get("message").and_then(Value::as_str) {
                record_log("info", &format!("[整理] {status}：{message}"));
            }
            if matches!(status, "completed" | "completed_warning" | "needs_review" | "failed") {
                if let Some(job_id) = payload.get("job_id").and_then(Value::as_str) {
                    send_event(TelegramEvent::JobUpdated {
                        job_id: job_id.to_string(),
                        status: status.to_string(),
                    });
                }
            }
        }
        _ => {}
    }
}

/// 光鸭登录失效时调用（auth.rs invalidate 路径）。
pub(crate) fn notify_auth_expired(reason: &str) {
    send_event(TelegramEvent::AuthExpired {
        reason: reason.to_string(),
    });
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct TelegramSettings {
    pub(crate) enabled: bool,
    pub(crate) mode: String,
    pub(crate) bot_token: String,
    pub(crate) api_base_url: String,
    pub(crate) api_id: String,
    pub(crate) api_hash: String,
    pub(crate) chat_ids: Vec<String>,
    pub(crate) notify: HashMap<String, bool>,
    managed: HashSet<&'static str>,
}

impl TelegramSettings {
    pub(crate) fn configured(&self) -> bool {
        if self.mode == "mtproto" {
            !self.bot_token.is_empty() && !self.api_id.is_empty() && !self.api_hash.is_empty()
        } else {
            !self.bot_token.is_empty()
        }
    }
    fn notify_enabled(&self, category: &str) -> bool {
        self.notify.get(category).copied().unwrap_or(true)
    }
    fn primary_chat(&self) -> Option<&str> {
        self.chat_ids.first().map(String::as_str)
    }
    fn allows(&self, chat_id: &str, sender_id: &str) -> bool {
        self.chat_ids.iter().any(|id| id == chat_id || id == sender_id)
    }
}

fn env_value(key: &str) -> String {
    std::env::var(key).map(|value| value.trim().to_string()).unwrap_or_default()
}

fn env_flag_enabled(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn stored_value(db_path: &Path, key: &str) -> String {
    load_app_state(db_path, key)
        .ok()
        .flatten()
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

pub(crate) fn parse_chat_ids(value: &str) -> Vec<String> {
    value
        .split([' ', ',', ';', '，', '、', '\n', '\t'])
        .map(str::trim)
        .filter(|item| {
            !item.is_empty()
                && item.len() <= 21
                && item.trim_start_matches('-').chars().all(|c| c.is_ascii_digit())
                && !item.trim_start_matches('-').is_empty()
        })
        .map(str::to_string)
        .collect()
}

pub(crate) fn normalize_telegram_api_base_url(value: &str) -> Result<String, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| "Telegram API 地址必须是完整的 HTTP(S) URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Telegram API 地址必须使用 HTTP 或 HTTPS".to_string());
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Telegram API 地址不能包含账号、查询参数或片段".to_string());
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

pub(crate) fn effective_settings(db_path: &Path) -> TelegramSettings {
    let mut managed = HashSet::new();
    let env_enabled = env_value("TELEGRAM_ENABLED");
    let enabled = if env_enabled.is_empty() {
        stored_value(db_path, "telegram_enabled") == "true"
    } else {
        managed.insert("enabled");
        env_flag_enabled(&env_enabled)
    };
    let env_mode = env_value("TELEGRAM_MODE");
    let mode_raw = if env_mode.is_empty() {
        stored_value(db_path, "telegram_mode")
    } else {
        managed.insert("mode");
        env_mode
    };
    let mode = if mode_raw.eq_ignore_ascii_case("mtproto") { "mtproto" } else { "bot_api" }.to_string();
    let env_token = env_value("TELEGRAM_BOT_TOKEN");
    let bot_token = if env_token.is_empty() {
        stored_value(db_path, "telegram_bot_token")
    } else {
        managed.insert("bot_token");
        env_token
    };
    let env_base = env_value("TELEGRAM_API_BASE_URL");
    let base_raw = if env_base.is_empty() {
        stored_value(db_path, "telegram_api_base_url")
    } else {
        managed.insert("api_base_url");
        env_base
    };
    let api_base_url = normalize_telegram_api_base_url(&base_raw).unwrap_or_default();
    let env_api_id = env_value("TELEGRAM_API_ID");
    let api_id = if env_api_id.is_empty() {
        stored_value(db_path, "telegram_api_id")
    } else {
        managed.insert("api_id");
        env_api_id
    };
    let env_api_hash = env_value("TELEGRAM_API_HASH");
    let api_hash = if env_api_hash.is_empty() {
        stored_value(db_path, "telegram_api_hash")
    } else {
        managed.insert("api_hash");
        env_api_hash
    };
    let env_chat = env_value("TELEGRAM_CHAT_ID");
    let chat_raw = if env_chat.is_empty() {
        stored_value(db_path, "telegram_chat_id")
    } else {
        managed.insert("chat_id");
        env_chat
    };
    let mut notify: HashMap<String, bool> =
        NOTIFY_CATEGORIES.iter().map(|key| (key.to_string(), true)).collect();
    if let Ok(stored) = serde_json::from_str::<HashMap<String, bool>>(&stored_value(db_path, "telegram_notify")) {
        for (key, value) in stored {
            if NOTIFY_CATEGORIES.contains(&key.as_str()) {
                notify.insert(key, value);
            }
        }
    }
    TelegramSettings {
        enabled,
        mode,
        bot_token,
        api_base_url,
        api_id,
        api_hash,
        chat_ids: parse_chat_ids(&chat_raw),
        notify,
        managed,
    }
}

pub(crate) fn webhook_secret(db_path: &Path) -> String {
    let existing = stored_value(db_path, "telegram_emby_webhook_secret");
    if !existing.is_empty() {
        return existing;
    }
    let secret = Uuid::new_v4().simple().to_string();
    let _ = save_app_state(db_path, "telegram_emby_webhook_secret", &secret);
    secret
}

pub(crate) fn public_settings(db_path: &Path) -> Value {
    let settings = effective_settings(db_path);
    let (connected, bot_username, last_error) = status_info();
    let secret = webhook_secret(db_path);
    json!({
        "enabled": settings.enabled,
        "mode": settings.mode,
        "chat_id": settings.chat_ids.join(","),
        "api_base_url": settings.api_base_url,
        "api_id": settings.api_id,
        "bot_token_configured": !settings.bot_token.is_empty(),
        "api_hash_configured": !settings.api_hash.is_empty(),
        "configured": settings.configured(),
        "notify": settings.notify,
        "enabled_managed_by_environment": settings.managed.contains("enabled"),
        "mode_managed_by_environment": settings.managed.contains("mode"),
        "bot_token_managed_by_environment": settings.managed.contains("bot_token"),
        "api_base_url_managed_by_environment": settings.managed.contains("api_base_url"),
        "api_id_managed_by_environment": settings.managed.contains("api_id"),
        "api_hash_managed_by_environment": settings.managed.contains("api_hash"),
        "chat_id_managed_by_environment": settings.managed.contains("chat_id"),
        "connected": connected,
        "bot_username": bot_username,
        "last_error": last_error,
        "webhook": {
            "secret": secret,
            "path": format!("/webhooks/emby?token={secret}"),
            "gateway_path": format!("/guangya/webhooks/emby?token={secret}"),
        },
    })
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn get_telegram_settings(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let db_path = state.lock().map_err(|error| error.to_string())?.db_path.clone();
    Ok(public_settings(&db_path))
}

#[tauri::command]
pub(crate) fn update_telegram_settings(
    state: tauri::State<'_, SharedState>,
    input: Value,
) -> Result<Value, String> {
    let db_path = state.lock().map_err(|error| error.to_string())?.db_path.clone();
    let before = effective_settings(&db_path);
    if let Some(enabled) = input.get("enabled").and_then(Value::as_bool) {
        save_app_state(&db_path, "telegram_enabled", &enabled.to_string())?;
    }
    if let Some(mode) = input.get("mode").and_then(Value::as_str) {
        let normalized = mode.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "bot_api" | "mtproto") {
            return Err("接入模式只支持 bot_api 或 mtproto".to_string());
        }
        save_app_state(&db_path, "telegram_mode", &normalized)?;
    }
    if let Some(token) = input.get("bot_token").and_then(Value::as_str) {
        let value = token.trim();
        if value == "off" {
            save_app_state(&db_path, "telegram_bot_token", "")?;
        } else if !value.is_empty() {
            let valid = value.split_once(':').is_some_and(|(id, rest)| {
                !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) && rest.len() >= 20
            });
            if !valid {
                return Err("Bot Token 格式不正确（应为 123456:ABC-DEF… 形式）".to_string());
            }
            save_app_state(&db_path, "telegram_bot_token", value)?;
        }
    }
    if let Some(base) = input.get("api_base_url").and_then(Value::as_str) {
        save_app_state(&db_path, "telegram_api_base_url", &normalize_telegram_api_base_url(base)?)?;
    }
    if let Some(api_id) = input.get("api_id") {
        let value = match api_id {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.trim().to_string(),
            _ => String::new(),
        };
        if !value.is_empty() && (value.len() > 12 || !value.chars().all(|c| c.is_ascii_digit())) {
            return Err("API ID 必须是数字".to_string());
        }
        save_app_state(&db_path, "telegram_api_id", &value)?;
    }
    if let Some(hash) = input.get("api_hash").and_then(Value::as_str) {
        let value = hash.trim();
        if value == "off" {
            save_app_state(&db_path, "telegram_api_hash", "")?;
        } else if !value.is_empty() {
            if value.len() != 32 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("API Hash 格式不正确（32 位十六进制）".to_string());
            }
            save_app_state(&db_path, "telegram_api_hash", value)?;
        }
    }
    if let Some(chat) = input.get("chat_id").and_then(Value::as_str) {
        let tokens: Vec<&str> = chat
            .split([' ', ',', ';', '，', '、', '\n', '\t'])
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect();
        let invalid: Vec<&str> = tokens
            .iter()
            .filter(|item| {
                let digits = item.trim_start_matches('-');
                digits.is_empty() || item.len() > 21 || !digits.chars().all(|c| c.is_ascii_digit())
            })
            .copied()
            .collect();
        if !invalid.is_empty() {
            return Err(format!(
                "Chat ID 必须是数字（可给机器人发送 /start 获取）：{}",
                invalid.join(", ")
            ));
        }
        save_app_state(&db_path, "telegram_chat_id", &tokens.join(","))?;
    }
    if let Some(notify) = input.get("notify").and_then(Value::as_object) {
        let mut merged: HashMap<String, bool> = effective_settings(&db_path).notify;
        for key in NOTIFY_CATEGORIES {
            if let Some(value) = notify.get(*key).and_then(Value::as_bool) {
                merged.insert((*key).to_string(), value);
            }
        }
        save_app_state(
            &db_path,
            "telegram_notify",
            &serde_json::to_string(&merged).unwrap_or_default(),
        )?;
    }
    if input.get("regenerate_webhook_secret").and_then(Value::as_bool) == Some(true) {
        save_app_state(&db_path, "telegram_emby_webhook_secret", &Uuid::new_v4().simple().to_string())?;
    }
    let after = effective_settings(&db_path);
    if after.enabled && !after.configured() {
        // 回滚启用状态，避免留下“已启用但未配置”的中间态。
        if !after.managed.contains("enabled") {
            save_app_state(&db_path, "telegram_enabled", &before.enabled.to_string())?;
        }
        return Err(if after.mode == "mtproto" {
            "启用 MTProto 模式需要填写 API ID、API Hash 和 Bot Token".to_string()
        } else {
            "启用前请先填写 Bot Token".to_string()
        });
    }
    // 凭据变化会使旧 MTProto 会话失效，删掉会话文件避免用错账号。
    if before.bot_token != after.bot_token || before.api_id != after.api_id || before.api_hash != after.api_hash {
        let _ = fs::remove_file(mtproto_session_path(&db_path));
    }
    send_event(TelegramEvent::Restart);
    Ok(public_settings(&db_path))
}

#[tauri::command]
pub(crate) async fn test_telegram_message(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let db_path = state.lock().map_err(|error| error.to_string())?.db_path.clone();
    let settings = effective_settings(&db_path);
    if !settings.configured() {
        return Err(if settings.mode == "mtproto" {
            "请先填写 API ID、API Hash 和 Bot Token".to_string()
        } else {
            "请先填写 Bot Token".to_string()
        });
    }
    let Some(chat_id) = settings.primary_chat().map(str::to_string) else {
        return Err("请先填写 Chat ID（可先给机器人发送 /start 获取）".to_string());
    };
    let text = format!(
        "✅ <b>Telegram 通知渠道连接正常</b>\n来自：光鸭云盘工作台 v{}（Windows 桌面端）\n模式：{}",
        env!("CARGO_PKG_VERSION"),
        if settings.mode == "mtproto" { "MTProto" } else { "Bot API" }
    );
    let connection = connect_transport(&db_path, &settings).await?;
    let username = connection.sender.get_me().await.unwrap_or_default();
    let result = connection.sender.send_message(&chat_id, &text, None, false).await;
    connection.shutdown().await;
    result?;
    Ok(json!({ "ok": true, "bot_username": username }))
}

#[tauri::command]
pub(crate) fn get_recent_logs(limit: Option<usize>) -> Value {
    let list: Vec<Value> = recent_logs(limit.unwrap_or(50))
        .into_iter()
        .map(|entry| json!({ "time": entry.time_ms, "level": entry.level, "message": entry.message }))
        .collect();
    json!({ "list": list })
}

// ---------------------------------------------------------------------------
// 纯文本处理与格式化（与 Node 端 telegram.mjs 对齐）
// ---------------------------------------------------------------------------

pub(crate) fn escape_html(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub(crate) fn short_job_id(id: &str) -> String {
    let compact: String = id.chars().filter(|c| *c != '-').take(8).collect();
    if compact.is_empty() { "--------".to_string() } else { compact }
}

pub(crate) type Keyboard = Vec<Vec<(String, String)>>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReParse {
    NotRe,
    Error(String),
    Command { job_ref: String, input: Value },
}

/// 解析覆盖参数（`tmdbid=… s=… e=… tv/movie` 等），输出 OrganizerJobInput 兼容 JSON。
pub(crate) fn parse_override_tokens(tokens: &[&str]) -> Result<Value, String> {
    let mut input = serde_json::Map::new();
    for raw in tokens {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if let Some((key, value)) = token.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "tmdbid" | "tmdb" | "tmdb_id" | "id" => {
                    let parsed: i64 = value
                        .parse()
                        .ok()
                        .filter(|id| *id > 0 && value.len() <= 10)
                        .ok_or_else(|| format!("TMDB ID 必须是数字：{token}"))?;
                    input.insert("tmdb_id".into(), json!(parsed));
                }
                "s" | "season" => {
                    let parsed: i64 = value
                        .parse()
                        .ok()
                        .filter(|_| value.len() <= 3)
                        .ok_or_else(|| format!("季号必须是数字：{token}"))?;
                    input.insert("season".into(), json!(parsed));
                }
                "e" | "ep" | "episode" => {
                    let parsed: i64 = value
                        .parse()
                        .ok()
                        .filter(|_| value.len() <= 4)
                        .ok_or_else(|| format!("集号必须是数字：{token}"))?;
                    input.insert("episode".into(), json!(parsed));
                }
                "type" | "t" | "media" | "media_type" => {
                    let mapped = match value.to_ascii_lowercase().as_str() {
                        "tv" | "剧集" | "电视剧" => "tv",
                        "movie" | "电影" => "movie",
                        _ => return Err(format!("类型只支持 tv 或 movie：{token}")),
                    };
                    input.insert("media_type".into(), json!(mapped));
                }
                "title" | "name" => {
                    if value.is_empty() {
                        return Err("标题不能为空".to_string());
                    }
                    input.insert("title".into(), json!(value));
                }
                "y" | "year" => {
                    let parsed: i64 = value
                        .parse()
                        .ok()
                        .filter(|_| value.len() == 4)
                        .ok_or_else(|| format!("年份必须是 4 位数字：{token}"))?;
                    input.insert("year".into(), json!(parsed));
                }
                _ => return Err(format!("无法识别参数：{token}（支持 tmdbid= s= e= type= title= year=）")),
            }
            continue;
        }
        match lower.as_str() {
            "tv" | "剧集" | "电视剧" => {
                input.insert("media_type".into(), json!("tv"));
                continue;
            }
            "movie" | "电影" => {
                input.insert("media_type".into(), json!("movie"));
                continue;
            }
            _ => {}
        }
        if lower.len() >= 2 && lower.len() <= 4 && lower.starts_with('s') && lower[1..].chars().all(|c| c.is_ascii_digit()) {
            input.insert("season".into(), json!(lower[1..].parse::<i64>().unwrap_or_default()));
            continue;
        }
        if lower.len() >= 2 && lower.len() <= 5 && lower.starts_with('e') && lower[1..].chars().all(|c| c.is_ascii_digit()) {
            input.insert("episode".into(), json!(lower[1..].parse::<i64>().unwrap_or_default()));
            continue;
        }
        if token.len() <= 10 && token.chars().all(|c| c.is_ascii_digit()) {
            input.insert("tmdb_id".into(), json!(token.parse::<i64>().unwrap_or_default()));
            continue;
        }
        return Err(format!("无法识别参数：{token}（支持 tmdbid= s= e= tv/movie 或直接给出 TMDB 数字 ID）"));
    }
    Ok(Value::Object(input))
}

/// 解析 `re <任务ID> tmdbid=12345 [tv|movie] [s=1] [e=2]` 重新整理命令。
pub(crate) fn parse_re_command(text: &str) -> ReParse {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ReParse::NotRe;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let head = tokens[0].to_ascii_lowercase();
    let head = head.split('@').next().unwrap_or_default();
    if head != "re" && head != "/re" {
        return ReParse::NotRe;
    }
    if tokens.len() < 2 {
        return ReParse::Error("用法：re <任务ID> tmdbid=12345 [tv|movie] [s=1] [e=2]".to_string());
    }
    match parse_override_tokens(&tokens[2..]) {
        Ok(input) => ReParse::Command {
            job_ref: tokens[1].to_string(),
            input,
        },
        Err(error) => ReParse::Error(error),
    }
}

pub(crate) fn find_job_by_ref<'a>(jobs: &'a [Value], reference: &str) -> Result<&'a Value, String> {
    let normalized: String = reference.trim().to_ascii_lowercase().chars().filter(|c| *c != '-').collect();
    if normalized.is_empty() {
        return Err("请提供任务 ID（可在 /jobs 或失败通知中查看）".to_string());
    }
    if normalized.len() < 4 || normalized.len() > 32 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("任务 ID 格式不正确：{reference}"));
    }
    let matches: Vec<&Value> = jobs
        .iter()
        .filter(|job| {
            job.get("id")
                .and_then(Value::as_str)
                .map(|id| {
                    id.to_ascii_lowercase()
                        .chars()
                        .filter(|c| *c != '-')
                        .collect::<String>()
                        .starts_with(&normalized)
                })
                .unwrap_or(false)
        })
        .collect();
    match matches.len() {
        0 => Err(format!("没有找到任务 {reference}，可先用 /jobs 查看最近任务")),
        1 => Ok(matches[0]),
        count => Err(format!("任务 ID 前缀 {reference} 匹配到 {count} 个任务，请使用更长的前缀")),
    }
}

fn value_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn source_base_name(job: &Value) -> String {
    let path = value_str(job, "source_path").replace('\\', "/");
    path.split('/')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or(if path.is_empty() { "未知来源" } else { &path })
        .to_string()
}

pub(crate) fn describe_job_title(job: &Value) -> String {
    let title = {
        let query = value_str(job, "query_title").trim();
        if query.is_empty() { source_base_name(job) } else { query.to_string() }
    };
    let year = job
        .get("query_year")
        .and_then(Value::as_i64)
        .map(|year| format!(" ({year})"))
        .unwrap_or_default();
    let media_type = value_str(job, "media_type");
    let type_label = match media_type {
        "tv" => " · 剧集",
        "movie" => " · 电影",
        _ => "",
    };
    let season = if media_type == "tv" {
        job.get("season")
            .and_then(Value::as_i64)
            .map(|season| format!(" S{season:02}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("{title}{year}{type_label}{season}")
}

fn error_code_label(code: &str) -> String {
    match code {
        "recognition_failed" => "识别失败",
        "tmdb_not_found" => "TMDB 没有找到匹配条目",
        "tmdb_not_configured" => "尚未配置 TMDB API Key",
        "tmdb_unavailable" => "TMDB 服务不可用",
        "ambiguous_match" => "匹配结果不唯一，需要人工确认",
        "title_required" => "未能从文件名解析出标题",
        "episode_required" => "未识别到季集号",
        "video_required" => "没有可整理的视频文件",
        "source_missing" => "云端源文件已不存在",
        "source_unavailable" => "云端源暂不可用",
        "transfer_failed" => "云端转移执行失败",
        "rearchive_failed" => "重新归档失败",
        "completed_warning" => "完成但有提示",
        "" => "未知原因",
        other => other,
    }
    .to_string()
}

fn job_target_path(job: &Value, mapping: Option<&Value>) -> String {
    let relative = job
        .pointer("/preview/share_relative_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .trim_start_matches('/');
    let base = mapping
        .map(|mapping| value_str(mapping, "target_path").trim_end_matches('/'))
        .unwrap_or("");
    if relative.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return relative.to_string();
    }
    format!("{base}/{relative}")
}

/// 整理完成（入库）通知文本。
pub(crate) fn format_organize_done(job: &Value, mapping: Option<&Value>) -> String {
    let mut lines = vec![
        format!("✅ <b>入库完成</b>：{}", escape_html(&describe_job_title(job))),
        format!("📁 来源：{}", escape_html(&source_base_name(job))),
    ];
    let target = job_target_path(job, mapping);
    if !target.is_empty() {
        lines.push(format!("🎯 目标：{}", escape_html(&target)));
    }
    let message = value_str(job, "message").trim();
    if !message.is_empty() {
        lines.push(format!("💬 {}", escape_html(message)));
    }
    if let Some(share) = job.pointer("/result/share/share_url").and_then(Value::as_str) {
        if !share.trim().is_empty() {
            lines.push(format!("🔗 {}", escape_html(share)));
        }
    }
    lines.push(format!("🆔 <code>{}</code>", escape_html(&short_job_id(value_str(job, "id")))));
    lines.join("\n")
}

/// 识别失败 / 整理失败通知，附重新整理 keyboard。
pub(crate) fn format_review_needed(job: &Value) -> (String, Keyboard) {
    let failed = value_str(job, "status") == "failed";
    let heading = if failed { "❌ <b>整理失败</b>" } else { "⚠️ <b>识别待处理</b>" };
    let id = value_str(job, "id");
    let short = short_job_id(id);
    let mut lines = vec![format!("{heading}：{}", escape_html(&describe_job_title(job)))];
    let source = value_str(job, "source_path");
    let source_label = if source.is_empty() { source_base_name(job) } else { source.to_string() };
    lines.push(format!("📁 来源：{}", escape_html(&source_label)));
    lines.push(format!("⛔ 原因：{}", escape_html(&error_code_label(value_str(job, "error_code")))));
    let message = value_str(job, "message").trim();
    if !message.is_empty() {
        lines.push(format!("💬 {}", escape_html(message)));
    }
    lines.push(format!("🆔 <code>{}</code>", escape_html(&short)));
    lines.push(format!("↩️ 手动指定：<code>re {} tmdbid=12345 tv s=1</code>", escape_html(&short)));
    let keyboard = vec![
        vec![
            ("🔁 重新识别".to_string(), format!("retry:{id}")),
            ("▶️ 重新整理".to_string(), format!("run:{id}")),
        ],
        vec![("🔎 填写 TMDB ID".to_string(), format!("ask:{id}"))],
    ];
    (lines.join("\n"), keyboard)
}

/// 长文本按行拆分为不超过 limit 字符的多段（Telegram 单条消息上限 4096）。
pub(crate) fn chunk_lines(lines: &[String], limit: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in lines {
        let piece: String = if line.chars().count() > limit {
            let mut truncated: String = line.chars().take(limit.saturating_sub(1)).collect();
            truncated.push('…');
            truncated
        } else {
            line.clone()
        };
        if !current.is_empty() && current.chars().count() + piece.chars().count() + 1 > limit {
            chunks.push(std::mem::take(&mut current));
            current = piece;
        } else if current.is_empty() {
            current = piece;
        } else {
            current.push('\n');
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

// ---------------------------------------------------------------------------
// Emby webhook 解析
// ---------------------------------------------------------------------------

/// 解析 Emby webhook 请求体：兼容 JSON、multipart/form-data 的 data 字段与 urlencoded。
pub(crate) fn parse_emby_webhook_body(content_type: &str, body: &[u8]) -> Option<Value> {
    let normalized = content_type.to_ascii_lowercase();
    if normalized.contains("multipart/form-data") {
        let boundary = content_type
            .split(';')
            .map(str::trim)
            .find_map(|part| part.strip_prefix("boundary="))
            .map(|value| value.trim_matches('"'))?;
        let fields = parse_multipart_text_fields(body, boundary);
        return fields.get("data").and_then(|data| serde_json::from_str(data).ok());
    }
    if normalized.contains("application/x-www-form-urlencoded") {
        let text = String::from_utf8_lossy(body);
        for pair in text.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                if key == "data" {
                    let plus_decoded = value.replace('+', " ");
                    let decoded = percent_decode_str(&plus_decoded).decode_utf8().ok()?;
                    return serde_json::from_str(&decoded).ok();
                }
            }
        }
        return None;
    }
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).ok();
    }
    None
}

fn parse_multipart_text_fields(body: &[u8], boundary: &str) -> HashMap<String, String> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut fields = HashMap::new();
    let mut cursor = find_bytes(body, &delimiter, 0);
    while let Some(index) = cursor {
        let start = index + delimiter.len();
        if body.get(start..start + 2) == Some(b"--") {
            break;
        }
        let next = find_bytes(body, &delimiter, start);
        let part = &body[start..next.unwrap_or(body.len())];
        if let Some(header_end) = find_bytes(part, b"\r\n\r\n", 0) {
            let header_text = String::from_utf8_lossy(&part[..header_end]);
            let is_file = header_text.to_ascii_lowercase().contains("filename=\"");
            if let Some(name) = header_text
                .split(';')
                .map(str::trim)
                .find_map(|piece| piece.strip_prefix("name=\""))
                .and_then(|rest| rest.split('"').next())
            {
                if !is_file {
                    let mut value = &part[header_end + 4..];
                    if value.ends_with(b"\r\n") {
                        value = &value[..value.len() - 2];
                    }
                    if value.len() <= 256 * 1024 {
                        fields.insert(name.to_string(), String::from_utf8_lossy(value).to_string());
                    }
                }
            }
        }
        cursor = next;
    }
    fields
}

fn find_bytes(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + from)
}

fn emby_item_label(item: &Value) -> String {
    let name = value_str(item, "Name").trim();
    if value_str(item, "Type") == "Episode" {
        let series = value_str(item, "SeriesName").trim();
        let season = item
            .get("ParentIndexNumber")
            .and_then(Value::as_i64)
            .map(|value| format!("S{value:02}"))
            .unwrap_or_default();
        let episode = item
            .get("IndexNumber")
            .and_then(Value::as_i64)
            .map(|value| format!("E{value:02}"))
            .unwrap_or_default();
        let numbers = format!("{season}{episode}");
        return [series, numbers.as_str(), name]
            .iter()
            .filter(|piece| !piece.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
    }
    if name.is_empty() {
        return String::new();
    }
    match item.get("ProductionYear").and_then(Value::as_i64) {
        Some(year) => format!("{name} ({year})"),
        None => name.to_string(),
    }
}

fn playback_progress_label(payload: &Value) -> String {
    let position = payload
        .pointer("/PlaybackInfo/PositionTicks")
        .or_else(|| payload.pointer("/Session/PlayState/PositionTicks"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if position <= 0 {
        return String::new();
    }
    let seconds = position / 10_000_000;
    let clock = format!("{:02}:{:02}", seconds / 60, seconds % 60);
    let total = payload.pointer("/Item/RunTimeTicks").and_then(Value::as_i64).unwrap_or(0);
    if total <= 0 {
        return clock;
    }
    let percent = ((position as f64 / total as f64) * 100.0).round().clamp(0.0, 100.0) as i64;
    format!("{clock}（{percent}%）")
}

/// 把 Emby webhook 事件映射为通知类别与文本；未知事件返回 None。
pub(crate) fn describe_emby_event(payload: &Value) -> Option<(String, String)> {
    let event = payload
        .get("Event")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if event.is_empty() {
        return None;
    }
    let empty = json!({});
    let item = payload.get("Item").unwrap_or(&empty);
    let user = payload.get("User").unwrap_or(&empty);
    let session = payload.get("Session").unwrap_or(&empty);
    let server = payload.pointer("/Server/Name").and_then(Value::as_str).unwrap_or("").trim();
    let server_suffix = if server.is_empty() {
        String::new()
    } else {
        format!("（{}）", escape_html(server))
    };
    let item_label = {
        let label = emby_item_label(item);
        if label.is_empty() {
            let title = value_str(payload, "Title").trim();
            if title.is_empty() { "未知条目".to_string() } else { title.to_string() }
        } else {
            label
        }
    };
    if event == "library.new" {
        let mut lines = vec![format!("📥 <b>Emby 入库</b>{server_suffix}"), escape_html(&item_label)];
        let path = value_str(item, "Path").trim();
        if !path.is_empty() {
            lines.push(format!("📁 {}", escape_html(path)));
        }
        let overview = value_str(item, "Overview").trim();
        if !overview.is_empty() {
            lines.push(escape_html(&overview.chars().take(200).collect::<String>()));
        }
        return Some(("emby_new".to_string(), lines.join("\n")));
    }
    if let Some(action_key) = event.strip_prefix("playback.") {
        let action = match action_key {
            "start" => "开始播放",
            "stop" => "停止播放",
            "pause" => "暂停播放",
            "unpause" => "继续播放",
            "progress" => "播放进度",
            other => other,
        };
        let icon = match action {
            "停止播放" => "⏹️",
            "暂停播放" => "⏸️",
            _ => "▶️",
        };
        let mut lines = vec![format!("{icon} <b>Emby {}</b>{server_suffix}", escape_html(action))];
        let user_name = value_str(user, "Name").trim();
        if !user_name.is_empty() {
            lines.push(format!("👤 {}", escape_html(user_name)));
        }
        lines.push(format!("🎬 {}", escape_html(&item_label)));
        let device = [value_str(session, "DeviceName").trim(), value_str(session, "Client").trim()]
            .iter()
            .filter(|piece| !piece.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        if !device.is_empty() {
            lines.push(format!("📱 {}", escape_html(&device)));
        }
        let progress = playback_progress_label(payload);
        if !progress.is_empty() {
            lines.push(format!("⏳ {}", escape_html(&progress)));
        }
        return Some(("emby_play".to_string(), lines.join("\n")));
    }
    if event == "user.authenticated" || event == "user.authenticationfailed" {
        let failed = event.ends_with("failed");
        let mut lines = vec![format!(
            "{} <b>Emby {}</b>{server_suffix}",
            if failed { "🚨" } else { "🔐" },
            if failed { "登录失败" } else { "用户登录" }
        )];
        let who = {
            let name = value_str(user, "Name").trim();
            if name.is_empty() { value_str(payload, "Title").trim() } else { name }
        };
        if !who.is_empty() {
            lines.push(format!("👤 {}", escape_html(who)));
        }
        let endpoint = value_str(session, "RemoteEndPoint").trim();
        if !endpoint.is_empty() {
            lines.push(format!("🌐 {}", escape_html(endpoint)));
        }
        let device = [value_str(session, "DeviceName").trim(), value_str(session, "Client").trim()]
            .iter()
            .filter(|piece| !piece.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        if !device.is_empty() {
            lines.push(format!("📱 {}", escape_html(&device)));
        }
        return Some(("emby_login".to_string(), lines.join("\n")));
    }
    None
}

fn secret_matches(db_path: &Path, provided: Option<&str>) -> bool {
    let Some(provided) = provided.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let secret = webhook_secret(db_path);
    let left = Sha256::digest(provided.as_bytes());
    let right = Sha256::digest(secret.as_bytes());
    left == right
}

/// Emby webhook 入口（由 virtual_library 网关路由调用），返回 (HTTP 状态码, JSON)。
pub(crate) fn handle_emby_webhook(db_path: &Path, token: Option<&str>, content_type: &str, body: &[u8]) -> (u16, Value) {
    if !secret_matches(db_path, token) {
        return (403, json!({ "error": "invalid webhook token" }));
    }
    let Some(payload) = parse_emby_webhook_body(content_type, body) else {
        record_log("warning", "[Emby] 收到无法解析的 webhook 请求");
        return (400, json!({ "error": "unrecognized payload" }));
    };
    let event_name = payload
        .get("Event")
        .or_else(|| payload.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let title = value_str(&payload, "Title").trim().to_string();
    record_log(
        "info",
        &format!("[Emby] webhook：{event_name}{}", if title.is_empty() { String::new() } else { format!("（{title}）") }),
    );
    let described = describe_emby_event(&payload);
    let handled = described.is_some();
    if let Some((category, text)) = described {
        send_event(TelegramEvent::EmbyNotify { category, text });
    }
    (200, json!({ "ok": true, "handled": handled }))
}

// ---------------------------------------------------------------------------
// 事件与传输抽象
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum TelegramEvent {
    JobUpdated { job_id: String, status: String },
    AuthExpired { reason: String },
    EmbyNotify { category: String, text: String },
    SendText { chat_id: String, text: String, keyboard: Option<Keyboard> },
    SendPhoto { chat_id: String, bytes: Vec<u8>, caption: String },
    Incoming(Incoming),
    TransportFailed { message: String, fatal: bool },
    Restart,
}

#[derive(Debug)]
pub(crate) enum Incoming {
    Message {
        chat_id: String,
        sender_id: String,
        text: String,
        reply_to: Option<i64>,
    },
    Callback {
        chat_id: String,
        sender_id: String,
        data: String,
        handle: CallbackHandle,
    },
}

pub(crate) enum CallbackHandle {
    BotApi { id: String },
    Mtproto { query: Box<grammers_client::update::CallbackQuery> },
}

impl std::fmt::Debug for CallbackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallbackHandle::BotApi { id } => write!(f, "CallbackHandle::BotApi({id})"),
            CallbackHandle::Mtproto { .. } => write!(f, "CallbackHandle::Mtproto"),
        }
    }
}

#[derive(Clone)]
pub(crate) enum TransportSender {
    BotApi(BotApiSender),
    Mtproto(MtprotoSender),
}

pub(crate) struct ConnectedTransport {
    pub(crate) sender: TransportSender,
    receiver_task: Option<tauri::async_runtime::JoinHandle<()>>,
    shutdown: Option<Box<dyn FnOnce() + Send>>,
}

impl ConnectedTransport {
    pub(crate) async fn shutdown(mut self) {
        if let Some(task) = self.receiver_task.take() {
            task.abort();
        }
        if let Some(shutdown) = self.shutdown.take() {
            shutdown();
        }
    }
}

impl TransportSender {
    async fn get_me(&self) -> Result<String, String> {
        match self {
            TransportSender::BotApi(sender) => sender.get_me().await,
            TransportSender::Mtproto(sender) => sender.get_me().await,
        }
    }
    async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        keyboard: Option<&Keyboard>,
        force_reply: bool,
    ) -> Result<i64, String> {
        match self {
            TransportSender::BotApi(sender) => sender.send_message(chat_id, text, keyboard, force_reply).await,
            TransportSender::Mtproto(sender) => sender.send_message(chat_id, text, keyboard, force_reply).await,
        }
    }
    async fn send_photo_bytes(&self, chat_id: &str, bytes: &[u8], caption: &str, filename: &str) -> Result<(), String> {
        match self {
            TransportSender::BotApi(sender) => sender.send_photo_bytes(chat_id, bytes, caption, filename).await,
            TransportSender::Mtproto(sender) => sender.send_photo_bytes(chat_id, bytes, caption, filename).await,
        }
    }
    async fn send_photo_url(&self, chat_id: &str, url: &str, caption: &str) -> Result<(), String> {
        match self {
            TransportSender::BotApi(sender) => sender.send_photo_url(chat_id, url, caption).await,
            TransportSender::Mtproto(sender) => sender.send_photo_url(chat_id, url, caption).await,
        }
    }
    async fn set_commands(&self) -> Result<(), String> {
        match self {
            TransportSender::BotApi(sender) => sender.set_commands().await,
            TransportSender::Mtproto(sender) => sender.set_commands().await,
        }
    }
    async fn answer_callback(&self, handle: &CallbackHandle, text: &str) {
        match (self, handle) {
            (TransportSender::BotApi(sender), CallbackHandle::BotApi { id }) => {
                sender.answer_callback(id, text).await;
            }
            (TransportSender::Mtproto(_), CallbackHandle::Mtproto { query }) => {
                let answer = query.answer();
                let result = if text.is_empty() {
                    answer.send().await
                } else {
                    answer.text(text.chars().take(190).collect::<String>()).send().await
                };
                if let Err(error) = result {
                    record_log("warning", &format!("Telegram 回调应答失败：{error}"));
                }
            }
            _ => {}
        }
    }
}

fn build_http_client(proxy: &str) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    let trimmed = proxy.trim();
    if !trimmed.is_empty() {
        let proxy = reqwest::Proxy::all(trimmed).map_err(|error| format!("代理地址无效：{error}"))?;
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|error| format!("创建 HTTP 客户端失败：{error}"))
}

// ---------------------------------------------------------------------------
// Bot API 传输
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct BotApiSender {
    client: reqwest::Client,
    base: String,
}

struct BotApiCallError {
    message: String,
    status: u16,
}

impl BotApiSender {
    fn new(db_path: &Path, settings: &TelegramSettings) -> Result<Self, String> {
        let proxy = load_global_network_proxy(db_path).unwrap_or_default();
        let client = build_http_client(&proxy)?;
        let base = if settings.api_base_url.is_empty() {
            DEFAULT_BOT_API_BASE.to_string()
        } else {
            settings.api_base_url.clone()
        };
        Ok(Self {
            client,
            base: format!("{}/bot{}", base.trim_end_matches('/'), settings.bot_token),
        })
    }

    async fn call(&self, method: &str, params: Value, timeout_secs: u64) -> Result<Value, BotApiCallError> {
        let response = self
            .client
            .post(format!("{}/{method}", self.base))
            .timeout(Duration::from_secs(timeout_secs))
            .json(&params)
            .send()
            .await
            .map_err(|error| BotApiCallError {
                message: format!("Telegram {method} 请求失败：{error}"),
                status: 0,
            })?;
        let http_status = response.status().as_u16();
        let payload: Value = response.json().await.unwrap_or_else(|_| json!({}));
        if payload.get("ok").and_then(Value::as_bool) != Some(true) {
            let description = payload
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {http_status}"));
            return Err(BotApiCallError {
                message: format!("Telegram {method} 失败：{description}"),
                status: payload
                    .get("error_code")
                    .and_then(Value::as_u64)
                    .map(|code| code as u16)
                    .unwrap_or(http_status),
            });
        }
        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    }

    fn reply_markup(keyboard: Option<&Keyboard>, force_reply: bool) -> Option<Value> {
        if let Some(keyboard) = keyboard.filter(|rows| !rows.is_empty()) {
            let rows: Vec<Value> = keyboard
                .iter()
                .map(|row| {
                    Value::Array(
                        row.iter()
                            .map(|(text, data)| json!({ "text": text, "callback_data": data }))
                            .collect(),
                    )
                })
                .collect();
            return Some(json!({ "inline_keyboard": rows }));
        }
        if force_reply {
            return Some(json!({ "force_reply": true, "selective": true }));
        }
        None
    }

    async fn get_me(&self) -> Result<String, String> {
        let result = self.call("getMe", json!({}), 30).await.map_err(|error| error.message)?;
        Ok(result.get("username").and_then(Value::as_str).unwrap_or("").to_string())
    }

    async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        keyboard: Option<&Keyboard>,
        force_reply: bool,
    ) -> Result<i64, String> {
        let mut params = json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
            "link_preview_options": { "is_disabled": true },
        });
        if let Some(markup) = Self::reply_markup(keyboard, force_reply) {
            params["reply_markup"] = markup;
        }
        let result = self.call("sendMessage", params, 30).await.map_err(|error| error.message)?;
        Ok(result.get("message_id").and_then(Value::as_i64).unwrap_or_default())
    }

    async fn send_photo_bytes(&self, chat_id: &str, bytes: &[u8], caption: &str, filename: &str) -> Result<(), String> {
        let part = reqwest::multipart::Part::bytes(bytes.to_vec())
            .file_name(filename.to_string())
            .mime_str("image/png")
            .map_err(|error| error.to_string())?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .text("caption", caption.to_string())
            .text("parse_mode", "HTML")
            .part("photo", part);
        let response = self
            .client
            .post(format!("{}/sendPhoto", self.base))
            .timeout(Duration::from_secs(60))
            .multipart(form)
            .send()
            .await
            .map_err(|error| format!("Telegram sendPhoto 请求失败：{error}"))?;
        let payload: Value = response.json().await.unwrap_or_else(|_| json!({}));
        if payload.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "Telegram sendPhoto 失败：{}",
                payload.get("description").and_then(Value::as_str).unwrap_or("未知错误")
            ));
        }
        Ok(())
    }

    async fn send_photo_url(&self, chat_id: &str, url: &str, caption: &str) -> Result<(), String> {
        self.call(
            "sendPhoto",
            json!({ "chat_id": chat_id, "photo": url, "caption": caption, "parse_mode": "HTML" }),
            60,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.message)
    }

    async fn set_commands(&self) -> Result<(), String> {
        let commands: Vec<Value> = BOT_COMMANDS
            .iter()
            .map(|(command, description)| json!({ "command": command, "description": description }))
            .collect();
        self.call("setMyCommands", json!({ "commands": commands }), 30)
            .await
            .map(|_| ())
            .map_err(|error| error.message)
    }

    async fn answer_callback(&self, callback_id: &str, text: &str) {
        let mut params = json!({ "callback_query_id": callback_id });
        if !text.is_empty() {
            params["text"] = json!(text.chars().take(190).collect::<String>());
        }
        if let Err(error) = self.call("answerCallbackQuery", params, 15).await {
            record_log("warning", &format!("Telegram 回调应答失败：{}", error.message));
        }
    }

    fn spawn_receiver(&self) -> tauri::async_runtime::JoinHandle<()> {
        let sender = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut offset: i64 = 0;
            let mut failure_delay = 5_u64;
            loop {
                let params = json!({
                    "timeout": 50,
                    "offset": offset,
                    "allowed_updates": ["message", "callback_query"],
                });
                match sender.call("getUpdates", params, 65).await {
                    Ok(result) => {
                        failure_delay = 5;
                        for update in result.as_array().cloned().unwrap_or_default() {
                            if let Some(update_id) = update.get("update_id").and_then(Value::as_i64) {
                                offset = offset.max(update_id + 1);
                            }
                            if let Some(incoming) = Self::map_update(&update) {
                                send_event(TelegramEvent::Incoming(incoming));
                            }
                        }
                    }
                    Err(error) => {
                        if matches!(error.status, 401 | 404) {
                            send_event(TelegramEvent::TransportFailed {
                                message: format!("Bot Token 无效，已停止轮询：{}", error.message),
                                fatal: true,
                            });
                            return;
                        }
                        if error.status == 409 {
                            record_log(
                                "warning",
                                "Telegram getUpdates 冲突（409）：同一个 Bot Token 正在其他实例轮询，请只在一端启用",
                            );
                        } else {
                            record_log(
                                "warning",
                                &format!("Telegram 轮询失败，{failure_delay} 秒后重试：{}", error.message),
                            );
                        }
                        sleep(Duration::from_secs(failure_delay)).await;
                        failure_delay = (failure_delay * 2).min(300);
                    }
                }
            }
        })
    }

    fn map_update(update: &Value) -> Option<Incoming> {
        if let Some(message) = update.get("message") {
            let text = message.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if text.is_empty() {
                return None;
            }
            return Some(Incoming::Message {
                chat_id: message.pointer("/chat/id").and_then(Value::as_i64)?.to_string(),
                sender_id: message
                    .pointer("/from/id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                text: text.to_string(),
                reply_to: message.pointer("/reply_to_message/message_id").and_then(Value::as_i64),
            });
        }
        if let Some(query) = update.get("callback_query") {
            return Some(Incoming::Callback {
                chat_id: query
                    .pointer("/message/chat/id")
                    .or_else(|| query.pointer("/from/id"))
                    .and_then(Value::as_i64)?
                    .to_string(),
                sender_id: query
                    .pointer("/from/id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
                    .unwrap_or_default(),
                data: query.get("data").and_then(Value::as_str).unwrap_or("").to_string(),
                handle: CallbackHandle::BotApi {
                    id: query.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                },
            });
        }
        None
    }
}

// ---------------------------------------------------------------------------
// MTProto 传输（grammers）
// ---------------------------------------------------------------------------

/// SessionData 本身不带 serde 派生，用可序列化的镜像结构落盘。
#[derive(Serialize, Deserialize)]
struct PersistedSessionData {
    home_dc: i32,
    dc_options: Vec<DcOption>,
    peers: Vec<PeerInfo>,
    updates_state: UpdatesState,
}

impl From<&SessionData> for PersistedSessionData {
    fn from(data: &SessionData) -> Self {
        Self {
            home_dc: data.home_dc,
            dc_options: data.dc_options.values().cloned().collect(),
            peers: data.peer_infos.values().cloned().collect(),
            updates_state: data.updates_state.clone(),
        }
    }
}

impl From<PersistedSessionData> for SessionData {
    fn from(persisted: PersistedSessionData) -> Self {
        let mut data = SessionData::default();
        data.home_dc = persisted.home_dc;
        for option in persisted.dc_options {
            data.dc_options.insert(option.id, option);
        }
        for peer in persisted.peers {
            data.peer_infos.insert(peer.id(), peer);
        }
        data.updates_state = persisted.updates_state;
        data
    }
}

/// 把 grammers 会话持久化为 JSON 文件（app 数据目录 telegram.session.json）。
struct JsonFileSession {
    path: PathBuf,
    data: Mutex<SessionData>,
}

impl JsonFileSession {
    fn load_or_create(path: PathBuf) -> Self {
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<PersistedSessionData>(&raw).ok())
            .map(SessionData::from)
            .unwrap_or_default();
        Self {
            path,
            data: Mutex::new(data),
        }
    }
    fn persist(&self, data: &SessionData) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_vec(&PersistedSessionData::from(data)) {
            let _ = fs::write(&self.path, raw);
        }
    }
}

#[derive(Debug)]
struct JsonSessionError(String);

impl std::fmt::Display for JsonSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for JsonSessionError {}

impl GrammersSession for JsonFileSession {
    type Error = JsonSessionError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        self.data
            .lock()
            .map(|data| data.home_dc)
            .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))
    }

    fn set_home_dc_id(&self, dc_id: i32) -> GrammersBoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut data = self
                .data
                .lock()
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))?;
            data.home_dc = dc_id;
            self.persist(&data);
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        self.data
            .lock()
            .map(|data| data.dc_options.get(&dc_id).cloned())
            .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> GrammersBoxFuture<'_, Result<(), Self::Error>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            let mut data = self
                .data
                .lock()
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))?;
            data.dc_options.insert(dc_option.id, dc_option);
            self.persist(&data);
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> GrammersBoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move {
            self.data
                .lock()
                .map(|data| data.peer_infos.get(&peer).cloned())
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))
        })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> GrammersBoxFuture<'_, Result<(), Self::Error>> {
        let peer = peer.clone();
        Box::pin(async move {
            let mut data = self
                .data
                .lock()
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))?;
            let entry = data.peer_infos.entry(peer.id()).or_insert_with(|| peer.clone());
            entry.extend_info(&peer);
            self.persist(&data);
            Ok(())
        })
    }

    fn updates_state(&self) -> GrammersBoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move {
            self.data
                .lock()
                .map(|data| data.updates_state.clone())
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))
        })
    }

    fn set_update_state(&self, update: UpdateState) -> GrammersBoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut data = self
                .data
                .lock()
                .map_err(|_| JsonSessionError("session 锁已损坏".to_string()))?;
            match update {
                UpdateState::All(updates_state) => data.updates_state = updates_state,
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => data.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    data.updates_state.channels.retain(|channel| channel.id != id);
                    data.updates_state
                        .channels
                        .push(grammers_client::session::types::ChannelState { id, pts });
                }
            }
            self.persist(&data);
            Ok(())
        })
    }
}

fn mtproto_session_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("telegram.session.json")
}

#[derive(Clone)]
pub(crate) struct MtprotoSender {
    client: GrammersClient,
    session: Arc<JsonFileSession>,
}

impl MtprotoSender {
    async fn peer(&self, chat_id: &str) -> Result<PeerRef, String> {
        let id: i64 = chat_id.trim().parse().map_err(|_| format!("无效的 Chat ID：{chat_id}"))?;
        let peer_id = PeerId::from_bot_api_dialog_id(id).ok_or_else(|| format!("无效的 Chat ID：{chat_id}"))?;
        // 已缓存的 peer 优先；bot 对互动过的对象可以用无 access_hash 的 ambient 引用兜底。
        let cached = self.session.peer_ref(peer_id).await.ok().flatten();
        Ok(cached.unwrap_or_else(|| peer_id.to_ambient_ref()))
    }

    fn markup(keyboard: Option<&Keyboard>, force_reply: bool) -> Option<ReplyMarkup> {
        if let Some(keyboard) = keyboard.filter(|rows| !rows.is_empty()) {
            let rows: Vec<Vec<Button>> = keyboard
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|(text, data)| Button::data(text.clone(), data.clone().into_bytes()))
                        .collect()
                })
                .collect();
            return Some(ReplyMarkup::from_buttons(&rows));
        }
        if force_reply {
            return Some(ReplyMarkup::force_reply().selective());
        }
        None
    }

    async fn get_me(&self) -> Result<String, String> {
        let me = self.client.get_me().await.map_err(|error| error.to_string())?;
        Ok(me.username().unwrap_or("").to_string())
    }

    async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        keyboard: Option<&Keyboard>,
        force_reply: bool,
    ) -> Result<i64, String> {
        let peer = self.peer(chat_id).await?;
        let mut message = InputMessage::new().html(text).link_preview(false);
        if let Some(markup) = Self::markup(keyboard, force_reply) {
            message = message.reply_markup(markup);
        }
        let sent = self
            .client
            .send_message(peer, message)
            .await
            .map_err(|error| format!("Telegram 发送消息失败：{error}"))?;
        Ok(i64::from(sent.id()))
    }

    async fn send_photo_bytes(&self, chat_id: &str, bytes: &[u8], caption: &str, filename: &str) -> Result<(), String> {
        let peer = self.peer(chat_id).await?;
        let mut reader = bytes;
        let uploaded = self
            .client
            .upload_stream(&mut reader, bytes.len(), filename.to_string())
            .await
            .map_err(|error| format!("Telegram 上传图片失败：{error}"))?;
        self.client
            .send_message(peer, InputMessage::new().html(caption).photo(uploaded))
            .await
            .map_err(|error| format!("Telegram 发送图片失败：{error}"))?;
        Ok(())
    }

    async fn send_photo_url(&self, chat_id: &str, url: &str, caption: &str) -> Result<(), String> {
        let peer = self.peer(chat_id).await?;
        self.client
            .send_message(peer, InputMessage::new().html(caption).photo_url(url))
            .await
            .map_err(|error| format!("Telegram 发送图片失败：{error}"))?;
        Ok(())
    }

    async fn set_commands(&self) -> Result<(), String> {
        let commands: Vec<tl::enums::BotCommand> = BOT_COMMANDS
            .iter()
            .map(|(command, description)| {
                tl::types::BotCommand {
                    command: command.to_string(),
                    description: description.to_string(),
                }
                .into()
            })
            .collect();
        self.client
            .invoke(&tl::functions::bots::SetBotCommands {
                scope: tl::types::BotCommandScopeDefault {}.into(),
                lang_code: String::new(),
                commands,
            })
            .await
            .map_err(|error| format!("注册命令菜单失败：{error}"))?;
        Ok(())
    }
}

async fn connect_mtproto(db_path: &Path, settings: &TelegramSettings) -> Result<ConnectedTransport, String> {
    let api_id: i32 = settings
        .api_id
        .parse()
        .map_err(|_| "API ID 必须是数字".to_string())?;
    let session = Arc::new(JsonFileSession::load_or_create(mtproto_session_path(db_path)));
    let mut params = ConnectionParams::default();
    let proxy = load_global_network_proxy(db_path).unwrap_or_default();
    let proxy = proxy.trim();
    if !proxy.is_empty() {
        let lowered = proxy.to_ascii_lowercase();
        if lowered.starts_with("socks5://") || lowered.starts_with("socks://") {
            params.proxy_url = Some(format!("socks5://{}", proxy.split("://").nth(1).unwrap_or_default()));
        } else {
            record_log("warning", "MTProto 模式仅支持 SOCKS5 代理，已忽略当前 HTTP 代理并尝试直连");
        }
    }
    let SenderPool { runner, updates, handle } =
        SenderPool::with_configuration(Arc::clone(&session), api_id, params);
    let client = GrammersClient::new(handle.clone());
    let pool_task = tauri::async_runtime::spawn(runner.run());
    let quit_handle = handle.clone();
    let shutdown: Box<dyn FnOnce() + Send> = Box::new(move || {
        let _ = quit_handle.thin.quit();
    });
    let connect_result: Result<(), String> = async {
        let authorized = client
            .is_authorized()
            .await
            .map_err(|error| format!("连接 Telegram 失败：{error}"))?;
        if !authorized {
            client
                .bot_sign_in(&settings.bot_token, &settings.api_hash)
                .await
                .map_err(|error| format!("Bot 登录失败：{error}"))?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = connect_result {
        shutdown();
        pool_task.abort();
        return Err(error);
    }
    let stream = client
        .stream_updates(updates, UpdatesConfiguration::default())
        .await
        .map_err(|error| format!("订阅 Telegram 更新失败：{error}"))?;
    let receiver_task = spawn_mtproto_receiver(stream);
    Ok(ConnectedTransport {
        sender: TransportSender::Mtproto(MtprotoSender { client, session }),
        receiver_task: Some(receiver_task),
        shutdown: Some(shutdown),
    })
}

fn spawn_mtproto_receiver(mut stream: grammers_client::client::UpdateStream) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            match stream.next().await {
                Ok(update) => match update {
                    GrammersUpdate::NewMessage(message) => {
                        if message.outgoing() {
                            continue;
                        }
                        let text = message.text().trim().to_string();
                        if text.is_empty() {
                            continue;
                        }
                        send_event(TelegramEvent::Incoming(Incoming::Message {
                            chat_id: message.peer_id().bot_api_dialog_id_unchecked().to_string(),
                            sender_id: message
                                .sender_id()
                                .map(|id| id.bot_api_dialog_id_unchecked().to_string())
                                .unwrap_or_default(),
                            text,
                            reply_to: message.reply_to_message_id().map(i64::from),
                        }));
                    }
                    GrammersUpdate::CallbackQuery(query) => {
                        let data = String::from_utf8_lossy(query.data()).to_string();
                        send_event(TelegramEvent::Incoming(Incoming::Callback {
                            chat_id: query.peer_id().bot_api_dialog_id_unchecked().to_string(),
                            sender_id: query.sender_id().bot_api_dialog_id_unchecked().to_string(),
                            data,
                            handle: CallbackHandle::Mtproto { query: Box::new(query) },
                        }));
                    }
                    _ => {}
                },
                Err(error) => {
                    send_event(TelegramEvent::TransportFailed {
                        message: format!("Telegram 更新流中断：{error}"),
                        fatal: false,
                    });
                    return;
                }
            }
        }
    })
}

async fn connect_transport(db_path: &Path, settings: &TelegramSettings) -> Result<ConnectedTransport, String> {
    if settings.mode == "mtproto" {
        return connect_mtproto(db_path, settings).await;
    }
    let sender = BotApiSender::new(db_path, settings)?;
    // getMe 作为连通性校验；轮询接收由会话建立后再启动。
    sender.get_me().await?;
    Ok(ConnectedTransport {
        sender: TransportSender::BotApi(sender),
        receiver_task: None,
        shutdown: None,
    })
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------

pub(crate) fn start(app: tauri::AppHandle, state: SharedState) {
    let (tx, rx) = mpsc::unbounded_channel();
    if EVENT_TX.set(tx).is_err() {
        return;
    }
    if let Ok(guard) = state.lock() {
        LAST_LOGGED_IN.store(guard.token.is_some(), Ordering::Relaxed);
    }
    tauri::async_runtime::spawn(telegram_loop(app, state, rx));
}

enum SessionExit {
    Restart,
    Shutdown,
    Failed(String),
}

async fn telegram_loop(app: tauri::AppHandle, state: SharedState, mut events: UnboundedReceiver<TelegramEvent>) {
    let Ok(db_path) = state.lock().map(|guard| guard.db_path.clone()) else {
        return;
    };
    let mut backoff = 5_u64;
    loop {
        let settings = effective_settings(&db_path);
        if !settings.enabled || !settings.configured() {
            set_status_info(false, "", None);
            match events.recv().await {
                None => return,
                Some(TelegramEvent::Restart) => {
                    backoff = 5;
                    continue;
                }
                Some(_) => continue,
            }
        }
        let mut connection = match connect_transport(&db_path, &settings).await {
            Ok(connection) => connection,
            Err(error) => {
                set_status_info(false, "", Some(error.clone()));
                record_log("warning", &format!("Telegram 连接失败，{backoff} 秒后重试：{error}"));
                if wait_for_restart_or(&mut events, backoff).await {
                    backoff = 5;
                } else {
                    backoff = (backoff * 2).min(600);
                }
                continue;
            }
        };
        backoff = 5;
        let username = connection.sender.get_me().await.unwrap_or_default();
        if matches!(connection.sender, TransportSender::BotApi(_)) {
            if let TransportSender::BotApi(sender) = &connection.sender {
                connection.receiver_task = Some(sender.spawn_receiver());
            }
        }
        set_status_info(true, &username, None);
        record_log(
            "info",
            &format!(
                "Telegram Bot 已连接{}（{} 模式）",
                if username.is_empty() { String::new() } else { format!("：@{username}") },
                if settings.mode == "mtproto" { "MTProto" } else { "Bot API" }
            ),
        );
        status(
            &app,
            "success",
            format!(
                "Telegram Bot 已连接{}",
                if username.is_empty() { String::new() } else { format!("：@{username}") }
            ),
        );
        if let Err(error) = connection.sender.set_commands().await {
            record_log("warning", &format!("注册 Telegram 命令菜单失败：{error}"));
        }
        let exit = run_session(&app, &state, &db_path, &settings, &connection.sender, &mut events).await;
        connection.shutdown().await;
        match exit {
            SessionExit::Shutdown => return,
            SessionExit::Restart => {
                set_status_info(false, "", None);
                continue;
            }
            SessionExit::Failed(error) => {
                set_status_info(false, "", Some(error.clone()));
                record_log("warning", &format!("Telegram 会话中断，{backoff} 秒后重连：{error}"));
                if wait_for_restart_or(&mut events, backoff).await {
                    backoff = 5;
                } else {
                    backoff = (backoff * 2).min(600);
                }
            }
        }
    }
}

/// 在退避等待期间继续响应 Restart 事件；返回 true 表示提前收到 Restart。
async fn wait_for_restart_or(events: &mut UnboundedReceiver<TelegramEvent>, seconds: u64) -> bool {
    let wait = sleep(Duration::from_secs(seconds));
    tokio::pin!(wait);
    loop {
        tokio::select! {
            _ = &mut wait => return false,
            event = events.recv() => match event {
                None => return false,
                Some(TelegramEvent::Restart) => return true,
                Some(_) => continue,
            }
        }
    }
}

#[derive(Default)]
struct RouterState {
    pending_tmdb: HashMap<String, (String, Instant)>,
}

async fn run_session(
    app: &tauri::AppHandle,
    state: &SharedState,
    db_path: &Path,
    settings: &TelegramSettings,
    sender: &TransportSender,
    events: &mut UnboundedReceiver<TelegramEvent>,
) -> SessionExit {
    let mut router = RouterState::default();
    loop {
        let Some(event) = events.recv().await else {
            return SessionExit::Shutdown;
        };
        match event {
            TelegramEvent::Restart => return SessionExit::Restart,
            TelegramEvent::TransportFailed { message, fatal } => {
                if fatal {
                    status(app, "warning", format!("Telegram Bot 已停止：{message}"));
                }
                return SessionExit::Failed(message);
            }
            TelegramEvent::JobUpdated { job_id, status } => {
                handle_job_updated(app, settings, sender, &job_id, &status).await;
            }
            TelegramEvent::AuthExpired { reason } => {
                handle_auth_expired(settings, sender, &reason).await;
            }
            TelegramEvent::EmbyNotify { category, text } => {
                if settings.notify_enabled(&category) {
                    if let Some(chat_id) = settings.primary_chat() {
                        if let Err(error) = sender.send_message(chat_id, &text, None, false).await {
                            record_log("warning", &format!("Telegram 通知发送失败：{error}"));
                        }
                    }
                }
            }
            TelegramEvent::SendText { chat_id, text, keyboard } => {
                if let Err(error) = sender.send_message(&chat_id, &text, keyboard.as_ref(), false).await {
                    record_log("warning", &format!("Telegram 消息发送失败：{error}"));
                }
            }
            TelegramEvent::SendPhoto { chat_id, bytes, caption } => {
                if let Err(error) = sender.send_photo_bytes(&chat_id, &bytes, &caption, "guangya-login.png").await {
                    record_log("warning", &format!("Telegram 图片发送失败：{error}"));
                }
            }
            TelegramEvent::Incoming(incoming) => {
                if let Err(error) = handle_incoming(app, state, db_path, settings, sender, &mut router, incoming).await {
                    record_log("warning", &format!("Telegram 消息处理失败：{error}"));
                }
            }
        }
    }
}

fn organizer_snapshot_value(app: &tauri::AppHandle) -> Value {
    let state = app.state::<organizer::OrganizerSharedState>();
    crate::organizer::get_organizer_state(state)
        .ok()
        .and_then(|snapshot| serde_json::to_value(snapshot).ok())
        .unwrap_or_else(|| json!({ "jobs": [], "mappings": [], "counts": {} }))
}

async fn handle_job_updated(
    app: &tauri::AppHandle,
    settings: &TelegramSettings,
    sender: &TransportSender,
    job_id: &str,
    job_status: &str,
) {
    let category = if matches!(job_status, "completed" | "completed_warning") { "organize" } else { "review" };
    if !settings.notify_enabled(category) {
        return;
    }
    let Some(chat_id) = settings.primary_chat() else {
        return;
    };
    let snapshot = organizer_snapshot_value(app);
    let jobs = snapshot.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(job) = jobs.iter().find(|job| value_str(job, "id") == job_id) else {
        return;
    };
    if value_str(job, "status") != job_status {
        return;
    }
    if category == "organize" {
        let mappings = snapshot.get("mappings").and_then(Value::as_array).cloned().unwrap_or_default();
        let mapping = mappings
            .iter()
            .find(|mapping| value_str(mapping, "id") == value_str(job, "mapping_id"));
        let text = format_organize_done(job, mapping);
        let poster = job
            .pointer("/preview/metadata/poster_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if !poster.is_empty() && sender.send_photo_url(chat_id, &poster, &text).await.is_ok() {
            return;
        }
        if let Err(error) = sender.send_message(chat_id, &text, None, false).await {
            record_log("warning", &format!("Telegram 通知发送失败：{error}"));
        }
        return;
    }
    let (text, keyboard) = format_review_needed(job);
    if let Err(error) = sender.send_message(chat_id, &text, Some(&keyboard), false).await {
        record_log("warning", &format!("Telegram 通知发送失败：{error}"));
    }
}

async fn handle_auth_expired(settings: &TelegramSettings, sender: &TransportSender, reason: &str) {
    if !settings.notify_enabled("auth") {
        return;
    }
    if AUTH_EXPIRED_NOTIFIED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(chat_id) = settings.primary_chat() else {
        return;
    };
    let mut lines = vec!["🔑 <b>光鸭登录已失效</b>".to_string()];
    if !reason.trim().is_empty() {
        lines.push(escape_html(reason.trim()));
    }
    lines.push("请重新扫码登录；也可以点击下方按钮，直接在 Telegram 中完成扫码。".to_string());
    let keyboard = vec![vec![("📷 获取扫码登录二维码".to_string(), "login".to_string())]];
    if let Err(error) = sender.send_message(chat_id, &lines.join("\n"), Some(&keyboard), false).await {
        record_log("warning", &format!("Telegram 通知发送失败：{error}"));
    }
}

// ---------------------------------------------------------------------------
// 命令路由
// ---------------------------------------------------------------------------

fn help_text(settings: &TelegramSettings, chat_id: &str) -> String {
    let mut lines = vec![
        format!("<b>光鸭云盘工作台</b> v{}（Windows 桌面端）", env!("CARGO_PKG_VERSION")),
        String::new(),
        "/status - 系统状态总览".to_string(),
        "/jobs - 最近整理任务与失败处理".to_string(),
        "/logs [数量] - 最新运行日志（默认 50 条）".to_string(),
        "/update - 检查更新".to_string(),
        "/login - 获取光鸭扫码登录二维码".to_string(),
        "/help - 本帮助".to_string(),
        String::new(),
        "重新整理命令：".to_string(),
        "<code>re &lt;任务ID&gt; tmdbid=12345 [tv|movie] [s=1] [e=2]</code>".to_string(),
        "例如：<code>re ab12cd34 tmdbid=94605 tv s=1</code>".to_string(),
        String::new(),
        format!("当前会话 Chat ID：<code>{}</code>", escape_html(chat_id)),
    ];
    if !settings.allows(chat_id, chat_id) {
        lines.push(String::new());
        lines.push("⚠️ 该会话尚未授权：请把上面的 Chat ID 填入「设置 → Telegram 通知」后再使用。".to_string());
    }
    lines.join("\n")
}

fn status_text(app: &tauri::AppHandle, state: &SharedState, settings: &TelegramSettings) -> String {
    let (logged_in, pending, active, paused, mapping_total, mapping_enabled, webdav_running, webdav_port, strm_port) =
        state
            .lock()
            .map(|guard| {
                (
                    guard.token.is_some(),
                    guard.queue.len() + guard.waiting_files.len() + guard.pending_cloud.len(),
                    guard.active_uploads,
                    guard.paused,
                    guard.mappings.len(),
                    guard.mappings.iter().filter(|mapping| mapping.enabled).count(),
                    guard.webdav_running,
                    guard.webdav_port,
                    guard.virtual_library.options().strm_port,
                )
            })
            .unwrap_or((false, 0, 0, false, 0, 0, false, 0, 0));
    let snapshot = organizer_snapshot_value(app);
    let counts = snapshot
        .get("counts")
        .and_then(Value::as_object)
        .map(|counts| {
            counts
                .iter()
                .map(|(key, value)| {
                    let label = match key.as_str() {
                        "recognizing" => "识别中",
                        "ready" => "待执行",
                        "needs_review" => "待人工处理",
                        "running" => "整理中",
                        "completed" => "已完成",
                        "completed_warning" => "完成（有提示）",
                        "failed" => "失败",
                        other => other,
                    };
                    format!("{label} {}", value.as_i64().unwrap_or_default())
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let organizer_mappings = snapshot
        .get("mappings")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let (connected, _, _) = status_info();
    [
        format!("<b>光鸭云盘工作台</b> v{}（Windows 桌面端）", env!("CARGO_PKG_VERSION")),
        format!(
            "登录状态：{}",
            if logged_in { "✅ 已登录" } else { "❌ 未登录（发送 /login 扫码登录）" }
        ),
        format!(
            "上传队列：等待 {pending} · 进行中 {active}{}",
            if paused { " · 已暂停" } else { "" }
        ),
        format!("备份任务：{mapping_total} 个（启用 {mapping_enabled} 个）"),
        format!("整理监控：{organizer_mappings} 个"),
        format!("整理任务：{}", if counts.is_empty() { "暂无记录" } else { &counts }),
        format!(
            "WebDAV：{}",
            if webdav_running { format!("运行中（端口 {webdav_port}）") } else { "未运行".to_string() }
        ),
        format!("Emby 网关：端口 {strm_port}"),
        format!(
            "Telegram：{}",
            if connected {
                format!("已连接（{}）", if settings.mode == "mtproto" { "MTProto" } else { "Bot API" })
            } else {
                "未连接".to_string()
            }
        ),
    ]
    .join("\n")
}

fn jobs_response(app: &tauri::AppHandle) -> (String, Option<Keyboard>) {
    let snapshot = organizer_snapshot_value(app);
    let jobs = snapshot.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
    if jobs.is_empty() {
        return ("暂无整理任务记录".to_string(), None);
    }
    let mut lines = vec!["<b>最近整理任务</b>".to_string()];
    for job in jobs.iter().take(10) {
        let job_status = value_str(job, "status");
        let icon = match job_status {
            "recognizing" => "🔍",
            "ready" => "🟡",
            "needs_review" => "⚠️",
            "running" => "🔄",
            "completed" => "✅",
            "completed_warning" => "☑️",
            "failed" => "❌",
            _ => "▫️",
        };
        let label = match job_status {
            "recognizing" => "识别中",
            "ready" => "待执行",
            "needs_review" => "待人工处理",
            "running" => "整理中",
            "completed" => "已完成",
            "completed_warning" => "完成（有提示）",
            "failed" => "失败",
            other => other,
        };
        lines.push(format!(
            "{icon} <code>{}</code> {} — {}",
            escape_html(&short_job_id(value_str(job, "id"))),
            escape_html(&describe_job_title(job)),
            escape_html(label)
        ));
    }
    let actionable: Vec<&Value> = jobs
        .iter()
        .take(10)
        .filter(|job| matches!(value_str(job, "status"), "needs_review" | "failed"))
        .take(5)
        .collect();
    let keyboard: Keyboard = actionable
        .iter()
        .map(|job| {
            let id = value_str(job, "id").to_string();
            vec![
                (format!("🔁 {}", short_job_id(&id)), format!("retry:{id}")),
                ("▶️ 整理".to_string(), format!("run:{id}")),
                ("🔎 TMDB".to_string(), format!("ask:{id}")),
            ]
        })
        .collect();
    if !keyboard.is_empty() {
        lines.push(String::new());
        lines.push("待处理任务可直接点击下方按钮操作：".to_string());
    }
    (lines.join("\n"), if keyboard.is_empty() { None } else { Some(keyboard) })
}

fn format_log_time(time_ms: u64) -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::from_unix_timestamp((time_ms / 1000) as i64)
        .map(|value| {
            let local = value.to_offset(offset);
            format!("{:02}:{:02}:{:02}", local.hour(), local.minute(), local.second())
        })
        .unwrap_or_else(|_| "--:--:--".to_string())
}

async fn send_logs(sender: &TransportSender, chat_id: &str, limit_token: Option<&str>) {
    let limit = limit_token
        .and_then(|token| token.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let entries = recent_logs(limit);
    if entries.is_empty() {
        let _ = sender.send_message(chat_id, "暂无日志记录", None, false).await;
        return;
    }
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| format!("{} [{}] {}", format_log_time(entry.time_ms), entry.level, entry.message))
        .collect();
    for chunk in chunk_lines(&lines, 3500).into_iter().take(5) {
        if let Err(error) = sender
            .send_message(chat_id, &format!("<pre>{}</pre>", escape_html(&chunk)), None, false)
            .await
        {
            record_log("warning", &format!("Telegram 日志发送失败：{error}"));
            return;
        }
    }
}

fn describe_overrides(input: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(id) = input.get("tmdb_id").and_then(Value::as_i64) {
        parts.push(format!("tmdbid={id}"));
    }
    if let Some(media_type) = input.get("media_type").and_then(Value::as_str) {
        parts.push(media_type.to_string());
    }
    if let Some(season) = input.get("season").and_then(Value::as_i64) {
        parts.push(format!("s={season}"));
    }
    if let Some(episode) = input.get("episode").and_then(Value::as_i64) {
        parts.push(format!("e={episode}"));
    }
    if let Some(title) = input.get("title").and_then(Value::as_str) {
        parts.push(format!("title={title}"));
    }
    if let Some(year) = input.get("year").and_then(Value::as_i64) {
        parts.push(format!("year={year}"));
    }
    if parts.is_empty() { String::new() } else { format!("（{}）", parts.join(" ")) }
}

/// 后台执行重新整理，结果经 job-updated 通知回推；错误单独发消息。
fn submit_job_run(app: &tauri::AppHandle, chat_id: &str, job_id: &str, input: Value, retry_only: bool) {
    let app = app.clone();
    let chat_id = chat_id.to_string();
    let job_id = job_id.to_string();
    tauri::async_runtime::spawn(async move {
        let parsed_input = serde_json::from_value::<OrganizerJobInput>(input).unwrap_or_default();
        let organizer_state = app.state::<organizer::OrganizerSharedState>();
        let result = if retry_only {
            retry_organizer_job(app.clone(), organizer_state, job_id.clone(), parsed_input).await
        } else {
            run_organizer_job(app.clone(), organizer_state, job_id.clone(), parsed_input).await
        };
        if let Err(error) = result {
            send_event(TelegramEvent::SendText {
                chat_id,
                text: format!(
                    "⚠️ {}提交失败：{}",
                    if retry_only { "重新识别" } else { "重新整理" },
                    escape_html(&error)
                ),
                keyboard: None,
            });
        }
    });
}

/// 生成登录二维码并后台轮询扫码结果，进度经事件回发到会话。
fn begin_login_flow(app: &tauri::AppHandle, chat_id: &str) {
    if LOGIN_FLOW_ACTIVE.swap(true, Ordering::SeqCst) {
        send_event(TelegramEvent::SendText {
            chat_id: chat_id.to_string(),
            text: "已有进行中的扫码登录，请先完成或等待其过期".to_string(),
            keyboard: None,
        });
        return;
    }
    let app = app.clone();
    let chat_id = chat_id.to_string();
    tauri::async_runtime::spawn(async move {
        let finished = login_flow_task(&app, &chat_id).await;
        LOGIN_FLOW_ACTIVE.store(false, Ordering::SeqCst);
        if let Err(error) = finished {
            send_event(TelegramEvent::SendText {
                chat_id,
                text: format!("扫码登录失败：{}", escape_html(&error)),
                keyboard: Some(vec![vec![("📷 重新获取二维码".to_string(), "login".to_string())]]),
            });
        }
    });
}

async fn login_flow_task(app: &tauri::AppHandle, chat_id: &str) -> Result<(), String> {
    let shared = app.state::<SharedState>();
    let data = crate::auth::start_device_login(shared.clone()).await?;
    let device_code = ["device_code", "deviceCode"]
        .iter()
        .find_map(|key| data.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let uri = [
        "short_uri_complete",
        "shortUriComplete",
        "verification_uri_complete",
        "verificationUriComplete",
        "verification_url",
        "verificationUrl",
        "verification_uri",
        "verificationUri",
    ]
    .iter()
    .find_map(|key| data.get(*key).and_then(Value::as_str))
    .unwrap_or("")
    .trim()
    .to_string();
    if device_code.is_empty() || uri.is_empty() {
        return Err("官方没有返回完整扫码信息，请稍后重试".to_string());
    }
    let expires_in = data.get("expires_in").and_then(Value::as_i64).unwrap_or(120).max(30) as u64;
    let user_code = ["user_code", "userCode"]
        .iter()
        .find_map(|key| data.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let png = render_qr_png(&uri)?;
    let caption = {
        let mut lines = vec![
            "📷 <b>光鸭扫码登录</b>".to_string(),
            "请使用光鸭 App 扫码并确认登录".to_string(),
        ];
        if !user_code.is_empty() {
            lines.push(format!("用户码：<code>{}</code>", escape_html(&user_code)));
        }
        lines.push(format!("二维码有效期约 {expires_in} 秒"));
        lines.join("\n")
    };
    send_event(TelegramEvent::SendPhoto {
        chat_id: chat_id.to_string(),
        bytes: png,
        caption,
    });
    let deadline = Instant::now() + Duration::from_secs(expires_in);
    let mut interval = data.get("interval").and_then(Value::as_i64).unwrap_or(3).max(2) as u64;
    loop {
        if Instant::now() >= deadline {
            send_event(TelegramEvent::SendText {
                chat_id: chat_id.to_string(),
                text: "二维码已过期，请重新获取".to_string(),
                keyboard: Some(vec![vec![("📷 重新获取二维码".to_string(), "login".to_string())]]),
            });
            return Ok(());
        }
        sleep(Duration::from_secs(interval)).await;
        let shared = app.state::<SharedState>();
        let result = crate::auth::poll_device_login(app.clone(), shared, device_code.clone()).await?;
        if result.get("authenticated").and_then(Value::as_bool) == Some(true) {
            AUTH_EXPIRED_NOTIFIED.store(false, Ordering::Relaxed);
            send_event(TelegramEvent::SendText {
                chat_id: chat_id.to_string(),
                text: "✅ 扫码登录成功，光鸭会话已恢复".to_string(),
                keyboard: None,
            });
            return Ok(());
        }
        if result.get("slow_down").and_then(Value::as_bool) == Some(true) {
            interval = (interval * 2).min(60);
        }
    }
}

fn render_qr_png(content: &str) -> Result<Vec<u8>, String> {
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|error| format!("生成二维码失败：{error}"))?;
    let image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(512, 512)
        .quiet_zone(true)
        .build();
    let mut bytes = Vec::new();
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|error| format!("二维码编码失败：{error}"))?;
    Ok(bytes)
}

async fn check_update_response(app: &tauri::AppHandle) -> (String, Option<Keyboard>) {
    match crate::updates::check_app_update_via_handle(app).await {
        Ok(Some(metadata)) => {
            let text = [
                "⬆️ <b>发现新版本</b>".to_string(),
                format!(
                    "当前 v{} → 最新 v{}",
                    escape_html(&metadata.current_version),
                    escape_html(&metadata.version)
                ),
                if metadata.notes.trim().is_empty() {
                    String::new()
                } else {
                    escape_html(metadata.notes.trim().chars().take(600).collect::<String>().as_str())
                },
            ]
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
            (
                text,
                Some(vec![vec![("⬇️ 下载并安装".to_string(), "update_install".to_string())]]),
            )
        }
        Ok(None) => (
            format!("当前已是最新版本：v{}", env!("CARGO_PKG_VERSION")),
            None,
        ),
        Err(error) => (format!("检查更新失败:{}", escape_html(&error)), None),
    }
}

async fn handle_incoming(
    app: &tauri::AppHandle,
    state: &SharedState,
    db_path: &Path,
    settings: &TelegramSettings,
    sender: &TransportSender,
    router: &mut RouterState,
    incoming: Incoming,
) -> Result<(), String> {
    router.pending_tmdb.retain(|_, (_, expires)| *expires > Instant::now());
    match incoming {
        Incoming::Message { chat_id, sender_id, text, reply_to } => {
            let command = text
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let command = command.split('@').next().unwrap_or("").to_string();
            if !settings.allows(&chat_id, &sender_id) {
                if command == "/start" || command == "/help" {
                    sender.send_message(&chat_id, &help_text(settings, &chat_id), None, false).await?;
                }
                return Ok(());
            }
            if let Some(reply_to) = reply_to {
                let key = format!("{chat_id}:{reply_to}");
                if let Some((job_id, _)) = router.pending_tmdb.remove(&key) {
                    return handle_tmdb_reply(app, sender, &chat_id, &job_id, &text).await;
                }
            }
            match parse_re_command(&text) {
                ReParse::Error(error) => {
                    sender.send_message(&chat_id, &escape_html(&error), None, false).await?;
                    return Ok(());
                }
                ReParse::Command { job_ref, input } => {
                    let snapshot = organizer_snapshot_value(app);
                    let jobs = snapshot.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
                    match find_job_by_ref(&jobs, &job_ref) {
                        Err(error) => {
                            sender.send_message(&chat_id, &escape_html(&error), None, false).await?;
                        }
                        Ok(job) => {
                            let job_id = value_str(job, "id").to_string();
                            sender
                                .send_message(
                                    &chat_id,
                                    &format!(
                                        "已提交重新整理：<code>{}</code> {}{}",
                                        escape_html(&short_job_id(&job_id)),
                                        escape_html(&describe_job_title(job)),
                                        escape_html(&describe_overrides(&input))
                                    ),
                                    None,
                                    false,
                                )
                                .await?;
                            submit_job_run(app, &chat_id, &job_id, input, false);
                        }
                    }
                    return Ok(());
                }
                ReParse::NotRe => {}
            }
            let argument = text.split_whitespace().nth(1).map(str::to_string);
            match command.as_str() {
                "/start" | "/help" => {
                    sender.send_message(&chat_id, &help_text(settings, &chat_id), None, false).await?;
                }
                "/status" => {
                    sender
                        .send_message(&chat_id, &status_text(app, state, settings), None, false)
                        .await?;
                }
                "/jobs" => {
                    let (text, keyboard) = jobs_response(app);
                    sender.send_message(&chat_id, &text, keyboard.as_ref(), false).await?;
                }
                "/logs" => {
                    send_logs(sender, &chat_id, argument.as_deref()).await;
                }
                "/update" => {
                    let (text, keyboard) = check_update_response(app).await;
                    sender.send_message(&chat_id, &text, keyboard.as_ref(), false).await?;
                }
                "/login" => {
                    begin_login_flow(app, &chat_id);
                }
                other => {
                    if other.starts_with('/') {
                        sender
                            .send_message(&chat_id, "未知命令，发送 /help 查看用法", None, false)
                            .await?;
                    }
                }
            }
            let _ = db_path;
            Ok(())
        }
        Incoming::Callback { chat_id, sender_id, data, handle } => {
            if !settings.allows(&chat_id, &sender_id) {
                sender.answer_callback(&handle, "该会话未授权").await;
                return Ok(());
            }
            let (action, argument) = data.split_once(':').unwrap_or((data.as_str(), ""));
            match action {
                "login" => {
                    sender.answer_callback(&handle, "正在获取二维码…").await;
                    begin_login_flow(app, &chat_id);
                }
                "update_install" => {
                    sender.answer_callback(&handle, "已提交安装").await;
                    let app = app.clone();
                    let chat_id = chat_id.clone();
                    tauri::async_runtime::spawn(async move {
                        match crate::updates::install_pending_update(&app).await {
                            Ok(()) => send_event(TelegramEvent::SendText {
                                chat_id,
                                text: "✅ 更新已下载安装，应用即将重启".to_string(),
                                keyboard: None,
                            }),
                            Err(error) => send_event(TelegramEvent::SendText {
                                chat_id,
                                text: format!("安装更新失败：{}", escape_html(&error)),
                                keyboard: None,
                            }),
                        }
                    });
                }
                "run" | "retry" | "ask" => {
                    let snapshot = organizer_snapshot_value(app);
                    let jobs = snapshot.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
                    match find_job_by_ref(&jobs, argument) {
                        Err(error) => {
                            sender.answer_callback(&handle, &error).await;
                        }
                        Ok(job) => {
                            let job_id = value_str(job, "id").to_string();
                            if action == "ask" {
                                sender.answer_callback(&handle, "").await;
                                let prompt = format!(
                                    "请<b>回复本条消息</b>填写 TMDB ID，可附加类型与季集号。\n例如：<code>94605 tv s=1</code>\n任务：<code>{}</code> {}",
                                    escape_html(&short_job_id(&job_id)),
                                    escape_html(&describe_job_title(job))
                                );
                                match sender.send_message(&chat_id, &prompt, None, true).await {
                                    Ok(message_id) => {
                                        router.pending_tmdb.insert(
                                            format!("{chat_id}:{message_id}"),
                                            (job_id, Instant::now() + Duration::from_secs(600)),
                                        );
                                    }
                                    Err(error) => record_log("warning", &format!("Telegram 消息发送失败：{error}")),
                                }
                            } else {
                                sender
                                    .answer_callback(&handle, if action == "retry" { "已提交重新识别" } else { "已提交重新整理" })
                                    .await;
                                submit_job_run(app, &chat_id, &job_id, json!({}), action == "retry");
                            }
                        }
                    }
                }
                _ => {
                    sender.answer_callback(&handle, "").await;
                }
            }
            Ok(())
        }
    }
}

async fn handle_tmdb_reply(
    app: &tauri::AppHandle,
    sender: &TransportSender,
    chat_id: &str,
    job_id: &str,
    text: &str,
) -> Result<(), String> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let input = match parse_override_tokens(&tokens) {
        Ok(input) => input,
        Err(error) => {
            sender.send_message(chat_id, &escape_html(&error), None, false).await?;
            return Ok(());
        }
    };
    if input.as_object().map(|map| map.is_empty()).unwrap_or(true) {
        sender
            .send_message(chat_id, "请提供 TMDB ID，例如：<code>94605 tv s=1</code>", None, false)
            .await?;
        return Ok(());
    }
    let snapshot = organizer_snapshot_value(app);
    let jobs = snapshot.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(job) = jobs.iter().find(|job| value_str(job, "id") == job_id) else {
        sender.send_message(chat_id, "任务已不存在，可能已被清理", None, false).await?;
        return Ok(());
    };
    sender
        .send_message(
            chat_id,
            &format!(
                "已提交重新整理：<code>{}</code>{}",
                escape_html(&short_job_id(value_str(job, "id"))),
                escape_html(&describe_overrides(&input))
            ),
            None,
            false,
        )
        .await?;
    submit_job_run(app, chat_id, job_id, input, false);
    Ok(())
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_command_parses_tmdb_type_and_season() {
        let ReParse::Command { job_ref, input } = parse_re_command("re ab12cd34 tmdbid=94605 tv s=1 e=3") else {
            panic!("应解析为命令");
        };
        assert_eq!(job_ref, "ab12cd34");
        assert_eq!(input, json!({ "tmdb_id": 94605, "media_type": "tv", "season": 1, "episode": 3 }));
    }

    #[test]
    fn re_command_supports_bare_number_and_snn() {
        let ReParse::Command { job_ref, input } = parse_re_command("/re ab12 94605 movie s02") else {
            panic!("应解析为命令");
        };
        assert_eq!(job_ref, "ab12");
        assert_eq!(input, json!({ "tmdb_id": 94605, "media_type": "movie", "season": 2 }));
    }

    #[test]
    fn re_command_rejects_invalid_input() {
        assert_eq!(parse_re_command("/status"), ReParse::NotRe);
        assert_eq!(parse_re_command("rearchive abc"), ReParse::NotRe);
        assert!(matches!(parse_re_command("re"), ReParse::Error(message) if message.contains("用法")));
        assert!(matches!(parse_re_command("re ab12 tmdbid=abc"), ReParse::Error(message) if message.contains("必须是数字")));
        assert!(matches!(parse_re_command("re ab12 foo=bar"), ReParse::Error(message) if message.contains("无法识别参数")));
    }

    #[test]
    fn override_tokens_support_chinese_type_and_title() {
        let input = parse_override_tokens(&["12345", "电影", "title=沙丘", "year=2021"]).expect("应解析成功");
        assert_eq!(input, json!({ "tmdb_id": 12345, "media_type": "movie", "title": "沙丘", "year": 2021 }));
    }

    #[test]
    fn job_ref_matching_detects_ambiguity() {
        let jobs = vec![
            json!({ "id": "ab12cd34-0000-4000-8000-000000000001" }),
            json!({ "id": "ab12ff00-0000-4000-8000-000000000002" }),
            json!({ "id": "ff340000-0000-4000-8000-000000000003" }),
        ];
        assert_eq!(
            value_str(find_job_by_ref(&jobs, "ff34").expect("应命中唯一任务"), "id"),
            "ff340000-0000-4000-8000-000000000003"
        );
        assert!(find_job_by_ref(&jobs, "ab12").unwrap_err().contains("匹配到 2 个"));
        assert!(find_job_by_ref(&jobs, "9999").unwrap_err().contains("没有找到"));
        assert!(find_job_by_ref(&jobs, "").unwrap_err().contains("请提供任务 ID"));
    }

    #[test]
    fn job_title_combines_metadata() {
        assert_eq!(
            describe_job_title(&json!({ "query_title": "凡人修仙传", "query_year": 2020, "media_type": "tv", "season": 1 })),
            "凡人修仙传 (2020) · 剧集 S01"
        );
        assert_eq!(
            describe_job_title(&json!({ "source_path": "/整理/来源/某电影.2021.mkv" })),
            "某电影.2021.mkv"
        );
    }

    #[test]
    fn organize_done_message_contains_target_and_short_id() {
        let text = format_organize_done(
            &json!({
                "id": "ab12cd34-0000-4000-8000-000000000001",
                "query_title": "沙丘",
                "query_year": 2021,
                "media_type": "movie",
                "source_path": "/watch/dune.mkv",
                "message": "云盘整理完成：转移 1 项，刮削 3 项",
                "preview": { "share_relative_path": "电影/沙丘 (2021)" },
            }),
            Some(&json!({ "target_path": "/媒体库" })),
        );
        assert!(text.contains("入库完成"));
        assert!(text.contains("沙丘 (2021)"));
        assert!(text.contains("/媒体库/电影/沙丘 (2021)"));
        assert!(text.contains("ab12cd34"));
    }

    #[test]
    fn review_message_contains_reason_keyboard_and_hint() {
        let (text, keyboard) = format_review_needed(&json!({
            "id": "ab12cd34-0000-4000-8000-000000000001",
            "status": "needs_review",
            "error_code": "tmdb_not_found",
            "source_path": "/watch/未知剧集.mkv",
            "message": "没有找到匹配",
        }));
        assert!(text.contains("识别待处理"));
        assert!(text.contains("TMDB 没有找到匹配条目"));
        assert!(text.contains("re ab12cd34 tmdbid=12345"));
        assert_eq!(keyboard.len(), 2);
        assert_eq!(keyboard[0][0].1, "retry:ab12cd34-0000-4000-8000-000000000001");
        assert_eq!(keyboard[1][0].1, "ask:ab12cd34-0000-4000-8000-000000000001");
        let (failed_text, _) = format_review_needed(&json!({ "id": "x", "status": "failed", "error_code": "transfer_failed", "source_path": "/a" }));
        assert!(failed_text.contains("整理失败"));
    }

    #[test]
    fn emby_webhook_body_supports_json_multipart_and_urlencoded() {
        let payload = json!({ "Event": "library.new", "Item": { "Name": "沙丘", "Type": "Movie", "ProductionYear": 2021 } });
        let raw = payload.to_string();
        assert_eq!(
            parse_emby_webhook_body("application/json", raw.as_bytes()).expect("应解析 JSON"),
            payload
        );
        let boundary = "----EmbyBoundaryX";
        let multipart = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"data\"\r\n\r\n{raw}\r\n--{boundary}--\r\n"
        );
        assert_eq!(
            parse_emby_webhook_body(
                &format!("multipart/form-data; boundary={boundary}"),
                multipart.as_bytes()
            )
            .expect("应解析 multipart"),
            payload
        );
        let encoded = format!(
            "data={}",
            utf8_percent_encode(&raw, NON_ALPHANUMERIC)
        );
        assert_eq!(
            parse_emby_webhook_body("application/x-www-form-urlencoded", encoded.as_bytes()).expect("应解析 urlencoded"),
            payload
        );
        assert!(parse_emby_webhook_body("application/json", b"not json").is_none());
    }

    #[test]
    fn emby_events_map_to_categories() {
        let created = describe_emby_event(&json!({
            "Event": "library.new",
            "Item": { "Name": "沙丘", "Type": "Movie", "ProductionYear": 2021, "Path": "/media/dune.mkv" },
        }))
        .expect("入库事件应有通知");
        assert_eq!(created.0, "emby_new");
        assert!(created.1.contains("Emby 入库"));
        assert!(created.1.contains("沙丘 (2021)"));

        let playback = describe_emby_event(&json!({
            "Event": "playback.start",
            "User": { "Name": "alice" },
            "Item": { "Name": "第一集", "Type": "Episode", "SeriesName": "凡人修仙传", "ParentIndexNumber": 1, "IndexNumber": 2, "RunTimeTicks": 12_000_000_000_i64 },
            "Session": { "DeviceName": "客厅电视", "Client": "Emby for Android" },
            "PlaybackInfo": { "PositionTicks": 6_000_000_000_i64 },
        }))
        .expect("播放事件应有通知");
        assert_eq!(playback.0, "emby_play");
        assert!(playback.1.contains("开始播放"));
        assert!(playback.1.contains("凡人修仙传 S01E02 第一集"));
        assert!(playback.1.contains("50%"));

        let login = describe_emby_event(&json!({
            "Event": "user.authenticated",
            "User": { "Name": "bob" },
            "Session": { "RemoteEndPoint": "192.168.1.2" },
        }))
        .expect("登录事件应有通知");
        assert_eq!(login.0, "emby_login");
        assert!(login.1.contains("用户登录"));

        assert!(describe_emby_event(&json!({ "Event": "system.serverrestartrequired" })).is_none());
        assert!(describe_emby_event(&json!({})).is_none());
    }

    #[test]
    fn chunking_respects_line_boundaries_and_truncates() {
        let chunks = chunk_lines(
            &["a".repeat(30), "b".repeat(30), "c".repeat(120)],
            64,
        );
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], format!("{}\n{}", "a".repeat(30), "b".repeat(30)));
        assert_eq!(chunks[1].chars().count(), 64);
        assert!(chunks[1].ends_with('…'));
    }

    #[test]
    fn chat_ids_filter_invalid_tokens() {
        assert_eq!(
            parse_chat_ids("123456, -1001234567890 abc; 42"),
            vec!["123456".to_string(), "-1001234567890".to_string(), "42".to_string()]
        );
        assert!(parse_chat_ids("").is_empty());
    }

    #[test]
    fn api_base_url_normalization() {
        assert_eq!(
            normalize_telegram_api_base_url("https://tg.example.com/").expect("应通过"),
            "https://tg.example.com"
        );
        assert_eq!(normalize_telegram_api_base_url("").expect("空值应通过"), "");
        assert!(normalize_telegram_api_base_url("ftp://x").is_err());
        assert!(normalize_telegram_api_base_url("https://user:pass@tg.example.com").is_err());
    }

    #[test]
    fn html_escaping() {
        assert_eq!(escape_html("<b>&\"x\""), "&lt;b&gt;&amp;\"x\"");
    }

    #[test]
    fn log_buffer_keeps_latest_entries() {
        for index in 0..600 {
            record_log("info", &format!("line-{index}"));
        }
        let entries = recent_logs(10);
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[9].message, "line-599");
        assert_eq!(entries[0].message, "line-590");
    }
}
