#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hmac::{Hmac, Mac};
use md5::Md5;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use opendal::{services::Oss, Operator};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::{sleep, timeout, Duration, Instant},
};
use uuid::Uuid;

const API_BASE: &str = "https://api.guangyapan.com";
const ACCOUNT_BASE: &str = "https://account.guangyapan.com";
const OAUTH_CLIENT_ID: &str = "aMe-8VSlkrbQXpUR";
const AUTH_URL: &str = "https://www.guangyapan.com/#/";
const MAX_UPLOADS: usize = 2;
const FILE_STABILITY_WAIT_MS: u64 = 1_200;
const FILE_BUSY_RETRY_SECS: u64 = 3;
const POLL_INTERVAL_SECS: u64 = 5;
const API_CONNECT_TIMEOUT_SECS: u64 = 15;
const API_REQUEST_TIMEOUT_SECS: u64 = 120;
const OSS_REQUEST_TIMEOUT_SECS: u64 = 600;
const CLOUD_CONFIRM_TIMEOUT_SECS: u64 = 600;
const AUTO_SHARE_QUIET_SECS: i64 = 30;
const TOKEN_REFRESH_INTERVAL_SECS: u64 = 20 * 60;
const DEFAULT_MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "heif", "avif", "tif", "tiff",
    "raw", "cr2", "nef", "arw", "dng", "mp4", "mov", "mkv", "avi", "wmv", "flv", "webm", "m4v",
    "ts", "mts", "m2ts", "3gp", "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff",
];
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "heif", "avif", "tif", "tiff",
    "raw", "cr2", "nef", "arw", "dng",
];
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "avi", "wmv", "flv", "webm", "m4v", "ts", "mts", "m2ts", "3gp",
];
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "lrc"];
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff",
];
type SharedState = Arc<Mutex<RuntimeState>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Mapping {
    id: String,
    local_path: String,
    remote_path: String,
    #[serde(default)]
    remote_parent_id: String,
    enabled: bool,
    #[serde(default = "default_source_policy")]
    source_policy: String,
    #[serde(default)]
    archive_path: Option<String>,
    #[serde(default = "default_true")]
    scan_existing: bool,
    #[serde(default = "default_sync_types")]
    sync_types: Vec<String>,
    #[serde(default)]
    watch_error: Option<String>,
    #[serde(default = "default_monitor_mode")]
    monitor_mode: String,
    #[serde(default)]
    auto_share: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedShare {
    id: String,
    label: String,
    url: String,
    created_at: u64,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    mappings: Vec<Mapping>,
    #[serde(default)]
    saved_shares: Vec<SavedShare>,
}
#[derive(Debug, Clone, Serialize)]
struct Snapshot {
    logged_in: bool,
    paused: bool,
    pending: usize,
    active_uploads: usize,
    mappings: Vec<Mapping>,
    saved_shares: Vec<SavedShare>,
    hdhive: HdhivePublicConfig,
    auto_share_receipts: Vec<AutoShareReceipt>,
}
#[derive(Debug, Clone)]
struct UploadItem {
    mapping_id: String,
    file_path: PathBuf,
    remote_parent_id: String,
    remote_dir: String,
    relative_path: String,
    change_kind: String,
    size: u64,
    modified_ms: u128,
}
#[derive(Debug, Clone)]
struct FsEvent {
    mapping_id: String,
    path: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified_ms: u128,
}

struct RuntimeState {
    token: Option<String>,
    refresh_token: Option<String>,
    config_path: PathBuf,
    db_path: PathBuf,
    mappings: Vec<Mapping>,
    saved_shares: Vec<SavedShare>,
    queue: VecDeque<UploadItem>,
    waiting_files: HashMap<String, UploadItem>,
    history: HashMap<String, Stamp>,
    inflight: HashMap<String, Stamp>,
    inflight_items: HashMap<String, UploadItem>,
    remote_cache: HashMap<String, String>,
    watchers: HashMap<String, RecommendedWatcher>,
    event_tx: UnboundedSender<FsEvent>,
    paused: bool,
    active_uploads: usize,
    device_id: String,
    hdhive_base_url: String,
    hdhive_secret: String,
    hdhive_instance_id: String,
    auto_share_processing: HashSet<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HdhivePublicConfig {
    configured: bool,
    base_url: String,
    instance_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct AutoShareReceipt {
    event_id: String,
    mapping_id: String,
    target_key: String,
    share_url: Option<String>,
    status: String,
    action: Option<String>,
    message: Option<String>,
    resource_url: Option<String>,
    notification_status: Option<String>,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct AutoShareTarget {
    key: String,
    target_type: String,
    title: String,
    relative_path: String,
}

#[derive(Debug, Clone)]
struct PendingAutoShare {
    mapping_id: String,
    target_key: String,
    target_type: String,
    title: String,
    remote_target_id: String,
    added: HashSet<String>,
    changed: HashSet<String>,
    event_id: String,
    retry_count: i64,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<Value>,
}
#[derive(Debug, Deserialize)]
struct UploadCredentials {
    #[serde(rename = "accessKeyID")]
    access_key_id: String,
    #[serde(rename = "secretAccessKey", alias = "accessKeySecret")]
    secret_access_key: String,
    #[serde(rename = "sessionToken")]
    session_token: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadToken {
    task_id: String,
    object_path: Option<String>,
    bucket_name: Option<String>,
    end_point: Option<String>,
    creds: Option<UploadCredentials>,
}

#[derive(Debug, Clone)]
struct AuthSession {
    access_token: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Clone)]
struct UploadOutcome {
    task_id: String,
    remote_file_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameRequest {
    file_id: String,
    current_name: String,
    new_name: String,
}

fn emit(app: &tauri::AppHandle, payload: impl Serialize + Clone) {
    let _ = app.emit("sync-event", payload);
}
fn status(app: &tauri::AppHandle, level: &str, message: impl Into<String>) {
    emit(
        app,
        json!({ "type": "status", "level": level, "message": message.into() }),
    );
}
fn snapshot(state: &RuntimeState) -> Snapshot {
    Snapshot {
        logged_in: state.token.is_some(),
        paused: state.paused,
        pending: state.queue.len() + state.waiting_files.len(),
        active_uploads: state.active_uploads,
        mappings: state.mappings.clone(),
        saved_shares: state.saved_shares.clone(),
        hdhive: HdhivePublicConfig {
            configured: !state.hdhive_base_url.is_empty() && !state.hdhive_secret.is_empty(),
            base_url: state.hdhive_base_url.clone(),
            instance_id: state.hdhive_instance_id.clone(),
        },
        auto_share_receipts: load_auto_share_receipts(&state.db_path).unwrap_or_default(),
    }
}
fn default_source_policy() -> String {
    "keep".to_string()
}
fn default_true() -> bool {
    true
}
fn default_sync_types() -> Vec<String> {
    DEFAULT_MEDIA_EXTENSIONS
        .iter()
        .into_iter()
        .map(|value| (*value).to_string())
        .collect()
}
fn default_monitor_mode() -> String {
    "native".to_string()
}
fn normalize_oss_endpoint(endpoint: &str, bucket: &str) -> String {
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
fn normalize_oss_endpoint_url(endpoint: &str, bucket: &str) -> String {
    let scheme = if endpoint.trim().starts_with("http://") {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{}", normalize_oss_endpoint(endpoint, bucket))
}
fn oss_part_size(size: u64) -> usize {
    if size <= 100 * 1024 * 1024 {
        1024 * 1024
    } else if size <= 1024 * 1024 * 1024 {
        2 * 1024 * 1024
    } else if size <= 10 * 1024 * 1024 * 1024 {
        4 * 1024 * 1024
    } else {
        8 * 1024 * 1024
    }
}
fn normalize_monitor_mode(value: &str) -> String {
    if value.eq_ignore_ascii_case("polling") {
        "polling".to_string()
    } else {
        default_monitor_mode()
    }
}
fn item_key(mapping_id: &str, path: &Path) -> String {
    format!("{mapping_id}::{}", path.to_string_lossy())
}
fn stamp_matches(item: &UploadItem, stamp: &Stamp) -> bool {
    stamp.size == item.size && stamp.modified_ms == item.modified_ms
}
fn upload_already_scheduled(
    history: &HashMap<String, Stamp>,
    inflight: &HashMap<String, Stamp>,
    queue: &VecDeque<UploadItem>,
    waiting_files: &HashMap<String, UploadItem>,
    item: &UploadItem,
) -> bool {
    let key = item_key(&item.mapping_id, &item.file_path);
    history
        .get(&key)
        .is_some_and(|stamp| stamp_matches(item, stamp))
        || inflight
            .get(&key)
            .is_some_and(|stamp| stamp_matches(item, stamp))
        || queue.iter().any(|queued| {
            item_key(&queued.mapping_id, &queued.file_path) == key
                && queued.size == item.size
                && queued.modified_ms == item.modified_ms
        })
        || waiting_files.contains_key(&key)
}
fn normalize_sync_types(values: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let value = value.trim().trim_start_matches('.').to_lowercase();
        let preset = match value.as_str() {
            "image" => Some(IMAGE_EXTENSIONS),
            "video" => Some(VIDEO_EXTENSIONS),
            "subtitle" => Some(SUBTITLE_EXTENSIONS),
            "audio" => Some(AUDIO_EXTENSIONS),
            _ => None,
        };
        if let Some(values) = preset {
            for extension in values {
                let extension = (*extension).to_string();
                if !result.contains(&extension) {
                    result.push(extension);
                }
            }
        } else if !value.is_empty()
            && value.len() <= 16
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
            && !result.contains(&value)
        {
            result.push(value);
        }
    }
    if result.is_empty() {
        default_sync_types()
    } else {
        result
    }
}
fn file_extension(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    extension
}
fn should_sync(path: &Path, sync_types: &[String]) -> bool {
    let extension = file_extension(path);
    !extension.is_empty()
        && normalize_sync_types(sync_types)
            .iter()
            .any(|value| value == &extension)
}
fn emit_state(app: &tauri::AppHandle, state: &SharedState) {
    if let Ok(guard) = state.lock() {
        emit(app, json!({ "type": "state", "state": snapshot(&guard) }));
    }
}
fn normalize_remote_path(input: &str) -> String {
    input
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}
fn ignored(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    name.starts_with("~$")
        || [
            ".tmp",
            ".part",
            ".crdownload",
            ".download",
            ".swp",
            ".ds_store",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}
fn modified_ms(meta: &fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0)
}
fn load_config(path: &Path) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok())
        .unwrap_or_default()
}
fn save_config(state: &RuntimeState) {
    if let Some(parent) = state.config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let payload = json!({ "mappings": state.mappings, "saved_shares": state.saved_shares });
    let _ = fs::write(
        &state.config_path,
        serde_json::to_vec_pretty(&payload).unwrap_or_default(),
    );
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建本地数据目录失败：{e}"))?;
    }
    let connection = Connection::open(path).map_err(|e| format!("打开 SQLite 失败：{e}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 SQLite 等待时间失败：{e}"))?;
    Ok(connection)
}

fn init_database(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS auth_session (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               access_token TEXT,
               refresh_token TEXT,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS uploaded_files (
               mapping_id TEXT NOT NULL,
               file_path TEXT NOT NULL,
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               task_id TEXT,
               remote_file_id TEXT,
               uploaded_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, file_path)
             );
             CREATE TABLE IF NOT EXISTS app_state (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS auto_share_targets (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               target_type TEXT NOT NULL,
               remote_target_id TEXT NOT NULL,
               title TEXT NOT NULL,
               share_id TEXT NOT NULL,
               share_url TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key)
             );
             CREATE TABLE IF NOT EXISTS auto_share_events (
               event_id TEXT PRIMARY KEY,
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               share_url TEXT,
               status TEXT NOT NULL,
               action TEXT,
               message TEXT,
               resource_url TEXT,
               notification_status TEXT,
               payload TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS auto_share_pending (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               target_type TEXT NOT NULL,
               title TEXT NOT NULL,
               remote_target_id TEXT NOT NULL,
               added_paths TEXT NOT NULL,
               changed_paths TEXT NOT NULL,
               event_id TEXT NOT NULL,
               retry_count INTEGER NOT NULL DEFAULT 0,
               due_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key)
             );
             CREATE TABLE IF NOT EXISTS auto_share_failures (
               mapping_id TEXT NOT NULL,
               target_key TEXT NOT NULL,
               relative_path TEXT NOT NULL,
               error TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, target_key, relative_path)
             );",
        )
        .map_err(|e| format!("初始化 SQLite 失败：{e}"))?;
    let _ = connection.execute(
        "ALTER TABLE auto_share_events ADD COLUMN notification_status TEXT",
        [],
    );
    Ok(())
}

fn load_auth_session(path: &Path) -> Result<AuthSession, String> {
    let connection = open_database(path)?;
    connection
        .query_row(
            "SELECT access_token, refresh_token FROM auth_session WHERE id = 1",
            [],
            |row| {
                Ok(AuthSession {
                    access_token: row.get(0)?,
                    refresh_token: row.get(1)?,
                })
            },
        )
        .optional()
        .map(|value| {
            value.unwrap_or(AuthSession {
                access_token: None,
                refresh_token: None,
            })
        })
        .map_err(|e| format!("读取登录状态失败：{e}"))
}

fn save_auth_session(
    path: &Path,
    access_token: Option<&str>,
    refresh_token: Option<&str>,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO auth_session (id, access_token, refresh_token, updated_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               access_token = COALESCE(excluded.access_token, auth_session.access_token),
               refresh_token = COALESCE(excluded.refresh_token, auth_session.refresh_token),
               updated_at = excluded.updated_at",
            params![access_token, refresh_token, unix_timestamp()],
        )
        .map_err(|e| format!("保存登录状态失败：{e}"))?;
    Ok(())
}

fn clear_persisted_access_token(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "UPDATE auth_session SET access_token = NULL, updated_at = ?1 WHERE id = 1",
            params![unix_timestamp()],
        )
        .map_err(|e| format!("清理过期登录状态失败：{e}"))?;
    Ok(())
}

fn load_upload_history(path: &Path) -> Result<HashMap<String, Stamp>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare("SELECT mapping_id, file_path, size, modified_ms FROM uploaded_files")
        .map_err(|e| format!("读取上传记录失败：{e}"))?;
    let rows = statement
        .query_map([], |row| {
            let mapping_id: String = row.get(0)?;
            let file_path: String = row.get(1)?;
            let size: u64 = row.get(2)?;
            let modified_raw: String = row.get(3)?;
            Ok((mapping_id, file_path, size, modified_raw))
        })
        .map_err(|e| format!("查询上传记录失败：{e}"))?;
    let mut history = HashMap::new();
    for row in rows {
        let (mapping_id, file_path, size, modified_raw) =
            row.map_err(|e| format!("解析上传记录失败：{e}"))?;
        let modified_ms = modified_raw.parse::<u128>().unwrap_or(0);
        history.insert(
            item_key(&mapping_id, Path::new(&file_path)),
            Stamp { size, modified_ms },
        );
    }
    Ok(history)
}

fn save_upload_history(
    path: &Path,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO uploaded_files
               (mapping_id, file_path, size, modified_ms, task_id, remote_file_id, uploaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(mapping_id, file_path) DO UPDATE SET
               size = excluded.size,
               modified_ms = excluded.modified_ms,
               task_id = excluded.task_id,
               remote_file_id = excluded.remote_file_id,
               uploaded_at = excluded.uploaded_at",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.size,
                item.modified_ms.to_string(),
                outcome.task_id,
                outcome.remote_file_id,
                unix_timestamp()
            ],
        )
        .map_err(|e| format!("保存上传记录失败：{e}"))?;
    Ok(())
}

fn remember_uploaded_item(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let database = state.lock().map_err(|e| e.to_string())?.db_path.clone();
    save_upload_history(&database, item, outcome)?;
    state.lock().map_err(|e| e.to_string())?.history.insert(
        item_key(&item.mapping_id, &item.file_path),
        Stamp {
            size: item.size,
            modified_ms: item.modified_ms,
        },
    );
    Ok(())
}

fn remove_mapping_history(path: &Path, mapping_id: &str) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM uploaded_files WHERE mapping_id = ?1",
            params![mapping_id],
        )
        .map_err(|e| format!("删除任务上传记录失败：{e}"))?;
    Ok(())
}

fn load_or_create_device_id(path: &Path) -> Result<String, String> {
    let connection = open_database(path)?;
    let current = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'device_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("读取设备 ID 失败：{e}"))?;
    if let Some(value) = current.filter(|value| !value.trim().is_empty()) {
        return Ok(value);
    }
    let value = Uuid::new_v4().to_string();
    connection
        .execute(
            "INSERT INTO app_state (key, value, updated_at) VALUES ('device_id', ?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![value, unix_timestamp()],
        )
        .map_err(|e| format!("保存设备 ID 失败：{e}"))?;
    Ok(value)
}

fn load_app_state(path: &Path, key: &str) -> Result<Option<String>, String> {
    open_database(path)?
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("读取本地设置失败：{error}"))
}

fn save_app_state(path: &Path, key: &str, value: &str) -> Result<(), String> {
    open_database(path)?
        .execute(
            "INSERT INTO app_state (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
            params![key, value, unix_timestamp()],
        )
        .map_err(|error| format!("保存本地设置失败：{error}"))?;
    Ok(())
}

fn load_auto_share_receipts(path: &Path) -> Result<Vec<AutoShareReceipt>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT event_id, mapping_id, target_key, share_url, status, action, message, resource_url, notification_status, updated_at
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
                message: row.get(6)?,
                resource_url: row.get(7)?,
                notification_status: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })
        .map_err(|error| format!("读取自动分享回执失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析自动分享回执失败：{error}"))?;
    Ok(rows)
}

fn auto_share_target(item: &UploadItem) -> Option<AutoShareTarget> {
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

fn target_has_work(state: &RuntimeState, mapping_id: &str, target_key: &str) -> bool {
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

fn save_auto_share_event(
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
               action=excluded.action, message=excluded.message, resource_url=excluded.resource_url,
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

fn record_auto_share_failure(path: &Path, item: &UploadItem, message: &str) -> Result<(), String> {
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

fn clear_auto_share_failure(path: &Path, item: &UploadItem) -> Result<(), String> {
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

fn auth_hook_script() -> &'static str {
    r#"(() => {
      if (window.__guangyaAuthHook) return;
      window.__guangyaAuthHook = true;
      const send = (value) => {
        if (typeof value !== 'string' || !value.startsWith('Bearer ')) return;
        const token = value.slice(7).trim();
        if (!token) return;
        const invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
        if (invoke) invoke('capture_token', { token }).catch(() => {});
        else setTimeout(() => send(value), 500);
      };
      const fetch0 = window.fetch;
      window.fetch = function(input, init) {
        try { const headers = new Headers((init && init.headers) || (input && input.headers) || {}); send(headers.get('authorization') || headers.get('Authorization')); } catch (_) {}
        return fetch0.apply(this, arguments);
      };
      const open0 = XMLHttpRequest.prototype.open;
      const set0 = XMLHttpRequest.prototype.setRequestHeader;
      XMLHttpRequest.prototype.open = function() { this.__gyHeaders = {}; return open0.apply(this, arguments); };
      XMLHttpRequest.prototype.setRequestHeader = function(key, value) { if (key && key.toLowerCase() === 'authorization') send(value); return set0.apply(this, arguments); };
    })();"#
}

async fn api_post(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: Value,
    allowed: &[i64],
) -> Result<ApiResponse, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("创建网络客户端失败：{e}"))?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| e.to_string())?,
    );
    headers.insert("dt", HeaderValue::from_static("4"));
    headers.insert(
        "did",
        HeaderValue::from_str(device_id).map_err(|e| e.to_string())?,
    );
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = Uuid::new_v4().simple().to_string()[..16].to_string();
    headers.insert(
        "traceparent",
        HeaderValue::from_str(&format!("00-{trace_id}-{span_id}-01")).map_err(|e| e.to_string())?,
    );
    let response = client
        .post(format!("{API_BASE}{endpoint}"))
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let http_status = response.status();
    let raw = response.text().await.map_err(|e| e.to_string())?;
    let payload = parse_api_response(&raw, http_status.as_u16(), endpoint)?;
    if http_status.as_u16() == 401 || payload.code == 117 {
        return Err("登录态已失效，请重新打开官方登录页".into());
    }
    if !http_status.is_success() || (payload.code != 0 && !allowed.contains(&payload.code)) {
        let message = if payload.msg.is_empty() {
            format!("光鸭接口失败：HTTP {http_status}/{}", payload.code)
        } else {
            payload.msg.clone()
        };
        if endpoint == "/userres/v1/share_file" {
            let request_preview =
                serde_json::to_string(&body).unwrap_or_else(|_| "<无法序列化分享参数>".to_string());
            return Err(format!(
                "{message}（HTTP {http_status}，业务码 {}；请求参数：{request_preview}）",
                payload.code
            ));
        }
        return Err(message);
    }
    Ok(payload)
}

fn parse_api_response(raw: &str, status: u16, endpoint: &str) -> Result<ApiResponse, String> {
    let trimmed = raw.trim().trim_start_matches('\u{feff}');
    if trimmed.is_empty() && (200..300).contains(&status) {
        return Ok(ApiResponse {
            code: 0,
            msg: String::new(),
            data: Some(json!({})),
        });
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|error| {
        let preview = trimmed.chars().take(240).collect::<String>();
        format!("光鸭接口 {endpoint} 返回了非 JSON 响应（HTTP {status}）：{preview}（{error}）")
    })?;
    let code = value
        .get("code")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0);
    let msg = value
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(ApiResponse {
        code,
        msg,
        data: value.get("data").cloned(),
    })
}

fn parse_guangya_share_link(value: &str) -> Result<(String, String), String> {
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

async fn account_post(endpoint: &str, body: Value) -> Result<(u16, Value), String> {
    let response = reqwest::Client::new()
        .post(format!("{ACCOUNT_BASE}{endpoint}"))
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let raw = response.text().await.map_err(|e| e.to_string())?;
    let payload = if raw.trim().is_empty() && (200..300).contains(&status) {
        json!({})
    } else {
        serde_json::from_str(raw.trim().trim_start_matches('\u{feff}')).map_err(|error| {
            format!("账号接口 {endpoint} 返回了非 JSON 响应（HTTP {status}）：{error}")
        })?
    };
    Ok((status, payload))
}

async fn account_get(token: &str, endpoint: &str) -> Result<Value, String> {
    let response = reqwest::Client::new()
        .get(format!("{ACCOUNT_BASE}{endpoint}"))
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
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
    serde_json::from_str(raw.trim().trim_start_matches('\u{feff}'))
        .map_err(|error| format!("账号接口 {endpoint} 返回了非 JSON 响应：{error}"))
}

async fn find_remote_folder(
    token: &str,
    device_id: &str,
    parent_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    for page in 0..100 {
        let result = api_post(token, device_id, "/userres/v1/file/get_file_list", json!({ "page": page, "pageSize": 100, "parentId": parent_id, "resType": 2, "needSubFolderStat": true }), &[]).await?;
        let data = result.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(found) = list.iter().find(|item| {
            item.get("resType").and_then(Value::as_i64) == Some(2)
                && item.get("fileName").and_then(Value::as_str) == Some(name)
        }) {
            return Ok(found
                .get("fileId")
                .and_then(Value::as_str)
                .map(str::to_owned));
        }
        let total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        if list.is_empty() || ((page + 1) * 100) as u64 >= total {
            break;
        }
    }
    Ok(None)
}

async fn ensure_remote_path(
    state: &SharedState,
    token: &str,
    device_id: &str,
    base_parent_id: &str,
    remote_path: &str,
) -> Result<String, String> {
    let normalized = normalize_remote_path(remote_path);
    if normalized.is_empty() {
        return Ok(base_parent_id.to_string());
    }
    let mut parent = base_parent_id.to_string();
    let mut prefix = String::new();
    for part in normalized.split('/') {
        prefix = if prefix.is_empty() {
            part.to_owned()
        } else {
            format!("{prefix}/{part}")
        };
        let cache_key = format!("{}::{prefix}", base_parent_id);
        if let Some(cached) = state
            .lock()
            .map_err(|e| e.to_string())?
            .remote_cache
            .get(&cache_key)
            .cloned()
        {
            parent = cached;
            continue;
        }
        let result = api_post(
            token,
            device_id,
            "/userres/v1/file/create_dir",
            json!({ "parentId": parent, "dirName": part, "failIfNameExist": true }),
            &[159],
        )
        .await?;
        let mut file_id = result
            .data
            .as_ref()
            .and_then(|data| data.get("fileId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if file_id.is_none() && result.code == 159 {
            file_id = find_remote_folder(token, device_id, &parent, part).await?;
        }
        let file_id = file_id.ok_or_else(|| format!("无法创建或定位远程目录：{prefix}"))?;
        state
            .lock()
            .map_err(|e| e.to_string())?
            .remote_cache
            .insert(cache_key, file_id.clone());
        parent = file_id;
    }
    Ok(parent)
}

fn load_due_auto_shares(path: &Path) -> Result<Vec<PendingAutoShare>, String> {
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

fn reschedule_auto_share(
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

fn persist_pending_auto_share(path: &Path, pending: &PendingAutoShare) -> Result<(), String> {
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

fn delete_pending_auto_share(
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

fn share_id_from_url(value: &str) -> String {
    value
        .split("/s/")
        .nth(1)
        .unwrap_or_default()
        .split(['?', '#', '/'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn share_id_for_hdhive(data: &Value, share_url: &str) -> String {
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

const DEFAULT_SHARE_TEMPLATE: &str =
    "光鸭云盘用户给你分享了{{filename}}，点击链接或复制整段内容，打开「光鸭APP」即可获取。\n链接：{{link}}";

fn share_file_payload(file_ids: &[String], title: &str) -> Value {
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
        "shareType": 0,
        "code": "",
        "autoFillCode": false,
        // 光鸭网页版的普通分享会同时提交下载限制和分享文案模板。
        "trafficLimit": "0",
        "maxRestoreCount": 0,
        "downloadType": 1,
        "shareTemplate": DEFAULT_SHARE_TEMPLATE
    })
}

fn manual_share_event_payload(
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

fn hdhive_signature(secret: &str, method: &str, path: &str, body: &str, timestamp: &str) -> String {
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

async fn hdhive_request(
    base_url: &str,
    secret: &str,
    instance_id: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    if base_url.is_empty() || secret.is_empty() {
        return Err("尚未配置 Hdhive 接入地址和密钥".to_string());
    }
    let body_text = body.map(Value::to_string).unwrap_or_default();
    let timestamp = unix_timestamp().to_string();
    let response = reqwest::Client::new()
        .request(
            method.clone(),
            format!("{}{path}", base_url.trim_end_matches('/')),
        )
        .header(CONTENT_TYPE, "application/json")
        .header("X-GuangYa-Instance-Id", instance_id)
        .header("X-GuangYa-Timestamp", &timestamp)
        .header(
            "X-GuangYa-Signature",
            hdhive_signature(secret, method.as_str(), path, &body_text, &timestamp),
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

async fn schedule_auto_share(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let Some(target) = auto_share_target(item) else {
        return Ok(());
    };
    let (mapping, token, device_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
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

async fn poll_hdhive_receipt(
    app: tauri::AppHandle,
    state: SharedState,
    pending: PendingAutoShare,
    share_url: String,
    payload: Value,
) {
    for attempt in 0..60_u64 {
        sleep(Duration::from_secs((2 + attempt / 2).min(10))).await;
        let (base_url, secret, instance_id, db_path) = match state.lock() {
            Ok(guard) => (
                guard.hdhive_base_url.clone(),
                guard.hdhive_secret.clone(),
                guard.hdhive_instance_id.clone(),
                guard.db_path.clone(),
            ),
            Err(_) => return,
        };
        let endpoint = format!("/api/integrations/guangya-sync/events/{}", pending.event_id);
        match hdhive_request(
            &base_url,
            &secret,
            &instance_id,
            reqwest::Method::GET,
            &endpoint,
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
                            "UPDATE auto_share_events SET notification_status=?1, updated_at=?2 WHERE event_id=?3",
                            params![
                                notification_status,
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

async fn process_auto_share(
    app: tauri::AppHandle,
    state: SharedState,
    pending: PendingAutoShare,
) -> Result<(), String> {
    let (mapping, token, device_id, db_path, base_url, secret, instance_id, has_work) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
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
    let Some(mapping) = mapping else {
        return delete_pending_auto_share(&db_path, &pending.mapping_id, &pending.target_key);
    };
    if !mapping.auto_share {
        return delete_pending_auto_share(&db_path, &pending.mapping_id, &pending.target_key);
    }
    if has_work {
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
    let accepted = hdhive_request(
        &base_url,
        &secret,
        &instance_id,
        reqwest::Method::POST,
        "/api/integrations/guangya-sync/events",
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

async fn auto_share_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(2)).await;
        let (db_path, configured) = match state.lock() {
            Ok(guard) => (
                guard.db_path.clone(),
                !guard.hdhive_base_url.is_empty() && !guard.hdhive_secret.is_empty(),
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

fn is_cloud_index_pending_message(message: &str) -> bool {
    [
        "文件上传中",
        "上传处理中",
        "正在上传",
        "正在处理",
        "正在入库",
        "任务处理中",
        "任务未完成",
        "稍后再试",
    ]
    .iter()
    .any(|pending| message.contains(pending))
}

async fn wait_upload_task(
    app: &tauri::AppHandle,
    token: &str,
    device_id: &str,
    task_id: &str,
    file_path: &Path,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(CLOUD_CONFIRM_TIMEOUT_SECS);
    let mut attempt = 0_u64;
    while Instant::now() < deadline {
        match api_post(
            token,
            device_id,
            "/userres/v1/file/get_info_by_task_id",
            json!({ "taskId": task_id }),
            &[145, 146, 155, 163],
        )
        .await
        {
            Ok(result) => {
                if let Some(data) = result.data.filter(|data| data.get("fileId").is_some()) {
                    return Ok(data);
                }
            }
            Err(message) if is_cloud_index_pending_message(&message) => {}
            Err(message) => return Err(message),
        }
        attempt += 1;
        emit(
            app,
            json!({ "type": "progress", "file_path": file_path.to_string_lossy(), "percent": 100, "stage": "文件已上传，云端正在入库" }),
        );
        let delay = Duration::from_secs(attempt.div_ceil(5).clamp(1, 5));
        sleep(delay.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    Err(format!(
        "云端入库超过 {CLOUD_CONFIRM_TIMEOUT_SECS} 秒仍未完成，请稍后刷新云盘确认"
    ))
}

async fn wait_operation_task(token: &str, device_id: &str, task_id: &str) -> Result<(), String> {
    for _ in 0..90 {
        let result = api_post(
            token,
            device_id,
            "/userres/v1/get_task_status",
            json!({ "taskId": task_id }),
            &[],
        )
        .await?;
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

async fn upload_oss(
    token_data: &UploadToken,
    path: &Path,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let credentials = token_data
        .creds
        .as_ref()
        .ok_or_else(|| "光鸭没有返回 OSS 临时凭证".to_string())?;
    let raw_endpoint = token_data
        .end_point
        .as_deref()
        .ok_or_else(|| "光鸭没有返回 OSS 端点".to_string())?;
    let bucket_name = token_data
        .bucket_name
        .as_deref()
        .ok_or_else(|| "光鸭没有返回 OSS 存储桶".to_string())?;
    let endpoint = normalize_oss_endpoint_url(raw_endpoint, bucket_name);
    if endpoint.is_empty() {
        return Err("光鸭返回的 OSS 端点无效".into());
    }
    let object_path = token_data
        .object_path
        .as_deref()
        .ok_or_else(|| "光鸭没有返回 OSS 对象路径".to_string())?
        .trim_start_matches('/');
    if object_path.is_empty() {
        return Err("光鸭返回的 OSS 对象路径无效".into());
    }

    let builder = Oss::default()
        .bucket(bucket_name)
        .endpoint(&endpoint)
        .access_key_id(&credentials.access_key_id)
        .access_key_secret(&credentials.secret_access_key)
        .security_token(&credentials.session_token);
    let operator = Operator::new(builder)
        .map_err(|error| format!("初始化 OSS 客户端失败：{error}"))?
        .finish();
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let part_size = oss_part_size(size);
    let writer_future = operator
        .writer_with(object_path)
        .chunk(part_size)
        .concurrent(3);
    let mut writer = timeout(Duration::from_secs(OSS_REQUEST_TIMEOUT_SECS), writer_future)
        .await
        .map_err(|_| "连接 OSS 超时".to_string())?
        .map_err(|error| format!("连接 OSS 失败：{error}"))?;

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut uploaded = 0u64;
    let upload_started_at = std::time::Instant::now();
    loop {
        let mut buffer = vec![0u8; part_size];
        let read = file.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        buffer.truncate(read);
        match timeout(
            Duration::from_secs(OSS_REQUEST_TIMEOUT_SECS),
            writer.write(buffer),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                let _ = timeout(Duration::from_secs(15), writer.abort()).await;
                return Err(format!("OSS 上传失败：{error}"));
            }
            Err(_) => {
                let _ = timeout(Duration::from_secs(15), writer.abort()).await;
                return Err(format!(
                    "OSS 上传超过 {OSS_REQUEST_TIMEOUT_SECS} 秒，已停止当前任务"
                ));
            }
        }
        uploaded += read as u64;
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": path.to_string_lossy(),
                "percent": if size == 0 { 100 } else { uploaded.saturating_mul(100) / size },
                "bytes_per_second": uploaded as f64 / upload_started_at.elapsed().as_secs_f64().max(0.001),
                "stage": "正在上传"
            }),
        );
    }

    emit(
        app,
        json!({
            "type": "progress",
            "file_path": path.to_string_lossy(),
            "percent": if size == 0 { 0 } else { 100 },
            "bytes_per_second": 0,
            "stage": "正在提交 OSS"
        }),
    );
    match timeout(
        Duration::from_secs(OSS_REQUEST_TIMEOUT_SECS),
        writer.close(),
    )
    .await
    {
        Ok(Ok(_)) => {
            emit(
                app,
                json!({ "type": "progress", "file_path": path.to_string_lossy(), "percent": 100, "bytes_per_second": 0, "stage": "OSS 上传完成" }),
            );
        }
        Ok(Err(error)) => {
            let _ = timeout(Duration::from_secs(15), writer.abort()).await;
            return Err(format!("提交 OSS 上传失败：{error}"));
        }
        Err(_) => {
            let _ = timeout(Duration::from_secs(15), writer.abort()).await;
            return Err("提交 OSS 上传超时".into());
        }
    }
    Ok(())
}

fn gcid_chunk_size(file_size: u64) -> usize {
    match file_size {
        0..=0x0800_0000 => 256 * 1024,
        0x0800_0001..=0x1000_0000 => 512 * 1024,
        0x1000_0001..=0x2000_0000 => 1024 * 1024,
        _ => 2 * 1024 * 1024,
    }
}

async fn calculate_file_md5(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("读取秒传文件失败：{error}"))?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0_u8; 2 * 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("计算文件 MD5 失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

async fn calculate_file_gcid(
    app: &tauri::AppHandle,
    path: &Path,
    file_size: u64,
) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("读取秒传文件失败：{error}"))?;
    let chunk_size = gcid_chunk_size(file_size);
    let mut buffer = vec![0_u8; chunk_size];
    let mut outer = Sha1::new();
    let mut hashed = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("计算文件 GCID 失败：{error}"))?;
        if read == 0 {
            break;
        }
        outer.update(Sha1::digest(&buffer[..read]));
        hashed += read as u64;
        let percent = if file_size == 0 {
            100
        } else {
            hashed.saturating_mul(100) / file_size
        };
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": path.to_string_lossy(),
                "percent": 0,
                "bytes_per_second": 0,
                "stage": format!("正在计算秒传指纹 {percent}%")
            }),
        );
    }
    Ok(hex::encode_upper(outer.finalize()))
}

#[cfg(windows)]
fn file_available_for_upload(path: &Path) -> Result<bool, String> {
    use std::os::windows::fs::OpenOptionsExt;

    match fs::OpenOptions::new().read(true).share_mode(0).open(path) {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Ok(false),
        Err(error) => Err(format!("读取源文件失败：{error}")),
    }
}

#[cfg(not(windows))]
fn file_available_for_upload(path: &Path) -> Result<bool, String> {
    fs::OpenOptions::new()
        .read(true)
        .open(path)
        .map(|_| true)
        .map_err(|error| format!("读取源文件失败：{error}"))
}

async fn prepare_upload_item(item: &UploadItem) -> Result<Option<UploadItem>, String> {
    if !file_available_for_upload(&item.file_path)? {
        return Ok(None);
    }
    let first =
        fs::metadata(&item.file_path).map_err(|error| format!("读取源文件失败：{error}"))?;
    if !first.is_file() {
        return Err("源路径不是文件".into());
    }
    sleep(Duration::from_millis(FILE_STABILITY_WAIT_MS)).await;
    if !file_available_for_upload(&item.file_path)? {
        return Ok(None);
    }
    let second =
        fs::metadata(&item.file_path).map_err(|error| format!("读取源文件失败：{error}"))?;
    if first.len() != second.len() || modified_ms(&first) != modified_ms(&second) {
        return Ok(None);
    }
    let mut ready = item.clone();
    ready.size = second.len();
    ready.modified_ms = modified_ms(&second);
    Ok(Some(ready))
}

async fn requeue_busy_upload(app: tauri::AppHandle, state: SharedState, mut item: UploadItem) {
    sleep(Duration::from_secs(FILE_BUSY_RETRY_SECS)).await;
    let metadata = fs::metadata(&item.file_path)
        .ok()
        .filter(|value| value.is_file());
    if let Some(metadata) = &metadata {
        item.size = metadata.len();
        item.modified_ms = modified_ms(metadata);
    }
    let key = item_key(&item.mapping_id, &item.file_path);
    let queued = if let Ok(mut guard) = state.lock() {
        guard.waiting_files.remove(&key);
        if metadata.is_none() {
            false
        } else if item.mapping_id != "__manual__"
            && !guard
                .mappings
                .iter()
                .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
        {
            false
        } else if upload_already_scheduled(
            &guard.history,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &item,
        ) {
            false
        } else {
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.queue.push_back(item.clone());
            true
        }
    } else {
        false
    };
    if queued {
        emit(
            &app,
            json!({
                "type": "file",
                "state": "waiting-file",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "stage": "另外的程序正在使用该文件，释放后将自动上传"
            }),
        );
        emit_state(&app, &state);
        drain_queue(app, state);
    } else {
        emit_state(&app, &state);
    }
}

async fn upload_item(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
) -> Result<UploadOutcome, String> {
    let (token, device_id) = {
        let guard = state.lock().map_err(|e| e.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
        )
    };
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "stage": "正在准备云端目录" }),
    );
    let parent_id = ensure_remote_path(
        state,
        &token,
        &device_id,
        &item.remote_parent_id,
        &item.remote_dir,
    )
    .await?;
    let name = item
        .file_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| "无法读取文件名".to_string())?;
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "stage": "正在申请上传凭证" }),
    );
    let mut res = json!({ "fileSize": item.size });
    if item.size < 1024 * 1024 {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "stage": "正在计算秒传 MD5" }),
        );
        res["md5"] = json!(calculate_file_md5(&item.file_path).await?);
    }
    let result = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_center_token",
        json!({ "capacity": 2, "name": name, "res": res, "parentId": parent_id }),
        &[156],
    )
    .await?;
    let mut data: UploadToken = serde_json::from_value(
        result
            .data
            .ok_or_else(|| "光鸭没有返回上传凭证".to_string())?,
    )
    .map_err(|e| format!("上传凭证格式异常：{e}"))?;
    let mut instant_upload = result.code == 156;
    if !instant_upload && item.size >= 1024 * 1024 {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "stage": "正在校验秒传" }),
        );
        match calculate_file_gcid(app, &item.file_path, item.size).await {
            Ok(gcid) => match api_post(
                &token,
                &device_id,
                "/userres/v1/check_can_flash_upload",
                json!({ "taskId": data.task_id, "gcid": gcid }),
                &[],
            )
            .await
            {
                Ok(check) => {
                    let check_data = check.data.unwrap_or_default();
                    instant_upload = check_data
                        .get("canFlashUpload")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if instant_upload {
                        if let Some(task_id) = check_data
                            .get("taskId")
                            .and_then(Value::as_str)
                            .filter(|value| !value.is_empty())
                        {
                            data.task_id = task_id.to_string();
                        }
                    }
                }
                Err(error) => status(
                    app,
                    "warning",
                    format!("秒传校验失败，继续普通上传：{error}"),
                ),
            },
            Err(error) => status(
                app,
                "warning",
                format!("秒传指纹计算失败，继续普通上传：{error}"),
            ),
        }
    }
    if !instant_upload {
        emit(
            app,
            json!({ "type": "file", "state": "uploading", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id }),
        );
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "stage": "正在连接 OSS" }),
        );
        upload_oss(&data, &item.file_path, app).await?;
    } else {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "stage": "已命中秒传" }),
        );
    }
    let pending_outcome = UploadOutcome {
        task_id: data.task_id.clone(),
        remote_file_id: None,
    };
    remember_uploaded_item(state, item, &pending_outcome)
        .map_err(|message| format!("文件已上传，但写入本地上传记录失败：{message}"))?;
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "stage": "已上传，正在等待云端入库" }),
    );
    emit(
        app,
        json!({ "type": "file", "state": "processing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "stage": "已上传，正在等待云端入库" }),
    );
    let task_data = wait_upload_task(app, &token, &device_id, &data.task_id, &item.file_path)
        .await
        .map_err(|message| {
            format!("文件已上传并已写入记录，不会重复上传；云端入库确认失败：{message}")
        })?;
    Ok(UploadOutcome {
        task_id: data.task_id,
        remote_file_id: task_data
            .get("fileId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn apply_source_policy(state: &SharedState, item: &UploadItem) -> Result<Option<String>, String> {
    if item.mapping_id == "__manual__" {
        return Ok(None);
    }
    let mapping = state
        .lock()
        .map_err(|e| e.to_string())?
        .mappings
        .iter()
        .find(|mapping| mapping.id == item.mapping_id)
        .cloned()
        .ok_or_else(|| "备份任务已被移除，源文件保持不变".to_string())?;
    if mapping.source_policy == "keep" {
        return Ok(None);
    }
    let metadata = fs::metadata(&item.file_path).map_err(|e| format!("读取源文件失败：{e}"))?;
    if metadata.len() != item.size || modified_ms(&metadata) != item.modified_ms {
        return Err("上传期间源文件发生变化，已保留源文件且不会执行上传后策略".into());
    }
    if mapping.source_policy == "delete" {
        fs::remove_file(&item.file_path).map_err(|e| format!("删除源文件失败：{e}"))?;
        return Ok(Some("已按任务策略删除源文件".into()));
    }
    if mapping.source_policy != "archive" {
        return Err(format!("未知的源文件策略：{}", mapping.source_policy));
    }
    let archive_root = mapping
        .archive_path
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "归档策略没有配置归档目录".to_string())?;
    let source_root = PathBuf::from(&mapping.local_path);
    let archive_root = PathBuf::from(archive_root);
    if archive_root.starts_with(&source_root) {
        return Err("归档目录不能位于被监控目录内部".into());
    }
    let relative = item
        .file_path
        .strip_prefix(&source_root)
        .map_err(|_| "无法计算源文件的相对路径".to_string())?;
    let mut destination = archive_root.join(relative);
    if destination.exists() {
        let stem = destination
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("file");
        let extension = destination
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{value}"))
            .unwrap_or_default();
        destination.set_file_name(format!("{stem}-{}{}", item.modified_ms, extension));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建归档目录失败：{e}"))?;
    }
    if fs::rename(&item.file_path, &destination).is_err() {
        fs::copy(&item.file_path, &destination).map_err(|e| format!("复制到归档目录失败：{e}"))?;
        fs::remove_file(&item.file_path).map_err(|e| format!("移除归档后的源文件失败：{e}"))?;
    }
    Ok(Some(format!("已移动到归档目录：{}", destination.display())))
}

fn drain_queue(app: tauri::AppHandle, state: SharedState) {
    loop {
        let item = {
            let mut guard = match state.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if guard.paused || guard.token.is_none() || guard.active_uploads >= MAX_UPLOADS {
                None
            } else {
                let item = guard.queue.pop_front();
                if let Some(item) = &item {
                    guard.active_uploads += 1;
                    guard.inflight.insert(
                        item_key(&item.mapping_id, &item.file_path),
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                    guard
                        .inflight_items
                        .insert(item_key(&item.mapping_id, &item.file_path), item.clone());
                }
                item
            }
        };
        let Some(item) = item else {
            emit_state(&app, &state);
            return;
        };
        emit(
            &app,
            json!({ "type": "file", "state": "preparing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id }),
        );
        let app2 = app.clone();
        let state2 = state.clone();
        tauri::async_runtime::spawn(async move {
            let upload_key = item_key(&item.mapping_id, &item.file_path);
            let mut item = item;
            let result = match prepare_upload_item(&item).await {
                Ok(Some(ready)) => {
                    item = ready;
                    upload_item(&app2, &state2, &item).await.map(Some)
                }
                Ok(None) => Ok(None),
                Err(message) => Err(message),
            };
            let waiting_for_file = result.as_ref().ok().is_some_and(Option::is_none);
            let auth_expired = result
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("登录态已失效"));
            let outcome = result.as_ref().ok().and_then(|value| value.clone());
            let error_message = result.as_ref().err().cloned();
            let mut db_path = None;
            if let Ok(mut guard) = state2.lock() {
                guard.active_uploads = guard.active_uploads.saturating_sub(1);
                guard.inflight.remove(&upload_key);
                guard.inflight_items.remove(&upload_key);
                db_path = Some(guard.db_path.clone());
                if auth_expired {
                    guard.token = None;
                }
                if outcome.is_some() {
                    guard.history.insert(
                        upload_key,
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                } else if waiting_for_file {
                    guard.waiting_files.insert(upload_key.clone(), item.clone());
                }
            }
            if auth_expired {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = clear_persisted_access_token(path) {
                        status(&app2, "error", message);
                    }
                }
            }
            if waiting_for_file {
                emit(
                    &app2,
                    json!({
                        "type": "file",
                        "state": "waiting-file",
                        "file_path": item.file_path.to_string_lossy(),
                        "mapping_id": item.mapping_id,
                        "stage": "另外的程序正在使用该文件，释放后将自动上传"
                    }),
                );
                tauri::async_runtime::spawn(requeue_busy_upload(
                    app2.clone(),
                    state2.clone(),
                    item.clone(),
                ));
            } else if let Some(outcome) = outcome {
                if let Some(path) = db_path.as_deref() {
                    if let Err(message) = save_upload_history(path, &item, &outcome) {
                        status(
                            &app2,
                            "error",
                            format!("文件已上传，但本地记录保存失败：{message}"),
                        );
                    }
                    if let Err(message) = clear_auto_share_failure(path, &item) {
                        status(&app2, "error", message);
                    }
                }
                if let Err(message) = schedule_auto_share(&state2, &item, &outcome).await {
                    status(
                        &app2,
                        "error",
                        format!("文件已上传，但自动分享排队失败：{message}"),
                    );
                }
                match apply_source_policy(&state2, &item) {
                    Ok(Some(message)) => status(&app2, "success", message),
                    Ok(None) => {}
                    Err(message) => status(&app2, "error", message),
                }
                emit(
                    &app2,
                    json!({ "type": "file", "state": "done", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id }),
                );
            } else {
                let message = error_message.unwrap_or_else(|| "上传失败".into());
                let auto_share_enabled = state2.lock().ok().is_some_and(|guard| {
                    guard
                        .mappings
                        .iter()
                        .any(|mapping| mapping.id == item.mapping_id && mapping.auto_share)
                });
                if auto_share_enabled {
                    if let Some(path) = db_path.as_deref() {
                        if let Err(error) = record_auto_share_failure(path, &item, &message) {
                            status(&app2, "error", error);
                        }
                    }
                }
                emit(
                    &app2,
                    json!({ "type": "file", "state": "error", "file_path": item.file_path.to_string_lossy(), "error": message.clone() }),
                );
            }
            emit_state(&app2, &state2);
            drain_queue(app2, state2);
        });
    }
}

async fn enqueue_path(app: &tauri::AppHandle, state: &SharedState, event: FsEvent) {
    let mapping = state.lock().ok().and_then(|guard| {
        guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == event.mapping_id && mapping.enabled)
            .cloned()
    });
    let Some(mapping) = mapping else {
        return;
    };
    let event_paths = collect_watch_event_files(&event.path, &mapping.sync_types);
    if event_paths.is_empty() {
        return;
    }
    if event_paths.len() != 1 || event_paths.first() != Some(&event.path) {
        if let Ok(guard) = state.lock() {
            for path in event_paths {
                let _ = guard.event_tx.send(FsEvent {
                    mapping_id: mapping.id.clone(),
                    path,
                });
            }
        }
        return;
    }
    let Ok(meta) = fs::metadata(&event.path) else {
        return;
    };
    let relative = event
        .path
        .strip_prefix(&mapping.local_path)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let relative_dir = Path::new(&relative)
        .parent()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let remote_dir = [
        if mapping.remote_parent_id.is_empty() {
            normalize_remote_path(&mapping.remote_path)
        } else {
            String::new()
        },
        relative_dir,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("/");
    let mut item = UploadItem {
        mapping_id: mapping.id,
        file_path: event.path.clone(),
        remote_parent_id: mapping.remote_parent_id,
        remote_dir,
        relative_path: relative,
        change_kind: "added".to_string(),
        size: meta.len(),
        modified_ms: modified_ms(&meta),
    };
    let waiting_for_login = if let Ok(mut guard) = state.lock() {
        let waiting_for_login = guard.token.is_none();
        let key = item_key(&item.mapping_id, &item.file_path);
        if upload_already_scheduled(
            &guard.history,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &item,
        ) {
            return;
        }
        if guard.history.contains_key(&key) {
            item.change_kind = "changed".to_string();
        }
        guard
            .queue
            .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
        guard.queue.push_back(item);
        waiting_for_login
    } else {
        return;
    };
    emit(
        app,
        json!({ "type": "file", "state": if waiting_for_login { "waiting-login" } else { "queued" }, "file_path": event.path.to_string_lossy() }),
    );
    emit_state(app, state);
    drain_queue(app.clone(), state.clone());
}

fn install_watcher(state: &SharedState, mapping: &Mapping) -> Result<(), String> {
    if !Path::new(&mapping.local_path).is_dir() {
        return Err("监控目录不存在或无法访问".to_string());
    }
    if let Ok(mut guard) = state.lock() {
        guard.watchers.remove(&mapping.id);
    }
    if mapping.monitor_mode == "polling" {
        return Ok(());
    }
    let tx = state.lock().map_err(|e| e.to_string())?.event_tx.clone();
    let mapping_id = mapping.id.clone();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result {
                if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                    for path in event.paths {
                        let _ = tx.send(FsEvent {
                            mapping_id: mapping_id.clone(),
                            path,
                        });
                    }
                }
            }
        },
        NotifyConfig::default(),
    )
    .map_err(|e| e.to_string())?;
    watcher
        .watch(Path::new(&mapping.local_path), RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.watchers.remove(&mapping.id);
    guard.watchers.insert(mapping.id.clone(), watcher);
    Ok(())
}

fn collect_existing_files(root: &Path, sync_types: &[String], files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_existing_files(&path, sync_types, files);
        } else if metadata.is_file() && !ignored(&path) && should_sync(&path, sync_types) {
            files.push(path);
        }
    }
}

fn collect_watch_event_files(path: &Path, sync_types: &[String]) -> Vec<PathBuf> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Vec::new();
    };
    if metadata.file_type().is_symlink() {
        return Vec::new();
    }
    if metadata.is_dir() {
        let mut files = Vec::new();
        collect_existing_files(path, sync_types, &mut files);
        return files;
    }
    if metadata.is_file() && !ignored(path) && should_sync(path, sync_types) {
        vec![path.to_path_buf()]
    } else {
        Vec::new()
    }
}

fn enqueue_existing_files(app: &tauri::AppHandle, state: &SharedState, mapping: &Mapping) {
    if !mapping.scan_existing {
        return;
    }
    let mut files = Vec::new();
    collect_existing_files(
        Path::new(&mapping.local_path),
        &mapping.sync_types,
        &mut files,
    );
    emit(
        app,
        json!({ "type": "status", "level": "info", "message": format!("正在扫描已有文件：{} 个", files.len()) }),
    );
    if let Ok(guard) = state.lock() {
        for path in files {
            let _ = guard.event_tx.send(FsEvent {
                mapping_id: mapping.id.clone(),
                path,
            });
        }
    }
    emit_state(app, state);
}

fn seed_existing_files(state: &SharedState, mapping: &Mapping) {
    let mut files = Vec::new();
    collect_existing_files(
        Path::new(&mapping.local_path),
        &mapping.sync_types,
        &mut files,
    );
    if let Ok(mut guard) = state.lock() {
        for path in files {
            if let Ok(metadata) = fs::metadata(&path) {
                guard.history.insert(
                    item_key(&mapping.id, &path),
                    Stamp {
                        size: metadata.len(),
                        modified_ms: modified_ms(&metadata),
                    },
                );
            }
        }
    }
}

async fn polling_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        let mappings = state
            .lock()
            .map(|guard| {
                guard
                    .mappings
                    .iter()
                    .filter(|mapping| mapping.enabled && mapping.monitor_mode == "polling")
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for mapping in mappings {
            let mut files = Vec::new();
            collect_existing_files(
                Path::new(&mapping.local_path),
                &mapping.sync_types,
                &mut files,
            );
            for path in files {
                enqueue_path(
                    &app,
                    &state,
                    FsEvent {
                        mapping_id: mapping.id.clone(),
                        path,
                    },
                )
                .await;
            }
        }
    }
}

#[tauri::command]
fn get_state(state: tauri::State<'_, SharedState>) -> Snapshot {
    state
        .lock()
        .map(|guard| snapshot(&guard))
        .unwrap_or(Snapshot {
            logged_in: false,
            paused: false,
            pending: 0,
            active_uploads: 0,
            mappings: vec![],
            saved_shares: vec![],
            hdhive: HdhivePublicConfig {
                configured: false,
                base_url: String::new(),
                instance_id: String::new(),
            },
            auto_share_receipts: vec![],
        })
}

fn auth_context(state: &tauri::State<'_, SharedState>) -> Result<(String, String), String> {
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok((
        guard
            .token
            .clone()
            .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
        guard.device_id.clone(),
    ))
}

#[tauri::command]
async fn get_overview(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let assets = api_post(&token, &device_id, "/assets/v1/get_assets", json!({}), &[]).await?;
    let profile = match account_get(&token, "/v1/user/me").await {
        Ok(value) => value,
        Err(_) => api_post(
            &token,
            &device_id,
            "/activity/v1/get_user_data",
            json!({}),
            &[],
        )
        .await
        .ok()
        .and_then(|response| response.data)
        .unwrap_or_else(|| json!({})),
    };
    Ok(json!({ "assets": assets.data.unwrap_or_else(|| json!({})), "profile": profile }))
}

#[tauri::command]
async fn list_files(
    state: tauri::State<'_, SharedState>,
    parent_id: String,
    page: u64,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/get_file_list",
        json!({ "page": page, "pageSize": 100, "parentId": parent_id, "orderBy": 0, "sortType": 0, "needSubFolderStat": true }),
        &[],
    )
    .await?;
    Ok(response
        .data
        .unwrap_or_else(|| json!({ "list": [], "total": 0 })))
}

fn collect_manual_uploads(path: &Path, remote_prefix: &str, files: &mut Vec<(PathBuf, String)>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() || ignored(path) {
        return;
    }
    if metadata.is_file() {
        files.push((path.to_path_buf(), normalize_remote_path(remote_prefix)));
        return;
    }
    if !metadata.is_dir() {
        return;
    }
    let Some(folder_name) = path.file_name().and_then(|value| value.to_str()) else {
        return;
    };
    let folder_prefix = [remote_prefix, folder_name]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_manual_uploads(&entry.path(), &folder_prefix, files);
    }
}

#[tauri::command]
fn select_upload_files() -> Vec<String> {
    rfd::FileDialog::new()
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

#[tauri::command]
fn select_upload_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
fn queue_upload_paths(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    paths: Vec<String>,
    parent_id: String,
) -> Result<usize, String> {
    if paths.is_empty() {
        return Err("没有选择需要上传的文件".into());
    }
    if state.lock().map_err(|e| e.to_string())?.token.is_none() {
        return Err("请先登录光鸭云盘".into());
    }
    let mut files = Vec::new();
    for input in paths {
        let path = PathBuf::from(input);
        if !path.exists() {
            return Err(format!("本地路径不存在：{}", path.display()));
        }
        collect_manual_uploads(&path, "", &mut files);
    }
    if files.is_empty() {
        return Err("选中的路径中没有可上传文件".into());
    }
    let mut count = 0usize;
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        for (path, remote_dir) in files {
            let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
            let item = UploadItem {
                mapping_id: "__manual__".into(),
                file_path: path,
                remote_parent_id: parent_id.clone(),
                remote_dir,
                relative_path: String::new(),
                change_kind: "added".to_string(),
                size: metadata.len(),
                modified_ms: modified_ms(&metadata),
            };
            let key = item_key(&item.mapping_id, &item.file_path);
            if guard
                .inflight
                .get(&key)
                .is_some_and(|stamp| stamp_matches(&item, stamp))
                || guard.queue.iter().any(|queued| {
                    item_key(&queued.mapping_id, &queued.file_path) == key
                        && queued.size == item.size
                        && queued.modified_ms == item.modified_ms
                })
            {
                continue;
            }
            guard
                .queue
                .retain(|queued| item_key(&queued.mapping_id, &queued.file_path) != key);
            guard.queue.push_back(item);
            count += 1;
        }
    }
    if count == 0 {
        return Ok(0);
    }
    status(&app, "info", format!("已加入上传队列：{count} 个文件"));
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(count)
}

async fn rename_remote(
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

fn validate_file_ids(file_ids: &[String]) -> Result<(), String> {
    if file_ids.is_empty() {
        Err("请至少选择一个文件或文件夹".into())
    } else {
        Ok(())
    }
}

async fn fetch_received_share_files(
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

async fn fetch_all_shares(token: &str, device_id: &str) -> Result<Value, String> {
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

fn value_as_id(value: Option<&Value>) -> String {
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

async fn find_existing_share_for_files(
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

#[tauri::command]
async fn list_shares(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    fetch_all_shares(&token, &device_id).await
}

#[tauri::command]
async fn delete_shares(
    state: tauri::State<'_, SharedState>,
    ids: Vec<Value>,
) -> Result<Value, String> {
    if ids.is_empty() {
        return Err("请至少选择一个分享".into());
    }
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/delete_share",
        json!({ "ids": ids }),
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
async fn open_received_share(
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
async fn list_received_share_files(
    state: tauri::State<'_, SharedState>,
    access_token: String,
    parent_id: String,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    fetch_received_share_files(&token, &device_id, &access_token, &parent_id).await
}

#[tauri::command]
async fn restore_received_share(
    state: tauri::State<'_, SharedState>,
    access_token: String,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
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
    Ok(data)
}

#[tauri::command]
async fn get_received_share_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    access_token: String,
    file_ids: Vec<String>,
    packaged: bool,
    file_name: String,
    destination_dir: String,
    download_id: String,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
    if access_token.trim().is_empty() {
        return Err("分享访问令牌为空，请重新打开分享链接".into());
    }
    if !packaged && file_ids.len() != 1 {
        return Err("单文件下载只能选择一个文件".into());
    }
    let (token, device_id) = auth_context(&state)?;
    if !packaged {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/get_share_download_url",
            json!({ "fileId": file_ids[0], "accessToken": access_token }),
            &[205, 206, 207, 504],
        )
        .await?;
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
        let result = api_post(
            &token,
            &device_id,
            "/scheduler/v1/query_packaging_task",
            json!({ "taskId": task_id, "accessToken": access_token }),
            &[],
        )
        .await?;
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
            )
            .await;
        }
        sleep(Duration::from_secs(1)).await;
    }
    Err("光鸭打包超过 10 分钟仍未完成，请稍后重试".into())
}

#[tauri::command]
async fn get_cloud_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    packaged: bool,
    file_name: String,
    destination_dir: String,
    download_id: String,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
    if !packaged && file_ids.len() != 1 {
        return Err("单文件下载只能选择一个文件".into());
    }
    let (token, device_id) = auth_context(&state)?;
    if !packaged {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/get_res_download_url",
            json!({ "fileId": file_ids[0] }),
            &[],
        )
        .await?;
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
        let result = api_post(
            &token,
            &device_id,
            "/scheduler/v1/query_packaging_task",
            json!({ "taskId": task_id }),
            &[],
        )
        .await?;
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
            )
            .await;
        }
        sleep(Duration::from_secs(1)).await;
    }
    Err("光鸭打包超过 10 分钟仍未完成，请稍后重试".into())
}

fn safe_download_name(value: &str) -> String {
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

fn available_download_path(directory: &Path, file_name: &str) -> PathBuf {
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

fn response_total_bytes(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("content-range")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .or_else(|| response.content_length().filter(|value| *value > 0))
}

async fn download_to_local(
    app: &tauri::AppHandle,
    download_url: &str,
    requested_name: &str,
    destination_dir: &str,
    download_id: &str,
) -> Result<Value, String> {
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
    let probed_total_bytes = match client.head(download_url).send().await {
        Ok(response) if response.status().is_success() => response_total_bytes(&response),
        _ => None,
    };
    let mut response = client
        .get(download_url)
        .send()
        .await
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
    emit(
        app,
        json!({
            "type": "download",
            "download_id": download_id,
            "state": "downloading",
            "file_name": actual_name,
            "downloaded_bytes": 0,
            "total_bytes": total_bytes,
            "percent": total_bytes.map(|_| 0),
            "bytes_per_second": 0
        }),
    );
    let result: Result<(), String> = async {
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("读取光鸭下载数据失败：{error}"))?
        {
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
                        "bytes_per_second": bytes_per_second
                    }),
                );
                last_emit = Instant::now();
                last_emit_bytes = downloaded_bytes;
            }
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新下载文件失败：{error}"))?;
        Ok(())
    }
    .await;
    drop(file);
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&partial).await;
        emit(
            app,
            json!({ "type": "download", "download_id": download_id, "state": "error", "error": error }),
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
            "bytes_per_second": 0
        }),
    );
    Ok(json!({
        "file_path": file_path,
        "file_name": actual_name,
        "bytes": downloaded_bytes
    }))
}

#[tauri::command]
async fn copy_files(
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/copy_file",
        json!({ "fileIds": file_ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id).await?;
    }
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
async fn move_files(
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    parent_id: String,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/move_file",
        json!({ "fileIds": file_ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id).await?;
    }
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
async fn delete_files(
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
) -> Result<Value, String> {
    validate_file_ids(&file_ids)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": file_ids }),
        &[],
    )
    .await?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id).await?;
    }
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
async fn batch_rename_files(
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
    Ok(json!({ "renamed": staged.len() }))
}

#[tauri::command]
async fn create_share(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    title: String,
    target_type: Option<String>,
) -> Result<Value, String> {
    if file_ids.is_empty() {
        return Err("请至少选择一个文件或文件夹".into());
    }
    let title = title.trim();
    let title = if title.is_empty() {
        "云盘分享".to_string()
    } else {
        title.to_string()
    };
    let (token, device_id) = auth_context(&state)?;
    let existing = find_existing_share_for_files(&token, &device_id, &file_ids).await?;
    let reused_existing = existing.is_some();
    let mut data = if let Some(existing) = existing {
        existing
    } else {
        api_post(
            &token,
            &device_id,
            "/userres/v1/share_file",
            share_file_payload(&file_ids, &title),
            &[],
        )
        .await?
        .data
        .ok_or_else(|| "光鸭没有返回分享信息".to_string())?
    };
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
    let (base_url, secret, instance_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard.hdhive_base_url.clone(),
            guard.hdhive_secret.clone(),
            guard.hdhive_instance_id.clone(),
            guard.db_path.clone(),
        )
    };
    let mapping_id = "__manual__";
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
    let (hdhive_status, hdhive_message) = match hdhive_request(
        &base_url,
        &secret,
        &instance_id,
        reqwest::Method::POST,
        "/api/integrations/guangya-sync/events",
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
async fn create_offline_task(
    state: tauri::State<'_, SharedState>,
    url: String,
    parent_id: String,
    new_name: String,
) -> Result<Value, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("请输入离线下载地址".into());
    }
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v1/create_task",
        json!({ "url": url, "parentId": parent_id, "newName": new_name.trim() }),
        &[],
    )
    .await?;
    response.data.ok_or_else(|| "光鸭没有返回离线任务".into())
}

#[tauri::command]
async fn list_offline_tasks(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/cloudcollection/v1/list_task",
        json!({ "page": 0, "pageSize": 100 }),
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({ "list": [] })))
}

#[tauri::command]
fn save_share_link(
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
fn remove_share_link(
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

async fn refresh_saved_session(app: tauri::AppHandle, state: SharedState) -> Result<bool, String> {
    let refresh_token = state
        .lock()
        .map_err(|e| e.to_string())?
        .refresh_token
        .clone();
    let Some(refresh_token) = refresh_token else {
        return Ok(false);
    };
    let (status_code, payload) = account_post(
        "/v1/auth/token",
        json!({ "grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": OAUTH_CLIENT_ID }),
    )
    .await?;
    if status_code >= 400 {
        return Err(payload
            .get("error_description")
            .or_else(|| payload.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("刷新登录状态失败")
            .to_string());
    }
    let access_token = payload
        .get("access_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("access_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "刷新登录状态时没有返回 access_token".to_string())?;
    let next_refresh = payload
        .get("refresh_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("refresh_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.token = Some(access_token.clone());
        if next_refresh.is_some() {
            guard.refresh_token = next_refresh.clone();
        }
        guard.remote_cache.clear();
        guard.db_path.clone()
    };
    save_auth_session(&db_path, Some(&access_token), next_refresh.as_deref())?;
    emit_state(&app, &state);
    drain_queue(app, state);
    Ok(true)
}

async fn token_refresh_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(TOKEN_REFRESH_INTERVAL_SECS)).await;
        let can_refresh = state
            .lock()
            .ok()
            .and_then(|guard| guard.refresh_token.clone())
            .is_some();
        if !can_refresh {
            continue;
        }
        if let Err(error) = refresh_saved_session(app.clone(), state.clone()).await {
            status(
                &app,
                "warning",
                format!("自动续期失败，将稍后重试：{error}"),
            );
        }
    }
}

#[tauri::command]
async fn start_device_login() -> Result<Value, String> {
    let (status, payload) = account_post(
        "/v1/auth/device/code",
        json!({ "scope": "user", "client_id": OAUTH_CLIENT_ID }),
    )
    .await?;
    if status >= 400 {
        return Err(payload
            .get("error_description")
            .or_else(|| payload.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("无法创建扫码登录任务")
            .to_string());
    }
    Ok(payload.get("data").cloned().unwrap_or(payload))
}

#[tauri::command]
async fn poll_device_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    device_code: String,
) -> Result<Value, String> {
    let (status_code, payload) = account_post(
        "/v1/auth/token",
        json!({ "grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": device_code, "client_id": OAUTH_CLIENT_ID }),
    )
    .await?;
    let token = payload
        .get("access_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("access_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let refresh_token = payload
        .get("refresh_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("refresh_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(token) = token {
        let db_path = {
            let mut guard = state.lock().map_err(|e| e.to_string())?;
            guard.token = Some(token.clone());
            if refresh_token.is_some() {
                guard.refresh_token = refresh_token.clone();
            }
            guard.remote_cache.clear();
            guard.db_path.clone()
        };
        if let Err(message) = save_auth_session(&db_path, Some(&token), refresh_token.as_deref()) {
            status(&app, "error", message);
        }
        status(&app, "success", "扫码登录成功，可以开始使用云盘和备份任务");
        emit_state(&app, state.inner());
        drain_queue(app, state.inner().clone());
        return Ok(json!({ "authenticated": true }));
    }
    if status_code == 400 || status_code == 202 || status_code == 428 {
        let pending_message =
            if payload.get("error").and_then(Value::as_str) == Some("authorization_pending") {
                "等待扫码确认"
            } else {
                payload
                    .get("error_description")
                    .or_else(|| payload.get("msg"))
                    .and_then(Value::as_str)
                    .filter(|message| *message != "Precondition Required")
                    .unwrap_or("等待扫码确认")
            };
        return Ok(json!({ "pending": true, "message": pending_message }));
    }
    Err(payload
        .get("error_description")
        .or_else(|| payload.get("msg"))
        .and_then(Value::as_str)
        .unwrap_or("扫码登录失败")
        .to_string())
}

#[tauri::command]
fn open_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("auth") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        "auth",
        WebviewUrl::External(
            AUTH_URL
                .parse()
                .map_err(|e| format!("登录页地址错误：{e}"))?,
        ),
    )
    .title("登录光鸭云盘")
    .inner_size(1120.0, 820.0)
    .initialization_script(auth_hook_script())
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
async fn capture_token(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    token: String,
) -> Result<(), String> {
    if token.len() < 20 {
        return Ok(());
    }
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        if guard.token.as_deref() == Some(token.as_str()) {
            return Ok(());
        }
        guard.token = Some(token.clone());
        guard.remote_cache.clear();
        guard.db_path.clone()
    };
    if let Err(message) = save_auth_session(&db_path, Some(&token), None) {
        status(&app, "error", message);
    }
    status(&app, "success", "已捕获官方登录态，可以开始监控上传");
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(())
}
#[tauri::command]
fn select_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}
#[tauri::command]
fn add_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    local_path: String,
    remote_path: String,
    remote_parent_id: String,
    source_policy: String,
    archive_path: Option<String>,
    scan_existing: bool,
    sync_types: Vec<String>,
    monitor_mode: String,
    auto_share: bool,
) -> Result<Mapping, String> {
    if !["keep", "archive", "delete"].contains(&source_policy.as_str()) {
        return Err("无效的上传后源文件策略".into());
    }
    if auto_share {
        let guard = state.lock().map_err(|e| e.to_string())?;
        if guard.hdhive_base_url.is_empty() || guard.hdhive_secret.is_empty() {
            return Err("开启自动分享前请先配置 Hdhive 地址和密钥".to_string());
        }
    }
    let mapping = Mapping {
        id: Uuid::new_v4().to_string(),
        local_path: PathBuf::from(local_path).to_string_lossy().to_string(),
        remote_path: normalize_remote_path(&remote_path),
        remote_parent_id,
        enabled: true,
        source_policy,
        archive_path: archive_path.filter(|value| !value.trim().is_empty()),
        scan_existing,
        sync_types: normalize_sync_types(&sync_types),
        watch_error: None,
        monitor_mode: normalize_monitor_mode(&monitor_mode),
        auto_share,
    };
    if !Path::new(&mapping.local_path).is_dir() {
        return Err("本地目录不存在".into());
    }
    if mapping.source_policy == "archive" {
        let archive_path = mapping
            .archive_path
            .as_ref()
            .ok_or_else(|| "归档策略需要选择归档目录".to_string())?;
        if Path::new(archive_path).starts_with(Path::new(&mapping.local_path)) {
            return Err("归档目录不能位于被监控目录内部".into());
        }
    }
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.mappings.push(mapping.clone());
        save_config(&guard);
    }
    if let Err(error) = install_watcher(state.inner(), &mapping) {
        if let Ok(mut guard) = state.lock() {
            guard.mappings.retain(|item| item.id != mapping.id);
            save_config(&guard);
        }
        return Err(format!("创建目录监控失败：{error}"));
    }
    if mapping.scan_existing {
        enqueue_existing_files(&app, state.inner(), &mapping);
    } else if mapping.monitor_mode == "polling" {
        seed_existing_files(state.inner(), &mapping);
    }
    emit_state(&app, state.inner());
    Ok(mapping)
}
#[tauri::command]
fn remove_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.watchers.remove(&id);
    guard.mappings.retain(|mapping| mapping.id != id);
    guard.queue.retain(|item| item.mapping_id != id);
    guard.waiting_files.retain(|_, item| item.mapping_id != id);
    let prefix = format!("{id}::");
    guard.history.retain(|key, _| !key.starts_with(&prefix));
    guard.inflight.retain(|key, _| !key.starts_with(&prefix));
    save_config(&guard);
    let db_path = guard.db_path.clone();
    drop(guard);
    remove_mapping_history(&db_path, &id)?;
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
fn toggle_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "监控目录不存在".to_string())?;
        mapping.enabled = enabled;
        let copy = mapping.clone();
        if !enabled {
            guard.watchers.remove(&id);
        }
        save_config(&guard);
        copy
    };
    if enabled {
        if let Err(error) = install_watcher(state.inner(), &mapping) {
            if let Ok(mut guard) = state.lock() {
                if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                    current.enabled = false;
                    current.watch_error = Some(error.clone());
                }
                save_config(&guard);
            }
            emit_state(&app, state.inner());
            return Err(format!("启动备份任务监控失败：{error}"));
        }
        if let Ok(mut guard) = state.lock() {
            if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                current.watch_error = None;
            }
            save_config(&guard);
        }
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
fn update_mapping_sync_types(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    sync_types: Vec<String>,
) -> Result<(), String> {
    let selected = normalize_sync_types(&sync_types);
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "备份任务不存在".to_string())?;
        mapping.sync_types = selected.clone();
        let mapping = mapping.clone();
        guard
            .queue
            .retain(|item| item.mapping_id != id || should_sync(&item.file_path, &selected));
        save_config(&guard);
        mapping
    };
    if mapping.enabled {
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}
#[tauri::command]
fn update_mapping_monitor_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    monitor_mode: String,
) -> Result<(), String> {
    let mode = normalize_monitor_mode(&monitor_mode);
    let mapping = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        let mapping = guard
            .mappings
            .iter_mut()
            .find(|mapping| mapping.id == id)
            .ok_or_else(|| "备份任务不存在".to_string())?;
        mapping.monitor_mode = mode;
        mapping.watch_error = None;
        let mapping = mapping.clone();
        save_config(&guard);
        mapping
    };
    if mapping.enabled {
        if let Err(error) = install_watcher(state.inner(), &mapping) {
            if let Ok(mut guard) = state.lock() {
                if let Some(current) = guard.mappings.iter_mut().find(|item| item.id == id) {
                    current.enabled = false;
                    current.watch_error = Some(error.clone());
                }
                save_config(&guard);
            }
            emit_state(&app, state.inner());
            return Err(format!("切换监控方式失败：{error}"));
        }
        if mapping.scan_existing {
            enqueue_existing_files(&app, state.inner(), &mapping);
        } else if mapping.monitor_mode == "polling" {
            seed_existing_files(state.inner(), &mapping);
        }
    }
    emit_state(&app, state.inner());
    Ok(())
}

#[tauri::command]
fn update_hdhive_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    base_url: String,
    secret: Option<String>,
) -> Result<HdhivePublicConfig, String> {
    let normalized = base_url.trim().trim_end_matches('/').to_string();
    if !normalized.is_empty() {
        let parsed = reqwest::Url::parse(&normalized)
            .map_err(|_| "Hdhive 地址必须是完整的 HTTP(S) URL".to_string())?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err("Hdhive 地址必须是完整的 HTTP(S) URL".to_string());
        }
    }
    let (db_path, result) = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard.hdhive_base_url = normalized;
        if let Some(value) = secret.filter(|value| !value.trim().is_empty()) {
            guard.hdhive_secret = value.trim().to_string();
        }
        let result = HdhivePublicConfig {
            configured: !guard.hdhive_base_url.is_empty() && !guard.hdhive_secret.is_empty(),
            base_url: guard.hdhive_base_url.clone(),
            instance_id: guard.hdhive_instance_id.clone(),
        };
        (guard.db_path.clone(), result)
    };
    save_app_state(&db_path, "hdhive_base_url", &result.base_url)?;
    let secret_value = state
        .lock()
        .map_err(|error| error.to_string())?
        .hdhive_secret
        .clone();
    save_app_state(&db_path, "hdhive_secret", &secret_value)?;
    emit_state(&app, state.inner());
    Ok(result)
}

#[tauri::command]
fn update_mapping_auto_share(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
    auto_share: bool,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    if auto_share && (guard.hdhive_base_url.is_empty() || guard.hdhive_secret.is_empty()) {
        return Err("开启自动分享前请先配置 Hdhive 地址和密钥".to_string());
    }
    let mapping = guard
        .mappings
        .iter_mut()
        .find(|mapping| mapping.id == id)
        .ok_or_else(|| "备份任务不存在".to_string())?;
    mapping.auto_share = auto_share;
    save_config(&guard);
    drop(guard);
    emit_state(&app, state.inner());
    Ok(())
}

#[tauri::command]
async fn backfill_auto_shares(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<usize, String> {
    let (mapping, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        let mapping = guard
            .mappings
            .iter()
            .find(|mapping| mapping.id == id)
            .cloned()
            .ok_or_else(|| "备份任务不存在".to_string())?;
        if !mapping.auto_share {
            return Err("请先开启该任务的自动分享".to_string());
        }
        (mapping, guard.db_path.clone())
    };
    let rows = {
        let connection = open_database(&db_path)?;
        let mut statement = connection
            .prepare(
                "SELECT file_path, size, modified_ms, remote_file_id FROM uploaded_files
                 WHERE mapping_id=?1 AND remote_file_id IS NOT NULL AND remote_file_id <> ''",
            )
            .map_err(|error| format!("读取已有上传记录失败：{error}"))?;
        let rows = statement
            .query_map(params![id], |row| {
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
        };
        let outcome = UploadOutcome {
            task_id: String::new(),
            remote_file_id: Some(remote_file_id),
        };
        schedule_auto_share(state.inner(), &item, &outcome).await?;
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
async fn retry_auto_share_event(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    event_id: String,
    tmdb_id: Option<String>,
    media_type: Option<String>,
) -> Result<Value, String> {
    let (base_url, secret, instance_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
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
                reqwest::Method::POST,
                "/api/integrations/guangya-sync/events",
                Some(&payload),
            )
            .await?,
            "Hdhive 已重新接收投稿事件",
        )
    } else {
        let endpoint = format!("/api/integrations/guangya-sync/events/{event_id}/retry");
        (
            hdhive_request(
                &base_url,
                &secret,
                &instance_id,
                reqwest::Method::POST,
                &endpoint,
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
#[tauri::command]
fn pause_queue(app: tauri::AppHandle, state: tauri::State<'_, SharedState>) {
    if let Ok(mut guard) = state.lock() {
        guard.paused = true;
    }
    emit_state(&app, state.inner());
}
#[tauri::command]
async fn resume_queue(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    if let Ok(mut guard) = state.lock() {
        guard.paused = false;
    }
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(())
}

async fn event_loop(app: tauri::AppHandle, state: SharedState, mut rx: UnboundedReceiver<FsEvent>) {
    while let Some(event) = rx.recv().await {
        let app = app.clone();
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            sleep(Duration::from_millis(900)).await;
            enqueue_path(&app, &state, event).await;
        });
    }
}

fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (event_tx, event_rx) = mpsc::unbounded_channel();
            let config_path = app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())?
                .join("config.json");
            let db_path = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("state.sqlite3");
            init_database(&db_path)?;
            let auth_session = load_auth_session(&db_path)?;
            let upload_history = load_upload_history(&db_path)?;
            let device_id = load_or_create_device_id(&db_path)?;
            let hdhive_base_url = std::env::var("HDHIVE_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_base_url")?)
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string();
            let hdhive_secret = std::env::var("HDHIVE_GUANGYA_SYNC_SECRET")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_secret")?)
                .unwrap_or_default();
            let hdhive_instance_id = std::env::var("HDHIVE_GUANGYA_SYNC_INSTANCE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_instance_id")?)
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            save_app_state(&db_path, "hdhive_instance_id", &hdhive_instance_id)?;
            let config = load_config(&config_path);
            let mappings = config
                .mappings
                .into_iter()
                .map(|mut mapping| {
                    mapping.sync_types = normalize_sync_types(&mapping.sync_types);
                    mapping.monitor_mode = normalize_monitor_mode(&mapping.monitor_mode);
                    mapping
                })
                .collect::<Vec<_>>();
            let state = Arc::new(Mutex::new(RuntimeState {
                token: auth_session.access_token,
                refresh_token: auth_session.refresh_token,
                config_path,
                db_path,
                mappings: mappings.clone(),
                saved_shares: config.saved_shares,
                queue: VecDeque::new(),
                waiting_files: HashMap::new(),
                history: upload_history,
                inflight: HashMap::new(),
                inflight_items: HashMap::new(),
                remote_cache: HashMap::from([(String::new(), String::new())]),
                watchers: HashMap::new(),
                event_tx,
                paused: false,
                active_uploads: 0,
                device_id,
                hdhive_base_url,
                hdhive_secret,
                hdhive_instance_id,
                auto_share_processing: HashSet::new(),
            }));
            app.manage(state.clone());
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(event_loop(app_handle.clone(), state.clone(), event_rx));
            if state
                .lock()
                .ok()
                .and_then(|guard| guard.refresh_token.clone())
                .is_some()
            {
                let refresh_app = app_handle.clone();
                let refresh_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        refresh_saved_session(refresh_app.clone(), refresh_state).await
                    {
                        status(
                            &refresh_app,
                            "warning",
                            format!("已恢复上次登录，但刷新会话失败：{error}"),
                        );
                    }
                });
            }
            for mapping in mappings {
                if mapping.enabled {
                    match install_watcher(&state, &mapping) {
                        Ok(()) => {
                            if mapping.scan_existing {
                                enqueue_existing_files(&app_handle, &state, &mapping);
                            } else if mapping.monitor_mode == "polling" {
                                seed_existing_files(&state, &mapping);
                            }
                        }
                        Err(error) => {
                            if let Ok(mut guard) = state.lock() {
                                if let Some(current) = guard
                                    .mappings
                                    .iter_mut()
                                    .find(|current| current.id == mapping.id)
                                {
                                    current.enabled = false;
                                    current.watch_error = Some(error.clone());
                                }
                                save_config(&guard);
                            }
                            status(
                                &app_handle,
                                "error",
                                format!("备份任务监控启动失败：{}：{}", mapping.local_path, error),
                            );
                        }
                    }
                }
            }
            tauri::async_runtime::spawn(polling_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(auto_share_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(token_refresh_loop(app_handle.clone(), state.clone()));
            emit_state(&app_handle, &state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            start_device_login,
            poll_device_login,
            get_overview,
            list_files,
            select_upload_files,
            select_upload_folder,
            queue_upload_paths,
            copy_files,
            move_files,
            delete_files,
            batch_rename_files,
            create_share,
            list_shares,
            delete_shares,
            open_received_share,
            list_received_share_files,
            restore_received_share,
            get_received_share_download,
            get_cloud_download,
            create_offline_task,
            list_offline_tasks,
            save_share_link,
            remove_share_link,
            open_login,
            capture_token,
            select_folder,
            add_mapping,
            remove_mapping,
            toggle_mapping,
            update_mapping_sync_types,
            update_mapping_monitor_mode,
            update_mapping_auto_share,
            update_hdhive_config,
            backfill_auto_shares,
            retry_auto_share_event,
            pause_queue,
            resume_queue
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mapping_syncs_media_only() {
        let mapping: Mapping = serde_json::from_value(json!({
            "id": "mapping-1",
            "local_path": "C:/watch",
            "remote_path": "",
            "enabled": true
        }))
        .expect("mapping should deserialize");

        assert_eq!(mapping.sync_types, default_sync_types());
        assert!(!mapping.auto_share);
        assert!(should_sync(Path::new("photo.HEIC"), &mapping.sync_types));
        assert!(should_sync(Path::new("movie.mkv"), &mapping.sync_types));
        assert!(should_sync(Path::new("sound.flac"), &mapping.sync_types));
        assert!(!should_sync(Path::new("notes.pdf"), &mapping.sync_types));
    }

    #[test]
    fn auto_share_uses_sync_root_first_level() {
        let root_file = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: PathBuf::from("C:/watch/movie.mkv"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "movie.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 1,
            modified_ms: 1,
        };
        let episode = UploadItem {
            relative_path: "tvname/season 1/s01.mkv".to_string(),
            file_path: PathBuf::from("C:/watch/tvname/season 1/s01.mkv"),
            ..root_file.clone()
        };
        let next_season = UploadItem {
            relative_path: "tvname/season 2/s02.mkv".to_string(),
            file_path: PathBuf::from("C:/watch/tvname/season 2/s02.mkv"),
            ..root_file.clone()
        };
        let file_target = auto_share_target(&root_file).expect("root file target");
        assert_eq!(file_target.key, "movie.mkv");
        assert_eq!(file_target.target_type, "file");
        let episode_target = auto_share_target(&episode).expect("episode target");
        assert_eq!(episode_target.key, "tvname");
        assert_eq!(episode_target.target_type, "folder");
        assert_eq!(auto_share_target(&next_season).unwrap().key, "tvname");
    }

    #[test]
    fn hdhive_hmac_matches_node_and_backend() {
        assert_eq!(
            hdhive_signature(
                "secret",
                "post",
                "/api/integrations/guangya-sync/events",
                r#"{"a":1}"#,
                "1700000000",
            ),
            "v1=83db0943a113d8cdd5786f9447ebf125c764a64fb935b577f43aae6a2a8c5c5d"
        );
    }

    #[test]
    fn share_file_payload_matches_official_web_contract() {
        let payload = share_file_payload(&["file-1".to_string()], "测试分享");

        assert_eq!(
            payload,
            json!({
                "fileIds": ["file-1"],
                "title": "测试分享",
                "validateDuration": 0,
                "shareType": 0,
                "code": "",
                "autoFillCode": false,
                "trafficLimit": "0",
                "maxRestoreCount": 0,
                "downloadType": 1,
                "shareTemplate": DEFAULT_SHARE_TEMPLATE
            })
        );
        assert_eq!(
            share_file_payload(&["file-1".to_string()], "   ")["title"],
            "云盘分享"
        );
    }

    #[test]
    fn parses_guangya_share_links_with_access_codes() {
        let parsed = parse_guangya_share_link(
            "https://www.guangyapan.com/s/1926585463106830337_al8cmYXLP9l33ld2?code=iv5k#/share",
        )
        .unwrap();
        assert_eq!(parsed.0, "1926585463106830337_al8cmYXLP9l33ld2");
        assert_eq!(parsed.1, "iv5k");
        assert!(parse_guangya_share_link("https://example.com/s/share-1").is_err());
    }

    #[test]
    fn uses_the_official_gcid_chunk_boundaries() {
        assert_eq!(gcid_chunk_size(128 * 1024 * 1024), 256 * 1024);
        assert_eq!(gcid_chunk_size(128 * 1024 * 1024 + 1), 512 * 1024);
        assert_eq!(gcid_chunk_size(256 * 1024 * 1024), 512 * 1024);
        assert_eq!(gcid_chunk_size(256 * 1024 * 1024 + 1), 1024 * 1024);
        assert_eq!(gcid_chunk_size(512 * 1024 * 1024 + 1), 2 * 1024 * 1024);
    }

    #[test]
    fn manual_share_event_is_a_new_hdhive_submission() {
        let share_data = json!({ "shareId": "1927007413038006365" });
        assert_eq!(
            share_id_for_hdhive(
                &share_data,
                "https://www.guangyapan.com/s/1927007413038006365_al3JUAaZz30d4FPe"
            ),
            "1927007413038006365_al3JUAaZz30d4FPe"
        );
        let payload = manual_share_event_payload(
            "00000000-0000-4000-8000-000000000001",
            &["folder-1".to_string()],
            "测试电视剧",
            "folder",
            "share-1",
            "https://www.guangyapan.com/s/share-1",
            "new",
        );

        assert_eq!(payload["mapping_id"], "__manual__");
        assert_eq!(payload["target_key"], "测试电视剧");
        assert_eq!(payload["target_type"], "folder");
        assert_eq!(payload["remote_target_id"], "folder-1");
        assert_eq!(payload["share_id"], "share-1");
        assert_eq!(payload["intent"], "new");

        let update_payload = manual_share_event_payload(
            "00000000-0000-4000-8000-000000000002",
            &["folder-1".to_string()],
            "测试电视剧",
            "folder",
            "share-1",
            "https://www.guangyapan.com/s/share-1",
            "update",
        );
        assert_eq!(update_payload["intent"], "update");
    }

    #[test]
    fn selected_sync_types_use_direct_extensions() {
        let selected = vec![".xlsx".to_string(), "srt".to_string(), "sqlite".to_string()];

        assert!(should_sync(Path::new("report.xlsx"), &selected));
        assert!(should_sync(Path::new("movie.srt"), &selected));
        assert!(should_sync(Path::new("database.sqlite"), &selected));
        assert!(!should_sync(Path::new("cover.jpg"), &selected));
    }

    #[test]
    fn directory_watch_events_expand_to_nested_syncable_files() {
        let root = std::env::temp_dir().join(format!("guangya-folder-event-{}", Uuid::new_v4()));
        let nested = root.join("season 1");
        fs::create_dir_all(&nested).expect("create nested fixture");
        fs::write(nested.join("episode-01.mp4"), b"video-1").expect("write first video");
        fs::write(nested.join("episode-02.mkv"), b"video-2").expect("write second video");
        fs::write(nested.join("notes.txt"), b"ignored").expect("write ignored fixture");

        let mut files = collect_watch_event_files(&root, &["mp4".to_string(), "mkv".to_string()]);
        files.sort();

        assert_eq!(
            files,
            vec![nested.join("episode-01.mp4"), nested.join("episode-02.mkv")]
        );
        fs::remove_dir_all(root).expect("remove directory event fixture");
    }

    #[test]
    fn invalid_or_empty_sync_types_fall_back_to_media() {
        assert_eq!(normalize_sync_types(&[]), default_sync_types());
        assert_eq!(
            normalize_sync_types(&["bad/name".to_string()]),
            default_sync_types()
        );
        assert_eq!(normalize_sync_types(&[".MP4".to_string()]), vec!["mp4"]);
    }

    #[test]
    fn duplicate_native_events_do_not_queue_an_inflight_file_again() {
        let item = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: PathBuf::from("C:/watch/photo.png"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "photo.png".to_string(),
            change_kind: "added".to_string(),
            size: 128,
            modified_ms: 42,
        };
        let history = HashMap::new();
        let mut inflight = HashMap::new();
        let queue = VecDeque::new();
        let mut waiting_files = HashMap::new();
        assert!(!upload_already_scheduled(
            &history,
            &inflight,
            &queue,
            &waiting_files,
            &item
        ));

        inflight.insert(
            item_key(&item.mapping_id, &item.file_path),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &inflight,
            &queue,
            &waiting_files,
            &item
        ));

        let mut changed = item.clone();
        changed.modified_ms += 1;
        assert!(!upload_already_scheduled(
            &history,
            &inflight,
            &queue,
            &waiting_files,
            &changed
        ));

        inflight.clear();
        waiting_files.insert(item_key(&item.mapping_id, &item.file_path), item.clone());
        assert!(upload_already_scheduled(
            &history,
            &inflight,
            &queue,
            &waiting_files,
            &item
        ));
    }

    #[cfg(windows)]
    #[test]
    fn detects_a_file_exclusively_opened_by_another_program() {
        use std::os::windows::fs::OpenOptionsExt;

        let path = std::env::temp_dir().join(format!("guangya-locked-{}.tmp", Uuid::new_v4()));
        fs::write(&path, b"locked").expect("write fixture");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .expect("hold fixture exclusively");
        assert!(!file_available_for_upload(&path).expect("probe locked file"));
        drop(held);
        assert!(file_available_for_upload(&path).expect("probe released file"));
        fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn monitor_mode_defaults_to_native_and_accepts_polling() {
        assert_eq!(normalize_monitor_mode(""), "native");
        assert_eq!(normalize_monitor_mode("local"), "native");
        assert_eq!(normalize_monitor_mode("POLLING"), "polling");
    }

    #[test]
    fn oss_parameters_are_normalized_for_the_rust_client() {
        assert_eq!(
            normalize_oss_endpoint_url(
                "https://bucket.oss-cn-shanghai.aliyuncs.com/path",
                "bucket"
            ),
            "https://oss-cn-shanghai.aliyuncs.com"
        );
        assert_eq!(
            normalize_oss_endpoint_url("http://oss-cn-hangzhou.aliyuncs.com", "bucket"),
            "http://oss-cn-hangzhou.aliyuncs.com"
        );
    }

    #[test]
    fn multipart_part_size_matches_the_official_upload_tiers() {
        assert_eq!(oss_part_size(100 * 1024 * 1024), 1024 * 1024);
        assert_eq!(oss_part_size(101 * 1024 * 1024), 2 * 1024 * 1024);
        assert_eq!(oss_part_size(2 * 1024 * 1024 * 1024), 4 * 1024 * 1024);
        assert_eq!(oss_part_size(11 * 1024 * 1024 * 1024), 8 * 1024 * 1024);
    }

    #[test]
    fn cloud_index_processing_messages_are_retried() {
        assert!(is_cloud_index_pending_message("文件上传中"));
        assert!(is_cloud_index_pending_message("任务处理中，请稍后再试"));
        assert!(!is_cloud_index_pending_message("文件违规，无法入库"));
    }

    #[test]
    fn download_names_are_safe_and_collisions_are_preserved() {
        assert_eq!(
            safe_download_name(" 剧集:S01/E01?.mkv "),
            "剧集_S01_E01_.mkv"
        );
        assert_eq!(safe_download_name("..."), "光鸭下载");

        let root = std::env::temp_dir().join(format!("guangya-download-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("download test directory should exist");
        fs::write(root.join("episode.mkv"), b"existing").expect("existing file should be created");
        assert_eq!(
            available_download_path(&root, "episode.mkv"),
            root.join("episode (1).mkv")
        );
        fs::remove_dir_all(root).expect("download test directory should be removable");
    }

    #[test]
    fn sqlite_persists_auth_device_and_uploaded_file_history() {
        let root = std::env::temp_dir().join(format!("guangya-sqlite-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("database should initialize");
        save_auth_session(&database, Some("access-token"), Some("refresh-token"))
            .expect("auth should persist");
        let auth = load_auth_session(&database).expect("auth should load");
        assert_eq!(auth.access_token.as_deref(), Some("access-token"));
        assert_eq!(auth.refresh_token.as_deref(), Some("refresh-token"));
        let device_id = load_or_create_device_id(&database).expect("device id should persist");
        assert_eq!(
            load_or_create_device_id(&database).expect("device id should reload"),
            device_id
        );

        let item = UploadItem {
            mapping_id: "mapping-1".into(),
            file_path: PathBuf::from("H:/test/photo.png"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "photo.png".to_string(),
            change_kind: "added".to_string(),
            size: 128,
            modified_ms: 42,
        };
        save_upload_history(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-1".into(),
                remote_file_id: Some("file-1".into()),
            },
        )
        .expect("upload history should persist");
        let history = load_upload_history(&database).expect("upload history should load");
        assert_eq!(
            history.get(&item_key(&item.mapping_id, &item.file_path)),
            Some(&Stamp {
                size: 128,
                modified_ms: 42
            })
        );
        save_upload_history(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-pending".into(),
                remote_file_id: None,
            },
        )
        .expect("OSS-complete history should persist before cloud indexing");
        let connection = open_database(&database).expect("database should reopen");
        let (task_id, remote_file_id): (String, Option<String>) = connection
            .query_row(
                "SELECT task_id, remote_file_id FROM uploaded_files WHERE mapping_id = ?1 AND file_path = ?2",
                params![item.mapping_id, item.file_path.to_string_lossy()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("pending upload should be queryable");
        assert_eq!(task_id, "task-pending");
        assert_eq!(remote_file_id, None);
        drop(connection);
        remove_mapping_history(&database, &item.mapping_id).expect("history should be removable");
        assert!(load_upload_history(&database)
            .expect("history should reload")
            .is_empty());
        fs::remove_dir_all(root).expect("test database should be removable");
    }
}

fn main() {
    run();
}
