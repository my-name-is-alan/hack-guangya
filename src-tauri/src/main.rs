#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod native_mount;
mod webdav;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use hmac::{Hmac, Mac};
use md5::Md5;
use native_mount::{NativeMountInfo, NativeMountManager, NativeMountOptions};
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, DATE, ETAG};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, OpenOptions},
    io::{self, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot, Semaphore,
    },
    time::{sleep, Duration, Instant},
};
use uuid::Uuid;

const API_BASE: &str = "https://api.guangyapan.com";
const ACCOUNT_BASE: &str = "https://account.guangyapan.com";
const OAUTH_CLIENT_ID: &str = "aMe-8VSlkrbQXpUR";
const AUTH_URL: &str = "https://www.guangyapan.com/#/";
const DEFAULT_UPLOAD_CONCURRENCY: usize = 2;
const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 2;
const MAX_TRANSFER_CONCURRENCY: usize = 8;
const FLASH_PREFLIGHT_CONCURRENCY: usize = 1;
const FLASH_PREFLIGHT_TOKEN_MAX_AGE_SECS: u64 = 10 * 60;
const DEFAULT_MULTIPART_PART_SIZE: &str = "auto";
const MULTIPART_PART_SIZE_OPTIONS: &[&str] = &["auto", "4m", "8m", "16m"];
const DEFAULT_OSS_PART_CONCURRENCY: usize = 4;
const MAX_OSS_PART_CONCURRENCY: usize = 8;
const DEFAULT_CACHE_MAX_ENTRIES: usize = 10_000;
const MIN_CACHE_MAX_ENTRIES: usize = 100;
const MAX_CACHE_MAX_ENTRIES: usize = 100_000;
const OSS_WRITE_RETRY_TIMES: usize = 5;
const FILE_STABILITY_WAIT_MS: u64 = 1_200;
const FILE_BUSY_RETRY_SECS: u64 = 3;
const POLL_INTERVAL_SECS: u64 = 5;
const API_CONNECT_TIMEOUT_SECS: u64 = 15;
const API_REQUEST_TIMEOUT_SECS: u64 = 120;
const OSS_REQUEST_TIMEOUT_SECS: u64 = 600;
const OSS_MULTIPART_TARGET_PARTS: u64 = 9_000;
const OSS_MIB: u64 = 1024 * 1024;
const OSS_LARGE_FILE_PART_SIZE: u64 = 16 * OSS_MIB;
const MAX_OSS_BUFFER_BYTES_PER_FILE: u64 = 128 * OSS_MIB;
const OSS_BUFFER_PERMIT_BYTES: u64 = 4 * OSS_MIB;
const MAX_TOTAL_OSS_BUFFER_BYTES: u64 = 256 * OSS_MIB;
const OSS_CHECKPOINT_FLUSH_BYTES: u64 = 64 * OSS_MIB;
const OSS_CHECKPOINT_FLUSH_INTERVAL_SECS: u64 = 2;
const OSS_SPEED_WINDOW_SECS: u64 = 5;
const GCID_PROGRESS_EMIT_INTERVAL_MS: u64 = 250;
const CLOUD_CONFIRM_TIMEOUT_SECS: u64 = 600;
const PENDING_UPLOAD_RETRY_SECS: u64 = 15;
const UPLOAD_STATE_OSS_COMPLETE: &str = "oss_complete";
const UPLOAD_STATE_CLOUD_CONFIRMED: &str = "cloud_confirmed";
const AUTO_SHARE_QUIET_SECS: i64 = 30;
const TOKEN_REFRESH_INTERVAL_SECS: u64 = 20 * 60;
const DEFAULT_WEBDAV_PORT: u16 = 19_090;
const DEFAULT_WEBDAV_USERNAME: &str = "guangya";
const MAX_GCID_IMPORT_CONCURRENCY: usize = 16;
const MAX_GCID_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_GCID_IMPORT_ATTEMPTS: i64 = 5;
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
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "rtf", "odt", "ods",
    "odp", "epub",
];
const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "zipx", "iso",
];
const CLOUD_FILE_TYPE_IMAGE: u8 = 1;
const CLOUD_FILE_TYPE_VIDEO: u8 = 2;
const CLOUD_FILE_TYPE_AUDIO: u8 = 3;
const CLOUD_FILE_TYPE_DOCUMENT: u8 = 4;
const CLOUD_FILE_TYPE_ARCHIVE: u8 = 5;
type SharedState = Arc<Mutex<RuntimeState>>;

#[derive(Clone)]
struct ApiAuthRuntime {
    app: tauri::AppHandle,
    state: SharedState,
}

static API_AUTH_RUNTIME: OnceLock<ApiAuthRuntime> = OnceLock::new();

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
#[derive(Debug, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    mappings: Vec<Mapping>,
    #[serde(default)]
    saved_shares: Vec<SavedShare>,
    #[serde(default = "default_upload_concurrency")]
    upload_concurrency: usize,
    #[serde(default = "default_download_concurrency")]
    download_concurrency: usize,
    #[serde(default = "default_multipart_part_size")]
    multipart_part_size: String,
    #[serde(default = "default_oss_part_concurrency")]
    oss_part_concurrency: usize,
    #[serde(default = "default_true")]
    webdav_enabled: bool,
    #[serde(default = "default_webdav_port")]
    webdav_port: u16,
    #[serde(default = "default_webdav_username")]
    webdav_username: String,
    #[serde(default, skip_serializing)]
    webdav_password: String,
    #[serde(default)]
    webdav_password_hash: String,
    #[serde(default)]
    native_mount: NativeMountOptions,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mappings: Vec::new(),
            saved_shares: Vec::new(),
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
            multipart_part_size: default_multipart_part_size(),
            oss_part_concurrency: DEFAULT_OSS_PART_CONCURRENCY,
            webdav_enabled: true,
            webdav_port: DEFAULT_WEBDAV_PORT,
            webdav_username: default_webdav_username(),
            webdav_password: String::new(),
            webdav_password_hash: String::new(),
            native_mount: NativeMountOptions::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
struct Snapshot {
    logged_in: bool,
    paused: bool,
    pending: usize,
    active_uploads: usize,
    upload_concurrency: usize,
    download_concurrency: usize,
    multipart_part_size: String,
    oss_part_concurrency: usize,
    mappings: Vec<Mapping>,
    saved_shares: Vec<SavedShare>,
    hdhive: HdhivePublicConfig,
    auto_share_receipts: Vec<AutoShareReceipt>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    flash_preflight_cache: HashMap<String, FlashPreflightCache>,
    waiting_files: HashMap<String, UploadItem>,
    history: HashMap<String, Stamp>,
    pending_cloud: HashMap<String, Stamp>,
    recovering_pending: HashSet<String>,
    inflight: HashMap<String, Stamp>,
    inflight_items: HashMap<String, UploadItem>,
    remote_cache: HashMap<String, String>,
    watchers: HashMap<String, RecommendedWatcher>,
    event_tx: UnboundedSender<FsEvent>,
    paused: bool,
    active_uploads: usize,
    active_flash_preflights: usize,
    upload_concurrency: usize,
    download_concurrency: usize,
    multipart_part_size: String,
    oss_part_concurrency: usize,
    oss_part_semaphore: Arc<Semaphore>,
    upload_checkpoint_writer: UploadCheckpointWriter,
    active_oss_uploads: usize,
    cache_enabled: bool,
    cache_max_entries: usize,
    device_id: String,
    hdhive_enabled: bool,
    hdhive_base_url: String,
    hdhive_secret: String,
    hdhive_instance_id: String,
    auto_share_processing: HashSet<String>,
    gcid_import_running: HashSet<String>,
    sms_verifications: HashMap<String, SmsVerificationSession>,
    webdav_enabled: bool,
    webdav_port: u16,
    webdav_username: String,
    webdav_password_hash: String,
    webdav_session_password: Option<String>,
    webdav_password_verifier: Option<[u8; 32]>,
    webdav_running: bool,
    webdav_error: Option<String>,
    native_mount: Arc<Mutex<NativeMountManager>>,
    auth_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    refreshed_access_tokens: HashMap<String, String>,
}

struct ActiveOssUploadGuard {
    state: SharedState,
}

impl ActiveOssUploadGuard {
    fn new(state: &SharedState) -> Result<Self, String> {
        state
            .lock()
            .map_err(|error| error.to_string())?
            .active_oss_uploads += 1;
        Ok(Self {
            state: state.clone(),
        })
    }
}

impl Drop for ActiveOssUploadGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.active_oss_uploads = guard.active_oss_uploads.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct HdhivePublicConfig {
    enabled: bool,
    configured: bool,
    base_url: String,
    instance_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TransferSettings {
    upload_concurrency: usize,
    download_concurrency: usize,
    multipart_part_size: String,
    oss_part_concurrency: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
struct CacheSettings {
    enabled: bool,
    max_entries: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct MetadataCacheStats {
    bytes: u64,
    entries: u64,
    file_fingerprints_bytes: u64,
    file_fingerprints_entries: u64,
    remote_cache_bytes: u64,
    remote_cache_entries: u64,
    policy: CacheSettings,
}

#[derive(Debug, Clone, Serialize)]
struct MountInfo {
    enabled: bool,
    running: bool,
    configured: bool,
    local_only: bool,
    endpoint: String,
    username: String,
    password: String,
    password_hint: String,
    error: Option<String>,
    protocol: String,
}

#[derive(Debug, Clone)]
struct SmsVerificationSession {
    phone_number: String,
    is_user: bool,
    captcha_token: Option<String>,
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
#[derive(Debug, Clone, Deserialize)]
struct UploadCredentials {
    #[serde(rename = "accessKeyID")]
    access_key_id: String,
    #[serde(rename = "secretAccessKey", alias = "accessKeySecret")]
    secret_access_key: String,
    #[serde(rename = "sessionToken")]
    session_token: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadToken {
    task_id: String,
    object_path: Option<String>,
    bucket_name: Option<String>,
    end_point: Option<String>,
    creds: Option<UploadCredentials>,
    #[serde(default)]
    provider: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OssUploadCheckpoint {
    task_id: String,
    object_path: String,
    bucket_name: String,
    end_point: String,
    provider: Option<Value>,
    upload_id: String,
    part_size: u64,
    completed_parts: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
struct PersistedUploadCheckpoint {
    checkpoint: OssUploadCheckpoint,
    uploaded_bytes: u64,
}

#[derive(Clone)]
struct UploadCheckpointWriter {
    sender: UnboundedSender<UploadCheckpointWriterCommand>,
}

#[derive(Clone)]
struct UploadCheckpointWrite {
    item: UploadItem,
    checkpoint: OssUploadCheckpoint,
    uploaded_bytes: u64,
}

enum UploadCheckpointWriterCommand {
    Save(UploadCheckpointWrite),
    Flush {
        key: String,
        latest: Option<UploadCheckpointWrite>,
        acknowledgement: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Default)]
struct UploadCheckpointWriteBatch {
    writes: HashMap<String, UploadCheckpointWrite>,
    acknowledgements: HashMap<String, Vec<oneshot::Sender<Result<(), String>>>>,
}

#[derive(Debug, Clone, Copy)]
struct UploadSpeedSample {
    bytes_per_second: f64,
    average_bytes_per_second: f64,
}

struct UploadSpeedTracker {
    samples: VecDeque<(Duration, u64)>,
    completed_parts: usize,
    minimum_samples: usize,
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

#[derive(Debug)]
struct FlashPreflightCache {
    stamp: Stamp,
    upload_token: Option<UploadToken>,
    created_at: Instant,
}

enum FlashPreflightOutcome {
    Accepted {
        task_id: String,
        token: String,
        device_id: String,
    },
    Miss(UploadToken),
    Skipped,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GcidExport {
    source: String,
    hash_type: String,
    #[serde(default)]
    uses_gcid_in_export: bool,
    #[serde(default)]
    common_path: String,
    #[serde(default)]
    total_files_count: Option<u64>,
    #[serde(default)]
    total_size: Option<Value>,
    files: Vec<GcidExportFile>,
}

#[derive(Debug, Deserialize)]
struct GcidExportFile {
    path: String,
    size: Value,
    gcid: String,
}

#[derive(Debug, Clone)]
struct GcidImportFile {
    path: String,
    folder_path: String,
    name: String,
    size: u64,
    gcid: String,
    attempts: i64,
}

#[derive(Debug)]
enum GcidImportOutcome {
    Imported { task_id: String, file_id: String },
    Existing { file_id: String },
    Missed { task_id: String },
    Conflict(String),
}

#[derive(Debug, Clone, Serialize)]
struct GcidImportSourceInfo {
    path: String,
    name: String,
    size: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
struct GcidImportCounts {
    pending: u64,
    processing: u64,
    imported: u64,
    existing: u64,
    missed: u64,
    conflict: u64,
    failed: u64,
}

#[derive(Debug, Clone, Serialize)]
struct GcidImportStatus {
    job_id: String,
    source_path: String,
    source_name: String,
    destination_parent_id: String,
    destination_name: String,
    total_files: u64,
    total_size: String,
    status: String,
    current_path: String,
    error: Option<String>,
    counts: GcidImportCounts,
    finished: u64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct PendingUpload {
    item: UploadItem,
    task_id: String,
}

#[derive(Debug)]
enum CloudTaskCheck {
    Confirmed(Value),
    Pending,
}

#[derive(Debug)]
enum CloudConfirmError {
    Retryable(String),
    Permanent(String),
}

impl CloudConfirmError {
    fn message(&self) -> &str {
        match self {
            Self::Retryable(message) | Self::Permanent(message) => message,
        }
    }
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
        pending: state.queue.len() + state.waiting_files.len() + state.pending_cloud.len(),
        active_uploads: state.active_uploads,
        upload_concurrency: state.upload_concurrency,
        download_concurrency: state.download_concurrency,
        multipart_part_size: state.multipart_part_size.clone(),
        oss_part_concurrency: state.oss_part_concurrency,
        mappings: state.mappings.clone(),
        saved_shares: state.saved_shares.clone(),
        hdhive: HdhivePublicConfig {
            enabled: state.hdhive_enabled,
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
fn default_upload_concurrency() -> usize {
    DEFAULT_UPLOAD_CONCURRENCY
}
fn default_download_concurrency() -> usize {
    DEFAULT_DOWNLOAD_CONCURRENCY
}
fn default_multipart_part_size() -> String {
    DEFAULT_MULTIPART_PART_SIZE.to_string()
}
fn default_oss_part_concurrency() -> usize {
    DEFAULT_OSS_PART_CONCURRENCY
}
fn default_webdav_port() -> u16 {
    DEFAULT_WEBDAV_PORT
}
fn default_webdav_username() -> String {
    DEFAULT_WEBDAV_USERNAME.to_string()
}
fn normalize_webdav_username(value: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.chars().count() < 3
        || normalized.chars().count() > 64
        || normalized.contains(':')
        || normalized.chars().any(char::is_control)
    {
        return Err("WebDAV 用户名必须为 3 到 64 个字符，且不能包含冒号或控制字符".to_string());
    }
    Ok(normalized.to_string())
}
fn normalize_webdav_password(value: &str) -> Result<String, String> {
    if value.chars().count() < 12 || value.chars().count() > 256 {
        return Err("WebDAV 密码必须为 12 到 256 个字符".to_string());
    }
    Ok(value.to_string())
}

const WEBDAV_PASSWORD_HASH_ITERATIONS: u32 = 120_000;

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut initial = Hmac::<Sha256>::new_from_slice(password).expect("HMAC accepts any key");
    initial.update(salt);
    initial.update(&1_u32.to_be_bytes());
    let mut previous = initial.finalize().into_bytes();
    let mut output = previous;
    for _ in 1..iterations.max(1) {
        let mut mac = Hmac::<Sha256>::new_from_slice(password).expect("HMAC accepts any key");
        mac.update(&previous);
        previous = mac.finalize().into_bytes();
        for (target, value) in output.iter_mut().zip(previous.iter()) {
            *target ^= value;
        }
    }
    output.into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn hash_webdav_password(password: &str) -> String {
    let salt = *Uuid::new_v4().as_bytes();
    let digest = pbkdf2_sha256(password.as_bytes(), &salt, WEBDAV_PASSWORD_HASH_ITERATIONS);
    format!(
        "pbkdf2-sha256${}${}${}",
        WEBDAV_PASSWORD_HASH_ITERATIONS,
        hex::encode(salt),
        hex::encode(digest)
    )
}

fn verify_webdav_password(password: &str, encoded: &str) -> bool {
    let mut parts = encoded.split('$');
    let Some("pbkdf2-sha256") = parts.next() else {
        return false;
    };
    let Some(iterations) = parts.next().and_then(|value| value.parse::<u32>().ok()) else {
        return false;
    };
    if !(10_000..=1_000_000).contains(&iterations) {
        return false;
    }
    let Some(salt) = parts.next().and_then(|value| hex::decode(value).ok()) else {
        return false;
    };
    let Some(expected) = parts.next().and_then(|value| hex::decode(value).ok()) else {
        return false;
    };
    if parts.next().is_some() || salt.len() < 16 || expected.len() != 32 {
        return false;
    }
    constant_time_eq(
        &pbkdf2_sha256(password.as_bytes(), &salt, iterations),
        &expected,
    )
}

fn webdav_password_verifier(password: &str) -> [u8; 32] {
    Sha256::digest(password.as_bytes()).into()
}
fn default_cache_enabled() -> bool {
    true
}
fn default_cache_max_entries() -> usize {
    DEFAULT_CACHE_MAX_ENTRIES
}
fn parse_cache_enabled(value: Option<&str>) -> bool {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("false" | "0" | "off" | "disabled") => false,
        Some("true" | "1" | "on" | "enabled") => true,
        _ => default_cache_enabled(),
    }
}
fn validate_cache_max_entries(value: usize) -> Result<usize, String> {
    if (MIN_CACHE_MAX_ENTRIES..=MAX_CACHE_MAX_ENTRIES).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "缓存条目上限必须在 {MIN_CACHE_MAX_ENTRIES}–{MAX_CACHE_MAX_ENTRIES} 之间"
        ))
    }
}
fn parse_cache_max_entries(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .and_then(|value| validate_cache_max_entries(value).ok())
        .unwrap_or_else(default_cache_max_entries)
}
fn cache_settings(state: &RuntimeState) -> CacheSettings {
    CacheSettings {
        enabled: state.cache_enabled,
        max_entries: state.cache_max_entries,
    }
}
fn default_hdhive_enabled() -> bool {
    true
}
fn parse_hdhive_enabled(value: Option<&str>) -> bool {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("false" | "0" | "off" | "disabled") => false,
        Some("true" | "1" | "on" | "enabled") => true,
        _ => default_hdhive_enabled(),
    }
}
fn hdhive_allowed_hosts() -> HashSet<String> {
    std::env::var("HDHIVE_ALLOWED_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}
fn normalize_hdhive_base_url_with_allowed_hosts(
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
fn normalize_hdhive_base_url(value: &str) -> Result<String, String> {
    normalize_hdhive_base_url_with_allowed_hosts(value, &hdhive_allowed_hosts())
}
fn build_hdhive_target_url(
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
fn safe_response_preview(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .filter_map(|character| match character {
            '\r' | '\n' | '\t' => Some(' '),
            character if character.is_control() => None,
            character => Some(character),
        })
        .take(max_chars)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn normalize_transfer_concurrency(value: usize, fallback: usize) -> usize {
    if (1..=MAX_TRANSFER_CONCURRENCY).contains(&value) {
        value
    } else {
        fallback
    }
}
fn validate_multipart_part_size(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if MULTIPART_PART_SIZE_OPTIONS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err("分片档位只支持 auto、4m、8m 或 16m".to_string())
    }
}
fn normalize_multipart_part_size(value: &str) -> String {
    validate_multipart_part_size(value).unwrap_or_else(|_| default_multipart_part_size())
}
fn normalize_oss_part_concurrency(value: usize) -> usize {
    if (1..=MAX_OSS_PART_CONCURRENCY).contains(&value) {
        value
    } else {
        DEFAULT_OSS_PART_CONCURRENCY
    }
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
fn ceil_div_u64(value: u64, divisor: u64) -> u64 {
    value / divisor + u64::from(value % divisor != 0)
}
fn oss_part_size(size: u64) -> u64 {
    let tier_size = if size <= 100 * 1024 * 1024 {
        4 * OSS_MIB
    } else if size <= 1024 * 1024 * 1024 {
        8 * OSS_MIB
    } else {
        OSS_LARGE_FILE_PART_SIZE
    };
    let minimum_size = ceil_div_u64(size, OSS_MULTIPART_TARGET_PARTS);
    let aligned_minimum_size = ceil_div_u64(minimum_size, OSS_MIB).saturating_mul(OSS_MIB);
    tier_size.max(aligned_minimum_size)
}
fn configured_oss_part_size(size: u64, multipart_part_size: &str) -> u64 {
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
fn effective_oss_part_concurrency(part_size: u64, requested: usize) -> usize {
    let requested = normalize_oss_part_concurrency(requested);
    let memory_limited = (MAX_OSS_BUFFER_BYTES_PER_FILE / part_size.max(1)).max(1);
    requested.min(usize::try_from(memory_limited).unwrap_or(usize::MAX))
}
fn oss_buffer_permits(part_size: u64) -> u32 {
    let total_permits = MAX_TOTAL_OSS_BUFFER_BYTES / OSS_BUFFER_PERMIT_BYTES;
    let required = ceil_div_u64(part_size.max(1), OSS_BUFFER_PERMIT_BYTES);
    u32::try_from(required.min(total_permits).max(1)).unwrap_or(u32::MAX)
}
fn should_flush_upload_checkpoint(
    last_saved_bytes: u64,
    uploaded_bytes: u64,
    total_bytes: u64,
    elapsed: Duration,
) -> bool {
    uploaded_bytes >= total_bytes
        || uploaded_bytes.saturating_sub(last_saved_bytes) >= OSS_CHECKPOINT_FLUSH_BYTES
        || elapsed >= Duration::from_secs(OSS_CHECKPOINT_FLUSH_INTERVAL_SECS)
}
impl UploadSpeedTracker {
    fn new(minimum_samples: usize) -> Self {
        Self {
            samples: VecDeque::from([(Duration::ZERO, 0)]),
            completed_parts: 0,
            minimum_samples: minimum_samples.max(1),
        }
    }

    fn observe(&mut self, elapsed: Duration, uploaded_bytes: u64) -> UploadSpeedSample {
        let elapsed_seconds = elapsed.as_secs_f64().max(0.001);
        let average_bytes_per_second = uploaded_bytes as f64 / elapsed_seconds;
        let uploaded_bytes = uploaded_bytes.max(
            self.samples
                .back()
                .map(|(_, bytes)| *bytes)
                .unwrap_or_default(),
        );
        self.completed_parts = self.completed_parts.saturating_add(1);
        self.samples.push_back((elapsed, uploaded_bytes));

        let cutoff = elapsed.saturating_sub(Duration::from_secs(OSS_SPEED_WINDOW_SECS));
        while self.samples.len() > 1
            && self
                .samples
                .get(1)
                .is_some_and(|(sample_elapsed, _)| *sample_elapsed <= cutoff)
        {
            self.samples.pop_front();
        }
        let (baseline_elapsed, baseline_bytes) =
            self.samples.front().copied().unwrap_or((Duration::ZERO, 0));
        let window_seconds = elapsed
            .saturating_sub(baseline_elapsed)
            .as_secs_f64()
            .max(0.001);
        let window_bytes_per_second =
            uploaded_bytes.saturating_sub(baseline_bytes) as f64 / window_seconds;

        // Completed-part accounting cannot see bytes that are still in flight.
        // Keep the public speed at zero until the entire first real request wave
        // has returned ETags, then report only measured window throughput.
        let bytes_per_second = if self.completed_parts >= self.minimum_samples {
            window_bytes_per_second
        } else {
            0.0
        };
        UploadSpeedSample {
            bytes_per_second,
            average_bytes_per_second,
        }
    }
}
fn should_emit_gcid_progress(
    last_emit_elapsed: Duration,
    last_percent: u64,
    elapsed: Duration,
    percent: u64,
    finished: bool,
) -> bool {
    finished
        || (percent != last_percent
            && elapsed.saturating_sub(last_emit_elapsed)
                >= Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS))
}
fn background_gcid_may_run(active_oss_uploads: usize) -> bool {
    active_oss_uploads == 0
}
fn normalize_monitor_mode(value: &str) -> String {
    if value.eq_ignore_ascii_case("polling") {
        "polling".to_string()
    } else {
        default_monitor_mode()
    }
}
fn destination_item_key(
    mapping_id: &str,
    path: &Path,
    remote_parent_id: &str,
    remote_dir: &str,
) -> String {
    format!(
        "{mapping_id}::{}\0{remote_parent_id}\0{remote_dir}",
        path.to_string_lossy()
    )
}
fn item_key(item: &UploadItem) -> String {
    destination_item_key(
        &item.mapping_id,
        &item.file_path,
        &item.remote_parent_id,
        &item.remote_dir,
    )
}
fn canonical_existing_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("无法读取{label}的真实路径：{error}"))?;
    if !canonical.is_dir() {
        return Err(format!("{label}不是目录"));
    }
    Ok(canonical)
}
fn canonical_watch_path(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("读取监控事件路径失败：{error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("监控事件路径不能是符号链接".to_string());
    }
    let canonical_root = canonical_existing_directory(root, "监控目录")?;
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("无法读取监控事件真实路径：{error}"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err("监控事件真实路径不在被监控目录内".to_string());
    }
    Ok(canonical)
}
fn canonical_upload_source(path: &Path, root: Option<&Path>) -> Result<PathBuf, String> {
    let link_metadata =
        fs::symlink_metadata(path).map_err(|error| format!("读取源文件失败：{error}"))?;
    if link_metadata.file_type().is_symlink() {
        return Err("源文件不能是符号链接".to_string());
    }
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("无法读取源文件真实路径：{error}"))?;
    if !canonical.is_file() {
        return Err("源路径不是文件".to_string());
    }
    if let Some(root) = root {
        let canonical_root = canonical_existing_directory(root, "监控目录")?;
        if !canonical.starts_with(&canonical_root) {
            return Err("源文件真实路径不在被监控目录内，已拒绝上传".to_string());
        }
    }
    Ok(canonical)
}
fn ensure_directory_tree_without_symlinks(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let canonical_root = canonical_existing_directory(root, "归档目录")?;
    let mut current = canonical_root.clone();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            if matches!(component, std::path::Component::CurDir) {
                continue;
            }
            return Err("归档相对路径包含不安全的路径分量".to_string());
        };
        current.push(component);
        loop {
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err("归档中间目录不能是符号链接，已保留源文件".to_string());
                    }
                    if !metadata.is_dir() {
                        return Err("归档中间路径不是目录，已保留源文件".to_string());
                    }
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => break,
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(error) => return Err(format!("创建归档目录失败：{error}")),
                    }
                }
                Err(error) => return Err(format!("核对归档中间目录失败：{error}")),
            }
        }
        let resolved = fs::canonicalize(&current)
            .map_err(|error| format!("核对归档目录真实路径失败：{error}"))?;
        if !resolved.starts_with(&canonical_root) {
            return Err("归档目标真实路径逃逸出归档目录，已保留源文件".to_string());
        }
        current = resolved;
    }
    Ok(current)
}
fn normalize_loaded_mapping_paths(mut mapping: Mapping) -> Mapping {
    mapping.watch_error = None;
    let canonical_local =
        match canonical_existing_directory(Path::new(&mapping.local_path), "本地目录") {
            Ok(path) => {
                mapping.local_path = path.to_string_lossy().to_string();
                Some(path)
            }
            Err(error) => {
                mapping.watch_error = Some(error);
                None
            }
        };
    let canonical_archive = mapping
        .archive_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|path| canonical_existing_directory(Path::new(path), "归档目录"));
    match canonical_archive {
        Some(Ok(path)) => mapping.archive_path = Some(path.to_string_lossy().to_string()),
        Some(Err(error)) if mapping.source_policy == "archive" => {
            mapping.watch_error = Some(error);
        }
        _ => {}
    }
    if mapping.source_policy == "archive" {
        match (canonical_local.as_ref(), mapping.archive_path.as_deref()) {
            (Some(local), Some(archive)) => {
                if let Ok(archive) = fs::canonicalize(archive) {
                    if archive == *local || archive.starts_with(local) {
                        mapping.watch_error = Some("归档目录不能位于被监控目录内部".to_string());
                    }
                }
            }
            (_, None) => {
                mapping.watch_error = Some("归档策略没有配置归档目录".to_string());
            }
            _ => {}
        }
    }
    mapping
}
#[cfg(unix)]
fn same_file_identity(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    first.dev() == second.dev() && first.ino() == second.ino()
}
#[cfg(not(unix))]
fn same_file_identity(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    first.len() == second.len() && modified_ms(first) == modified_ms(second)
}
fn mapping_upload_destination(
    mapping: &Mapping,
    file_path: &Path,
) -> Result<(String, String, String), String> {
    let canonical_root = canonical_existing_directory(Path::new(&mapping.local_path), "监控目录")?;
    let canonical_file =
        fs::canonicalize(file_path).map_err(|error| format!("无法读取源文件真实路径：{error}"))?;
    if !canonical_file.starts_with(&canonical_root) {
        return Err("源文件真实路径不在被监控目录内".to_string());
    }
    let relative = canonical_file
        .strip_prefix(&canonical_root)
        .map_err(|_| "源文件不在被监控目录内".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    if relative.is_empty() {
        return Err("无法计算源文件的相对路径".to_string());
    }
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
    Ok((mapping.remote_parent_id.clone(), remote_dir, relative))
}
fn stamp_matches(item: &UploadItem, stamp: &Stamp) -> bool {
    stamp.size == item.size && stamp.modified_ms == item.modified_ms
}
fn flash_preflight_cached(state: &RuntimeState, item: &UploadItem) -> bool {
    state
        .flash_preflight_cache
        .get(&item_key(item))
        .is_some_and(|cached| stamp_matches(item, &cached.stamp))
}
fn take_flash_preflight_token(
    state: &SharedState,
    item: &UploadItem,
) -> Result<Option<UploadToken>, String> {
    let key = item_key(item);
    let cached = state
        .lock()
        .map_err(|error| error.to_string())?
        .flash_preflight_cache
        .remove(&key);
    Ok(cached.and_then(|cached| {
        (stamp_matches(item, &cached.stamp)
            && cached.created_at.elapsed()
                <= Duration::from_secs(FLASH_PREFLIGHT_TOKEN_MAX_AGE_SECS))
        .then_some(cached.upload_token)
        .flatten()
    }))
}
fn upload_already_scheduled(
    history: &HashMap<String, Stamp>,
    pending_cloud: &HashMap<String, Stamp>,
    inflight: &HashMap<String, Stamp>,
    queue: &VecDeque<UploadItem>,
    waiting_files: &HashMap<String, UploadItem>,
    item: &UploadItem,
) -> bool {
    let key = item_key(item);
    history
        .get(&key)
        .is_some_and(|stamp| stamp_matches(item, stamp))
        || pending_cloud.contains_key(&key)
        || inflight
            .get(&key)
            .is_some_and(|stamp| stamp_matches(item, stamp))
        || queue.iter().any(|queued| {
            item_key(queued) == key
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
fn normalize_search_file_type(value: Option<&str>) -> Result<Option<String>, String> {
    let normalized = value.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "all" {
        return Ok(None);
    }
    if ["image", "video", "audio", "document", "archive", "folder"].contains(&normalized.as_str()) {
        Ok(Some(normalized))
    } else {
        Err("文件类型只支持 image、video、audio、document、archive 或 folder".to_string())
    }
}
fn normalize_search_extension(value: Option<&str>) -> Option<String> {
    let normalized = value
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}
fn cloud_item_is_folder(item: &Value) -> bool {
    item.get("resType").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.parse::<i64>().ok())
    }) == Some(2)
        || item
            .get("isFolder")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}
fn cloud_item_extension(item: &Value) -> String {
    let explicit = ["fileSuffix", "extension", "ext"]
        .iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if !explicit.is_empty() {
        return explicit;
    }
    ["fileName", "name"]
        .iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .unwrap_or_default()
        .to_ascii_lowercase()
}
fn cloud_item_matches_search_filters(
    item: &Value,
    file_type: Option<&str>,
    extension: Option<&str>,
) -> bool {
    let is_folder = cloud_item_is_folder(item);
    let item_extension = cloud_item_extension(item);
    let type_matches = match file_type {
        None => true,
        Some("folder") => is_folder,
        Some("image") => !is_folder && IMAGE_EXTENSIONS.contains(&item_extension.as_str()),
        Some("video") => !is_folder && VIDEO_EXTENSIONS.contains(&item_extension.as_str()),
        Some("audio") => !is_folder && AUDIO_EXTENSIONS.contains(&item_extension.as_str()),
        Some("document") => !is_folder && DOCUMENT_EXTENSIONS.contains(&item_extension.as_str()),
        Some("archive") => !is_folder && ARCHIVE_EXTENSIONS.contains(&item_extension.as_str()),
        Some(_) => false,
    };
    type_matches && extension.is_none_or(|expected| !is_folder && item_extension == expected)
}
fn cloud_search_file_type(file_type: Option<&str>, extension: Option<&str>) -> Option<u8> {
    match file_type {
        Some("image") => Some(CLOUD_FILE_TYPE_IMAGE),
        Some("video") => Some(CLOUD_FILE_TYPE_VIDEO),
        Some("audio") => Some(CLOUD_FILE_TYPE_AUDIO),
        Some("document") => Some(CLOUD_FILE_TYPE_DOCUMENT),
        Some("archive") => Some(CLOUD_FILE_TYPE_ARCHIVE),
        Some("folder") => None,
        _ => extension.and_then(|extension| {
            if IMAGE_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_IMAGE)
            } else if VIDEO_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_VIDEO)
            } else if AUDIO_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_AUDIO)
            } else if DOCUMENT_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_DOCUMENT)
            } else if ARCHIVE_EXTENSIONS.contains(&extension) {
                Some(CLOUD_FILE_TYPE_ARCHIVE)
            } else {
                None
            }
        }),
    }
}
fn cloud_search_request(
    query: &str,
    file_type: Option<&str>,
    extension: Option<&str>,
    page: u64,
) -> (&'static str, Value) {
    let query = query.trim();
    if !query.is_empty() {
        return (
            "/userres/v1/file/search_files",
            json!({ "name": query, "pageSize": 100, "page": page }),
        );
    }

    let mut request = json!({
        "parentId": "*",
        "pageSize": 100,
        "page": page,
        "orderBy": 3,
        "sortType": 1,
        "resType": if file_type == Some("folder") { 2 } else { 1 }
    });
    if let Some(file_type) = cloud_search_file_type(file_type, extension) {
        request["fileTypes"] = json!([file_type]);
    }
    ("/userres/v1/file/get_file_list", request)
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

#[cfg(not(windows))]
fn atomic_replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut c_void,
            reserved: *mut c_void,
        ) -> i32;
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let destination_exists = destination.exists();
    let source = wide(source);
    let destination = wide(destination);
    let success = unsafe {
        if destination_exists {
            ReplaceFileW(
                destination.as_ptr(),
                source.as_ptr(),
                ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } else {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn save_config_checked(state: &RuntimeState) -> Result<(), String> {
    let parent = state
        .config_path
        .parent()
        .ok_or_else(|| "配置文件路径没有上级目录".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败：{error}"))?;
    let native_mount_options = state
        .native_mount
        .lock()
        .map_err(|error| format!("读取原生挂载设置失败：{error}"))?
        .options();
    let payload = json!({
        "mappings": state.mappings,
        "saved_shares": state.saved_shares,
        "upload_concurrency": state.upload_concurrency,
        "download_concurrency": state.download_concurrency,
        "multipart_part_size": state.multipart_part_size,
        "oss_part_concurrency": state.oss_part_concurrency,
        "webdav_enabled": state.webdav_enabled,
        "webdav_port": state.webdav_port,
        "webdav_username": state.webdav_username,
        "webdav_password_hash": state.webdav_password_hash,
        "native_mount": native_mount_options
    });
    let bytes =
        serde_json::to_vec_pretty(&payload).map_err(|error| format!("序列化配置失败：{error}"))?;
    let temporary_path = parent.join(format!(
        ".{}.{}.tmp",
        state
            .config_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("config"),
        Uuid::new_v4().simple()
    ));
    let write_result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&temporary_path)
            .map_err(|error| format!("创建临时配置文件失败：{error}"))?;
        output
            .write_all(&bytes)
            .map_err(|error| format!("写入临时配置文件失败：{error}"))?;
        output
            .sync_all()
            .map_err(|error| format!("同步临时配置文件失败：{error}"))?;
        drop(output);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("限制配置文件权限失败：{error}"))?;
        }
        atomic_replace_file(&temporary_path, &state.config_path)
            .map_err(|error| format!("原子替换配置文件失败：{error}"))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

fn save_config(state: &RuntimeState) {
    let _ = save_config_checked(state);
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

fn table_primary_key_columns(connection: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("读取 {table} 表结构失败：{error}"))?;
    let mut columns = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(5)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("查询 {table} 表结构失败：{error}"))?
        .filter_map(|row| match row {
            Ok((position, name)) if position > 0 => Some(Ok((position, name))),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析 {table} 主键失败：{error}"))?;
    columns.sort_by_key(|(position, _)| *position);
    Ok(columns.into_iter().map(|(_, name)| name).collect())
}

fn migrate_upload_destination_identity(connection: &mut Connection) -> Result<(), String> {
    let expected = vec![
        "mapping_id".to_string(),
        "file_path".to_string(),
        "remote_parent_id".to_string(),
        "remote_dir".to_string(),
    ];
    if table_primary_key_columns(connection, "uploaded_files")? != expected {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("开始迁移上传记录失败：{error}"))?;
        transaction
            .execute_batch(
                "ALTER TABLE uploaded_files RENAME TO uploaded_files_legacy_destination;
                 CREATE TABLE uploaded_files (
                   mapping_id TEXT NOT NULL,
                   file_path TEXT NOT NULL,
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   task_id TEXT,
                   remote_file_id TEXT,
                   upload_state TEXT NOT NULL DEFAULT 'cloud_confirmed',
                   remote_parent_id TEXT NOT NULL DEFAULT '',
                   remote_dir TEXT NOT NULL DEFAULT '',
                   relative_path TEXT NOT NULL DEFAULT '',
                   change_kind TEXT NOT NULL DEFAULT 'added',
                   uploaded_at INTEGER NOT NULL,
                   PRIMARY KEY (mapping_id, file_path, remote_parent_id, remote_dir)
                 );
                 INSERT OR REPLACE INTO uploaded_files
                   (mapping_id, file_path, size, modified_ms, task_id, remote_file_id,
                    upload_state, remote_parent_id, remote_dir, relative_path, change_kind,
                    uploaded_at)
                 SELECT mapping_id, file_path, size, modified_ms, task_id, remote_file_id,
                        upload_state, remote_parent_id, remote_dir, relative_path, change_kind,
                        uploaded_at
                 FROM uploaded_files_legacy_destination
                 ORDER BY uploaded_at;
                 DROP TABLE uploaded_files_legacy_destination;",
            )
            .map_err(|error| format!("迁移上传记录目标身份失败：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("提交上传记录迁移失败：{error}"))?;
    }

    if table_primary_key_columns(connection, "upload_checkpoints")? != expected {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("开始迁移上传断点失败：{error}"))?;
        transaction
            .execute_batch(
                "ALTER TABLE upload_checkpoints RENAME TO upload_checkpoints_legacy_destination;
                 CREATE TABLE upload_checkpoints (
                   mapping_id TEXT NOT NULL,
                   file_path TEXT NOT NULL,
                   remote_parent_id TEXT NOT NULL DEFAULT '',
                   remote_dir TEXT NOT NULL DEFAULT '',
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   item_json TEXT NOT NULL,
                   checkpoint_json TEXT NOT NULL,
                   uploaded_bytes INTEGER NOT NULL DEFAULT 0,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY (mapping_id, file_path, remote_parent_id, remote_dir)
                 );",
            )
            .map_err(|error| format!("准备上传断点目标身份迁移失败：{error}"))?;
        let rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT mapping_id, file_path, size, modified_ms, item_json,
                            checkpoint_json, uploaded_bytes, updated_at
                     FROM upload_checkpoints_legacy_destination",
                )
                .map_err(|error| format!("读取旧上传断点失败：{error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, u64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                })
                .map_err(|error| format!("查询旧上传断点失败：{error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("解析旧上传断点失败：{error}"))?;
            rows
        };
        for (
            mapping_id,
            file_path,
            size,
            modified_ms,
            item_json,
            checkpoint_json,
            uploaded_bytes,
            updated_at,
        ) in rows
        {
            let (remote_parent_id, remote_dir) = serde_json::from_str::<UploadItem>(&item_json)
                .map(|item| (item.remote_parent_id, item.remote_dir))
                .unwrap_or_default();
            transaction
                .execute(
                    "INSERT OR REPLACE INTO upload_checkpoints
                       (mapping_id, file_path, remote_parent_id, remote_dir, size, modified_ms,
                        item_json, checkpoint_json, uploaded_bytes, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        mapping_id,
                        file_path,
                        remote_parent_id,
                        remote_dir,
                        size,
                        modified_ms,
                        item_json,
                        checkpoint_json,
                        uploaded_bytes,
                        updated_at
                    ],
                )
                .map_err(|error| format!("迁移上传断点失败：{error}"))?;
        }
        transaction
            .execute("DROP TABLE upload_checkpoints_legacy_destination", [])
            .map_err(|error| format!("清理旧上传断点表失败：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("提交上传断点迁移失败：{error}"))?;
    }
    Ok(())
}

fn init_database(path: &Path) -> Result<(), String> {
    let mut connection = open_database(path)?;
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
               upload_state TEXT NOT NULL DEFAULT 'cloud_confirmed',
               remote_parent_id TEXT NOT NULL DEFAULT '',
               remote_dir TEXT NOT NULL DEFAULT '',
               relative_path TEXT NOT NULL DEFAULT '',
               change_kind TEXT NOT NULL DEFAULT 'added',
               uploaded_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, file_path, remote_parent_id, remote_dir)
             );
             CREATE TABLE IF NOT EXISTS upload_checkpoints (
               mapping_id TEXT NOT NULL,
               file_path TEXT NOT NULL,
               remote_parent_id TEXT NOT NULL DEFAULT '',
               remote_dir TEXT NOT NULL DEFAULT '',
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               item_json TEXT NOT NULL,
               checkpoint_json TEXT NOT NULL,
               uploaded_bytes INTEGER NOT NULL DEFAULT 0,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (mapping_id, file_path, remote_parent_id, remote_dir)
             );
             CREATE TABLE IF NOT EXISTS app_state (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS file_fingerprints (
               file_path TEXT NOT NULL,
               size INTEGER NOT NULL,
               modified_ms TEXT NOT NULL,
               gcid TEXT NOT NULL,
               computed_at INTEGER NOT NULL,
               PRIMARY KEY (file_path, size, modified_ms)
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
             );
             CREATE TABLE IF NOT EXISTS gcid_import_jobs (
               job_id TEXT PRIMARY KEY,
               source_path TEXT NOT NULL,
               source_name TEXT NOT NULL,
               destination_parent_id TEXT NOT NULL,
               destination_name TEXT NOT NULL,
               total_files INTEGER NOT NULL,
               total_size TEXT NOT NULL,
               status TEXT NOT NULL,
               current_path TEXT NOT NULL DEFAULT '',
               error TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS gcid_import_files (
               job_id TEXT NOT NULL,
               path TEXT NOT NULL,
               folder_path TEXT NOT NULL,
               file_name TEXT NOT NULL,
               file_size INTEGER NOT NULL,
               gcid TEXT NOT NULL,
               status TEXT NOT NULL DEFAULT 'pending',
               attempts INTEGER NOT NULL DEFAULT 0,
               task_id TEXT,
               file_id TEXT,
               error TEXT,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (job_id, path)
             );",
        )
        .map_err(|e| format!("初始化 SQLite 失败：{e}"))?;
    connection
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS gcid_import_files_status
               ON gcid_import_files(job_id, status, path);
             UPDATE gcid_import_files
               SET status = 'pending', error = '应用上次退出，已等待继续'
               WHERE status = 'processing';
             UPDATE gcid_import_jobs
               SET status = 'paused', error = '应用上次退出，点击继续导入'
               WHERE status IN ('preparing', 'running');",
        )
        .map_err(|e| format!("初始化 GCID 导入状态失败：{e}"))?;
    let _ = connection.execute(
        "ALTER TABLE auto_share_events ADD COLUMN notification_status TEXT",
        [],
    );
    for migration in [
        "ALTER TABLE uploaded_files ADD COLUMN upload_state TEXT NOT NULL DEFAULT 'cloud_confirmed'",
        "ALTER TABLE uploaded_files ADD COLUMN remote_parent_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN remote_dir TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN relative_path TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE uploaded_files ADD COLUMN change_kind TEXT NOT NULL DEFAULT 'added'",
    ] {
        let _ = connection.execute(migration, []);
    }
    migrate_upload_destination_identity(&mut connection)?;
    connection
        .execute(
            "UPDATE uploaded_files
             SET upload_state = CASE
               WHEN task_id IS NOT NULL AND TRIM(task_id) <> ''
                 AND (remote_file_id IS NULL OR TRIM(remote_file_id) = '')
               THEN ?1 ELSE ?2 END
             WHERE upload_state IS NULL OR upload_state = ''
                OR upload_state NOT IN (?1, ?2)
                OR (upload_state = ?2 AND task_id IS NOT NULL AND TRIM(task_id) <> ''
                    AND (remote_file_id IS NULL OR TRIM(remote_file_id) = ''))",
            params![UPLOAD_STATE_OSS_COMPLETE, UPLOAD_STATE_CLOUD_CONFIRMED],
        )
        .map_err(|e| format!("迁移上传状态失败：{e}"))?;
    Ok(())
}

fn load_cached_file_gcid(
    database: &Path,
    file_path: &Path,
    size: u64,
    modified_ms: u128,
    settings: CacheSettings,
) -> Result<Option<String>, String> {
    if !settings.enabled {
        return Ok(None);
    }
    let size = i64::try_from(size).map_err(|_| "文件过大，无法缓存秒传指纹".to_string())?;
    let connection = open_database(database)?;
    let gcid = connection
        .query_row(
            "SELECT gcid FROM file_fingerprints
             WHERE file_path = ?1 AND size = ?2 AND modified_ms = ?3",
            params![
                file_path.to_string_lossy().as_ref(),
                size,
                modified_ms.to_string()
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取秒传指纹缓存失败：{error}"))?;
    Ok(
        gcid.filter(|value| {
            value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }),
    )
}

fn save_cached_file_gcid(
    database: &Path,
    file_path: &Path,
    size: u64,
    modified_ms: u128,
    gcid: &str,
    settings: CacheSettings,
) -> Result<(), String> {
    if !settings.enabled {
        return Ok(());
    }
    let size = i64::try_from(size).map_err(|_| "文件过大，无法缓存秒传指纹".to_string())?;
    let connection = open_database(database)?;
    let file_path = file_path.to_string_lossy();
    let modified_ms = modified_ms.to_string();
    connection
        .execute(
            "DELETE FROM file_fingerprints
             WHERE file_path = ?1 AND (size <> ?2 OR modified_ms <> ?3)",
            params![file_path.as_ref(), size, modified_ms],
        )
        .map_err(|error| format!("清理旧秒传指纹失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO file_fingerprints
               (file_path, size, modified_ms, gcid, computed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(file_path, size, modified_ms)
             DO UPDATE SET gcid = excluded.gcid, computed_at = excluded.computed_at",
            params![
                file_path.as_ref(),
                size,
                modified_ms,
                gcid,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存秒传指纹缓存失败：{error}"))?;
    trim_file_fingerprint_cache(database, settings.max_entries)?;
    Ok(())
}

fn reset_remote_cache(remote_cache: &mut HashMap<String, String>) {
    remote_cache.clear();
    remote_cache.insert(String::new(), String::new());
}

fn trim_file_fingerprint_cache(database: &Path, max_entries: usize) -> Result<(), String> {
    let max_entries = i64::try_from(max_entries).map_err(|_| "缓存条目上限无效".to_string())?;
    open_database(database)?
        .execute(
            "DELETE FROM file_fingerprints
             WHERE rowid IN (
               SELECT rowid FROM file_fingerprints
               ORDER BY computed_at DESC, rowid DESC
               LIMIT -1 OFFSET ?1
             )",
            params![max_entries],
        )
        .map_err(|error| format!("裁剪秒传指纹缓存失败：{error}"))?;
    Ok(())
}

fn trim_remote_cache(remote_cache: &mut HashMap<String, String>, max_entries: usize) {
    let excess = remote_cache
        .len()
        .saturating_sub(usize::from(remote_cache.contains_key("")))
        .saturating_sub(max_entries);
    let keys = remote_cache
        .keys()
        .filter(|key| !key.is_empty())
        .take(excess)
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        remote_cache.remove(&key);
    }
}

fn file_fingerprint_cache_usage(database: &Path) -> Result<(u64, u64), String> {
    let connection = open_database(database)?;
    let mut statement = connection
        .prepare("SELECT file_path, modified_ms, gcid FROM file_fingerprints")
        .map_err(|error| format!("读取秒传指纹缓存统计失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("读取秒传指纹缓存统计失败：{error}"))?;
    let mut entries = 0_u64;
    let mut bytes = 0_u64;
    for row in rows {
        let (file_path, modified_ms, gcid) =
            row.map_err(|error| format!("解析秒传指纹缓存统计失败：{error}"))?;
        entries = entries.saturating_add(1);
        bytes = bytes.saturating_add(
            u64::try_from(file_path.len() + modified_ms.len() + gcid.len())
                .unwrap_or(u64::MAX)
                .saturating_add(16),
        );
    }
    Ok((entries, bytes))
}

fn remote_cache_usage(remote_cache: &HashMap<String, String>) -> (u64, u64) {
    remote_cache
        .iter()
        .filter(|(key, value)| !(key.is_empty() && value.is_empty()))
        .fold((0_u64, 0_u64), |(entries, bytes), (key, value)| {
            (
                entries.saturating_add(1),
                bytes.saturating_add(u64::try_from(key.len() + value.len()).unwrap_or(u64::MAX)),
            )
        })
}

fn metadata_cache_stats(
    database: &Path,
    remote_cache: &HashMap<String, String>,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    let (file_fingerprints_entries, file_fingerprints_bytes) =
        file_fingerprint_cache_usage(database)?;
    let (remote_cache_entries, remote_cache_bytes) = remote_cache_usage(remote_cache);
    Ok(MetadataCacheStats {
        bytes: file_fingerprints_bytes.saturating_add(remote_cache_bytes),
        entries: file_fingerprints_entries.saturating_add(remote_cache_entries),
        file_fingerprints_bytes,
        file_fingerprints_entries,
        remote_cache_bytes,
        remote_cache_entries,
        policy,
    })
}

fn clear_metadata_cache_storage(
    database: &Path,
    remote_cache: &mut HashMap<String, String>,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    open_database(database)?
        .execute("DELETE FROM file_fingerprints", [])
        .map_err(|error| format!("清理秒传指纹缓存失败：{error}"))?;
    reset_remote_cache(remote_cache);
    metadata_cache_stats(database, remote_cache, policy)
}

fn apply_cache_policy(
    database: &Path,
    remote_cache: &mut HashMap<String, String>,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    if !policy.enabled {
        return clear_metadata_cache_storage(database, remote_cache, policy);
    }
    trim_file_fingerprint_cache(database, policy.max_entries)?;
    trim_remote_cache(remote_cache, policy.max_entries);
    metadata_cache_stats(database, remote_cache, policy)
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
               access_token = excluded.access_token,
               refresh_token = excluded.refresh_token,
               updated_at = excluded.updated_at",
            params![access_token, refresh_token, unix_timestamp()],
        )
        .map_err(|e| format!("保存登录状态失败：{e}"))?;
    Ok(())
}

fn save_refreshed_auth_session(
    path: &Path,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO auth_session (id, access_token, refresh_token, updated_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               access_token = excluded.access_token,
               refresh_token = COALESCE(excluded.refresh_token, auth_session.refresh_token),
               updated_at = excluded.updated_at",
            params![access_token, refresh_token, unix_timestamp()],
        )
        .map_err(|e| format!("保存续期登录状态失败：{e}"))?;
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

fn clear_persisted_auth_session(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "UPDATE auth_session
             SET access_token = NULL, refresh_token = NULL, updated_at = ?1
             WHERE id = 1",
            params![unix_timestamp()],
        )
        .map_err(|e| format!("清理过期登录状态失败：{e}"))?;
    Ok(())
}

fn invalidate_auth_session(app: &tauri::AppHandle, state: &SharedState) -> Result<(), String> {
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.token = None;
        guard.refresh_token = None;
        guard.refreshed_access_tokens.clear();
        reset_remote_cache(&mut guard.remote_cache);
        guard.db_path.clone()
    };
    let result = clear_persisted_auth_session(&db_path);
    emit_state(app, state);
    result
}

fn load_upload_history(path: &Path) -> Result<HashMap<String, Stamp>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, file_path, size, modified_ms, remote_parent_id, remote_dir
             FROM uploaded_files
             WHERE upload_state = ?1",
        )
        .map_err(|e| format!("读取上传记录失败：{e}"))?;
    let rows = statement
        .query_map(params![UPLOAD_STATE_CLOUD_CONFIRMED], |row| {
            let mapping_id: String = row.get(0)?;
            let file_path: String = row.get(1)?;
            let size: u64 = row.get(2)?;
            let modified_raw: String = row.get(3)?;
            let remote_parent_id: String = row.get(4)?;
            let remote_dir: String = row.get(5)?;
            Ok((
                mapping_id,
                file_path,
                size,
                modified_raw,
                remote_parent_id,
                remote_dir,
            ))
        })
        .map_err(|e| format!("查询上传记录失败：{e}"))?;
    let mut history = HashMap::new();
    for row in rows {
        let (mapping_id, file_path, size, modified_raw, remote_parent_id, remote_dir) =
            row.map_err(|e| format!("解析上传记录失败：{e}"))?;
        let modified_ms = modified_raw.parse::<u128>().unwrap_or(0);
        history.insert(
            destination_item_key(
                &mapping_id,
                Path::new(&file_path),
                &remote_parent_id,
                &remote_dir,
            ),
            Stamp { size, modified_ms },
        );
    }
    Ok(history)
}

fn reuse_matching_confirmed_upload(
    path: &Path,
    item: &UploadItem,
) -> Result<Option<(String, UploadOutcome)>, String> {
    let connection = open_database(path)?;
    let matched = connection
        .query_row(
            "SELECT mapping_id, task_id, remote_file_id
             FROM uploaded_files
             WHERE upload_state = ?1
               AND substr(mapping_id, 1, 2) <> '__'
               AND file_path = ?2
               AND size = ?3
               AND modified_ms = ?4
               AND remote_parent_id = ?5
               AND remote_dir = ?6
               AND relative_path = ?7
             ORDER BY uploaded_at DESC
             LIMIT 1",
            params![
                UPLOAD_STATE_CLOUD_CONFIRMED,
                item.file_path.to_string_lossy().as_ref(),
                item.size,
                item.modified_ms.to_string(),
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    UploadOutcome {
                        task_id: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        remote_file_id: row.get(2)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|error| format!("复用历史上传记录失败：{error}"))?;
    drop(connection);
    if let Some((_, outcome)) = matched.as_ref() {
        save_upload_record(path, item, outcome, UPLOAD_STATE_CLOUD_CONFIRMED)?;
    }
    Ok(matched)
}

fn load_pending_uploads(path: &Path) -> Result<Vec<PendingUpload>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(
            "SELECT mapping_id, file_path, size, modified_ms, task_id,
                    remote_parent_id, remote_dir, relative_path, change_kind
             FROM uploaded_files
             WHERE upload_state = ?1 AND task_id IS NOT NULL AND TRIM(task_id) <> ''",
        )
        .map_err(|e| format!("读取待确认上传记录失败：{e}"))?;
    let rows = statement
        .query_map(params![UPLOAD_STATE_OSS_COMPLETE], |row| {
            let modified_raw: String = row.get(3)?;
            Ok(PendingUpload {
                item: UploadItem {
                    mapping_id: row.get(0)?,
                    file_path: PathBuf::from(row.get::<_, String>(1)?),
                    size: row.get(2)?,
                    modified_ms: modified_raw.parse::<u128>().unwrap_or(0),
                    remote_parent_id: row.get(5)?,
                    remote_dir: row.get(6)?,
                    relative_path: row.get(7)?,
                    change_kind: row.get(8)?,
                },
                task_id: row.get(4)?,
            })
        })
        .map_err(|e| format!("查询待确认上传记录失败：{e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析待确认上传记录失败：{e}"))
}

fn pending_upload_stamps(path: &Path) -> Result<HashMap<String, Stamp>, String> {
    Ok(load_pending_uploads(path)?
        .into_iter()
        .map(|pending| {
            (
                item_key(&pending.item),
                Stamp {
                    size: pending.item.size,
                    modified_ms: pending.item.modified_ms,
                },
            )
        })
        .collect())
}

fn clear_upload_checkpoint(path: &Path, item: &UploadItem) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM upload_checkpoints
             WHERE mapping_id = ?1 AND file_path = ?2
               AND remote_parent_id = ?3 AND remote_dir = ?4",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.remote_parent_id,
                item.remote_dir
            ],
        )
        .map_err(|error| format!("清除上传断点失败：{error}"))?;
    Ok(())
}

fn load_upload_checkpoint(
    path: &Path,
    item: &UploadItem,
) -> Result<Option<PersistedUploadCheckpoint>, String> {
    let connection = open_database(path)?;
    let row = connection
        .query_row(
            "SELECT size, modified_ms, checkpoint_json, uploaded_bytes
             FROM upload_checkpoints
             WHERE mapping_id = ?1 AND file_path = ?2
               AND remote_parent_id = ?3 AND remote_dir = ?4",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.remote_parent_id,
                item.remote_dir
            ],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取上传断点失败：{error}"))?;
    let Some((size, modified_ms, checkpoint_json, uploaded_bytes)) = row else {
        return Ok(None);
    };
    if size != item.size || modified_ms != item.modified_ms.to_string() {
        clear_upload_checkpoint(path, item)?;
        return Ok(None);
    }
    let checkpoint = match serde_json::from_str::<OssUploadCheckpoint>(&checkpoint_json) {
        Ok(value) => value,
        Err(_) => {
            clear_upload_checkpoint(path, item)?;
            return Ok(None);
        }
    };
    Ok(Some(PersistedUploadCheckpoint {
        checkpoint,
        uploaded_bytes: uploaded_bytes.min(item.size),
    }))
}

fn save_upload_checkpoint(
    path: &Path,
    item: &UploadItem,
    checkpoint: &OssUploadCheckpoint,
    uploaded_bytes: u64,
) -> Result<(), String> {
    let connection = open_database(path)?;
    let item_json =
        serde_json::to_string(item).map_err(|error| format!("序列化上传任务失败：{error}"))?;
    let checkpoint_json = serde_json::to_string(checkpoint)
        .map_err(|error| format!("序列化上传断点失败：{error}"))?;
    connection
        .execute(
            "INSERT INTO upload_checkpoints
               (mapping_id, file_path, remote_parent_id, remote_dir, size, modified_ms,
                item_json, checkpoint_json, uploaded_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(mapping_id, file_path, remote_parent_id, remote_dir) DO UPDATE SET
               size = excluded.size,
               modified_ms = excluded.modified_ms,
               item_json = excluded.item_json,
               checkpoint_json = excluded.checkpoint_json,
               uploaded_bytes = excluded.uploaded_bytes,
               updated_at = excluded.updated_at",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.remote_parent_id,
                item.remote_dir,
                item.size,
                item.modified_ms.to_string(),
                item_json,
                checkpoint_json,
                uploaded_bytes.min(item.size),
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存上传断点失败：{error}"))?;
    Ok(())
}

async fn save_upload_checkpoint_async(
    path: &Path,
    item: &UploadItem,
    checkpoint: &OssUploadCheckpoint,
    uploaded_bytes: u64,
) -> Result<(), String> {
    let path = path.to_path_buf();
    let item = item.clone();
    let checkpoint = checkpoint.clone();
    tokio::task::spawn_blocking(move || {
        save_upload_checkpoint(&path, &item, &checkpoint, uploaded_bytes)
    })
    .await
    .map_err(|error| format!("等待保存上传断点失败：{error}"))?
}

impl UploadCheckpointWriteBatch {
    fn push(&mut self, command: UploadCheckpointWriterCommand) {
        match command {
            UploadCheckpointWriterCommand::Save(write) => {
                self.writes.insert(item_key(&write.item), write);
            }
            UploadCheckpointWriterCommand::Flush {
                key,
                latest,
                acknowledgement,
            } => {
                if let Some(write) = latest {
                    self.writes.insert(key.clone(), write);
                }
                self.acknowledgements
                    .entry(key)
                    .or_default()
                    .push(acknowledgement);
            }
        }
    }
}

impl UploadCheckpointWriter {
    fn new(database: PathBuf) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        tauri::async_runtime::spawn(run_upload_checkpoint_writer(database, receiver));
        Self { sender }
    }

    fn enqueue(
        &self,
        item: &UploadItem,
        checkpoint: &OssUploadCheckpoint,
        uploaded_bytes: u64,
    ) -> Result<(), String> {
        self.sender
            .send(UploadCheckpointWriterCommand::Save(UploadCheckpointWrite {
                item: item.clone(),
                checkpoint: checkpoint.clone(),
                uploaded_bytes,
            }))
            .map_err(|_| "上传断点写入任务已停止".to_string())
    }

    async fn flush(
        &self,
        item: &UploadItem,
        checkpoint: Option<&OssUploadCheckpoint>,
        uploaded_bytes: u64,
    ) -> Result<(), String> {
        let key = item_key(item);
        let latest = checkpoint.map(|checkpoint| UploadCheckpointWrite {
            item: item.clone(),
            checkpoint: checkpoint.clone(),
            uploaded_bytes,
        });
        let (acknowledgement, completion) = oneshot::channel();
        self.sender
            .send(UploadCheckpointWriterCommand::Flush {
                key,
                latest,
                acknowledgement,
            })
            .map_err(|_| "上传断点写入任务已停止".to_string())?;
        completion
            .await
            .map_err(|_| "等待上传断点写入确认失败".to_string())?
    }
}

async fn run_upload_checkpoint_writer(
    database: PathBuf,
    mut receiver: UnboundedReceiver<UploadCheckpointWriterCommand>,
) {
    while let Some(first) = receiver.recv().await {
        let mut batch = UploadCheckpointWriteBatch::default();
        batch.push(first);
        while let Ok(command) = receiver.try_recv() {
            batch.push(command);
        }

        let mut results = HashMap::new();
        for (key, write) in batch.writes {
            let result = save_upload_checkpoint_async(
                &database,
                &write.item,
                &write.checkpoint,
                write.uploaded_bytes,
            )
            .await;
            results.insert(key, result);
        }
        for (key, acknowledgements) in batch.acknowledgements {
            let result = results.get(&key).cloned().unwrap_or(Ok(()));
            for acknowledgement in acknowledgements {
                let _ = acknowledgement.send(result.clone());
            }
        }
    }
}

fn load_resumable_uploads(path: &Path) -> Result<VecDeque<UploadItem>, String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM upload_checkpoints
             WHERE EXISTS (
               SELECT 1 FROM uploaded_files AS uploaded
               WHERE uploaded.mapping_id = upload_checkpoints.mapping_id
                 AND uploaded.file_path = upload_checkpoints.file_path
                 AND uploaded.remote_parent_id = upload_checkpoints.remote_parent_id
                 AND uploaded.remote_dir = upload_checkpoints.remote_dir
                 AND uploaded.size = upload_checkpoints.size
                 AND uploaded.modified_ms = upload_checkpoints.modified_ms
                 AND uploaded.upload_state IN (?1, ?2)
             )",
            params![UPLOAD_STATE_OSS_COMPLETE, UPLOAD_STATE_CLOUD_CONFIRMED],
        )
        .map_err(|error| format!("清理已完成上传的残留断点失败：{error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT checkpoint.mapping_id, checkpoint.file_path,
                    checkpoint.remote_parent_id, checkpoint.remote_dir,
                    checkpoint.size, checkpoint.modified_ms, checkpoint.item_json
             FROM upload_checkpoints AS checkpoint
             WHERE NOT EXISTS (
               SELECT 1 FROM uploaded_files AS uploaded
               WHERE uploaded.mapping_id = checkpoint.mapping_id
                 AND uploaded.file_path = checkpoint.file_path
                 AND uploaded.remote_parent_id = checkpoint.remote_parent_id
                 AND uploaded.remote_dir = checkpoint.remote_dir
                 AND uploaded.size = checkpoint.size
                 AND uploaded.modified_ms = checkpoint.modified_ms
                 AND uploaded.upload_state IN (?1, ?2)
             )
             ORDER BY checkpoint.updated_at",
        )
        .map_err(|error| format!("读取待续传任务失败：{error}"))?;
    let rows = statement
        .query_map(
            params![UPLOAD_STATE_OSS_COMPLETE, UPLOAD_STATE_CLOUD_CONFIRMED],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|error| format!("查询待续传任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析待续传任务失败：{error}"))?;
    drop(statement);
    let mut restored = VecDeque::new();
    for (mapping_id, file_path, remote_parent_id, remote_dir, size, modified_ms_value, item_json) in
        rows
    {
        let parsed = serde_json::from_str::<UploadItem>(&item_json).ok();
        let valid = parsed.as_ref().is_some_and(|item| {
            item.mapping_id == mapping_id
                && item.file_path == PathBuf::from(&file_path)
                && item.remote_parent_id == remote_parent_id
                && item.remote_dir == remote_dir
                && item.size == size
                && item.modified_ms.to_string() == modified_ms_value
                && fs::metadata(&item.file_path).ok().is_some_and(|metadata| {
                    metadata.is_file()
                        && metadata.len() == size
                        && modified_ms(&metadata) == item.modified_ms
                })
        });
        if valid {
            restored.push_back(parsed.expect("checked resumable upload item"));
        } else {
            connection
                .execute(
                    "DELETE FROM upload_checkpoints
                     WHERE mapping_id = ?1 AND file_path = ?2
                       AND remote_parent_id = ?3 AND remote_dir = ?4",
                    params![mapping_id, file_path, remote_parent_id, remote_dir],
                )
                .map_err(|error| format!("清理失效上传断点失败：{error}"))?;
        }
    }
    Ok(restored)
}

fn save_upload_record(
    path: &Path,
    item: &UploadItem,
    outcome: &UploadOutcome,
    upload_state: &str,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO uploaded_files
               (mapping_id, file_path, size, modified_ms, task_id, remote_file_id,
                upload_state, remote_parent_id, remote_dir, relative_path, change_kind, uploaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(mapping_id, file_path, remote_parent_id, remote_dir) DO UPDATE SET
               size = excluded.size,
               modified_ms = excluded.modified_ms,
               task_id = excluded.task_id,
               remote_file_id = excluded.remote_file_id,
               upload_state = excluded.upload_state,
               remote_parent_id = excluded.remote_parent_id,
               remote_dir = excluded.remote_dir,
               relative_path = excluded.relative_path,
               change_kind = excluded.change_kind,
               uploaded_at = excluded.uploaded_at",
            params![
                item.mapping_id,
                item.file_path.to_string_lossy(),
                item.size,
                item.modified_ms.to_string(),
                outcome.task_id,
                outcome.remote_file_id,
                upload_state,
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path,
                item.change_kind,
                unix_timestamp()
            ],
        )
        .map_err(|e| format!("保存上传记录失败：{e}"))?;
    Ok(())
}

fn remember_pending_upload(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let database = state.lock().map_err(|e| e.to_string())?.db_path.clone();
    save_upload_record(&database, item, outcome, UPLOAD_STATE_OSS_COMPLETE)?;
    state
        .lock()
        .map_err(|e| e.to_string())?
        .pending_cloud
        .insert(
            item_key(item),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
    Ok(())
}

fn confirm_pending_record(
    database: &Path,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<bool, String> {
    open_database(database)?
        .execute(
            "UPDATE uploaded_files SET remote_file_id = ?1, upload_state = ?2,
                    remote_parent_id = ?3, remote_dir = ?4, relative_path = ?5,
                    change_kind = ?6, uploaded_at = ?7
             WHERE mapping_id = ?8 AND file_path = ?9 AND task_id = ?10
               AND upload_state = ?11 AND remote_parent_id = ?12 AND remote_dir = ?13",
            params![
                outcome.remote_file_id,
                UPLOAD_STATE_CLOUD_CONFIRMED,
                item.remote_parent_id,
                item.remote_dir,
                item.relative_path,
                item.change_kind,
                unix_timestamp(),
                item.mapping_id,
                item.file_path.to_string_lossy(),
                outcome.task_id,
                UPLOAD_STATE_OSS_COMPLETE,
                item.remote_parent_id,
                item.remote_dir
            ],
        )
        .map(|changed| changed > 0)
        .map_err(|e| format!("更新云端确认状态失败：{e}"))
}

fn remember_confirmed_upload(
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let database = state.lock().map_err(|e| e.to_string())?.db_path.clone();
    if !confirm_pending_record(&database, item, outcome)? {
        return Err("待确认上传记录已被移除或已由其他任务更新".into());
    }
    let key = item_key(&item);
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.pending_cloud.remove(&key);
    guard.history.insert(
        key,
        Stamp {
            size: item.size,
            modified_ms: item.modified_ms,
        },
    );
    Ok(())
}

fn delete_pending_upload(path: &Path, pending: &PendingUpload) -> Result<bool, String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM uploaded_files
             WHERE mapping_id = ?1 AND file_path = ?2 AND task_id = ?3 AND upload_state = ?4
               AND remote_parent_id = ?5 AND remote_dir = ?6",
            params![
                pending.item.mapping_id,
                pending.item.file_path.to_string_lossy(),
                pending.task_id,
                UPLOAD_STATE_OSS_COMPLETE,
                pending.item.remote_parent_id,
                pending.item.remote_dir
            ],
        )
        .map(|changed| changed > 0)
        .map_err(|e| format!("清理待确认上传记录失败：{e}"))
}

fn remove_mapping_transient_uploads(path: &Path, mapping_id: &str) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "DELETE FROM uploaded_files
             WHERE mapping_id = ?1 AND upload_state <> ?2",
            params![mapping_id, UPLOAD_STATE_CLOUD_CONFIRMED],
        )
        .map_err(|e| format!("清理任务待确认上传记录失败：{e}"))?;
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

fn reuse_auto_share_binding(
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

fn target_has_pending_cloud(
    database: &Path,
    mapping_id: &str,
    target_key: &str,
) -> Result<bool, String> {
    Ok(load_pending_uploads(database)?.iter().any(|pending| {
        pending.item.mapping_id == mapping_id
            && auto_share_target(&pending.item).is_some_and(|target| target.key == target_key)
    }))
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

async fn api_post_once(
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

#[derive(Debug, PartialEq, Eq)]
enum AuthRecoveryDecision {
    RetryWithCurrent(String),
    Refresh,
    Invalidate,
    SessionChanged,
}

fn auth_recovery_decision(
    observed_token: &str,
    current_token: Option<&str>,
    refreshed_to_current: bool,
    has_refresh_token: bool,
) -> AuthRecoveryDecision {
    match current_token {
        Some(current) if current != observed_token && refreshed_to_current => {
            AuthRecoveryDecision::RetryWithCurrent(current.to_string())
        }
        Some(current) if current != observed_token => AuthRecoveryDecision::SessionChanged,
        Some(_) if has_refresh_token => AuthRecoveryDecision::Refresh,
        Some(_) => AuthRecoveryDecision::Invalidate,
        None => AuthRecoveryDecision::SessionChanged,
    }
}

fn refreshed_token_chain_reaches_current(
    refreshed_tokens: &HashMap<String, String>,
    observed_token: &str,
    current_token: &str,
) -> bool {
    let mut candidate = observed_token;
    for _ in 0..=refreshed_tokens.len().min(32) {
        let Some(next) = refreshed_tokens.get(candidate) else {
            return false;
        };
        if next == current_token {
            return true;
        }
        candidate = next;
    }
    false
}

fn is_api_auth_expired_error(message: &str) -> bool {
    message.starts_with("登录态已失效")
}

fn invalidate_auth_session_if_current(
    app: &tauri::AppHandle,
    state: &SharedState,
    expected_access_token: &str,
) -> Result<bool, String> {
    let persistence_result = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        if guard.token.as_deref() != Some(expected_access_token) {
            return Ok(false);
        }
        let result = clear_persisted_auth_session(&guard.db_path);
        guard.token = None;
        guard.refresh_token = None;
        guard.refreshed_access_tokens.clear();
        reset_remote_cache(&mut guard.remote_cache);
        result
    };
    emit_state(app, state);
    persistence_result?;
    Ok(true)
}

async fn recover_api_auth_and_retry(
    runtime: ApiAuthRuntime,
    observed_token: String,
    endpoint: &str,
    body: Value,
    allowed: &[i64],
) -> Result<ApiResponse, String> {
    let refresh_lock = runtime
        .state
        .lock()
        .map_err(|error| error.to_string())?
        .auth_refresh_lock
        .clone();
    let _refresh_guard = refresh_lock.lock().await;
    let decision = {
        let guard = runtime.state.lock().map_err(|error| error.to_string())?;
        let current_token = guard.token.as_deref();
        auth_recovery_decision(
            &observed_token,
            current_token,
            current_token.is_some_and(|current| {
                refreshed_token_chain_reaches_current(
                    &guard.refreshed_access_tokens,
                    &observed_token,
                    current,
                )
            }),
            guard.refresh_token.is_some(),
        )
    };
    match decision {
        AuthRecoveryDecision::RetryWithCurrent(_) => {}
        AuthRecoveryDecision::Refresh => {
            if !refresh_saved_session(runtime.app.clone(), runtime.state.clone()).await? {
                invalidate_auth_session_if_current(&runtime.app, &runtime.state, &observed_token)?;
                return Err("登录态已失效，请重新打开官方登录页".to_string());
            }
        }
        AuthRecoveryDecision::Invalidate => {
            invalidate_auth_session_if_current(&runtime.app, &runtime.state, &observed_token)?;
            return Err("登录态已失效，请重新打开官方登录页".to_string());
        }
        AuthRecoveryDecision::SessionChanged => {
            return Err("登录账户已切换，请重新执行当前操作".to_string());
        }
    }
    let (retry_token, retry_device_id) = shared_auth_context(&runtime.state)?;
    let retry = api_post_once(&retry_token, &retry_device_id, endpoint, body, allowed).await;
    if retry
        .as_ref()
        .err()
        .is_some_and(|message| is_api_auth_expired_error(message))
    {
        invalidate_auth_session_if_current(&runtime.app, &runtime.state, &retry_token)?;
    }
    retry
}

async fn api_post(
    token: &str,
    device_id: &str,
    endpoint: &str,
    body: Value,
    allowed: &[i64],
) -> Result<ApiResponse, String> {
    let runtime = API_AUTH_RUNTIME.get().cloned();
    let effective_token = token.to_string();
    let result = api_post_once(&effective_token, device_id, endpoint, body.clone(), allowed).await;
    match result {
        Err(message) if is_api_auth_expired_error(&message) => {
            let Some(runtime) = runtime else {
                return Err(message);
            };
            recover_api_auth_and_retry(runtime, effective_token, endpoint, body, allowed).await
        }
        result => result,
    }
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
    account_post_with_captcha(endpoint, body, None).await
}

async fn account_post_with_captcha(
    endpoint: &str,
    body: Value,
    captcha_token: Option<&str>,
) -> Result<(u16, Value), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("创建账号请求客户端失败：{error}"))?;
    let mut request = client
        .post(format!("{ACCOUNT_BASE}{endpoint}"))
        .header(CONTENT_TYPE, "application/json")
        .header("x-client-id", OAUTH_CLIENT_ID)
        .header("x-sdk-version", "9.0.2")
        .header("x-protocol-version", "301")
        .header("accept-language", "zh-CN")
        .json(&body);
    if let Some(captcha_token) = captcha_token.filter(|value| !value.trim().is_empty()) {
        request = request.header("x-captcha-token", captcha_token.trim());
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
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

fn account_payload_value<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    payload
        .get(key)
        .or_else(|| payload.get("data").and_then(|data| data.get(key)))
}

fn account_payload_string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = account_payload_value(payload, key)?;
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_i64().map(|value| value.to_string()))
            .filter(|value| !value.trim().is_empty())
    })
}

fn account_payload_bool(payload: &Value, key: &str) -> Option<bool> {
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

fn account_error_message(payload: &Value, fallback: &str) -> String {
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

fn payload_mentions_captcha(payload: &Value) -> bool {
    let serialized = payload.to_string().to_ascii_lowercase();
    serialized.contains("captcha")
        || serialized.contains("人机验证")
        || serialized.contains("安全验证")
}

fn flatten_account_payload(payload: &Value) -> serde_json::Map<String, Value> {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    if let Some(data) = payload.get("data").and_then(Value::as_object) {
        for (key, value) in data {
            object.insert(key.clone(), value.clone());
        }
    }
    object.remove("data");
    object
}

fn captcha_challenge_response(payload: &Value, force: bool) -> Option<Value> {
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

fn normalize_china_phone(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|character| !character.is_ascii_digit() && !"+ -()".contains(character))
    {
        return Err("请输入有效的中国大陆手机号".to_string());
    }
    let digits = trimmed
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    let local = if digits.len() == 13 && digits.starts_with("86") {
        &digits[2..]
    } else if digits.len() == 11 {
        digits.as_str()
    } else {
        return Err("请输入 11 位中国大陆手机号".to_string());
    };
    if !local.starts_with('1') {
        return Err("请输入有效的中国大陆手机号".to_string());
    }
    Ok(format!("+86 {local}"))
}

fn masked_phone_name(phone_number: &str) -> String {
    let local = phone_number
        .trim()
        .strip_prefix("+86 ")
        .unwrap_or(phone_number);
    if local.len() == 11 && local.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("用户{}****{}", &local[..3], &local[7..])
    } else {
        "光鸭用户".to_string()
    }
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
        let cached = {
            let guard = state.lock().map_err(|e| e.to_string())?;
            guard
                .cache_enabled
                .then(|| guard.remote_cache.get(&cache_key).cloned())
                .flatten()
        };
        if let Some(cached) = cached {
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
        {
            let mut guard = state.lock().map_err(|e| e.to_string())?;
            if guard.cache_enabled {
                guard.remote_cache.insert(cache_key, file_id.clone());
                let max_entries = guard.cache_max_entries;
                trim_remote_cache(&mut guard.remote_cache, max_entries);
            }
        }
        parent = file_id;
    }
    Ok(parent)
}

fn parse_gcid_file_size(value: &Value) -> Result<u64, String> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .filter(|size| *size > 0)
            .ok_or_else(|| "文件大小必须是正整数".to_string()),
        Value::String(value) => value
            .parse::<u64>()
            .ok()
            .filter(|size| *size > 0)
            .ok_or_else(|| "文件大小必须是正整数字符串".to_string()),
        _ => Err("文件大小格式无效".to_string()),
    }
}

fn normalize_gcid_relative_path(value: &str) -> Result<String, String> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized
            .as_bytes()
            .get(1)
            .is_some_and(|value| *value == b':')
    {
        return Err(format!("不是合法的相对路径：{value}"));
    }
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!("路径包含空目录或越界片段：{value}"));
    }
    if parts.iter().any(|part| part.chars().any(char::is_control)) {
        return Err(format!("路径包含控制字符：{value}"));
    }
    Ok(parts.join("/"))
}

fn parse_gcid_export(raw: &[u8]) -> Result<(Vec<GcidImportFile>, u128, String), String> {
    let export: GcidExport =
        serde_json::from_slice(raw).map_err(|error| format!("JSON 格式无效：{error}"))?;
    if export.source != "guangya" || export.hash_type != "gcid" || !export.uses_gcid_in_export {
        return Err("只支持光鸭 GCID 导出格式".to_string());
    }
    if export.files.is_empty() {
        return Err("导入文件不包含 files 记录".to_string());
    }
    if export
        .total_files_count
        .is_some_and(|total| total != export.files.len() as u64)
    {
        return Err(format!(
            "文件总数不一致：声明 {}，实际 {}",
            export.total_files_count.unwrap_or_default(),
            export.files.len()
        ));
    }
    let mut seen = HashSet::with_capacity(export.files.len());
    let mut total_size = 0_u128;
    let mut files = Vec::with_capacity(export.files.len());
    for (index, item) in export.files.into_iter().enumerate() {
        let relative_path = normalize_gcid_relative_path(&item.path)
            .map_err(|error| format!("第 {} 条记录：{error}", index + 1))?;
        if !seen.insert(relative_path.clone()) {
            return Err(format!("存在重复路径：{relative_path}"));
        }
        let size = parse_gcid_file_size(&item.size)
            .map_err(|error| format!("第 {} 条记录：{error}", index + 1))?;
        let gcid = item.gcid.to_ascii_lowercase();
        if gcid.len() != 40 || !gcid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("第 {} 条记录的 GCID 无效", index + 1));
        }
        let (folder_path, name) = relative_path
            .rsplit_once('/')
            .map(|(folder, name)| (folder.to_string(), name.to_string()))
            .unwrap_or_else(|| (String::new(), relative_path.clone()));
        total_size = total_size
            .checked_add(size as u128)
            .ok_or_else(|| "导入文件总大小溢出".to_string())?;
        files.push(GcidImportFile {
            path: relative_path,
            folder_path,
            name,
            size,
            gcid,
            attempts: 0,
        });
    }
    if let Some(declared) = export.total_size.as_ref() {
        let declared = match declared {
            Value::Number(number) => number.as_u64().map(u128::from),
            Value::String(value) => value.parse::<u128>().ok(),
            _ => None,
        };
        if declared.is_some_and(|declared| declared != total_size) {
            return Err(format!(
                "文件总大小不一致：声明 {}，实际 {total_size}",
                declared.unwrap_or_default()
            ));
        }
    }
    Ok((files, total_size, export.common_path))
}

fn validate_gcid_destination(value: &str) -> Result<String, String> {
    let destination = value.trim();
    if destination.is_empty()
        || destination == "."
        || destination == ".."
        || destination.contains('/')
        || destination.contains('\\')
        || destination.chars().any(char::is_control)
    {
        return Err("目标文件夹名称不能为空，也不能包含斜杠、控制字符或越界片段".to_string());
    }
    Ok(destination.to_string())
}

fn gcid_import_job_id(raw: &[u8], destination_parent_id: &str, destination_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    hasher.update(b"\0");
    hasher.update(destination_parent_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(destination_name.as_bytes());
    hex::encode(hasher.finalize())[..32].to_string()
}

fn prepare_gcid_import_database(
    database_path: &Path,
    raw: &[u8],
    source_path: &Path,
    destination_parent_id: &str,
    destination_name: &str,
) -> Result<String, String> {
    let destination_name = validate_gcid_destination(destination_name)?;
    let (files, total_size, _) = parse_gcid_export(raw)?;
    let job_id = gcid_import_job_id(raw, destination_parent_id, &destination_name);
    let source_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("GCID 导入.json")
        .to_string();
    let source_path = source_path.to_string_lossy().to_string();
    let now = unix_timestamp();
    let mut connection = open_database(database_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始导入事务失败：{error}"))?;
    transaction
        .execute(
            "INSERT INTO gcid_import_jobs
               (job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ready', '', NULL, ?8, ?8)
             ON CONFLICT(job_id) DO UPDATE SET
               source_path = excluded.source_path,
               source_name = excluded.source_name,
               destination_parent_id = excluded.destination_parent_id,
               destination_name = excluded.destination_name,
               total_files = excluded.total_files,
               total_size = excluded.total_size,
               status = CASE
                 WHEN gcid_import_jobs.status IN ('completed', 'completed_with_errors')
                   THEN gcid_import_jobs.status
                 ELSE 'ready'
               END,
               error = NULL,
               updated_at = excluded.updated_at",
            params![
                job_id,
                source_path,
                source_name,
                destination_parent_id,
                destination_name,
                files.len() as i64,
                total_size.to_string(),
                now
            ],
        )
        .map_err(|error| format!("保存导入任务失败：{error}"))?;
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO gcid_import_files
                   (job_id, path, folder_path, file_name, file_size, gcid,
                    status, attempts, task_id, file_id, error, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, NULL, NULL, NULL, ?7)
                 ON CONFLICT(job_id, path) DO UPDATE SET
                   folder_path = excluded.folder_path,
                   file_name = excluded.file_name,
                   file_size = excluded.file_size,
                   gcid = excluded.gcid
                 WHERE gcid_import_files.status NOT IN ('imported', 'existing')",
            )
            .map_err(|error| format!("准备导入记录失败：{error}"))?;
        for file in files {
            let size = i64::try_from(file.size).map_err(|_| format!("文件过大：{}", file.path))?;
            insert
                .execute(params![
                    job_id,
                    file.path,
                    file.folder_path,
                    file.name,
                    size,
                    file.gcid,
                    now
                ])
                .map_err(|error| format!("保存导入记录失败：{error}"))?;
        }
    }
    transaction
        .execute(
            "UPDATE gcid_import_files
             SET status = 'pending', error = '上次任务中断，已等待继续', updated_at = ?2
             WHERE job_id = ?1 AND status = 'processing'",
            params![job_id, now],
        )
        .map_err(|error| format!("恢复导入记录失败：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("提交导入任务失败：{error}"))?;
    Ok(job_id)
}

fn load_gcid_import_counts(
    connection: &Connection,
    job_id: &str,
) -> Result<GcidImportCounts, String> {
    let mut counts = GcidImportCounts::default();
    let mut statement = connection
        .prepare(
            "SELECT status, COUNT(*)
             FROM gcid_import_files
             WHERE job_id = ?1
             GROUP BY status",
        )
        .map_err(|error| format!("读取导入统计失败：{error}"))?;
    let rows = statement
        .query_map(params![job_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|error| format!("查询导入统计失败：{error}"))?;
    for row in rows {
        let (status, count) = row.map_err(|error| format!("解析导入统计失败：{error}"))?;
        match status.as_str() {
            "pending" => counts.pending = count,
            "processing" => counts.processing = count,
            "imported" => counts.imported = count,
            "existing" => counts.existing = count,
            "missed" => counts.missed = count,
            "conflict" => counts.conflict = count,
            "failed" => counts.failed = count,
            _ => {}
        }
    }
    Ok(counts)
}

fn load_gcid_import_status(
    database_path: &Path,
    job_id: Option<&str>,
) -> Result<Option<GcidImportStatus>, String> {
    let connection = open_database(database_path)?;
    let query = if job_id.is_some() {
        "SELECT job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, updated_at
         FROM gcid_import_jobs WHERE job_id = ?1"
    } else {
        "SELECT job_id, source_path, source_name, destination_parent_id, destination_name,
                total_files, total_size, status, current_path, error, updated_at
         FROM gcid_import_jobs ORDER BY updated_at DESC LIMIT 1"
    };
    let load = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, u64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, i64>(10)?,
        ))
    };
    let record = if let Some(job_id) = job_id {
        connection
            .query_row(query, params![job_id], load)
            .optional()
    } else {
        connection.query_row(query, [], load).optional()
    }
    .map_err(|error| format!("读取导入任务失败：{error}"))?;
    let Some((
        job_id,
        source_path,
        source_name,
        destination_parent_id,
        destination_name,
        total_files,
        total_size,
        status,
        current_path,
        error,
        updated_at,
    )) = record
    else {
        return Ok(None);
    };
    let counts = load_gcid_import_counts(&connection, &job_id)?;
    let finished =
        counts.imported + counts.existing + counts.missed + counts.conflict + counts.failed;
    Ok(Some(GcidImportStatus {
        job_id,
        source_path,
        source_name,
        destination_parent_id,
        destination_name,
        total_files,
        total_size,
        status,
        current_path,
        error,
        counts,
        finished,
        updated_at,
    }))
}

fn emit_gcid_import_status(app: &tauri::AppHandle, database_path: &Path, job_id: &str) {
    if let Ok(Some(import_status)) = load_gcid_import_status(database_path, Some(job_id)) {
        emit(
            app,
            json!({ "type": "gcid-import", "status": import_status }),
        );
    }
}

fn claim_gcid_import_file(
    database_path: &Path,
    job_id: &str,
) -> Result<Option<GcidImportFile>, String> {
    let mut connection = open_database(database_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始领取导入记录失败：{error}"))?;
    let record = transaction
        .query_row(
            "SELECT path, folder_path, file_name, file_size, gcid, attempts
             FROM gcid_import_files
             WHERE job_id = ?1 AND status = 'pending'
             ORDER BY path
             LIMIT 1",
            params![job_id],
            |row| {
                Ok(GcidImportFile {
                    path: row.get(0)?,
                    folder_path: row.get(1)?,
                    name: row.get(2)?,
                    size: row.get(3)?,
                    gcid: row.get(4)?,
                    attempts: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("领取导入记录失败：{error}"))?;
    if let Some(record) = record.as_ref() {
        let changed = transaction
            .execute(
                "UPDATE gcid_import_files
                 SET status = 'processing', error = NULL, updated_at = ?3
                 WHERE job_id = ?1 AND path = ?2 AND status = 'pending'",
                params![job_id, record.path, unix_timestamp()],
            )
            .map_err(|error| format!("锁定导入记录失败：{error}"))?;
        if changed == 0 {
            transaction
                .rollback()
                .map_err(|error| format!("回滚导入记录失败：{error}"))?;
            return claim_gcid_import_file(database_path, job_id);
        }
        transaction
            .execute(
                "UPDATE gcid_import_jobs
                 SET current_path = ?2, updated_at = ?3
                 WHERE job_id = ?1",
                params![job_id, record.path, unix_timestamp()],
            )
            .map_err(|error| format!("更新当前导入文件失败：{error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("提交导入记录失败：{error}"))?;
    Ok(record)
}

fn update_gcid_import_attempt(
    database_path: &Path,
    job_id: &str,
    path: &str,
    attempt: i64,
    error: Option<&str>,
) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET attempts = ?3, error = ?4, updated_at = ?5
             WHERE job_id = ?1 AND path = ?2",
            params![job_id, path, attempt, error, unix_timestamp()],
        )
        .map_err(|error| format!("更新导入重试状态失败：{error}"))?;
    Ok(())
}

fn finish_gcid_import_file(
    database_path: &Path,
    job_id: &str,
    path: &str,
    status: &str,
    task_id: Option<&str>,
    file_id: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    open_database(database_path)?
        .execute(
            "UPDATE gcid_import_files
             SET status = ?3, task_id = ?4, file_id = ?5, error = ?6, updated_at = ?7
             WHERE job_id = ?1 AND path = ?2",
            params![
                job_id,
                path,
                status,
                task_id,
                file_id,
                error,
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存导入结果失败：{error}"))?;
    Ok(())
}

fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| match value {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.parse::<u64>().ok(),
        _ => None,
    })
}

async fn find_remote_file(
    token: &str,
    device_id: &str,
    parent_id: &str,
    name: &str,
) -> Result<Option<(String, u64, i64)>, String> {
    for page in 0..1000 {
        let result = api_post(
            token,
            device_id,
            "/userres/v1/file/get_file_list",
            json!({
                "page": page,
                "pageSize": 100,
                "parentId": parent_id,
                "orderBy": 0,
                "sortType": 0,
                "needSubFolderStat": true
            }),
            &[],
        )
        .await?;
        let data = result.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(found) = list
            .iter()
            .find(|item| item.get("fileName").and_then(Value::as_str) == Some(name))
        {
            let file_id = found
                .get("fileId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let file_size = value_as_u64(found.get("fileSize")).unwrap_or_default();
            let res_type = found
                .get("resType")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if !file_id.is_empty() {
                return Ok(Some((file_id, file_size, res_type)));
            }
        }
        let total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        if list.is_empty() || ((page + 1) * 100) as u64 >= total {
            break;
        }
    }
    Ok(None)
}

async fn wait_gcid_import_task(
    token: &str,
    device_id: &str,
    task_id: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(CLOUD_CONFIRM_TIMEOUT_SECS);
    let mut attempt = 0_u64;
    while Instant::now() < deadline {
        match check_upload_task(token, device_id, task_id).await {
            Ok(CloudTaskCheck::Confirmed(data)) => {
                return data
                    .get("fileId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| "云端入库完成但没有返回文件 ID".to_string());
            }
            Ok(CloudTaskCheck::Pending) => {}
            Err(CloudConfirmError::Retryable(message)) => {
                if message.contains("登录态已失效") {
                    return Err(message);
                }
            }
            Err(CloudConfirmError::Permanent(message)) => return Err(message),
        }
        attempt += 1;
        let wait = Duration::from_millis((500 * attempt.div_ceil(5)).clamp(500, 5_000));
        sleep(wait.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    Err(format!(
        "云端入库超过 {CLOUD_CONFIRM_TIMEOUT_SECS} 秒仍未完成"
    ))
}

async fn process_gcid_import_file(
    state: &SharedState,
    destination_parent_id: &str,
    destination_name: &str,
    record: &GcidImportFile,
) -> Result<GcidImportOutcome, String> {
    let (token, device_id) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
        )
    };
    let remote_path = if record.folder_path.is_empty() {
        destination_name.to_string()
    } else {
        format!("{destination_name}/{}", record.folder_path)
    };
    let parent_id = ensure_remote_path(
        state,
        &token,
        &device_id,
        destination_parent_id,
        &remote_path,
    )
    .await?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_center_token",
        json!({
            "capacity": 2,
            "name": record.name,
            "res": { "fileSize": record.size },
            "parentId": parent_id
        }),
        &[156, 159],
    )
    .await?;
    if response.code == 159 {
        return match find_remote_file(&token, &device_id, &parent_id, &record.name).await? {
            Some((file_id, file_size, 1)) if file_size == record.size => {
                Ok(GcidImportOutcome::Existing { file_id })
            }
            Some((_, file_size, 1)) => Ok(GcidImportOutcome::Conflict(format!(
                "同名文件大小不一致：云端 {file_size}，导入 {}",
                record.size
            ))),
            Some(_) => Ok(GcidImportOutcome::Conflict("同名项是文件夹".to_string())),
            None => Err("光鸭返回名称冲突，但未找到同名文件".to_string()),
        };
    }
    let mut task_id = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "光鸭没有返回上传任务 ID".to_string())?;
    let mut instant = response.code == 156;
    if !instant {
        let flash = api_post(
            &token,
            &device_id,
            "/userres/v1/check_can_flash_upload",
            json!({ "taskId": task_id, "gcid": record.gcid }),
            &[],
        )
        .await?;
        let data = flash.data.unwrap_or_default();
        instant = data
            .get("canFlashUpload")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(next_task_id) = data
            .get("taskId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            task_id = next_task_id.to_string();
        }
    }
    if !instant {
        return Ok(GcidImportOutcome::Missed { task_id });
    }
    let file_id = wait_gcid_import_task(&token, &device_id, &task_id).await?;
    Ok(GcidImportOutcome::Imported { task_id, file_id })
}

async fn gcid_import_worker(
    app: tauri::AppHandle,
    state: SharedState,
    database_path: PathBuf,
    job_id: String,
    destination_parent_id: String,
    destination_name: String,
    completed_since_emit: Arc<AtomicUsize>,
) {
    loop {
        let record = match claim_gcid_import_file(&database_path, &job_id) {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(error) => {
                status(&app, "error", error);
                break;
            }
        };
        let first_attempt = (record.attempts + 1).clamp(1, MAX_GCID_IMPORT_ATTEMPTS);
        let mut terminal = false;
        for attempt in first_attempt..=MAX_GCID_IMPORT_ATTEMPTS {
            let _ =
                update_gcid_import_attempt(&database_path, &job_id, &record.path, attempt, None);
            match process_gcid_import_file(
                &state,
                &destination_parent_id,
                &destination_name,
                &record,
            )
            .await
            {
                Ok(GcidImportOutcome::Imported { task_id, file_id }) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "imported",
                        Some(&task_id),
                        Some(&file_id),
                        None,
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Existing { file_id }) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "existing",
                        None,
                        Some(&file_id),
                        None,
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Missed { task_id }) => {
                    let message = "光鸭未命中该 GCID，且没有本地源文件可普通上传";
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "missed",
                        Some(&task_id),
                        None,
                        Some(message),
                    );
                    terminal = true;
                    break;
                }
                Ok(GcidImportOutcome::Conflict(error)) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "conflict",
                        None,
                        None,
                        Some(&error),
                    );
                    terminal = true;
                    break;
                }
                Err(error) if attempt < MAX_GCID_IMPORT_ATTEMPTS => {
                    let _ = update_gcid_import_attempt(
                        &database_path,
                        &job_id,
                        &record.path,
                        attempt,
                        Some(&error),
                    );
                    sleep(Duration::from_secs((attempt as u64).clamp(1, 5))).await;
                }
                Err(error) => {
                    let _ = finish_gcid_import_file(
                        &database_path,
                        &job_id,
                        &record.path,
                        "failed",
                        None,
                        None,
                        Some(&error),
                    );
                    status(
                        &app,
                        "error",
                        format!("GCID 导入失败：{}：{error}", record.path),
                    );
                    terminal = true;
                    break;
                }
            }
        }
        if !terminal {
            let _ = finish_gcid_import_file(
                &database_path,
                &job_id,
                &record.path,
                "failed",
                None,
                None,
                Some("达到最大重试次数"),
            );
        }
        let completed = completed_since_emit.fetch_add(1, Ordering::Relaxed) + 1;
        if completed % 50 == 0 {
            emit_gcid_import_status(&app, &database_path, &job_id);
        }
    }
}

async fn run_gcid_import(
    app: tauri::AppHandle,
    state: SharedState,
    database_path: PathBuf,
    job_id: String,
    destination_parent_id: String,
    destination_name: String,
    concurrency: usize,
) {
    let completed_since_emit = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        workers.push(tauri::async_runtime::spawn(gcid_import_worker(
            app.clone(),
            state.clone(),
            database_path.clone(),
            job_id.clone(),
            destination_parent_id.clone(),
            destination_name.clone(),
            completed_since_emit.clone(),
        )));
    }
    for worker in workers {
        let _ = worker.await;
    }
    let final_status = load_gcid_import_status(&database_path, Some(&job_id))
        .ok()
        .flatten();
    let (status_value, error_value) = match final_status.as_ref() {
        Some(result) if result.counts.pending > 0 || result.counts.processing > 0 => {
            ("paused", Some("仍有未处理记录，可点击继续导入".to_string()))
        }
        Some(result)
            if result.counts.failed > 0
                || result.counts.missed > 0
                || result.counts.conflict > 0 =>
        {
            (
                "completed_with_errors",
                Some("导入完成，但存在异常记录".to_string()),
            )
        }
        Some(_) => ("completed", None),
        None => ("failed", Some("无法读取导入任务状态".to_string())),
    };
    if let Ok(connection) = open_database(&database_path) {
        let _ = connection.execute(
            "UPDATE gcid_import_jobs
             SET status = ?2, current_path = '', error = ?3, updated_at = ?4
             WHERE job_id = ?1",
            params![job_id, status_value, error_value, unix_timestamp()],
        );
    }
    if let Ok(mut guard) = state.lock() {
        guard.gcid_import_running.remove(&job_id);
    }
    emit_gcid_import_status(&app, &database_path, &job_id);
    if status_value == "completed" {
        status(&app, "success", "GCID JSON 秒传导入完成");
    } else {
        status(&app, "warning", "GCID JSON 秒传导入结束，请查看导入统计");
    }
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

fn normalize_share_access(
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

fn share_file_payload(
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
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS))
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
    let payload: Value = serde_json::from_str(&raw).map_err(|error| {
        let preview = safe_response_preview(&raw, 200);
        if preview.is_empty() {
            format!("Hdhive 返回非 JSON 响应（HTTP {status_code}）：{error}；响应正文为空")
        } else {
            format!("Hdhive 返回非 JSON 响应（HTTP {status_code}）：{error}；响应正文：{preview}")
        }
    })?;
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
            Ok(guard) if guard.hdhive_enabled => (
                guard.hdhive_base_url.clone(),
                guard.hdhive_secret.clone(),
                guard.hdhive_instance_id.clone(),
                guard.db_path.clone(),
            ),
            Ok(_) => return,
            Err(_) => return,
        };
        match hdhive_request(
            &base_url,
            &secret,
            &instance_id,
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
    let accepted = hdhive_request(
        &base_url,
        &secret,
        &instance_id,
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

async fn auto_share_loop(app: tauri::AppHandle, state: SharedState) {
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

fn is_cloud_index_permanent_message(message: &str) -> bool {
    [
        "文件违规",
        "违规文件",
        "禁止上传",
        "不允许上传",
        "上传失败",
        "入库失败",
        "任务失败",
        "任务不存在",
        "文件不存在",
        "任务已取消",
        "任务被取消",
        "任务已过期",
    ]
    .iter()
    .any(|permanent| message.contains(permanent))
}

async fn check_upload_task(
    token: &str,
    device_id: &str,
    task_id: &str,
) -> Result<CloudTaskCheck, CloudConfirmError> {
    match api_post(
        token,
        device_id,
        "/userres/v1/file/get_info_by_task_id",
        json!({ "taskId": task_id }),
        &[145, 146, 155, 163],
    )
    .await
    {
        Ok(result) if is_cloud_index_permanent_message(&result.msg) => {
            Err(CloudConfirmError::Permanent(result.msg))
        }
        Ok(result) => Ok(result
            .data
            .filter(|data| {
                data.get("fileId")
                    .and_then(Value::as_str)
                    .is_some_and(|file_id| !file_id.is_empty())
            })
            .map(CloudTaskCheck::Confirmed)
            .unwrap_or(CloudTaskCheck::Pending)),
        Err(message) if is_cloud_index_pending_message(&message) => Ok(CloudTaskCheck::Pending),
        Err(message) if is_cloud_index_permanent_message(&message) => {
            Err(CloudConfirmError::Permanent(message))
        }
        Err(message) => Err(CloudConfirmError::Retryable(message)),
    }
}

async fn wait_upload_task(
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

fn oss_checkpoint_uploaded_bytes(checkpoint: &OssUploadCheckpoint, size: u64) -> u64 {
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

fn oss_request_url(
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

fn oss_string_to_sign(
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

async fn oss_signed_request(
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
    let body = body.map(Bytes::from);
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

fn xml_tag_value(body: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = body.find(&start_tag)? + start_tag.len();
    let end = body[start..].find(&end_tag)? + start;
    Some(body[start..end].trim().to_string())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn upload_oss(
    token_data: &UploadToken,
    item: &UploadItem,
    app: &tauri::AppHandle,
    multipart_part_size: &str,
    oss_part_concurrency: usize,
    oss_part_semaphore: Arc<Semaphore>,
    checkpoint_writer: &UploadCheckpointWriter,
    persisted: Option<PersistedUploadCheckpoint>,
) -> Result<(), String> {
    let credentials = token_data
        .creds
        .as_ref()
        .ok_or_else(|| "光鸭没有返回 OSS 临时凭证".to_string())?;
    let size = fs::metadata(&item.file_path)
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
            end_point: token_data
                .end_point
                .clone()
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
        checkpoint_writer.flush(item, Some(&checkpoint), 0).await?;
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
    let effective_part_concurrency =
        effective_oss_part_concurrency(checkpoint.part_size, oss_part_concurrency);
    let pending_parts = (1..=total_parts as u32)
        .filter(|part| !checkpoint.completed_parts.contains_key(part))
        .collect::<Vec<_>>();
    let initial_parallelism = effective_part_concurrency.min(pending_parts.len()).max(1);
    let mut speed_tracker = UploadSpeedTracker::new(initial_parallelism);
    let mut part_uploads = stream::iter(pending_parts)
        .map(|part| {
            let client = &client;
            let request_checkpoint = &request_checkpoint;
            let file_path = &item.file_path;
            let oss_part_semaphore = oss_part_semaphore.clone();
            async move {
                let _permit = oss_part_semaphore
                    .acquire_many_owned(oss_buffer_permits(request_checkpoint.part_size))
                    .await
                    .map_err(|_| "OSS 分片并发控制已关闭".to_string())?;
                let offset = u64::from(part - 1) * request_checkpoint.part_size;
                let length = request_checkpoint.part_size.min(size - offset);
                let mut file = tokio::fs::File::open(file_path)
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
                    file_path,
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
                Ok::<_, String>((part, etag, length))
            }
        })
        .buffer_unordered(effective_part_concurrency);
    let mut checkpoint_dirty = false;
    let mut checkpoint_saved_bytes = uploaded_at_start;
    let mut checkpoint_saved_at = Instant::now();
    let mut checkpoint_flush_interval =
        tokio::time::interval(Duration::from_secs(OSS_CHECKPOINT_FLUSH_INTERVAL_SECS));
    checkpoint_flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    checkpoint_flush_interval.tick().await;
    loop {
        tokio::select! {
            result = part_uploads.next() => {
                let Some(result) = result else { break };
                let (part, etag, part_length) = match result {
                    Ok(value) => value,
                    Err(error) => {
                        let uploaded = oss_checkpoint_uploaded_bytes(&checkpoint, size);
                        if let Err(save_error) = checkpoint_writer
                            .flush(item, Some(&checkpoint), uploaded)
                            .await
                        {
                            return Err(format!("{error}；{save_error}"));
                        }
                        return Err(error);
                    }
                };
                checkpoint.completed_parts.insert(part, etag);
                checkpoint_dirty = true;
                let uploaded = oss_checkpoint_uploaded_bytes(&checkpoint, size);
                let speed = speed_tracker.observe(
                    upload_started_at.elapsed(),
                    uploaded.saturating_sub(uploaded_at_start),
                );
                emit(
                    app,
                    json!({
                        "type": "progress",
                        "file_path": item.file_path.to_string_lossy(),
                        "percent": uploaded.saturating_mul(100) / size,
                        "uploaded_bytes": uploaded,
                        "total_bytes": size,
                        "bytes_per_second": speed.bytes_per_second,
                        "average_bytes_per_second": speed.average_bytes_per_second,
                        "completed_part_bytes": part_length,
                        "stage": if resumed { "正在断点续传" } else { "正在上传" }
                    }),
                );
                if should_flush_upload_checkpoint(
                    checkpoint_saved_bytes,
                    uploaded,
                    size,
                    checkpoint_saved_at.elapsed(),
                ) {
                    checkpoint_writer.enqueue(item, &checkpoint, uploaded)?;
                    checkpoint_dirty = false;
                    checkpoint_saved_bytes = uploaded;
                    checkpoint_saved_at = Instant::now();
                }
            }
            _ = checkpoint_flush_interval.tick(), if checkpoint_dirty => {
                let uploaded = oss_checkpoint_uploaded_bytes(&checkpoint, size);
                checkpoint_writer.enqueue(item, &checkpoint, uploaded)?;
                checkpoint_dirty = false;
                checkpoint_saved_bytes = uploaded;
                checkpoint_saved_at = Instant::now();
            }
        }
    }
    let uploaded = oss_checkpoint_uploaded_bytes(&checkpoint, size);
    checkpoint_writer
        .flush(item, Some(&checkpoint), uploaded)
        .await?;

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
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": size, "total_bytes": size, "bytes_per_second": 0, "stage": "OSS 上传完成" }),
    );
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
    upload_priority: Option<&SharedState>,
) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("读取秒传文件失败：{error}"))?;
    let chunk_size = gcid_chunk_size(file_size);
    let mut buffer = vec![0_u8; chunk_size];
    let mut outer = Sha1::new();
    let mut hashed = 0_u64;
    let progress_started_at = Instant::now();
    let mut last_emit_elapsed = Duration::ZERO;
    let mut last_percent = 0;
    loop {
        if let Some(state) = upload_priority {
            loop {
                let can_run = state
                    .lock()
                    .map_err(|error| error.to_string())
                    .map(|guard| background_gcid_may_run(guard.active_oss_uploads))?;
                if can_run {
                    break;
                }
                sleep(Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS)).await;
            }
        }
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
        let elapsed = progress_started_at.elapsed();
        if should_emit_gcid_progress(
            last_emit_elapsed,
            last_percent,
            elapsed,
            percent,
            hashed >= file_size,
        ) {
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
            last_emit_elapsed = elapsed;
            last_percent = percent;
        }
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

fn upload_source_root(state: &SharedState, item: &UploadItem) -> Result<Option<PathBuf>, String> {
    if item.mapping_id == "__manual__" {
        return Ok(None);
    }
    state
        .lock()
        .map_err(|error| error.to_string())?
        .mappings
        .iter()
        .find(|mapping| mapping.id == item.mapping_id && mapping.enabled)
        .map(|mapping| Some(PathBuf::from(&mapping.local_path)))
        .ok_or_else(|| "备份任务已停用或已被移除".to_string())
}

async fn prepare_upload_item(
    item: &UploadItem,
    source_root: Option<&Path>,
) -> Result<Option<UploadItem>, String> {
    let canonical_source = canonical_upload_source(&item.file_path, source_root)?;
    if !file_available_for_upload(&canonical_source)? {
        return Ok(None);
    }
    let first =
        fs::metadata(&canonical_source).map_err(|error| format!("读取源文件失败：{error}"))?;
    if !first.is_file() {
        return Err("源路径不是文件".into());
    }
    sleep(Duration::from_millis(FILE_STABILITY_WAIT_MS)).await;
    let second_canonical = canonical_upload_source(&item.file_path, source_root)?;
    if second_canonical != canonical_source {
        return Err("源文件真实路径在上传准备期间发生变化，已拒绝上传".to_string());
    }
    if !file_available_for_upload(&canonical_source)? {
        return Ok(None);
    }
    let second =
        fs::metadata(&canonical_source).map_err(|error| format!("读取源文件失败：{error}"))?;
    if !same_file_identity(&first, &second)
        || first.len() != second.len()
        || modified_ms(&first) != modified_ms(&second)
    {
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
    let key = item_key(&item);
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
            &guard.pending_cloud,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &item,
        ) {
            false
        } else {
            guard.queue.retain(|queued| item_key(queued) != key);
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

async fn requeue_resumable_upload(app: tauri::AppHandle, state: SharedState, item: UploadItem) {
    sleep(Duration::from_secs(PENDING_UPLOAD_RETRY_SECS)).await;
    let db_path = match state.lock() {
        Ok(guard) => guard.db_path.clone(),
        Err(_) => return,
    };
    let checkpoint = match load_upload_checkpoint(&db_path, &item) {
        Ok(Some(value)) => value,
        _ => return,
    };
    let key = item_key(&item);
    let queued = if let Ok(mut guard) = state.lock() {
        let mapping_active = item.mapping_id == "__manual__"
            || guard
                .mappings
                .iter()
                .any(|mapping| mapping.id == item.mapping_id && mapping.enabled);
        if !mapping_active
            || upload_already_scheduled(
                &guard.history,
                &guard.pending_cloud,
                &guard.inflight,
                &guard.queue,
                &guard.waiting_files,
                &item,
            )
        {
            false
        } else {
            guard.queue.retain(|queued| item_key(queued) != key);
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
                "state": "queued",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": checkpoint.uploaded_bytes,
                "total_bytes": item.size,
                "stage": "上传中断，已保留断点并自动重试"
            }),
        );
        emit_state(&app, &state);
        drain_queue(app, state);
    }
}

async fn preflight_flash_upload(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
) -> Result<FlashPreflightOutcome, String> {
    let (token, device_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.db_path.clone(),
        )
    };
    if load_upload_checkpoint(&db_path, item)?.is_some() {
        return Ok(FlashPreflightOutcome::Skipped);
    }

    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 0,
            "uploaded_bytes": 0,
            "total_bytes": item.size,
            "bytes_per_second": 0,
            "stage": "正在后台校验秒传"
        }),
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
        .and_then(|value| value.to_str())
        .ok_or_else(|| "无法读取文件名".to_string())?;
    let mut res = json!({ "fileSize": item.size });
    if item.size < OSS_MIB {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": 0,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "bytes_per_second": 0,
                "stage": "正在后台计算秒传 MD5"
            }),
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
    let mut instant_upload = result.code == 156;
    let mut data: UploadToken = serde_json::from_value(
        result
            .data
            .ok_or_else(|| "光鸭没有返回上传凭证".to_string())?,
    )
    .map_err(|error| format!("上传凭证格式异常：{error}"))?;

    if !instant_upload && item.size >= OSS_MIB {
        let cached_gcid = match {
            let guard = state.lock().map_err(|error| error.to_string())?;
            load_cached_file_gcid(
                &guard.db_path,
                &item.file_path,
                item.size,
                item.modified_ms,
                cache_settings(&guard),
            )
        } {
            Ok(value) => value,
            Err(error) => {
                status(app, "warning", error);
                None
            }
        };
        let gcid_result = if let Some(gcid) = cached_gcid {
            emit(
                app,
                json!({
                    "type": "progress",
                    "file_path": item.file_path.to_string_lossy(),
                    "percent": 0,
                    "uploaded_bytes": 0,
                    "total_bytes": item.size,
                    "bytes_per_second": 0,
                    "stage": "后台已复用本地秒传指纹"
                }),
            );
            Ok(gcid)
        } else {
            let result = calculate_file_gcid(app, &item.file_path, item.size, Some(state)).await;
            if let Ok(gcid) = &result {
                let saved = {
                    let guard = state.lock().map_err(|error| error.to_string())?;
                    save_cached_file_gcid(
                        &guard.db_path,
                        &item.file_path,
                        item.size,
                        item.modified_ms,
                        gcid,
                        cache_settings(&guard),
                    )
                };
                if let Err(error) = saved {
                    status(app, "warning", error);
                }
            }
            result
        };
        match gcid_result {
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
                    format!("后台秒传校验失败，稍后继续普通上传：{error}"),
                ),
            },
            Err(error) => status(
                app,
                "warning",
                format!("后台秒传指纹计算失败，稍后继续普通上传：{error}"),
            ),
        }
    }

    if !instant_upload {
        return Ok(FlashPreflightOutcome::Miss(data));
    }

    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 100,
            "uploaded_bytes": item.size,
            "total_bytes": item.size,
            "bytes_per_second": 0,
            "stage": "已命中秒传"
        }),
    );
    let pending_outcome = UploadOutcome {
        task_id: data.task_id.clone(),
        remote_file_id: None,
    };
    remember_pending_upload(state, item, &pending_outcome)
        .map_err(|message| format!("文件已秒传，但写入本地上传记录失败：{message}"))?;
    if let Err(error) = clear_upload_checkpoint(&db_path, item) {
        status(
            app,
            "warning",
            format!("秒传记录已保存，但清理旧上传断点失败：{error}"),
        );
    }
    emit(
        app,
        json!({
            "type": "file",
            "state": "processing",
            "file_path": item.file_path.to_string_lossy(),
            "mapping_id": item.mapping_id,
            "uploaded_bytes": item.size,
            "total_bytes": item.size,
            "stage": "已秒传，正在等待云端入库"
        }),
    );
    Ok(FlashPreflightOutcome::Accepted {
        task_id: data.task_id,
        token,
        device_id,
    })
}

async fn upload_item(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
    released_queue_slot: Option<&AtomicBool>,
) -> Result<UploadOutcome, String> {
    let (
        token,
        device_id,
        multipart_part_size,
        oss_part_concurrency,
        oss_part_semaphore,
        checkpoint_writer,
        db_path,
    ) = {
        let guard = state.lock().map_err(|e| e.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.multipart_part_size.clone(),
            guard.oss_part_concurrency,
            guard.oss_part_semaphore.clone(),
            guard.upload_checkpoint_writer.clone(),
            guard.db_path.clone(),
        )
    };
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在准备云端目录" }),
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
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在申请上传凭证" }),
    );
    let mut persisted = load_upload_checkpoint(&db_path, item)?;
    let mut resumed_data = None;
    if let Some(saved) = persisted.as_ref() {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": if item.size == 0 { 0 } else { saved.uploaded_bytes.saturating_mul(100) / item.size },
                "uploaded_bytes": saved.uploaded_bytes,
                "total_bytes": item.size,
                "stage": "正在恢复上传断点"
            }),
        );
        let resume_result = api_post(
            &token,
            &device_id,
            "/userres/v1/get_res_center_resume_token",
            json!({
                "capacity": 2,
                "res": { "fileSize": item.size },
                "taskId": saved.checkpoint.task_id,
                "object": {
                    "objectPath": saved.checkpoint.object_path,
                    "provider": saved.checkpoint.provider
                }
            }),
            &[156],
        )
        .await;
        match resume_result {
            Ok(result) => {
                let mut token_data: UploadToken = serde_json::from_value(
                    result
                        .data
                        .ok_or_else(|| "光鸭没有返回续传凭证".to_string())?,
                )
                .map_err(|error| format!("续传凭证格式异常：{error}"))?;
                if token_data
                    .object_path
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.object_path = Some(saved.checkpoint.object_path.clone());
                }
                if token_data
                    .bucket_name
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.bucket_name = Some(saved.checkpoint.bucket_name.clone());
                }
                if token_data
                    .end_point
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    token_data.end_point = Some(saved.checkpoint.end_point.clone());
                }
                if token_data.provider.is_none() {
                    token_data.provider = saved.checkpoint.provider.clone();
                }
                resumed_data = Some(token_data);
            }
            Err(error) => {
                status(
                    app,
                    "warning",
                    format!("恢复上传断点失败，将重新创建上传任务：{error}"),
                );
                clear_upload_checkpoint(&db_path, item)?;
                persisted = None;
            }
        }
    }
    let preflight_token = if resumed_data.is_none() && persisted.is_none() {
        take_flash_preflight_token(state, item)?
    } else {
        None
    };
    let (mut data, mut instant_upload) = if let Some(data) = resumed_data {
        (data, false)
    } else if let Some(data) = preflight_token {
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": item.file_path.to_string_lossy(),
                "percent": 0,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "bytes_per_second": 0,
                "stage": "秒传未命中，正在进入上传通道"
            }),
        );
        (data, false)
    } else {
        let mut res = json!({ "fileSize": item.size });
        if item.size < 1024 * 1024 {
            emit(
                app,
                json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在计算秒传 MD5" }),
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
        let instant_upload = result.code == 156;
        let data: UploadToken = serde_json::from_value(
            result
                .data
                .ok_or_else(|| "光鸭没有返回上传凭证".to_string())?,
        )
        .map_err(|e| format!("上传凭证格式异常：{e}"))?;
        (data, instant_upload)
    };
    if !instant_upload && persisted.is_none() && item.size >= 1024 * 1024 {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 0, "uploaded_bytes": 0, "total_bytes": item.size, "stage": "正在校验秒传" }),
        );
        let cached_gcid = match {
            let guard = state.lock().map_err(|error| error.to_string())?;
            load_cached_file_gcid(
                &guard.db_path,
                &item.file_path,
                item.size,
                item.modified_ms,
                cache_settings(&guard),
            )
        } {
            Ok(value) => value,
            Err(error) => {
                status(app, "warning", error);
                None
            }
        };
        let gcid_result = if let Some(gcid) = cached_gcid {
            emit(
                app,
                json!({
                    "type": "progress",
                    "file_path": item.file_path.to_string_lossy(),
                    "percent": 0,
                    "uploaded_bytes": 0,
                    "total_bytes": item.size,
                    "bytes_per_second": 0,
                    "stage": "已复用本地秒传指纹"
                }),
            );
            Ok(gcid)
        } else {
            let result = calculate_file_gcid(app, &item.file_path, item.size, None).await;
            if let Ok(gcid) = &result {
                let saved = {
                    let guard = state.lock().map_err(|error| error.to_string())?;
                    save_cached_file_gcid(
                        &guard.db_path,
                        &item.file_path,
                        item.size,
                        item.modified_ms,
                        gcid,
                        cache_settings(&guard),
                    )
                };
                if let Err(error) = saved {
                    status(app, "warning", error);
                }
            }
            result
        };
        match gcid_result {
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
        let uploaded_bytes = persisted
            .as_ref()
            .map(|checkpoint| checkpoint.uploaded_bytes)
            .unwrap_or(0);
        let resumed = persisted.is_some();
        emit(
            app,
            json!({ "type": "file", "state": "uploading", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": uploaded_bytes, "total_bytes": item.size }),
        );
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": if item.size == 0 { 0 } else { uploaded_bytes.saturating_mul(100) / item.size }, "uploaded_bytes": uploaded_bytes, "total_bytes": item.size, "stage": if resumed { "正在从断点继续上传" } else { "正在连接 OSS" } }),
        );
        let oss_upload_guard = ActiveOssUploadGuard::new(state)?;
        let upload_result = upload_oss(
            &data,
            item,
            app,
            &multipart_part_size,
            oss_part_concurrency,
            oss_part_semaphore,
            &checkpoint_writer,
            persisted,
        )
        .await;
        drop(oss_upload_guard);
        drain_flash_preflight(app.clone(), state.clone());
        upload_result?;
    } else {
        emit(
            app,
            json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已命中秒传" }),
        );
    }
    let pending_outcome = UploadOutcome {
        task_id: data.task_id.clone(),
        remote_file_id: None,
    };
    remember_pending_upload(state, item, &pending_outcome)
        .map_err(|message| format!("文件已上传，但写入本地上传记录失败：{message}"))?;
    if let Err(error) = clear_upload_checkpoint(&db_path, item) {
        status(
            app,
            "warning",
            format!("上传记录已保存，但清理旧上传断点失败：{error}"),
        );
    }
    emit(
        app,
        json!({ "type": "progress", "file_path": item.file_path.to_string_lossy(), "percent": 100, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已上传，正在等待云端入库" }),
    );
    emit(
        app,
        json!({ "type": "file", "state": "processing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": item.size, "total_bytes": item.size, "stage": "已上传，正在等待云端入库" }),
    );
    if let Some(released_queue_slot) = released_queue_slot {
        {
            let mut guard = state.lock().map_err(|error| error.to_string())?;
            guard.active_uploads = guard.active_uploads.saturating_sub(1);
        }
        released_queue_slot.store(true, Ordering::Release);
        emit_state(app, state);
        drain_queue(app.clone(), state.clone());
    }
    let task_data = wait_upload_task(app, &token, &device_id, &data.task_id, &item.file_path)
        .await
        .map_err(|error| {
            format!(
                "文件已上传并已写入待确认记录，不会重复上传；后台将继续确认云端入库：{}",
                error.message()
            )
        })?;
    let outcome = UploadOutcome {
        task_id: data.task_id,
        remote_file_id: task_data
            .get("fileId")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    remember_confirmed_upload(state, item, &outcome)
        .map_err(|message| format!("云端已入库，但更新本地确认状态失败：{message}"))?;
    Ok(outcome)
}

fn archive_candidate(base: &Path, modified_ms: u128, collision: u64) -> PathBuf {
    if collision == 0 {
        return base.to_path_buf();
    }
    let stem = base
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let suffix = if collision == 1 {
        format!("-{modified_ms}")
    } else {
        format!("-{modified_ms}-{collision}")
    };
    let extension = base
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    base.with_file_name(format!("{stem}{suffix}{extension}"))
}

fn remove_partial_archive(path: &Path, original_error: String) -> String {
    match fs::remove_file(path) {
        Ok(()) => original_error,
        Err(cleanup_error) if cleanup_error.kind() == io::ErrorKind::NotFound => original_error,
        Err(cleanup_error) => {
            format!("{original_error}；清理未完成的归档副本也失败：{cleanup_error}")
        }
    }
}

fn copy_archive_exclusive(
    source: &Path,
    destination: &Path,
    expected_size: u64,
    expected_modified_ms: u128,
) -> Result<bool, String> {
    let mut source_file = fs::File::open(source).map_err(|e| format!("打开源文件失败：{e}"))?;
    let before = source_file
        .metadata()
        .map_err(|e| format!("读取源文件元数据失败：{e}"))?;
    if before.len() != expected_size || modified_ms(&before) != expected_modified_ms {
        return Err("归档前源文件发生变化，已保留源文件".into());
    }
    let mut destination_file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(format!("创建排他归档文件失败：{error}")),
    };
    let copied = match io::copy(&mut source_file, &mut destination_file) {
        Ok(bytes) => bytes,
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("复制到归档目录失败：{error}"),
            ));
        }
    };
    if let Err(error) = destination_file.sync_all() {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            format!("同步归档文件失败：{error}"),
        ));
    }
    let destination_size = match destination_file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("核对归档文件失败：{error}"),
            ));
        }
    };
    let after = match source_file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            drop(destination_file);
            return Err(remove_partial_archive(
                destination,
                format!("复制后核对源文件失败：{error}"),
            ));
        }
    };
    if copied != expected_size || destination_size != expected_size {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            format!(
                "归档复制字节数不一致（预期 {expected_size}，读取 {copied}，写入 {destination_size}），已保留源文件"
            ),
        ));
    }
    if after.len() != expected_size || modified_ms(&after) != expected_modified_ms {
        drop(destination_file);
        return Err(remove_partial_archive(
            destination,
            "归档复制期间源文件发生变化，已保留源文件".into(),
        ));
    }
    drop(destination_file);
    if let Err(error) = fs::remove_file(source) {
        return Err(remove_partial_archive(
            destination,
            format!("归档副本已核对，但移除源文件失败：{error}"),
        ));
    }
    Ok(true)
}

fn archive_file_without_overwrite(
    source: &Path,
    requested_destination: &Path,
    expected_size: u64,
    expected_modified_ms: u128,
) -> Result<PathBuf, String> {
    for collision in 0..u64::MAX {
        let destination = archive_candidate(requested_destination, expected_modified_ms, collision);
        match fs::hard_link(source, &destination) {
            Ok(()) => {
                let source_metadata = fs::metadata(source).map_err(|e| {
                    remove_partial_archive(&destination, format!("核对源文件失败：{e}"))
                })?;
                let destination_metadata = fs::metadata(&destination).map_err(|e| {
                    remove_partial_archive(&destination, format!("核对归档文件失败：{e}"))
                })?;
                if source_metadata.len() != expected_size
                    || modified_ms(&source_metadata) != expected_modified_ms
                    || destination_metadata.len() != expected_size
                {
                    return Err(remove_partial_archive(
                        &destination,
                        "归档期间源文件发生变化，已保留源文件".into(),
                    ));
                }
                if let Err(error) = fs::remove_file(source) {
                    return Err(remove_partial_archive(
                        &destination,
                        format!("创建归档链接后移除源文件失败：{error}"),
                    ));
                }
                return Ok(destination);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => match copy_archive_exclusive(
                source,
                &destination,
                expected_size,
                expected_modified_ms,
            )? {
                true => return Ok(destination),
                false => continue,
            },
        }
    }
    Err("无法生成唯一的归档文件名".into())
}

fn restore_staged_source(staged: &Path, original: &Path, message: String) -> String {
    if fs::symlink_metadata(original).is_ok() {
        return format!(
            "{message}；原路径已出现新文件，待处理的原文件保留在 {}",
            staged.display()
        );
    }
    match fs::rename(staged, original) {
        Ok(()) => message,
        Err(error) => format!(
            "{message}；恢复源文件失败，待处理文件保留在 {}：{error}",
            staged.display()
        ),
    }
}

fn stage_source_for_policy(
    source: &Path,
    expected_size: u64,
    expected_modified_ms: u128,
) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "源文件路径没有上级目录，已保留源文件".to_string())?;
    let staged = parent.join(format!(".guangya-source-{}", Uuid::new_v4().simple()));
    fs::rename(source, &staged).map_err(|error| format!("锁定上传后的源文件失败：{error}"))?;
    let metadata = match fs::metadata(&staged) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(restore_staged_source(
                &staged,
                source,
                format!("锁定后核对源文件失败：{error}"),
            ));
        }
    };
    if !metadata.is_file()
        || metadata.len() != expected_size
        || modified_ms(&metadata) != expected_modified_ms
    {
        return Err(restore_staged_source(
            &staged,
            source,
            "上传后源文件发生变化，已保留源文件且不会执行上传后策略".to_string(),
        ));
    }
    Ok(staged)
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
    let source_root = canonical_existing_directory(Path::new(&mapping.local_path), "监控目录")?;
    let source_path = canonical_upload_source(&item.file_path, Some(&source_root))?;
    let metadata = fs::metadata(&source_path).map_err(|e| format!("读取源文件失败：{e}"))?;
    if metadata.len() != item.size || modified_ms(&metadata) != item.modified_ms {
        return Err("上传期间源文件发生变化，已保留源文件且不会执行上传后策略".into());
    }
    let staged_source = stage_source_for_policy(&source_path, item.size, item.modified_ms)?;
    if mapping.source_policy == "delete" {
        if let Err(error) = fs::remove_file(&staged_source) {
            return Err(restore_staged_source(
                &staged_source,
                &source_path,
                format!("删除源文件失败：{error}"),
            ));
        }
        return Ok(Some("已按任务策略删除源文件".into()));
    }
    if mapping.source_policy != "archive" {
        return Err(restore_staged_source(
            &staged_source,
            &source_path,
            format!("未知的源文件策略：{}", mapping.source_policy),
        ));
    }
    let archive_root = match mapping
        .archive_path
        .filter(|value| !value.trim().is_empty())
    {
        Some(path) => path,
        None => {
            return Err(restore_staged_source(
                &staged_source,
                &source_path,
                "归档策略没有配置归档目录".to_string(),
            ));
        }
    };
    let archive_root = canonical_existing_directory(Path::new(&archive_root), "归档目录")
        .map_err(|error| restore_staged_source(&staged_source, &source_path, error))?;
    if archive_root == source_root || archive_root.starts_with(&source_root) {
        return Err(restore_staged_source(
            &staged_source,
            &source_path,
            "归档目录不能位于被监控目录内部".to_string(),
        ));
    }
    let relative = source_path.strip_prefix(&source_root).map_err(|_| {
        restore_staged_source(
            &staged_source,
            &source_path,
            "无法计算源文件的相对路径".to_string(),
        )
    })?;
    let relative_parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let destination_parent = ensure_directory_tree_without_symlinks(&archive_root, relative_parent)
        .map_err(|error| restore_staged_source(&staged_source, &source_path, error))?;
    let file_name = relative.file_name().ok_or_else(|| {
        restore_staged_source(
            &staged_source,
            &source_path,
            "无法计算归档文件名".to_string(),
        )
    })?;
    let requested_destination = destination_parent.join(file_name);
    let destination = archive_file_without_overwrite(
        &staged_source,
        &requested_destination,
        item.size,
        item.modified_ms,
    )
    .map_err(|error| restore_staged_source(&staged_source, &source_path, error))?;
    Ok(Some(format!("已移动到归档目录：{}", destination.display())))
}

fn resubmit_source_if_changed(state: &SharedState, item: &UploadItem) {
    if item.mapping_id == "__manual__" {
        return;
    }
    let changed = fs::metadata(&item.file_path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .is_some_and(|metadata| {
            metadata.len() != item.size || modified_ms(&metadata) != item.modified_ms
        });
    if changed {
        if let Ok(guard) = state.lock() {
            let _ = guard.event_tx.send(FsEvent {
                mapping_id: item.mapping_id.clone(),
                path: item.file_path.clone(),
            });
        }
    }
}

async fn finalize_successful_upload(
    app: &tauri::AppHandle,
    state: &SharedState,
    item: &UploadItem,
    outcome: &UploadOutcome,
) {
    let db_path = state.lock().ok().map(|guard| guard.db_path.clone());
    if let Some(path) = db_path.as_deref() {
        if let Err(message) = clear_auto_share_failure(path, item) {
            status(app, "error", message);
        }
    }
    if let Err(message) = schedule_auto_share(state, item, outcome).await {
        status(
            app,
            "error",
            format!("文件已上传，但自动分享排队失败：{message}"),
        );
    }
    match apply_source_policy(state, item) {
        Ok(Some(message)) => status(app, "success", message),
        Ok(None) => {}
        Err(message) => {
            status(app, "error", message);
            resubmit_source_if_changed(state, item);
        }
    }
    emit(
        app,
        json!({
            "type": "file",
            "state": "done",
            "file_path": item.file_path.to_string_lossy(),
            "mapping_id": item.mapping_id,
            "uploaded_bytes": item.size,
            "total_bytes": item.size
        }),
    );
}

fn drain_flash_preflight(app: tauri::AppHandle, state: SharedState) {
    loop {
        let item = {
            let mut guard = match state.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if guard.paused
                || guard.token.is_none()
                || guard.active_uploads == 0
                || guard.active_flash_preflights >= FLASH_PREFLIGHT_CONCURRENCY
                || !background_gcid_may_run(guard.active_oss_uploads)
            {
                None
            } else {
                let position = guard
                    .queue
                    .iter()
                    .position(|candidate| !flash_preflight_cached(&guard, candidate));
                position.and_then(|position| {
                    let item = guard.queue.remove(position)?;
                    let key = item_key(&item);
                    guard.active_flash_preflights += 1;
                    guard.inflight.insert(
                        key.clone(),
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                    guard.inflight_items.insert(key, item.clone());
                    Some(item)
                })
            }
        };
        let Some(item) = item else {
            emit_state(&app, &state);
            return;
        };
        emit(
            &app,
            json!({
                "type": "file",
                "state": "preparing",
                "file_path": item.file_path.to_string_lossy(),
                "mapping_id": item.mapping_id,
                "uploaded_bytes": 0,
                "total_bytes": item.size,
                "stage": "正在后台校验秒传"
            }),
        );
        let app2 = app.clone();
        let state2 = state.clone();
        tauri::async_runtime::spawn(async move {
            let upload_key = item_key(&item);
            let mut item = item;
            let result = match upload_source_root(&state2, &item) {
                Ok(source_root) => match prepare_upload_item(&item, source_root.as_deref()).await {
                    Ok(Some(ready)) => {
                        item = ready;
                        preflight_flash_upload(&app2, &state2, &item)
                            .await
                            .map(Some)
                    }
                    Ok(None) => Ok(None),
                    Err(message) => Err(message),
                },
                Err(message) => Err(message),
            };
            let waiting_for_file = result.as_ref().ok().is_some_and(Option::is_none);
            let auth_expired = result
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("登录态已失效"));
            let cloud_pending = state2
                .lock()
                .ok()
                .is_some_and(|guard| guard.pending_cloud.contains_key(&upload_key));
            let mut db_path = None;
            let mut requeued = false;
            if let Ok(mut guard) = state2.lock() {
                guard.active_flash_preflights = guard.active_flash_preflights.saturating_sub(1);
                guard.inflight.remove(&upload_key);
                guard.inflight_items.remove(&upload_key);
                db_path = Some(guard.db_path.clone());
                if auth_expired {
                    guard.token = None;
                }
                let mapping_active = item.mapping_id == "__manual__"
                    || guard
                        .mappings
                        .iter()
                        .any(|mapping| mapping.id == item.mapping_id && mapping.enabled);
                match &result {
                    Ok(Some(FlashPreflightOutcome::Miss(_))) if mapping_active => {
                        let token = match result.as_ref().ok().and_then(Option::as_ref) {
                            Some(FlashPreflightOutcome::Miss(token)) => Some(token.clone()),
                            _ => None,
                        };
                        guard.flash_preflight_cache.insert(
                            upload_key.clone(),
                            FlashPreflightCache {
                                stamp: Stamp {
                                    size: item.size,
                                    modified_ms: item.modified_ms,
                                },
                                upload_token: token,
                                created_at: Instant::now(),
                            },
                        );
                        guard.queue.push_front(item.clone());
                        requeued = true;
                    }
                    Ok(Some(FlashPreflightOutcome::Skipped)) | Err(_)
                        if !cloud_pending && mapping_active =>
                    {
                        guard.flash_preflight_cache.insert(
                            upload_key.clone(),
                            FlashPreflightCache {
                                stamp: Stamp {
                                    size: item.size,
                                    modified_ms: item.modified_ms,
                                },
                                upload_token: None,
                                created_at: Instant::now(),
                            },
                        );
                        guard.queue.push_front(item.clone());
                        requeued = true;
                    }
                    _ => {
                        guard.flash_preflight_cache.remove(&upload_key);
                    }
                }
                if waiting_for_file {
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
            match result {
                Ok(Some(FlashPreflightOutcome::Accepted {
                    task_id,
                    token,
                    device_id,
                })) => {
                    let confirm_app = app2.clone();
                    let confirm_state = state2.clone();
                    let confirm_item = item.clone();
                    tauri::async_runtime::spawn(async move {
                        match wait_upload_task(
                            &confirm_app,
                            &token,
                            &device_id,
                            &task_id,
                            &confirm_item.file_path,
                        )
                        .await
                        {
                            Ok(task_data) => {
                                let outcome = UploadOutcome {
                                    task_id,
                                    remote_file_id: task_data
                                        .get("fileId")
                                        .and_then(Value::as_str)
                                        .map(str::to_owned),
                                };
                                match remember_confirmed_upload(
                                    &confirm_state,
                                    &confirm_item,
                                    &outcome,
                                ) {
                                    Ok(()) => {
                                        finalize_successful_upload(
                                            &confirm_app,
                                            &confirm_state,
                                            &confirm_item,
                                            &outcome,
                                        )
                                        .await;
                                    }
                                    Err(message) => status(&confirm_app, "warning", message),
                                }
                            }
                            Err(error) => status(
                                &confirm_app,
                                "warning",
                                format!(
                                    "秒传已完成，云端入库将在后台继续确认：{}",
                                    error.message()
                                ),
                            ),
                        }
                        emit_state(&confirm_app, &confirm_state);
                    });
                }
                Ok(Some(FlashPreflightOutcome::Miss(_))) => {
                    if requeued {
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": "queued",
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": "秒传未命中，等待上传通道"
                            }),
                        );
                    }
                }
                Ok(Some(FlashPreflightOutcome::Skipped)) => {
                    if requeued {
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": "queued",
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": "已有上传断点，等待上传通道"
                            }),
                        );
                    }
                }
                Ok(None) => {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "waiting-file",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": 0,
                            "total_bytes": item.size,
                            "stage": "另外的程序正在使用该文件，释放后将自动上传"
                        }),
                    );
                    tauri::async_runtime::spawn(requeue_busy_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
                Err(message) if cloud_pending => {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "processing",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": item.size,
                            "total_bytes": item.size,
                            "stage": "秒传已完成，后台将继续确认云端入库"
                        }),
                    );
                    status(&app2, "warning", message);
                }
                Err(message) => {
                    if requeued {
                        status(
                            &app2,
                            "warning",
                            format!("后台秒传预检失败，已回到上传队列：{message}"),
                        );
                        emit(
                            &app2,
                            json!({
                                "type": "file",
                                "state": if auth_expired { "waiting-login" } else { "queued" },
                                "file_path": item.file_path.to_string_lossy(),
                                "mapping_id": item.mapping_id,
                                "uploaded_bytes": 0,
                                "total_bytes": item.size,
                                "stage": if auth_expired { "登录态已失效，重新登录后继续" } else { "秒传预检失败，等待普通上传" }
                            }),
                        );
                    }
                }
            }
            emit_state(&app2, &state2);
            drain_queue(app2, state2);
        });
    }
}

fn drain_queue(app: tauri::AppHandle, state: SharedState) {
    loop {
        let item = {
            let mut guard = match state.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if guard.paused
                || guard.token.is_none()
                || guard.active_uploads >= guard.upload_concurrency
            {
                None
            } else {
                let item = guard.queue.pop_front();
                if let Some(item) = &item {
                    guard.active_uploads += 1;
                    guard.inflight.insert(
                        item_key(item),
                        Stamp {
                            size: item.size,
                            modified_ms: item.modified_ms,
                        },
                    );
                    guard.inflight_items.insert(item_key(item), item.clone());
                }
                item
            }
        };
        let Some(item) = item else {
            emit_state(&app, &state);
            drain_flash_preflight(app, state);
            return;
        };
        emit(
            &app,
            json!({ "type": "file", "state": "preparing", "file_path": item.file_path.to_string_lossy(), "mapping_id": item.mapping_id, "uploaded_bytes": 0, "total_bytes": item.size }),
        );
        let app2 = app.clone();
        let state2 = state.clone();
        tauri::async_runtime::spawn(async move {
            let upload_key = item_key(&item);
            let released_queue_slot = Arc::new(AtomicBool::new(false));
            let mut item = item;
            let result = match upload_source_root(&state2, &item) {
                Ok(source_root) => match prepare_upload_item(&item, source_root.as_deref()).await {
                    Ok(Some(ready)) => {
                        item = ready;
                        upload_item(&app2, &state2, &item, Some(&released_queue_slot))
                            .await
                            .map(Some)
                    }
                    Ok(None) => Ok(None),
                    Err(message) => Err(message),
                },
                Err(message) => Err(message),
            };
            let waiting_for_file = result.as_ref().ok().is_some_and(Option::is_none);
            let auth_expired = result
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("登录态已失效"));
            let upload_slot_released = released_queue_slot.load(Ordering::Acquire);
            let outcome = result.as_ref().ok().and_then(|value| value.clone());
            let error_message = result.as_ref().err().cloned();
            let mut db_path = None;
            let mut cloud_pending = false;
            if let Ok(mut guard) = state2.lock() {
                cloud_pending = guard.pending_cloud.contains_key(&upload_key);
                if !upload_slot_released && !cloud_pending {
                    guard.active_uploads = guard.active_uploads.saturating_sub(1);
                }
                guard.inflight.remove(&upload_key);
                guard.inflight_items.remove(&upload_key);
                db_path = Some(guard.db_path.clone());
                if auth_expired {
                    guard.token = None;
                }
                if waiting_for_file {
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
                        "uploaded_bytes": 0,
                        "total_bytes": item.size,
                        "stage": "另外的程序正在使用该文件，释放后将自动上传"
                    }),
                );
                tauri::async_runtime::spawn(requeue_busy_upload(
                    app2.clone(),
                    state2.clone(),
                    item.clone(),
                ));
            } else if let Some(outcome) = outcome {
                finalize_successful_upload(&app2, &state2, &item, &outcome).await;
            } else {
                let message = error_message.unwrap_or_else(|| "上传失败".into());
                if cloud_pending {
                    emit(
                        &app2,
                        json!({
                            "type": "file",
                            "state": "processing",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id,
                            "uploaded_bytes": item.size,
                            "total_bytes": item.size,
                            "stage": "OSS 已完成，后台将继续确认云端入库"
                        }),
                    );
                    status(&app2, "warning", message);
                    emit_state(&app2, &state2);
                    drain_queue(app2, state2);
                    return;
                }
                let resumable = db_path
                    .as_deref()
                    .and_then(|path| load_upload_checkpoint(path, &item).ok().flatten());
                let auto_share_enabled = resumable.is_none()
                    && state2.lock().ok().is_some_and(|guard| {
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
                let uploaded_bytes = resumable
                    .as_ref()
                    .map(|checkpoint| checkpoint.uploaded_bytes)
                    .unwrap_or(0);
                emit(
                    &app2,
                    json!({ "type": "file", "state": "error", "file_path": item.file_path.to_string_lossy(), "uploaded_bytes": uploaded_bytes, "total_bytes": item.size, "error": message.clone() }),
                );
                if resumable.is_some() {
                    tauri::async_runtime::spawn(requeue_resumable_upload(
                        app2.clone(),
                        state2.clone(),
                        item.clone(),
                    ));
                }
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
    if !event.path.exists() {
        return;
    }
    let event_path = match canonical_watch_path(&event.path, Path::new(&mapping.local_path)) {
        Ok(path) => path,
        Err(error) => {
            status(app, "warning", error);
            return;
        }
    };
    let event_paths = collect_watch_event_files(&event_path, &mapping.sync_types);
    if event_paths.is_empty() {
        return;
    }
    if event_paths.len() != 1 || event_paths.first() != Some(&event_path) {
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
    let Ok(meta) = fs::metadata(&event_path) else {
        return;
    };
    let (remote_parent_id, remote_dir, relative) =
        match mapping_upload_destination(&mapping, &event_path) {
            Ok(destination) => destination,
            Err(error) => {
                status(app, "warning", error);
                return;
            }
        };
    let auto_share_enabled = mapping.auto_share;
    let mut item = UploadItem {
        mapping_id: mapping.id,
        file_path: event_path.clone(),
        remote_parent_id,
        remote_dir,
        relative_path: relative,
        change_kind: "added".to_string(),
        size: meta.len(),
        modified_ms: modified_ms(&meta),
    };
    if let Ok(guard) = state.lock() {
        if upload_already_scheduled(
            &guard.history,
            &guard.pending_cloud,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &item,
        ) {
            return;
        }
    } else {
        return;
    }
    let db_path = match state.lock() {
        Ok(guard) => guard.db_path.clone(),
        Err(_) => return,
    };
    let reused_upload = match reuse_matching_confirmed_upload(&db_path, &item) {
        Ok(value) => value,
        Err(error) => {
            status(app, "warning", error);
            None
        }
    };
    let waiting_for_login = if let Ok(mut guard) = state.lock() {
        let waiting_for_login = guard.token.is_none();
        let key = item_key(&item);
        if upload_already_scheduled(
            &guard.history,
            &guard.pending_cloud,
            &guard.inflight,
            &guard.queue,
            &guard.waiting_files,
            &item,
        ) {
            return;
        }
        if reused_upload.is_some() {
            guard.history.insert(
                key,
                Stamp {
                    size: item.size,
                    modified_ms: item.modified_ms,
                },
            );
            false
        } else {
            if guard.history.contains_key(&key) {
                item.change_kind = "changed".to_string();
            }
            guard.queue.retain(|queued| item_key(queued) != key);
            guard.queue.push_back(item.clone());
            waiting_for_login
        }
    } else {
        return;
    };
    if let Some((source_mapping_id, outcome)) = reused_upload {
        if auto_share_enabled {
            let target = auto_share_target(&item);
            let binding_reused = match target.as_ref() {
                Some(target) => reuse_auto_share_binding(
                    &db_path,
                    &item.mapping_id,
                    &source_mapping_id,
                    &target.key,
                ),
                None => Ok(false),
            };
            match binding_reused {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = schedule_auto_share(state, &item, &outcome).await {
                        status(
                            app,
                            "error",
                            format!("历史文件无需重复上传，但自动分享排队失败：{error}"),
                        );
                    }
                }
                Err(error) => status(app, "error", error),
            }
        }
        emit_state(app, state);
        return;
    }
    emit(
        app,
        json!({ "type": "file", "state": if waiting_for_login { "waiting-login" } else { "queued" }, "file_path": event_path.to_string_lossy() }),
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
            let Ok(path) = canonical_upload_source(&path, Some(Path::new(&mapping.local_path)))
            else {
                continue;
            };
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            let Ok((remote_parent_id, remote_dir, relative_path)) =
                mapping_upload_destination(mapping, &path)
            else {
                continue;
            };
            let item = UploadItem {
                mapping_id: mapping.id.clone(),
                file_path: path,
                remote_parent_id,
                remote_dir,
                relative_path,
                change_kind: "added".to_string(),
                size: metadata.len(),
                modified_ms: modified_ms(&metadata),
            };
            guard.history.insert(
                item_key(&item),
                Stamp {
                    size: item.size,
                    modified_ms: item.modified_ms,
                },
            );
        }
    }
}

fn hydrate_pending_item(_state: &SharedState, pending: &PendingUpload) -> UploadItem {
    // A cloud task belongs to the destination recorded when it was created. Retargeting here
    // would confirm or delete the wrong composite-key row after a mapping was edited.
    pending.item.clone()
}

fn queue_rejected_pending_upload(state: &SharedState, pending: &PendingUpload) -> bool {
    let pending_key = item_key(&pending.item);
    if let Ok(mut guard) = state.lock() {
        guard.pending_cloud.remove(&pending_key);
    } else {
        return false;
    }
    let mut item = pending.item.clone();
    if item.mapping_id != "__manual__" {
        let mapping = state.lock().ok().and_then(|guard| {
            guard
                .mappings
                .iter()
                .find(|mapping| mapping.id == item.mapping_id && mapping.enabled)
                .cloned()
        });
        let Some(mapping) = mapping else {
            return false;
        };
        let Ok((remote_parent_id, remote_dir, relative_path)) =
            mapping_upload_destination(&mapping, &item.file_path)
        else {
            return false;
        };
        item.remote_parent_id = remote_parent_id;
        item.remote_dir = remote_dir;
        item.relative_path = relative_path;
    }
    let key = item_key(&item);
    let Ok(metadata) = fs::metadata(&item.file_path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    item.size = metadata.len();
    item.modified_ms = modified_ms(&metadata);
    let Ok(mut guard) = state.lock() else {
        return false;
    };
    if item.mapping_id != "__manual__"
        && !guard
            .mappings
            .iter()
            .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
    {
        return false;
    }
    if upload_already_scheduled(
        &guard.history,
        &guard.pending_cloud,
        &guard.inflight,
        &guard.queue,
        &guard.waiting_files,
        &item,
    ) {
        return false;
    }
    guard.queue.retain(|queued| item_key(queued) != key);
    guard.queue.push_back(item);
    true
}

async fn recover_pending_upload(app: tauri::AppHandle, state: SharedState, pending: PendingUpload) {
    let key = item_key(&pending.item);
    let (token, device_id, db_path) = match state.lock() {
        Ok(guard) => {
            let Some(token) = guard.token.clone() else {
                drop(guard);
                if let Ok(mut guard) = state.lock() {
                    guard.recovering_pending.remove(&key);
                }
                return;
            };
            (token, guard.device_id.clone(), guard.db_path.clone())
        }
        Err(_) => return,
    };
    match check_upload_task(&token, &device_id, &pending.task_id).await {
        Ok(CloudTaskCheck::Confirmed(task_data)) => {
            let item = hydrate_pending_item(&state, &pending);
            let outcome = UploadOutcome {
                task_id: pending.task_id.clone(),
                remote_file_id: task_data
                    .get("fileId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            match remember_confirmed_upload(&state, &item, &outcome) {
                Ok(()) => {
                    if let Err(message) = clear_auto_share_failure(&db_path, &item) {
                        status(&app, "error", message);
                    }
                    if let Err(message) = schedule_auto_share(&state, &item, &outcome).await {
                        status(
                            &app,
                            "error",
                            format!("云端已确认，但自动分享排队失败：{message}"),
                        );
                    }
                    match apply_source_policy(&state, &item) {
                        Ok(Some(message)) => status(&app, "success", message),
                        Ok(None) => status(
                            &app,
                            "success",
                            format!("已恢复并确认云端入库：{}", item.file_path.display()),
                        ),
                        Err(message) => {
                            status(&app, "error", message);
                            resubmit_source_if_changed(&state, &item);
                        }
                    }
                    emit(
                        &app,
                        json!({
                            "type": "file",
                            "state": "done",
                            "file_path": item.file_path.to_string_lossy(),
                            "mapping_id": item.mapping_id
                        }),
                    );
                }
                Err(message) => status(&app, "warning", message),
            }
        }
        Ok(CloudTaskCheck::Pending) => {}
        Err(CloudConfirmError::Retryable(message)) => {
            if message.contains("登录态已失效") {
                if let Ok(mut guard) = state.lock() {
                    guard.token = None;
                }
                if let Err(error) = clear_persisted_access_token(&db_path) {
                    status(&app, "error", error);
                }
            } else {
                status(
                    &app,
                    "warning",
                    format!("待确认上传将在后台重试：{message}"),
                );
            }
        }
        Err(CloudConfirmError::Permanent(message)) => {
            match delete_pending_upload(&db_path, &pending) {
                Ok(true) => {
                    let queued = queue_rejected_pending_upload(&state, &pending);
                    status(
                        &app,
                        "error",
                        if queued {
                            format!("云端明确拒绝入库，已清理待确认记录并重新排队：{message}")
                        } else {
                            format!("云端明确拒绝入库，已清理待确认记录；源文件当前无法重新排队：{message}")
                        },
                    );
                    if queued {
                        drain_queue(app.clone(), state.clone());
                    }
                }
                Ok(false) => {}
                Err(error) => status(&app, "error", error),
            }
        }
    }
    if let Ok(mut guard) = state.lock() {
        guard.recovering_pending.remove(&key);
    }
    emit_state(&app, &state);
}

async fn pending_upload_recovery_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(PENDING_UPLOAD_RETRY_SECS)).await;
        let (db_path, can_recover) = match state.lock() {
            Ok(guard) => (guard.db_path.clone(), guard.token.is_some()),
            Err(_) => continue,
        };
        if !can_recover {
            continue;
        }
        let pending_uploads = match load_pending_uploads(&db_path) {
            Ok(pending) => pending,
            Err(message) => {
                status(&app, "error", message);
                continue;
            }
        };
        for pending in pending_uploads {
            let key = item_key(&pending.item);
            let should_start = state.lock().ok().is_some_and(|mut guard| {
                !guard.inflight.contains_key(&key) && guard.recovering_pending.insert(key.clone())
            });
            if should_start {
                tauri::async_runtime::spawn(recover_pending_upload(
                    app.clone(),
                    state.clone(),
                    pending,
                ));
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
                    .filter(|mapping| mapping.enabled)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for mapping in mappings {
            let mapping = normalize_loaded_mapping_paths(mapping);
            let validation_error = mapping.watch_error.clone();
            if let Ok(mut guard) = state.lock() {
                if let Some(current) = guard
                    .mappings
                    .iter_mut()
                    .find(|current| current.id == mapping.id)
                {
                    let changed = current.local_path != mapping.local_path
                        || current.archive_path != mapping.archive_path
                        || current.watch_error != mapping.watch_error;
                    current.local_path = mapping.local_path.clone();
                    current.archive_path = mapping.archive_path.clone();
                    current.watch_error = mapping.watch_error.clone();
                    if changed {
                        save_config(&guard);
                    }
                }
            }
            if validation_error.is_some() {
                continue;
            }
            if mapping.monitor_mode != "polling" {
                let watcher_missing = state
                    .lock()
                    .map(|guard| !guard.watchers.contains_key(&mapping.id))
                    .unwrap_or(false);
                if watcher_missing {
                    match install_watcher(&state, &mapping) {
                        Ok(()) => {
                            if mapping.scan_existing {
                                enqueue_existing_files(&app, &state, &mapping);
                            }
                            emit_state(&app, &state);
                        }
                        Err(error) => {
                            if let Ok(mut guard) = state.lock() {
                                if let Some(current) = guard
                                    .mappings
                                    .iter_mut()
                                    .find(|current| current.id == mapping.id)
                                {
                                    current.watch_error = Some(error);
                                }
                                save_config(&guard);
                            }
                        }
                    }
                }
                continue;
            }
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
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
            multipart_part_size: default_multipart_part_size(),
            oss_part_concurrency: DEFAULT_OSS_PART_CONCURRENCY,
            mappings: vec![],
            saved_shares: vec![],
            hdhive: HdhivePublicConfig {
                enabled: default_hdhive_enabled(),
                configured: false,
                base_url: String::new(),
                instance_id: String::new(),
            },
            auto_share_receipts: vec![],
        })
}

fn mount_info(state: &RuntimeState) -> MountInfo {
    MountInfo {
        enabled: state.webdav_enabled,
        running: state.webdav_running,
        configured: !state.webdav_username.is_empty() && !state.webdav_password_hash.is_empty(),
        local_only: true,
        endpoint: format!("http://127.0.0.1:{}/", state.webdav_port),
        username: state.webdav_username.clone(),
        password: String::new(),
        password_hint: if state.webdav_password_hash.is_empty() {
            "尚未设置，请输入 12 位以上密码".to_string()
        } else {
            "已设置；输入新密码可更新".to_string()
        },
        error: state.webdav_error.clone(),
        protocol: "webdav".to_string(),
    }
}

#[tauri::command]
fn get_mount_info(state: tauri::State<'_, SharedState>) -> Result<MountInfo, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(mount_info(&guard))
}

#[tauri::command]
async fn update_mount_credentials(
    state: tauri::State<'_, SharedState>,
    username: String,
    password: String,
) -> Result<MountInfo, String> {
    let username = normalize_webdav_username(&username)?;
    let password = normalize_webdav_password(&password)?;
    let password_for_hash = password.clone();
    let password_hash =
        tokio::task::spawn_blocking(move || hash_webdav_password(&password_for_hash))
            .await
            .map_err(|error| format!("计算 WebDAV 密码哈希失败：{error}"))?;
    let password_verifier = webdav_password_verifier(&password);
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let previous_username = guard.webdav_username.clone();
    let previous_hash = guard.webdav_password_hash.clone();
    let previous_session_password = guard.webdav_session_password.clone();
    let previous_verifier = guard.webdav_password_verifier;
    guard.webdav_username = username;
    guard.webdav_password_hash = password_hash;
    guard.webdav_session_password = Some(password.clone());
    guard.webdav_password_verifier = Some(password_verifier);
    if let Err(error) = save_config_checked(&guard) {
        guard.webdav_username = previous_username;
        guard.webdav_password_hash = previous_hash;
        guard.webdav_session_password = previous_session_password;
        guard.webdav_password_verifier = previous_verifier;
        return Err(error);
    }
    Ok(mount_info(&guard))
}

#[tauri::command]
fn get_native_mount_info(state: tauri::State<'_, SharedState>) -> Result<NativeMountInfo, String> {
    let manager = state
        .lock()
        .map_err(|error| error.to_string())?
        .native_mount
        .clone();
    let info = manager
        .lock()
        .map_err(|error| format!("读取原生挂载状态失败：{error}"))?
        .info();
    Ok(info)
}

#[tauri::command]
fn update_native_mount_options(
    state: tauri::State<'_, SharedState>,
    options: NativeMountOptions,
) -> Result<NativeMountInfo, String> {
    let manager = state
        .lock()
        .map_err(|error| error.to_string())?
        .native_mount
        .clone();
    let previous = {
        let mut mount = manager
            .lock()
            .map_err(|error| format!("更新原生挂载设置失败：{error}"))?;
        let previous = mount.options();
        mount.set_options(options)?;
        previous
    };
    let save_result = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        save_config_checked(&guard)
    };
    if let Err(error) = save_result {
        if let Ok(mut mount) = manager.lock() {
            let _ = mount.set_options(previous);
        }
        return Err(error);
    }
    let info = manager
        .lock()
        .map_err(|error| format!("读取原生挂载状态失败：{error}"))?
        .info();
    Ok(info)
}

#[tauri::command]
fn start_native_mount(
    state: tauri::State<'_, SharedState>,
    password: Option<String>,
) -> Result<NativeMountInfo, String> {
    let (manager, endpoint, username, password_hash, session_password) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.webdav_running {
            return Err(guard
                .webdav_error
                .clone()
                .unwrap_or_else(|| "WebDAV 本地服务尚未就绪".to_string()));
        }
        if guard.webdav_password_hash.is_empty() {
            return Err("请先设置 WebDAV 挂载账号密码".to_string());
        }
        (
            guard.native_mount.clone(),
            format!("http://127.0.0.1:{}/", guard.webdav_port),
            guard.webdav_username.clone(),
            guard.webdav_password_hash.clone(),
            guard.webdav_session_password.clone(),
        )
    };
    let password = password
        .filter(|value| !value.is_empty())
        .or(session_password)
        .ok_or_else(|| "为避免明文保存，请重新输入当前 WebDAV 密码后再挂载".to_string())?;
    if !verify_webdav_password(&password, &password_hash) {
        return Err("WebDAV 挂载密码不正确".to_string());
    }
    let result = manager
        .lock()
        .map_err(|error| format!("启动原生挂载失败：{error}"))?
        .start(&endpoint, &username, &password);
    result
}

#[tauri::command]
fn stop_native_mount(state: tauri::State<'_, SharedState>) -> Result<NativeMountInfo, String> {
    let manager = state
        .lock()
        .map_err(|error| error.to_string())?
        .native_mount
        .clone();
    let result = manager
        .lock()
        .map_err(|error| format!("卸载原生挂载失败：{error}"))?
        .stop();
    result
}

#[tauri::command]
fn select_native_mount_target() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
fn select_rclone_binary(state: tauri::State<'_, SharedState>) -> Option<String> {
    let dialog = rfd::FileDialog::new();
    #[cfg(windows)]
    let dialog = dialog.add_filter("rclone", &["exe"]);
    let path = dialog.pick_file()?;
    let manager = state.lock().ok()?.native_mount.clone();
    manager.lock().ok()?.approve_rclone_path(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn shared_auth_context(state: &SharedState) -> Result<(String, String), String> {
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok((
        guard
            .token
            .clone()
            .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
        guard.device_id.clone(),
    ))
}

fn auth_context(state: &tauri::State<'_, SharedState>) -> Result<(String, String), String> {
    shared_auth_context(state.inner())
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

#[tauri::command]
async fn search_files(
    state: tauri::State<'_, SharedState>,
    query: String,
    file_type: Option<String>,
    extension: Option<String>,
    page: Option<u64>,
) -> Result<Value, String> {
    let file_type = normalize_search_file_type(file_type.as_deref())?;
    let extension = normalize_search_extension(extension.as_deref());
    let (token, device_id) = auth_context(&state)?;
    let page = page.unwrap_or(0);
    let (endpoint, request) =
        cloud_search_request(&query, file_type.as_deref(), extension.as_deref(), page);
    let response = api_post(&token, &device_id, endpoint, request, &[]).await?;
    let data = response
        .data
        .unwrap_or_else(|| json!({ "list": [], "total": 0 }));
    let source_total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
    let source_list = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let remote_count = source_list.len() as u64;
    let list = source_list
        .into_iter()
        .filter(|item| {
            cloud_item_matches_search_filters(item, file_type.as_deref(), extension.as_deref())
        })
        .collect::<Vec<_>>();
    let total = if file_type.is_some() || extension.is_some() {
        list.len() as u64
    } else {
        source_total
    };
    Ok(json!({
        "list": list,
        "total": total,
        "remote_total": source_total,
        "remote_count": remote_count,
        "page": page,
        "page_size": 100
    }))
}

fn collect_manual_uploads(path: &Path, remote_prefix: &str, files: &mut Vec<(PathBuf, String)>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() || ignored(path) {
        return;
    }
    if metadata.is_file() {
        if let Ok(canonical) = fs::canonicalize(path) {
            files.push((canonical, normalize_remote_path(remote_prefix)));
        }
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
fn select_gcid_import_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("光鸭 GCID JSON", &["json"])
        .pick_file()
        .map(|path| path.to_string_lossy().to_string())
}

#[tauri::command]
async fn stage_gcid_import_text(
    state: tauri::State<'_, SharedState>,
    content: String,
) -> Result<GcidImportSourceInfo, String> {
    if content.trim().is_empty() {
        return Err("请先粘贴 JSON 内容".to_string());
    }
    let size = content.len() as u64;
    if size > MAX_GCID_IMPORT_BYTES {
        return Err(format!(
            "粘贴内容超过 {} MB，请改用文件导入",
            MAX_GCID_IMPORT_BYTES / 1024 / 1024
        ));
    }
    let hash = hex::encode(Sha256::digest(content.as_bytes()));
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let directory = database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("imports");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("创建导入暂存目录失败：{error}"))?;
    let file_name = format!("粘贴导入-{}.json", &hash[..12]);
    let file_path = directory.join(&file_name);
    tokio::fs::write(&file_path, content.as_bytes())
        .await
        .map_err(|error| format!("写入粘贴 JSON 文件失败：{error}"))?;
    Ok(GcidImportSourceInfo {
        path: file_path.to_string_lossy().to_string(),
        name: file_name,
        size,
    })
}

#[tauri::command]
async fn prepare_gcid_import(
    state: tauri::State<'_, SharedState>,
    source_path: String,
    destination_parent_id: String,
    destination_name: String,
) -> Result<GcidImportStatus, String> {
    let source_path = PathBuf::from(source_path);
    let metadata = tokio::fs::metadata(&source_path)
        .await
        .map_err(|error| format!("读取 JSON 文件失败：{error}"))?;
    if !metadata.is_file() {
        return Err("导入来源不是文件".to_string());
    }
    if metadata.len() > MAX_GCID_IMPORT_BYTES {
        return Err(format!(
            "JSON 文件超过 {} MB，拒绝载入",
            MAX_GCID_IMPORT_BYTES / 1024 / 1024
        ));
    }
    let raw = tokio::fs::read(&source_path)
        .await
        .map_err(|error| format!("读取 JSON 文件失败：{error}"))?;
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let job_id = prepare_gcid_import_database(
        &database_path,
        &raw,
        &source_path,
        &destination_parent_id,
        &destination_name,
    )?;
    load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "创建导入任务后无法读取状态".to_string())
}

#[tauri::command]
fn get_gcid_import_status(
    state: tauri::State<'_, SharedState>,
    job_id: Option<String>,
) -> Result<Option<GcidImportStatus>, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    load_gcid_import_status(&database_path, job_id.as_deref())
}

#[tauri::command]
fn start_gcid_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    job_id: String,
    concurrency: usize,
) -> Result<GcidImportStatus, String> {
    if !(1..=MAX_GCID_IMPORT_CONCURRENCY).contains(&concurrency) {
        return Err(format!(
            "秒传导入并发数必须在 1–{MAX_GCID_IMPORT_CONCURRENCY} 之间"
        ));
    }
    let (database_path, has_token) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (guard.db_path.clone(), guard.token.is_some())
    };
    if !has_token {
        return Err("请先登录光鸭云盘".to_string());
    }
    let current = load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "导入任务不存在，请重新选择 JSON".to_string())?;
    if current.counts.pending == 0 && current.counts.processing == 0 && current.counts.failed == 0 {
        return Ok(current);
    }
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        if !guard.gcid_import_running.insert(job_id.clone()) {
            return Err("这个导入任务已经在运行".to_string());
        }
    }
    let connection = open_database(&database_path)?;
    if let Err(error) = connection.execute(
        "UPDATE gcid_import_files
         SET status = 'pending', attempts = 0, error = NULL, updated_at = ?2
         WHERE job_id = ?1 AND status IN ('processing', 'failed')",
        params![job_id, unix_timestamp()],
    ) {
        if let Ok(mut guard) = state.lock() {
            guard.gcid_import_running.remove(&job_id);
        }
        return Err(format!("恢复未完成导入记录失败：{error}"));
    }
    if let Err(error) = connection.execute(
        "UPDATE gcid_import_jobs
         SET status = 'running', error = NULL, updated_at = ?2
         WHERE job_id = ?1",
        params![job_id, unix_timestamp()],
    ) {
        if let Ok(mut guard) = state.lock() {
            guard.gcid_import_running.remove(&job_id);
        }
        return Err(format!("启动导入任务失败：{error}"));
    }
    let running = load_gcid_import_status(&database_path, Some(&job_id))?
        .ok_or_else(|| "启动后无法读取导入任务".to_string())?;
    let destination_parent_id = running.destination_parent_id.clone();
    let destination_name = running.destination_name.clone();
    emit_gcid_import_status(&app, &database_path, &job_id);
    tauri::async_runtime::spawn(run_gcid_import(
        app,
        state.inner().clone(),
        database_path,
        job_id,
        destination_parent_id,
        destination_name,
        concurrency,
    ));
    Ok(running)
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
            let key = item_key(&item);
            if guard
                .inflight
                .get(&key)
                .is_some_and(|stamp| stamp_matches(&item, stamp))
                || guard.queue.iter().any(|queued| {
                    item_key(queued) == key
                        && queued.size == item.size
                        && queued.modified_ms == item.modified_ms
                })
            {
                continue;
            }
            guard.queue.retain(|queued| item_key(queued) != key);
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

fn validate_folder_name(folder_name: &str) -> Result<String, String> {
    let name = folder_name.trim();
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.chars().count() > 255
        || name
            .chars()
            .any(|value| value.is_control() || "\\/:*?\"<>|".contains(value))
    {
        return Err("无效的文件夹名称".into());
    }
    Ok(name.to_string())
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
async fn create_folder(
    state: tauri::State<'_, SharedState>,
    parent_id: String,
    folder_name: String,
) -> Result<Value, String> {
    let folder_name = validate_folder_name(&folder_name)?;
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/create_dir",
        json!({
            "parentId": parent_id,
            "dirName": folder_name,
            "failIfNameExist": true
        }),
        &[],
    )
    .await?;
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
    share_type: Option<u8>,
    code: Option<String>,
    auto_fill_code: Option<bool>,
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
    let (share_type, code, auto_fill_code) =
        normalize_share_access(share_type, code.as_deref(), auto_fill_code)?;
    // 手动分享始终创建当前快照。复用旧的文件夹分享会保留创建时的
    // 空目录状态，导致云盘已有文件而分享页仍为空。
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
        let message = payload
            .get("error_description")
            .or_else(|| payload.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("刷新登录状态失败")
            .to_string();
        if matches!(status_code, 400 | 401 | 403) {
            let invalidated = {
                let mut guard = state.lock().map_err(|error| error.to_string())?;
                if guard.refresh_token.as_deref() != Some(refresh_token.as_str()) {
                    false
                } else {
                    clear_persisted_auth_session(&guard.db_path)?;
                    guard.token = None;
                    guard.refresh_token = None;
                    guard.refreshed_access_tokens.clear();
                    reset_remote_cache(&mut guard.remote_cache);
                    true
                }
            };
            if !invalidated {
                return Ok(false);
            }
            emit_state(&app, &state);
            return Err(format!("登录态已失效，请重新扫码登录：{message}"));
        }
        return Err(message);
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
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        if guard.refresh_token.as_deref() != Some(refresh_token.as_str()) {
            return Ok(false);
        }
        save_refreshed_auth_session(&guard.db_path, &access_token, next_refresh.as_deref())?;
        if let Some(previous_access_token) = guard.token.clone() {
            if previous_access_token != access_token {
                if guard.refreshed_access_tokens.len() >= 32 {
                    guard.refreshed_access_tokens.clear();
                }
                guard
                    .refreshed_access_tokens
                    .insert(previous_access_token, access_token.clone());
            }
        }
        guard.token = Some(access_token.clone());
        if next_refresh.is_some() {
            guard.refresh_token = next_refresh.clone();
        }
        reset_remote_cache(&mut guard.remote_cache);
    }
    emit_state(&app, &state);
    drain_queue(app, state);
    Ok(true)
}

async fn refresh_saved_session_singleflight(
    app: tauri::AppHandle,
    state: SharedState,
) -> Result<bool, String> {
    let (observed_access_token, observed_refresh_token, refresh_lock) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard.token.clone(),
            guard.refresh_token.clone(),
            guard.auth_refresh_lock.clone(),
        )
    };
    if observed_refresh_token.is_none() {
        return Ok(false);
    }
    let _refresh_guard = refresh_lock.lock().await;
    {
        let guard = state.lock().map_err(|error| error.to_string())?;
        if guard.token != observed_access_token || guard.refresh_token != observed_refresh_token {
            return Ok(guard.token.is_some());
        }
    }
    refresh_saved_session(app, state).await
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
        if let Err(error) = refresh_saved_session_singleflight(app.clone(), state.clone()).await {
            status(
                &app,
                "warning",
                format!("自动续期失败，将稍后重试：{error}"),
            );
        }
    }
}

#[tauri::command]
async fn request_sms_code(
    state: tauri::State<'_, SharedState>,
    phone: String,
    captcha_token: Option<String>,
) -> Result<Value, String> {
    let phone_number = normalize_china_phone(&phone)?;
    let supplied_captcha_token = captcha_token.filter(|value| !value.trim().is_empty());
    let resolved_captcha_token = if let Some(token) = supplied_captcha_token {
        Some(token.trim().to_string())
    } else {
        let device_id = state
            .lock()
            .map_err(|error| error.to_string())?
            .device_id
            .clone();
        let (status_code, payload) = account_post_with_captcha(
            "/v1/shield/captcha/init",
            json!({
                "client_id": OAUTH_CLIENT_ID,
                "action": "POST:/v1/auth/verification",
                "device_id": device_id,
                "captcha_token": Value::Null,
                "meta": { "phone_number": phone_number }
            }),
            None,
        )
        .await?;
        if !(200..300).contains(&status_code) {
            if let Some(challenge) =
                captcha_challenge_response(&payload, payload_mentions_captcha(&payload))
            {
                return Ok(challenge);
            }
            return Err(account_error_message(&payload, "初始化短信安全验证失败"));
        }
        if account_payload_string(&payload, &["captcha_url", "captchaUrl", "url"]).is_some() {
            return captcha_challenge_response(&payload, true)
                .ok_or_else(|| "短信安全验证响应无效".to_string());
        }
        Some(
            account_payload_string(&payload, &["captcha_token", "captchaToken"])
                .ok_or_else(|| "短信安全验证没有返回 token 或验证页面".to_string())?,
        )
    };

    let (status_code, payload) = account_post_with_captcha(
        "/v1/auth/verification",
        json!({
            "phone_number": phone_number,
            "target": "ANY",
            "client_id": OAUTH_CLIENT_ID
        }),
        resolved_captcha_token.as_deref(),
    )
    .await?;
    if !(200..300).contains(&status_code) {
        if let Some(challenge) =
            captcha_challenge_response(&payload, payload_mentions_captcha(&payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&payload, "发送短信验证码失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&payload, false) {
        return Ok(challenge);
    }
    let verification_id = account_payload_string(&payload, &["verification_id"])
        .ok_or_else(|| "短信接口没有返回 verification_id".to_string())?;
    let is_user = account_payload_bool(&payload, "is_user")
        .ok_or_else(|| "短信接口没有返回 is_user".to_string())?;
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard
            .sms_verifications
            .retain(|_, verification| verification.phone_number != phone_number);
        guard.sms_verifications.insert(
            verification_id.clone(),
            SmsVerificationSession {
                phone_number: phone_number.clone(),
                is_user,
                captcha_token: resolved_captcha_token,
            },
        );
    }
    let mut result = flatten_account_payload(&payload);
    result.insert("request_id".to_string(), json!(verification_id));
    result.insert("phone_number".to_string(), json!(phone_number));
    result.insert("is_user".to_string(), json!(is_user));
    result.insert("captcha_required".to_string(), json!(false));
    Ok(Value::Object(result))
}

#[tauri::command]
async fn login_with_sms(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    phone: String,
    code: String,
    request_id: String,
    captcha_token: Option<String>,
) -> Result<Value, String> {
    let phone_number = normalize_china_phone(&phone)?;
    let verification_code = code.trim();
    if !(4..=8).contains(&verification_code.len())
        || !verification_code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("请输入有效的短信验证码".to_string());
    }
    let verification_id = request_id.trim();
    if verification_id.is_empty() {
        return Err("请先获取短信验证码".to_string());
    }
    let verification = state
        .lock()
        .map_err(|error| error.to_string())?
        .sms_verifications
        .get(verification_id)
        .cloned()
        .ok_or_else(|| "短信验证码请求已失效，请重新获取".to_string())?;
    if verification.phone_number != phone_number {
        return Err("手机号与验证码请求不一致，请重新获取".to_string());
    }
    let resolved_captcha_token = captcha_token
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or(verification.captcha_token.clone());
    let (verify_status, verify_payload) = account_post_with_captcha(
        "/v1/auth/verification/verify",
        json!({
            "verification_id": verification_id,
            "verification_code": verification_code,
            "client_id": OAUTH_CLIENT_ID
        }),
        resolved_captcha_token.as_deref(),
    )
    .await?;
    if !(200..300).contains(&verify_status) {
        if let Some(challenge) =
            captcha_challenge_response(&verify_payload, payload_mentions_captcha(&verify_payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&verify_payload, "短信验证码校验失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&verify_payload, false) {
        return Ok(challenge);
    }
    let verification_token = account_payload_string(&verify_payload, &["verification_token"])
        .ok_or_else(|| "短信校验接口没有返回 verification_token".to_string())?;
    let (endpoint, body) = if verification.is_user {
        (
            "/v1/auth/signin",
            json!({
                "username": phone_number,
                "verification_code": verification_code,
                "verification_token": verification_token,
                "client_id": OAUTH_CLIENT_ID
            }),
        )
    } else {
        (
            "/v1/auth/signup",
            json!({
                "phone_number": phone_number,
                "verification_code": verification_code,
                "verification_token": verification_token,
                "client_id": OAUTH_CLIENT_ID,
                "name": masked_phone_name(&phone_number)
            }),
        )
    };
    let (login_status, login_payload) =
        account_post_with_captcha(endpoint, body, resolved_captcha_token.as_deref()).await?;
    if !(200..300).contains(&login_status) {
        if let Some(challenge) =
            captcha_challenge_response(&login_payload, payload_mentions_captcha(&login_payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&login_payload, "手机号登录失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&login_payload, false) {
        return Ok(challenge);
    }
    let access_token = account_payload_string(&login_payload, &["access_token"])
        .ok_or_else(|| "登录接口没有返回 access_token".to_string())?;
    let refresh_token = account_payload_string(&login_payload, &["refresh_token"]);
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        save_auth_session(
            &guard.db_path,
            Some(&access_token),
            refresh_token.as_deref(),
        )?;
        guard.refreshed_access_tokens.clear();
        guard.token = Some(access_token);
        guard.refresh_token = refresh_token;
        guard.sms_verifications.remove(verification_id);
        reset_remote_cache(&mut guard.remote_cache);
    }
    status(
        &app,
        "success",
        "手机号登录成功，可以开始使用云盘和备份任务",
    );
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(json!({ "authenticated": true, "is_user": verification.is_user }))
}

#[tauri::command]
fn clear_expired_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    invalidate_auth_session(&app, state.inner())
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
        {
            let mut guard = state.lock().map_err(|e| e.to_string())?;
            save_auth_session(&guard.db_path, Some(&token), refresh_token.as_deref())?;
            guard.refreshed_access_tokens.clear();
            guard.token = Some(token.clone());
            guard.refresh_token = refresh_token.clone();
            reset_remote_cache(&mut guard.remote_cache);
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
    {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        if guard.token.as_deref() == Some(token.as_str()) {
            return Ok(());
        }
        save_auth_session(&guard.db_path, Some(&token), None)?;
        guard.refreshed_access_tokens.clear();
        guard.token = Some(token.clone());
        guard.refresh_token = None;
        reset_remote_cache(&mut guard.remote_cache);
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
    let canonical_local = canonical_existing_directory(Path::new(&local_path), "本地目录")?;
    let canonical_archive = archive_path
        .filter(|value| !value.trim().is_empty())
        .map(|value| canonical_existing_directory(Path::new(&value), "归档目录"))
        .transpose()?;
    if source_policy == "archive" {
        let archive_root = canonical_archive
            .as_ref()
            .ok_or_else(|| "归档策略需要选择归档目录".to_string())?;
        if archive_root == &canonical_local || archive_root.starts_with(&canonical_local) {
            return Err("归档目录不能位于被监控目录内部".into());
        }
    }
    let mapping = Mapping {
        id: Uuid::new_v4().to_string(),
        local_path: canonical_local.to_string_lossy().to_string(),
        remote_path: normalize_remote_path(&remote_path),
        remote_parent_id,
        enabled: true,
        source_policy,
        archive_path: canonical_archive.map(|path| path.to_string_lossy().to_string()),
        scan_existing,
        sync_types: normalize_sync_types(&sync_types),
        watch_error: None,
        monitor_mode: normalize_monitor_mode(&monitor_mode),
        auto_share,
    };
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
    guard
        .pending_cloud
        .retain(|key, _| !key.starts_with(&prefix));
    guard
        .recovering_pending
        .retain(|key| !key.starts_with(&prefix));
    guard.inflight.retain(|key, _| !key.starts_with(&prefix));
    save_config(&guard);
    let db_path = guard.db_path.clone();
    drop(guard);
    remove_mapping_transient_uploads(&db_path, &id)?;
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
        (mapping, guard.db_path.clone())
    };
    let rows = {
        let connection = open_database(&db_path)?;
        let mut statement = connection
            .prepare(
                "SELECT file_path, size, modified_ms, remote_file_id,
                        remote_parent_id, remote_dir, relative_path
                 FROM uploaded_files
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
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| format!("读取已有上传记录失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("解析已有上传记录失败：{error}"))?;
        rows
    };
    let mut scheduled = 0;
    for (
        file_path,
        size,
        modified_raw,
        remote_file_id,
        stored_parent_id,
        stored_remote_dir,
        stored_relative_path,
    ) in rows
    {
        let file_path = PathBuf::from(file_path);
        let Ok((expected_parent_id, expected_remote_dir, expected_relative_path)) =
            mapping_upload_destination(&mapping, &file_path)
        else {
            continue;
        };
        if stored_parent_id != expected_parent_id
            || stored_remote_dir != expected_remote_dir
            || stored_relative_path != expected_relative_path
        {
            continue;
        }
        let item = UploadItem {
            mapping_id: mapping.id.clone(),
            file_path,
            remote_parent_id: stored_parent_id,
            remote_dir: stored_remote_dir,
            relative_path: stored_relative_path,
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
#[tauri::command]
fn pause_queue(app: tauri::AppHandle, state: tauri::State<'_, SharedState>) {
    if let Ok(mut guard) = state.lock() {
        guard.paused = true;
    }
    emit_state(&app, state.inner());
}
#[tauri::command]
fn get_transfer_settings(state: tauri::State<'_, SharedState>) -> Result<TransferSettings, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(TransferSettings {
        upload_concurrency: guard.upload_concurrency,
        download_concurrency: guard.download_concurrency,
        multipart_part_size: guard.multipart_part_size.clone(),
        oss_part_concurrency: guard.oss_part_concurrency,
    })
}
#[tauri::command]
fn update_transfer_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    upload_concurrency: usize,
    download_concurrency: usize,
    multipart_part_size: Option<String>,
    oss_part_concurrency: Option<usize>,
) -> Result<Snapshot, String> {
    if !(1..=MAX_TRANSFER_CONCURRENCY).contains(&upload_concurrency)
        || !(1..=MAX_TRANSFER_CONCURRENCY).contains(&download_concurrency)
    {
        return Err(format!(
            "上传和下载并发数必须在 1–{MAX_TRANSFER_CONCURRENCY} 之间"
        ));
    }
    let multipart_part_size = multipart_part_size
        .map(|value| validate_multipart_part_size(&value))
        .transpose()?;
    let oss_part_concurrency = oss_part_concurrency
        .map(|value| {
            if (1..=MAX_OSS_PART_CONCURRENCY).contains(&value) {
                Ok(value)
            } else {
                Err(format!(
                    "单文件 OSS 分片并发必须在 1–{MAX_OSS_PART_CONCURRENCY} 之间"
                ))
            }
        })
        .transpose()?;
    let next = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        let previous = (
            guard.upload_concurrency,
            guard.download_concurrency,
            guard.multipart_part_size.clone(),
            guard.oss_part_concurrency,
        );
        guard.upload_concurrency = upload_concurrency;
        guard.download_concurrency = download_concurrency;
        if let Some(multipart_part_size) = multipart_part_size {
            guard.multipart_part_size = multipart_part_size;
        }
        if let Some(oss_part_concurrency) = oss_part_concurrency {
            guard.oss_part_concurrency = oss_part_concurrency;
        }
        if let Err(error) = save_config_checked(&guard) {
            guard.upload_concurrency = previous.0;
            guard.download_concurrency = previous.1;
            guard.multipart_part_size = previous.2;
            guard.oss_part_concurrency = previous.3;
            return Err(error);
        }
        snapshot(&guard)
    };
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(next)
}
#[tauri::command]
fn get_cache_settings(state: tauri::State<'_, SharedState>) -> Result<CacheSettings, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(cache_settings(&guard))
}
#[tauri::command]
fn update_cache_settings(
    state: tauri::State<'_, SharedState>,
    enabled: Option<bool>,
    max_entries: Option<usize>,
) -> Result<CacheSettings, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let next = CacheSettings {
        enabled: enabled.unwrap_or(guard.cache_enabled),
        max_entries: max_entries
            .map(validate_cache_max_entries)
            .transpose()?
            .unwrap_or(guard.cache_max_entries),
    };
    let db_path = guard.db_path.clone();
    apply_cache_policy(&db_path, &mut guard.remote_cache, next)?;
    save_app_state(&db_path, "cache_enabled", &next.enabled.to_string())?;
    save_app_state(&db_path, "cache_max_entries", &next.max_entries.to_string())?;
    guard.cache_enabled = next.enabled;
    guard.cache_max_entries = next.max_entries;
    Ok(next)
}
#[tauri::command]
fn get_metadata_cache_stats(
    state: tauri::State<'_, SharedState>,
) -> Result<MetadataCacheStats, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    metadata_cache_stats(&guard.db_path, &guard.remote_cache, cache_settings(&guard))
}
#[tauri::command]
fn clear_metadata_cache(
    state: tauri::State<'_, SharedState>,
) -> Result<MetadataCacheStats, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let db_path = guard.db_path.clone();
    let policy = cache_settings(&guard);
    clear_metadata_cache_storage(&db_path, &mut guard.remote_cache, policy)
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
            let native_mount_data_dir = db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
            init_database(&db_path)?;
            let upload_checkpoint_writer = UploadCheckpointWriter::new(db_path.clone());
            let auth_session = load_auth_session(&db_path)?;
            let upload_history = load_upload_history(&db_path)?;
            let pending_cloud = pending_upload_stamps(&db_path)?;
            let device_id = load_or_create_device_id(&db_path)?;
            let cache_policy = CacheSettings {
                enabled: parse_cache_enabled(load_app_state(&db_path, "cache_enabled")?.as_deref()),
                max_entries: parse_cache_max_entries(
                    load_app_state(&db_path, "cache_max_entries")?.as_deref(),
                ),
            };
            save_app_state(&db_path, "cache_enabled", &cache_policy.enabled.to_string())?;
            save_app_state(
                &db_path,
                "cache_max_entries",
                &cache_policy.max_entries.to_string(),
            )?;
            let mut remote_cache = HashMap::from([(String::new(), String::new())]);
            apply_cache_policy(&db_path, &mut remote_cache, cache_policy)?;
            let hdhive_enabled =
                parse_hdhive_enabled(load_app_state(&db_path, "hdhive_enabled")?.as_deref());
            let raw_hdhive_base_url = std::env::var("HDHIVE_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_base_url")?)
                .unwrap_or_default();
            let hdhive_base_url = normalize_hdhive_base_url(&raw_hdhive_base_url)?;
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
            let mut config = load_config(&config_path);
            if let Some(port) = std::env::var("GUANGYA_WEBDAV_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
            {
                if port > 0 {
                    config.webdav_port = port;
                }
            }
            if config.webdav_port == 0 {
                config.webdav_port = DEFAULT_WEBDAV_PORT;
            }
            if config.webdav_username.trim().is_empty() {
                config.webdav_username = default_webdav_username();
            }
            let legacy_webdav_password = normalize_webdav_password(&config.webdav_password).ok();
            if config.webdav_password_hash.trim().is_empty() {
                if let Some(password) = legacy_webdav_password.as_deref() {
                    config.webdav_password_hash = hash_webdav_password(password);
                }
            }
            let webdav_enabled = config.webdav_enabled;
            let webdav_port = config.webdav_port;
            let webdav_username = config.webdav_username.clone();
            let webdav_password_hash = config.webdav_password_hash.clone();
            let webdav_session_password = legacy_webdav_password.clone();
            let webdav_password_verifier = legacy_webdav_password
                .as_deref()
                .map(webdav_password_verifier);
            let native_mount_options = config.native_mount.clone();
            let upload_concurrency = normalize_transfer_concurrency(
                config.upload_concurrency,
                DEFAULT_UPLOAD_CONCURRENCY,
            );
            let download_concurrency = normalize_transfer_concurrency(
                config.download_concurrency,
                DEFAULT_DOWNLOAD_CONCURRENCY,
            );
            let multipart_part_size = normalize_multipart_part_size(&config.multipart_part_size);
            let oss_part_concurrency = normalize_oss_part_concurrency(config.oss_part_concurrency);
            let mappings = config
                .mappings
                .into_iter()
                .map(|mut mapping| {
                    mapping.sync_types = normalize_sync_types(&mapping.sync_types);
                    mapping.monitor_mode = normalize_monitor_mode(&mapping.monitor_mode);
                    normalize_loaded_mapping_paths(mapping)
                })
                .collect::<Vec<_>>();
            let mut resumable_uploads = load_resumable_uploads(&db_path)?;
            resumable_uploads.retain(|item| {
                item.mapping_id == "__manual__"
                    || mappings
                        .iter()
                        .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
            });
            let state = Arc::new(Mutex::new(RuntimeState {
                token: auth_session.access_token,
                refresh_token: auth_session.refresh_token,
                config_path,
                db_path,
                mappings: mappings.clone(),
                saved_shares: config.saved_shares,
                queue: resumable_uploads,
                flash_preflight_cache: HashMap::new(),
                waiting_files: HashMap::new(),
                history: upload_history,
                pending_cloud,
                recovering_pending: HashSet::new(),
                inflight: HashMap::new(),
                inflight_items: HashMap::new(),
                remote_cache,
                watchers: HashMap::new(),
                event_tx,
                paused: false,
                active_uploads: 0,
                active_flash_preflights: 0,
                upload_concurrency,
                download_concurrency,
                multipart_part_size,
                oss_part_concurrency,
                oss_part_semaphore: Arc::new(Semaphore::new(
                    usize::try_from(MAX_TOTAL_OSS_BUFFER_BYTES / OSS_BUFFER_PERMIT_BYTES)
                        .unwrap_or(usize::MAX),
                )),
                upload_checkpoint_writer,
                active_oss_uploads: 0,
                cache_enabled: cache_policy.enabled,
                cache_max_entries: cache_policy.max_entries,
                device_id,
                hdhive_enabled,
                hdhive_base_url,
                hdhive_secret,
                hdhive_instance_id,
                auto_share_processing: HashSet::new(),
                gcid_import_running: HashSet::new(),
                sms_verifications: HashMap::new(),
                webdav_enabled,
                webdav_port,
                webdav_username,
                webdav_password_hash,
                webdav_session_password,
                webdav_password_verifier,
                webdav_running: false,
                webdav_error: None,
                native_mount: Arc::new(Mutex::new(NativeMountManager::new(
                    native_mount_options,
                    native_mount_data_dir,
                    resource_dir,
                ))),
                auth_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
                refreshed_access_tokens: HashMap::new(),
            }));
            if let Ok(guard) = state.lock() {
                save_config_checked(&guard)?;
            }
            app.manage(state.clone());
            let app_handle = app.handle().clone();
            if webdav_enabled {
                tauri::async_runtime::spawn(webdav::serve(
                    app_handle.clone(),
                    state.clone(),
                    webdav_port,
                ));
            }
            let _ = API_AUTH_RUNTIME.set(ApiAuthRuntime {
                app: app_handle.clone(),
                state: state.clone(),
            });
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
                        refresh_saved_session_singleflight(refresh_app.clone(), refresh_state).await
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
            tauri::async_runtime::spawn(pending_upload_recovery_loop(
                app_handle.clone(),
                state.clone(),
            ));
            tauri::async_runtime::spawn(auto_share_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(token_refresh_loop(app_handle.clone(), state.clone()));
            drain_queue(app_handle.clone(), state.clone());
            emit_state(&app_handle, &state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_mount_info,
            update_mount_credentials,
            get_native_mount_info,
            update_native_mount_options,
            start_native_mount,
            stop_native_mount,
            select_native_mount_target,
            select_rclone_binary,
            clear_expired_session,
            start_device_login,
            request_sms_code,
            login_with_sms,
            poll_device_login,
            get_overview,
            list_files,
            search_files,
            select_gcid_import_file,
            stage_gcid_import_text,
            prepare_gcid_import,
            get_gcid_import_status,
            start_gcid_import,
            select_upload_files,
            select_upload_folder,
            queue_upload_paths,
            create_folder,
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
            get_transfer_settings,
            update_transfer_settings,
            get_cache_settings,
            update_cache_settings,
            get_metadata_cache_stats,
            clear_metadata_cache,
            resume_queue
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app_handle.state::<SharedState>();
                let manager = state.lock().ok().map(|guard| guard.native_mount.clone());
                if let Some(manager) = manager {
                    if let Ok(mut mount) = manager.lock() {
                        mount.shutdown();
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webdav_credentials_require_safe_explicit_values() {
        assert_eq!(
            normalize_webdav_username("  mount-user  ").unwrap(),
            "mount-user"
        );
        assert!(normalize_webdav_username("ab").is_err());
        assert!(normalize_webdav_username("bad:user").is_err());
        assert!(normalize_webdav_password("short").is_err());
        assert_eq!(
            normalize_webdav_password("correct horse battery staple").unwrap(),
            "correct horse battery staple"
        );
        let encoded = hash_webdav_password("correct horse battery staple");
        assert!(verify_webdav_password(
            "correct horse battery staple",
            &encoded
        ));
        assert!(!verify_webdav_password("wrong password", &encoded));
        assert!(!encoded.contains("correct horse battery staple"));
        assert_eq!(
            hex::encode(pbkdf2_sha256(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
    }

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
    fn auto_share_waits_for_pending_cloud_files_in_the_same_target() {
        let root =
            std::env::temp_dir().join(format!("guangya-auto-share-pending-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("state.sqlite3");
        init_database(&database).unwrap();
        let item = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: root.join("children").join("episode-02.mkv"),
            remote_parent_id: String::new(),
            remote_dir: "children".to_string(),
            relative_path: "children/episode-02.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 1024,
            modified_ms: 123,
        };
        save_upload_record(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-pending".to_string(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .unwrap();

        assert!(target_has_pending_cloud(&database, "mapping-1", "children").unwrap());
        assert!(!target_has_pending_cloud(&database, "mapping-1", "another-folder").unwrap());

        fs::remove_dir_all(root).unwrap();
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
    fn hdhive_response_preview_removes_controls_and_is_bounded() {
        let raw = format!("  forbidden\n\u{1b}[31m{}  ", "x".repeat(300));
        let preview = safe_response_preview(&raw, 200);
        assert!(!preview.contains('\n'));
        assert!(!preview.contains('\u{1b}'));
        assert!(preview.chars().count() <= 200);
        assert!(preview.starts_with("forbidden [31m"));
    }

    #[test]
    fn share_file_payload_matches_official_web_contract() {
        let payload = share_file_payload(&["file-1".to_string()], "测试分享", 0, "", false);

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
            share_file_payload(&["file-1".to_string()], "   ", 0, "", false)["title"],
            "云盘分享"
        );
        assert_eq!(
            share_file_payload(&["file-1".to_string()], "私密分享", 2, "a1B2", false)["code"],
            "a1B2"
        );
        assert!(normalize_share_access(Some(2), Some("bad"), None).is_err());
    }

    #[test]
    fn folder_names_are_trimmed_and_invalid_paths_are_rejected() {
        assert_eq!(
            validate_folder_name("  新建文件夹  ").unwrap(),
            "新建文件夹"
        );
        for invalid in ["", "   ", ".", "..", "a/b", "a\\b", "bad\nname"] {
            assert!(
                validate_folder_name(invalid).is_err(),
                "folder name should be rejected: {invalid:?}"
            );
        }
        assert!(validate_folder_name(&"a".repeat(256)).is_err());
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
        let mut pending_cloud = HashMap::new();
        let mut inflight = HashMap::new();
        let queue = VecDeque::new();
        let mut waiting_files = HashMap::new();
        assert!(!upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &item
        ));

        inflight.insert(
            item_key(&item),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &item
        ));

        let mut changed = item.clone();
        changed.modified_ms += 1;
        assert!(!upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &changed
        ));

        inflight.clear();
        pending_cloud.insert(
            item_key(&item),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &changed
        ));
        pending_cloud.clear();
        waiting_files.insert(item_key(&item), item.clone());
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
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
    fn upload_token_preserves_numeric_provider_values() {
        let token: UploadToken = serde_json::from_value(json!({
            "taskId": "task-1",
            "objectPath": "objects/video.mkv",
            "bucketName": "bucket",
            "endPoint": "https://oss-cn-shanghai.aliyuncs.com",
            "provider": 1,
            "creds": {
                "accessKeyID": "access-key",
                "secretAccessKey": "secret-key",
                "sessionToken": "security-token"
            }
        }))
        .expect("numeric provider should deserialize");

        assert_eq!(token.provider, Some(json!(1)));
    }

    #[test]
    fn oss_signature_uses_security_token_and_multipart_subresource() {
        let checkpoint = OssUploadCheckpoint {
            task_id: "task-1".into(),
            object_path: "folder/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss-cn-shanghai.aliyuncs.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-1".into(),
            part_size: OSS_MIB,
            completed_parts: BTreeMap::new(),
        };
        assert_eq!(
            oss_string_to_sign(
                "PUT",
                "Sun, 26 Jul 2026 12:00:00 GMT",
                "security-token",
                &checkpoint,
                Some("partNumber=2&uploadId=upload-1")
            ),
            "PUT\n\n\nSun, 26 Jul 2026 12:00:00 GMT\nx-oss-security-token:security-token\n/bucket/folder/video.mkv?partNumber=2&uploadId=upload-1"
        );
    }

    #[test]
    fn upload_checkpoint_persists_parts_and_is_restored_after_restart() {
        let root =
            std::env::temp_dir().join(format!("guangya-upload-checkpoint-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create upload checkpoint fixture");
        let database = root.join("state.sqlite3");
        let file_path = root.join("video.mkv");
        fs::write(&file_path, b"video-content").expect("write upload checkpoint fixture");
        let metadata = fs::metadata(&file_path).expect("read upload checkpoint fixture metadata");
        let item = UploadItem {
            mapping_id: "__manual__".into(),
            file_path: file_path.clone(),
            remote_parent_id: "parent-1".into(),
            remote_dir: String::new(),
            relative_path: String::new(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        };
        init_database(&database).expect("initialize upload checkpoint database");
        let checkpoint = OssUploadCheckpoint {
            task_id: "task-1".into(),
            object_path: "objects/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss-cn-shanghai.aliyuncs.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-1".into(),
            part_size: 5,
            completed_parts: BTreeMap::from([(1, "\"etag-1\"".into())]),
        };
        save_upload_checkpoint(&database, &item, &checkpoint, 5).expect("save upload checkpoint");

        let loaded = load_upload_checkpoint(&database, &item)
            .expect("load upload checkpoint")
            .expect("upload checkpoint should exist");
        assert_eq!(loaded.uploaded_bytes, 5);
        assert_eq!(
            loaded.checkpoint.completed_parts.get(&1).unwrap(),
            "\"etag-1\""
        );
        assert_eq!(
            load_resumable_uploads(&database)
                .expect("restore upload checkpoints")
                .len(),
            1
        );

        fs::write(&file_path, b"changed-video-content").expect("change upload fixture");
        assert!(load_resumable_uploads(&database)
            .expect("clean stale upload checkpoints")
            .is_empty());
        fs::remove_dir_all(root).expect("remove upload checkpoint fixture");
    }

    #[test]
    fn manual_upload_records_and_checkpoints_are_scoped_to_destination() {
        let root = std::env::temp_dir().join(format!(
            "guangya-upload-destination-scope-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create destination scope fixture");
        let database = root.join("state.sqlite3");
        let file_path = root.join("video.mkv");
        fs::write(&file_path, b"video-content").expect("write destination scope fixture");
        let metadata = fs::metadata(&file_path).expect("read destination scope metadata");
        init_database(&database).expect("initialize destination scope database");
        let first = UploadItem {
            mapping_id: "__manual__".into(),
            file_path: file_path.clone(),
            remote_parent_id: "parent-a".into(),
            remote_dir: "season-1".into(),
            relative_path: String::new(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        };
        let second = UploadItem {
            remote_parent_id: "parent-b".into(),
            remote_dir: "season-2".into(),
            ..first.clone()
        };
        assert_ne!(item_key(&first), item_key(&second));
        save_upload_record(
            &database,
            &first,
            &UploadOutcome {
                task_id: "task-a".into(),
                remote_file_id: Some("file-a".into()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("save first destination record");
        save_upload_record(
            &database,
            &second,
            &UploadOutcome {
                task_id: "task-b".into(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .expect("save second destination record");
        assert!(load_upload_history(&database)
            .expect("load destination history")
            .contains_key(&item_key(&first)));
        let pending = load_pending_uploads(&database).expect("load destination pending uploads");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].item.remote_parent_id, "parent-b");
        assert_eq!(pending[0].item.remote_dir, "season-2");

        let checkpoint = OssUploadCheckpoint {
            task_id: "task-checkpoint".into(),
            object_path: "objects/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss.example.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-id".into(),
            part_size: 5,
            completed_parts: BTreeMap::new(),
        };
        save_upload_checkpoint(&database, &first, &checkpoint, 1)
            .expect("save first destination checkpoint");
        save_upload_checkpoint(&database, &second, &checkpoint, 2)
            .expect("save second destination checkpoint");
        assert_eq!(
            load_upload_checkpoint(&database, &first)
                .unwrap()
                .unwrap()
                .uploaded_bytes,
            1
        );
        assert_eq!(
            load_upload_checkpoint(&database, &second)
                .unwrap()
                .unwrap()
                .uploaded_bytes,
            2
        );
        assert!(load_resumable_uploads(&database).unwrap().is_empty());
        let remaining_checkpoints = open_database(&database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM upload_checkpoints", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(remaining_checkpoints, 0);
        fs::remove_dir_all(root).expect("remove destination scope fixture");
    }

    #[test]
    fn multipart_part_size_uses_safe_tiers_and_stays_below_the_oss_part_limit() {
        assert_eq!(oss_part_size(100 * OSS_MIB), 4 * OSS_MIB);
        assert_eq!(oss_part_size(100 * OSS_MIB + 1), 8 * OSS_MIB);
        assert_eq!(oss_part_size(1024 * OSS_MIB), 8 * OSS_MIB);
        assert_eq!(oss_part_size(1024 * OSS_MIB + 1), OSS_LARGE_FILE_PART_SIZE);
        assert_eq!(oss_part_size(10 * 1024 * OSS_MIB), OSS_LARGE_FILE_PART_SIZE);

        let failed_file_size = 96_220_456_048;
        let part_size = oss_part_size(failed_file_size);
        assert_eq!(part_size, OSS_LARGE_FILE_PART_SIZE);
        assert_eq!(ceil_div_u64(failed_file_size, part_size), 5_736);
        assert!(ceil_div_u64(failed_file_size, part_size) <= OSS_MULTIPART_TARGET_PARTS);

        let tier_boundary = OSS_LARGE_FILE_PART_SIZE * OSS_MULTIPART_TARGET_PARTS;
        assert_eq!(oss_part_size(tier_boundary), OSS_LARGE_FILE_PART_SIZE);
        assert_eq!(
            oss_part_size(tier_boundary + 1),
            OSS_LARGE_FILE_PART_SIZE + OSS_MIB
        );

        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "auto"), 4 * OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "4m"), 4 * OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "8m"), 8 * OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "16m"), 16 * OSS_MIB);
        for tier in MULTIPART_PART_SIZE_OPTIONS {
            let configured = configured_oss_part_size(u64::MAX / 2, tier);
            assert!(ceil_div_u64(u64::MAX / 2, configured) <= OSS_MULTIPART_TARGET_PARTS);
            assert!(ceil_div_u64(u64::MAX / 2, configured) <= 10_000);
        }
    }

    #[test]
    fn multipart_concurrency_and_checkpoint_flush_bound_resource_use() {
        assert_eq!(effective_oss_part_concurrency(4 * OSS_MIB, 8), 8);
        assert_eq!(effective_oss_part_concurrency(16 * OSS_MIB, 8), 8);
        assert_eq!(effective_oss_part_concurrency(64 * OSS_MIB, 8), 2);
        assert_eq!(effective_oss_part_concurrency(256 * OSS_MIB, 8), 1);
        assert_eq!(oss_buffer_permits(4 * OSS_MIB), 1);
        assert_eq!(oss_buffer_permits(16 * OSS_MIB), 4);
        assert_eq!(oss_buffer_permits(64 * OSS_MIB), 16);
        assert_eq!(oss_buffer_permits(256 * OSS_MIB), 64);
        assert_eq!(oss_buffer_permits(512 * OSS_MIB), 64);
        assert_eq!(
            normalize_oss_part_concurrency(0),
            DEFAULT_OSS_PART_CONCURRENCY
        );
        assert_eq!(normalize_oss_part_concurrency(8), 8);

        assert!(!should_flush_upload_checkpoint(
            0,
            OSS_CHECKPOINT_FLUSH_BYTES - 1,
            10 * OSS_CHECKPOINT_FLUSH_BYTES,
            Duration::from_millis(500),
        ));
        assert!(should_flush_upload_checkpoint(
            0,
            OSS_CHECKPOINT_FLUSH_BYTES,
            10 * OSS_CHECKPOINT_FLUSH_BYTES,
            Duration::from_millis(500),
        ));
        assert!(should_flush_upload_checkpoint(
            0,
            OSS_MIB,
            10 * OSS_CHECKPOINT_FLUSH_BYTES,
            Duration::from_secs(OSS_CHECKPOINT_FLUSH_INTERVAL_SECS),
        ));
        assert!(should_flush_upload_checkpoint(
            0,
            10 * OSS_MIB,
            10 * OSS_MIB,
            Duration::ZERO,
        ));
    }

    #[test]
    fn upload_speed_waits_for_real_samples_and_keeps_the_full_average() {
        let mut tracker = UploadSpeedTracker::new(4);
        let first = tracker.observe(Duration::from_secs(4), 4 * OSS_MIB);
        assert_eq!(first.bytes_per_second, 0.0);
        assert_eq!(first.average_bytes_per_second, OSS_MIB as f64);

        let second = tracker.observe(Duration::from_secs(5), 8 * OSS_MIB);
        assert_eq!(second.bytes_per_second, 0.0);
        assert_eq!(second.average_bytes_per_second, 8.0 * OSS_MIB as f64 / 5.0);

        let third = tracker.observe(Duration::from_secs(6), 12 * OSS_MIB);
        assert_eq!(third.bytes_per_second, 0.0);
        assert_eq!(third.average_bytes_per_second, 2.0 * OSS_MIB as f64);

        let fourth = tracker.observe(Duration::from_secs(7), 16 * OSS_MIB);
        assert!(fourth.bytes_per_second > 0.0);
        assert_eq!(fourth.average_bytes_per_second, 16.0 * OSS_MIB as f64 / 7.0);

        let later = tracker.observe(Duration::from_secs(12), 24 * OSS_MIB);
        assert!(later.bytes_per_second.is_finite());
        assert_eq!(later.average_bytes_per_second, 2.0 * OSS_MIB as f64);
    }

    #[test]
    fn upload_speed_uses_the_actual_pending_part_count_for_small_files() {
        let mut tracker = UploadSpeedTracker::new(1);
        let first = tracker.observe(Duration::from_secs(2), 2 * OSS_MIB);
        assert_eq!(first.bytes_per_second, OSS_MIB as f64);
        assert_eq!(first.average_bytes_per_second, OSS_MIB as f64);
    }

    #[test]
    fn gcid_progress_is_throttled_and_background_hashing_yields_to_oss() {
        assert!(!should_emit_gcid_progress(
            Duration::ZERO,
            0,
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS - 1),
            1,
            false,
        ));
        assert!(should_emit_gcid_progress(
            Duration::ZERO,
            0,
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS),
            1,
            false,
        ));
        assert!(!should_emit_gcid_progress(
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS),
            1,
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS * 2),
            1,
            false,
        ));
        assert!(should_emit_gcid_progress(
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS),
            1,
            Duration::from_millis(GCID_PROGRESS_EMIT_INTERVAL_MS + 1),
            100,
            true,
        ));
        assert!(background_gcid_may_run(0));
        assert!(!background_gcid_may_run(1));
    }

    #[test]
    fn checkpoint_writer_flushes_latest_snapshot_before_acknowledging() {
        let root =
            std::env::temp_dir().join(format!("guangya-checkpoint-writer-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create checkpoint writer fixture");
        let database = root.join("state.sqlite3");
        let file_path = root.join("video.mkv");
        fs::write(&file_path, b"0123456789").expect("write checkpoint writer fixture");
        let metadata = fs::metadata(&file_path).expect("read checkpoint writer metadata");
        let item = UploadItem {
            mapping_id: "__manual__".into(),
            file_path,
            remote_parent_id: "parent-1".into(),
            remote_dir: String::new(),
            relative_path: String::new(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        };
        init_database(&database).expect("initialize checkpoint writer database");
        let mut checkpoint = OssUploadCheckpoint {
            task_id: "task-1".into(),
            object_path: "objects/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss.example.com".into(),
            provider: Some(json!("oss")),
            upload_id: "upload-1".into(),
            part_size: 5,
            completed_parts: BTreeMap::from([(1, "etag-1".into())]),
        };
        tauri::async_runtime::block_on(async {
            let writer = UploadCheckpointWriter::new(database.clone());
            writer
                .enqueue(&item, &checkpoint, 5)
                .expect("enqueue first checkpoint");
            checkpoint.completed_parts.insert(2, "etag-2".into());
            writer
                .enqueue(&item, &checkpoint, 10)
                .expect("enqueue latest checkpoint");
            writer
                .flush(&item, Some(&checkpoint), 10)
                .await
                .expect("flush latest checkpoint");

            let loaded = load_upload_checkpoint(&database, &item)
                .expect("load flushed checkpoint")
                .expect("flushed checkpoint should exist");
            assert_eq!(loaded.uploaded_bytes, 10);
            assert_eq!(loaded.checkpoint.completed_parts.len(), 2);
            clear_upload_checkpoint(&database, &item).expect("clear flushed checkpoint");
            sleep(Duration::from_millis(25)).await;
            assert!(load_upload_checkpoint(&database, &item)
                .expect("check for late checkpoint write")
                .is_none());
            drop(writer);
        });
        fs::remove_dir_all(root).expect("remove checkpoint writer fixture");
    }

    #[test]
    fn retryable_oss_body_clones_share_the_same_allocation() {
        let body = Bytes::from(vec![7_u8; 1024]);
        let retry = body.clone();
        assert_eq!(body.as_ptr(), retry.as_ptr());
        assert_eq!(body, retry);
    }

    #[test]
    fn multipart_part_size_validation_accepts_only_supported_tiers() {
        for tier in MULTIPART_PART_SIZE_OPTIONS {
            assert_eq!(validate_multipart_part_size(tier).unwrap(), *tier);
        }
        assert_eq!(validate_multipart_part_size(" 8M ").unwrap(), "8m");
        for invalid in ["", "1m", "2m", "32m", "custom"] {
            assert!(validate_multipart_part_size(invalid).is_err());
        }
        assert_eq!(normalize_multipart_part_size("invalid"), "auto");
    }

    #[test]
    fn transfer_concurrency_defaults_and_bounds_are_stable() {
        let config: AppConfig = serde_json::from_str("{}").expect("deserialize defaults");
        assert_eq!(config.upload_concurrency, DEFAULT_UPLOAD_CONCURRENCY);
        assert_eq!(config.download_concurrency, DEFAULT_DOWNLOAD_CONCURRENCY);
        assert_eq!(config.multipart_part_size, DEFAULT_MULTIPART_PART_SIZE);
        assert_eq!(config.oss_part_concurrency, DEFAULT_OSS_PART_CONCURRENCY);
        assert!(parse_cache_enabled(None));
        assert!(!parse_cache_enabled(Some("false")));
        assert_eq!(parse_cache_max_entries(None), DEFAULT_CACHE_MAX_ENTRIES);
        assert_eq!(
            parse_cache_max_entries(Some("99")),
            DEFAULT_CACHE_MAX_ENTRIES
        );
        assert_eq!(parse_cache_max_entries(Some("100")), 100);
        assert_eq!(parse_cache_max_entries(Some("100000")), 100_000);
        assert!(parse_hdhive_enabled(None));
        assert!(parse_hdhive_enabled(Some("true")));
        assert!(!parse_hdhive_enabled(Some("false")));
        assert_eq!(
            normalize_transfer_concurrency(0, DEFAULT_UPLOAD_CONCURRENCY),
            DEFAULT_UPLOAD_CONCURRENCY
        );
        assert_eq!(
            normalize_transfer_concurrency(MAX_TRANSFER_CONCURRENCY, DEFAULT_UPLOAD_CONCURRENCY),
            MAX_TRANSFER_CONCURRENCY
        );
    }

    #[test]
    fn hdhive_base_url_rejects_unsafe_parts_and_normalizes_paths() {
        let unrestricted = HashSet::new();
        assert_eq!(
            normalize_hdhive_base_url_with_allowed_hosts(
                "  https://Example.COM/integration///  ",
                &unrestricted,
            )
            .unwrap(),
            "https://example.com/integration"
        );
        assert_eq!(
            normalize_hdhive_base_url_with_allowed_hosts("", &unrestricted).unwrap(),
            ""
        );
        for unsafe_url in [
            "ftp://example.com",
            "https://user:secret@example.com",
            "https://example.com?next=https://evil.example",
            "https://example.com/#fragment",
        ] {
            assert!(
                normalize_hdhive_base_url_with_allowed_hosts(unsafe_url, &unrestricted).is_err(),
                "unsafe URL should be rejected: {unsafe_url}"
            );
        }
    }

    #[test]
    fn hdhive_base_url_honors_the_optional_host_allowlist() {
        let host_only = HashSet::from(["api.example.com".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:8443/root",
            &host_only,
        )
        .is_ok());
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://other.example.com/root",
            &host_only,
        )
        .is_err());

        let host_and_port = HashSet::from(["api.example.com:8443".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:8443/root",
            &host_and_port,
        )
        .is_ok());
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:9443/root",
            &host_and_port,
        )
        .is_err());

        let ipv6_host_and_port = HashSet::from(["[::1]:8080".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "http://[::1]:8080/root",
            &ipv6_host_and_port,
        )
        .is_ok());
        let ipv6_host = HashSet::from(["[::1]".to_string()]);
        assert!(
            normalize_hdhive_base_url_with_allowed_hosts("http://[::1]:8080/root", &ipv6_host,)
                .is_ok()
        );
    }

    #[test]
    fn hdhive_target_url_appends_only_a_structured_path() {
        let (target, signature_path) = build_hdhive_target_url(
            "https://api.example.com/integration",
            &["api", "guangya-sync", "events"],
        )
        .unwrap();
        assert_eq!(
            target.as_str(),
            "https://api.example.com/integration/api/guangya-sync/events"
        );
        assert_eq!(signature_path, "/api/guangya-sync/events");
        for unsafe_segment in [
            "event/id",
            r"event\id",
            ".",
            "..",
            "event?redirect=evil",
            "event#fragment",
        ] {
            assert!(build_hdhive_target_url(
                "https://api.example.com",
                &["api", "events", unsafe_segment],
            )
            .is_err());
        }
    }

    #[test]
    fn file_gcid_cache_is_reused_only_for_an_unchanged_file_stamp() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-cache-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create gcid cache test root");
        let database = root.join("state.sqlite3");
        let file = root.join("movie.mkv");
        init_database(&database).expect("initialize gcid cache database");
        fs::write(&file, b"fixture").expect("write gcid fixture");
        let gcid = "0123456789ABCDEF0123456789ABCDEF01234567";
        let policy = CacheSettings {
            enabled: true,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        };

        assert_eq!(
            load_cached_file_gcid(&database, &file, 7, 100, policy).expect("load empty cache"),
            None
        );
        save_cached_file_gcid(&database, &file, 7, 100, gcid, policy).expect("save gcid cache");
        assert_eq!(
            load_cached_file_gcid(&database, &file, 7, 100, policy).expect("load cached gcid"),
            Some(gcid.to_string())
        );
        assert_eq!(
            load_cached_file_gcid(&database, &file, 7, 101, policy).expect("reject changed mtime"),
            None
        );
        assert_eq!(
            load_cached_file_gcid(&database, &file, 8, 100, policy).expect("reject changed size"),
            None
        );

        fs::remove_dir_all(root).expect("remove gcid cache test root");
    }

    #[test]
    fn metadata_cache_policy_persists_disables_and_bounds_each_cache() {
        let root =
            std::env::temp_dir().join(format!("guangya-cache-policy-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create cache policy test root");
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize cache policy database");
        let enabled = CacheSettings {
            enabled: true,
            max_entries: MIN_CACHE_MAX_ENTRIES,
        };

        for index in 0..105_u64 {
            save_cached_file_gcid(
                &database,
                &root.join(format!("cached-{index}.bin")),
                index + 1,
                u128::from(index),
                "0123456789ABCDEF0123456789ABCDEF01234567",
                enabled,
            )
            .expect("save bounded fingerprint");
        }
        assert_eq!(
            open_database(&database)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM file_fingerprints", [], |row| row
                    .get::<_, usize>(0))
                .unwrap(),
            MIN_CACHE_MAX_ENTRIES
        );

        let mut remote_cache = HashMap::from([(String::new(), String::new())]);
        for index in 0..105 {
            remote_cache.insert(format!("root::{index}"), format!("folder-{index}"));
        }
        let bounded = apply_cache_policy(&database, &mut remote_cache, enabled)
            .expect("apply enabled cache policy");
        assert_eq!(bounded.file_fingerprints_entries, 100);
        assert_eq!(bounded.remote_cache_entries, 100);
        assert_eq!(bounded.policy, enabled);
        assert_eq!(remote_cache.get(""), Some(&String::new()));

        let disabled = CacheSettings {
            enabled: false,
            max_entries: MIN_CACHE_MAX_ENTRIES,
        };
        let disabled_path = root.join("disabled.bin");
        save_cached_file_gcid(
            &database,
            &disabled_path,
            1,
            1,
            "89ABCDEF0123456789ABCDEF0123456789ABCDEF",
            disabled,
        )
        .expect("disabled cache write is a no-op");
        assert!(
            load_cached_file_gcid(&database, &disabled_path, 1, 1, disabled)
                .expect("disabled cache read is a no-op")
                .is_none()
        );
        let cleared = apply_cache_policy(&database, &mut remote_cache, disabled)
            .expect("disable and clear cache");
        assert_eq!(cleared.entries, 0);
        assert_eq!(cleared.policy, disabled);
        assert_eq!(
            remote_cache,
            HashMap::from([(String::new(), String::new())])
        );

        save_app_state(&database, "cache_enabled", &disabled.enabled.to_string())
            .expect("persist cache switch");
        save_app_state(
            &database,
            "cache_max_entries",
            &disabled.max_entries.to_string(),
        )
        .expect("persist cache limit");
        assert!(!parse_cache_enabled(
            load_app_state(&database, "cache_enabled")
                .unwrap()
                .as_deref()
        ));
        assert_eq!(
            parse_cache_max_entries(
                load_app_state(&database, "cache_max_entries")
                    .unwrap()
                    .as_deref()
            ),
            MIN_CACHE_MAX_ENTRIES
        );
        assert!(validate_cache_max_entries(MIN_CACHE_MAX_ENTRIES - 1).is_err());
        assert!(validate_cache_max_entries(MAX_CACHE_MAX_ENTRIES + 1).is_err());

        fs::remove_dir_all(root).expect("remove cache policy test root");
    }

    #[test]
    fn metadata_cache_clear_preserves_upload_records_files_and_root_mapping() {
        let root =
            std::env::temp_dir().join(format!("guangya-cache-clear-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create metadata cache test root");
        let database = root.join("state.sqlite3");
        let file = root.join("movie.mkv");
        init_database(&database).expect("initialize metadata cache database");
        fs::write(&file, b"fixture").expect("write cache fixture");
        let policy = CacheSettings {
            enabled: true,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        };
        save_cached_file_gcid(
            &database,
            &file,
            7,
            100,
            "0123456789ABCDEF0123456789ABCDEF01234567",
            policy,
        )
        .expect("save fingerprint cache");
        let upload = UploadItem {
            mapping_id: "mapping-cache-test".to_string(),
            file_path: file.clone(),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "movie.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 7,
            modified_ms: 100,
        };
        save_upload_record(
            &database,
            &upload,
            &UploadOutcome {
                task_id: "task-cache-test".to_string(),
                remote_file_id: Some("file-cache-test".to_string()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("save upload record");
        let mut remote_cache = HashMap::from([
            (String::new(), String::new()),
            ("root::Movies".to_string(), "folder-1".to_string()),
        ]);
        let before =
            metadata_cache_stats(&database, &remote_cache, policy).expect("read cache stats");
        assert_eq!(before.file_fingerprints_entries, 1);
        assert_eq!(before.remote_cache_entries, 1);
        assert_eq!(before.entries, 2);
        assert!(before.bytes > 0);

        let after = clear_metadata_cache_storage(&database, &mut remote_cache, policy)
            .expect("clear metadata cache");
        assert_eq!(after.entries, 0);
        assert_eq!(after.bytes, 0);
        assert_eq!(
            remote_cache,
            HashMap::from([(String::new(), String::new())])
        );
        assert!(file.exists());
        assert!(load_cached_file_gcid(&database, &file, 7, 100, policy)
            .expect("read cleared fingerprint")
            .is_none());
        assert!(load_upload_history(&database)
            .expect("load preserved upload history")
            .contains_key(&item_key(&upload)));

        fs::remove_dir_all(root).expect("remove metadata cache test root");
    }

    #[test]
    fn search_filters_use_cloud_item_suffixes_on_the_current_page() {
        let folder = json!({ "fileName": "相册", "resType": 2 });
        let image = json!({ "fileName": "封面.JPG", "fileSuffix": ".JPG", "resType": 1 });
        let video = json!({ "fileName": "电影.MKV", "resType": "1" });
        let document = json!({ "fileName": "说明.pdf", "resType": 1 });
        let archive = json!({ "fileName": "备份.7z", "resType": 1 });

        assert!(cloud_item_matches_search_filters(
            &folder,
            Some("folder"),
            None
        ));
        assert!(!cloud_item_matches_search_filters(
            &folder,
            Some("image"),
            None
        ));
        assert!(cloud_item_matches_search_filters(
            &image,
            Some("image"),
            Some("jpg")
        ));
        assert!(cloud_item_matches_search_filters(
            &video,
            Some("video"),
            Some("mkv")
        ));
        assert!(cloud_item_matches_search_filters(
            &document,
            Some("document"),
            None
        ));
        assert!(cloud_item_matches_search_filters(
            &archive,
            Some("archive"),
            Some("7z")
        ));
        assert!(!cloud_item_matches_search_filters(
            &archive,
            Some("document"),
            None
        ));
        assert_eq!(normalize_search_file_type(Some("ALL")).unwrap(), None);
        assert!(normalize_search_file_type(Some("executable")).is_err());
        assert_eq!(
            normalize_search_extension(Some(" .MP4 ")).as_deref(),
            Some("mp4")
        );

        let (search_endpoint, search_request) =
            cloud_search_request(" holiday ", Some("video"), None, 2);
        assert_eq!(search_endpoint, "/userres/v1/file/search_files");
        assert_eq!(
            search_request,
            json!({ "name": "holiday", "pageSize": 100, "page": 2 })
        );

        let (video_endpoint, video_request) = cloud_search_request("", Some("video"), None, 3);
        assert_eq!(video_endpoint, "/userres/v1/file/get_file_list");
        assert_eq!(
            video_request,
            json!({
                "parentId": "*",
                "pageSize": 100,
                "page": 3,
                "orderBy": 3,
                "sortType": 1,
                "resType": 1,
                "fileTypes": [CLOUD_FILE_TYPE_VIDEO]
            })
        );

        let (_, extension_request) = cloud_search_request("", None, Some("pdf"), 0);
        assert_eq!(
            extension_request.get("fileTypes"),
            Some(&json!([CLOUD_FILE_TYPE_DOCUMENT]))
        );
        let (_, folder_request) = cloud_search_request("", Some("folder"), None, 0);
        assert_eq!(folder_request.get("resType"), Some(&json!(2)));
        assert!(folder_request.get("fileTypes").is_none());
        let (_, unknown_extension_request) = cloud_search_request("", None, Some("blend"), 0);
        assert!(unknown_extension_request.get("fileTypes").is_none());
    }

    #[test]
    fn sms_phone_normalization_and_masked_signup_name_are_stable() {
        assert_eq!(
            normalize_china_phone("13800138000").unwrap(),
            "+86 13800138000"
        );
        assert_eq!(
            normalize_china_phone("+86 13800138000").unwrap(),
            "+86 13800138000"
        );
        assert_eq!(
            normalize_china_phone("86-138-0013-8000").unwrap(),
            "+86 13800138000"
        );
        assert!(normalize_china_phone("23800138000").is_err());
        assert!(normalize_china_phone("1380013800").is_err());
        assert_eq!(masked_phone_name("+86 13800138000"), "用户138****8000");
    }

    #[test]
    fn cloud_index_processing_messages_are_retried() {
        assert!(is_cloud_index_pending_message("文件上传中"));
        assert!(is_cloud_index_pending_message("任务处理中，请稍后再试"));
        assert!(!is_cloud_index_pending_message("文件违规，无法入库"));
        assert!(is_cloud_index_permanent_message("文件违规，无法入库"));
        assert!(!is_cloud_index_permanent_message("operation timed out"));
    }

    #[test]
    fn archive_collisions_never_overwrite_existing_files() {
        let root = std::env::temp_dir().join(format!("guangya-archive-test-{}", Uuid::new_v4()));
        let source_dir = root.join("source");
        let archive_dir = root.join("archive");
        fs::create_dir_all(&source_dir).expect("source directory");
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let source = source_dir.join("episode.mkv");
        fs::write(&source, b"new upload").expect("source fixture");
        let metadata = fs::metadata(&source).expect("source metadata");
        let modified = modified_ms(&metadata);
        let requested = archive_dir.join("episode.mkv");
        let first_collision = archive_candidate(&requested, modified, 1);
        fs::write(&requested, b"old archive").expect("base collision");
        fs::write(&first_collision, b"older archive").expect("suffix collision");

        let archived =
            archive_file_without_overwrite(&source, &requested, metadata.len(), modified)
                .expect("archive should find a unique name");
        assert_eq!(archived, archive_candidate(&requested, modified, 2));
        assert_eq!(fs::read(&requested).unwrap(), b"old archive");
        assert_eq!(fs::read(&first_collision).unwrap(), b"older archive");
        assert_eq!(fs::read(&archived).unwrap(), b"new upload");
        assert!(!source.exists());
        fs::remove_dir_all(root).expect("archive fixture cleanup");
    }

    #[test]
    fn staged_source_policy_preserves_a_replacement_created_during_archive() {
        let root = std::env::temp_dir().join(format!("guangya-archive-race-{}", Uuid::new_v4()));
        let watch = root.join("watch");
        let archive = root.join("archive");
        fs::create_dir_all(&watch).expect("watch directory");
        fs::create_dir_all(&archive).expect("archive directory");
        let source = watch.join("episode.mkv");
        fs::write(&source, b"uploaded-version").expect("source fixture");
        let metadata = fs::metadata(&source).expect("source metadata");
        let staged = stage_source_for_policy(&source, metadata.len(), modified_ms(&metadata))
            .expect("source should be staged atomically");

        fs::write(&source, b"new-local-version").expect("replacement fixture");
        let destination = archive.join("episode.mkv");
        archive_file_without_overwrite(
            &staged,
            &destination,
            metadata.len(),
            modified_ms(&metadata),
        )
        .expect("staged upload should archive");

        assert_eq!(fs::read(&source).unwrap(), b"new-local-version");
        assert_eq!(fs::read(&destination).unwrap(), b"uploaded-version");
        assert!(!staged.exists());
        fs::remove_dir_all(root).expect("archive race fixture cleanup");
    }

    #[test]
    fn unavailable_loaded_mapping_keeps_the_users_enabled_intent() {
        let missing =
            std::env::temp_dir().join(format!("guangya-offline-mapping-{}", Uuid::new_v4()));
        let mapping = Mapping {
            id: "offline".to_string(),
            local_path: missing.to_string_lossy().to_string(),
            remote_path: "backup".to_string(),
            remote_parent_id: String::new(),
            enabled: true,
            source_policy: "keep".to_string(),
            archive_path: None,
            scan_existing: true,
            sync_types: default_sync_types(),
            watch_error: None,
            monitor_mode: default_monitor_mode(),
            auto_share: false,
        };

        let loaded = normalize_loaded_mapping_paths(mapping);
        assert!(loaded.enabled);
        assert!(loaded.watch_error.is_some());
    }

    #[test]
    fn exclusive_archive_copy_preserves_source_on_collision_or_mismatch() {
        let root = std::env::temp_dir().join(format!("guangya-copy-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("copy directory");
        let source = root.join("source.bin");
        let destination = root.join("destination.bin");
        fs::write(&source, b"source bytes").expect("source fixture");
        fs::write(&destination, b"existing bytes").expect("destination fixture");
        let metadata = fs::metadata(&source).expect("source metadata");
        assert!(!copy_archive_exclusive(
            &source,
            &destination,
            metadata.len(),
            modified_ms(&metadata),
        )
        .expect("collision should not be an error"));
        assert_eq!(fs::read(&destination).unwrap(), b"existing bytes");
        assert!(source.exists());

        let mismatch_destination = root.join("mismatch.bin");
        assert!(copy_archive_exclusive(
            &source,
            &mismatch_destination,
            metadata.len() + 1,
            modified_ms(&metadata),
        )
        .is_err());
        assert!(source.exists());
        assert!(!mismatch_destination.exists());
        fs::remove_dir_all(root).expect("copy fixture cleanup");
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
        clear_persisted_auth_session(&database).expect("expired auth should clear");
        let cleared_auth = load_auth_session(&database).expect("cleared auth should load");
        assert!(cleared_auth.access_token.is_none());
        assert!(cleared_auth.refresh_token.is_none());
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
        save_upload_record(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-1".into(),
                remote_file_id: Some("file-1".into()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("upload history should persist");
        let history = load_upload_history(&database).expect("upload history should load");
        assert_eq!(
            history.get(&item_key(&item)),
            Some(&Stamp {
                size: 128,
                modified_ms: 42
            })
        );
        let mut pending_item = item.clone();
        pending_item.file_path = PathBuf::from("H:/test/pending.png");
        pending_item.relative_path = "pending.png".into();
        save_upload_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "task-pending".into(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .expect("OSS-complete history should persist before cloud indexing");
        let connection = open_database(&database).expect("database should reopen");
        let (task_id, remote_file_id, upload_state): (String, Option<String>, String) = connection
            .query_row(
                "SELECT task_id, remote_file_id, upload_state FROM uploaded_files WHERE mapping_id = ?1 AND file_path = ?2",
                params![pending_item.mapping_id, pending_item.file_path.to_string_lossy()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("pending upload should be queryable");
        assert_eq!(task_id, "task-pending");
        assert_eq!(remote_file_id, None);
        assert_eq!(upload_state, UPLOAD_STATE_OSS_COMPLETE);
        drop(connection);
        let pending = load_pending_uploads(&database).expect("pending uploads should load");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, "task-pending");
        assert_eq!(pending[0].item.relative_path, "pending.png");
        let history = load_upload_history(&database).expect("confirmed history should load");
        assert!(history.contains_key(&item_key(&item)));
        assert!(!history.contains_key(&item_key(&pending_item)));
        assert!(!confirm_pending_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "stale-task".into(),
                remote_file_id: Some("wrong-file".into()),
            },
        )
        .expect("a stale task must not replace the pending record"));
        assert!(confirm_pending_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "task-pending".into(),
                remote_file_id: Some("file-pending".into()),
            },
        )
        .expect("pending record should transition to confirmed"));
        assert!(load_pending_uploads(&database)
            .expect("pending rows should reload")
            .is_empty());
        assert!(load_upload_history(&database)
            .expect("confirmed rows should reload")
            .contains_key(&item_key(&pending_item)));
        let mut recreated_item = item.clone();
        recreated_item.mapping_id = "mapping-2".into();
        let reused = reuse_matching_confirmed_upload(&database, &recreated_item)
            .expect("confirmed upload history should be reusable")
            .expect("matching history should exist");
        assert_eq!(reused.0, item.mapping_id);
        assert_eq!(reused.1.remote_file_id.as_deref(), Some("file-1"));
        assert!(load_upload_history(&database)
            .expect("reused history should reload")
            .contains_key(&item_key(&recreated_item)));
        remove_mapping_transient_uploads(&database, &item.mapping_id)
            .expect("transient uploads should be removable");
        let history = load_upload_history(&database).expect("history should reload");
        assert!(history.contains_key(&item_key(&item)));
        assert!(history.contains_key(&item_key(&pending_item)));
        fs::remove_dir_all(root).expect("test database should be removable");
    }

    #[test]
    fn new_login_replaces_refresh_token_but_session_refresh_can_reuse_it() {
        let root =
            std::env::temp_dir().join(format!("guangya-auth-replacement-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize auth replacement database");
        save_auth_session(
            &database,
            Some("account-a-access"),
            Some("account-a-refresh"),
        )
        .expect("save first account");
        save_auth_session(&database, Some("account-b-access"), None)
            .expect("replace with account without refresh token");
        let replaced = load_auth_session(&database).expect("load replaced account");
        assert_eq!(replaced.access_token.as_deref(), Some("account-b-access"));
        assert!(replaced.refresh_token.is_none());

        save_auth_session(
            &database,
            Some("account-c-access"),
            Some("account-c-refresh"),
        )
        .expect("save refreshable account");
        save_refreshed_auth_session(&database, "account-c-renewed-access", None)
            .expect("refresh access token without rotating refresh token");
        let refreshed = load_auth_session(&database).expect("load refreshed account");
        assert_eq!(
            refreshed.access_token.as_deref(),
            Some("account-c-renewed-access")
        );
        assert_eq!(
            refreshed.refresh_token.as_deref(),
            Some("account-c-refresh")
        );
        fs::remove_dir_all(root).expect("remove auth replacement fixture");
    }

    #[test]
    fn api_auth_recovery_is_single_retry_without_reusing_a_stale_session() {
        assert_eq!(
            auth_recovery_decision("old-access", Some("old-access"), false, true),
            AuthRecoveryDecision::Refresh
        );
        assert_eq!(
            auth_recovery_decision("old-access", Some("new-access"), true, true),
            AuthRecoveryDecision::RetryWithCurrent("new-access".to_string())
        );
        assert_eq!(
            auth_recovery_decision("old-access", Some("new-account-access"), false, true),
            AuthRecoveryDecision::SessionChanged
        );
        assert_eq!(
            auth_recovery_decision("expired", Some("expired"), false, false),
            AuthRecoveryDecision::Invalidate
        );
        assert_eq!(
            auth_recovery_decision("expired", None, false, true),
            AuthRecoveryDecision::SessionChanged
        );
        let chain = HashMap::from([
            ("old-access".to_string(), "renewed-1".to_string()),
            ("renewed-1".to_string(), "renewed-2".to_string()),
        ]);
        assert!(refreshed_token_chain_reaches_current(
            &chain,
            "old-access",
            "renewed-2"
        ));
        assert!(is_api_auth_expired_error(
            "登录态已失效，请重新打开官方登录页"
        ));
    }

    #[test]
    fn sqlite_migrates_legacy_null_remote_ids_to_pending_state() {
        let root = std::env::temp_dir().join(format!("guangya-migration-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        fs::create_dir_all(&root).expect("migration directory");
        let connection = open_database(&database).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE uploaded_files (
                   mapping_id TEXT NOT NULL,
                   file_path TEXT NOT NULL,
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   task_id TEXT,
                   remote_file_id TEXT,
                   uploaded_at INTEGER NOT NULL,
                   PRIMARY KEY (mapping_id, file_path)
                 );
                 INSERT INTO uploaded_files VALUES
                   ('mapping-1', '/watch/confirmed.mkv', 10, '20', 'task-1', 'file-1', 1),
                   ('mapping-1', '/watch/pending.mkv', 11, '21', 'task-2', NULL, 1);",
            )
            .expect("legacy schema fixture");
        drop(connection);

        init_database(&database).expect("legacy database should migrate");
        let history = load_upload_history(&database).expect("confirmed history");
        assert!(history.contains_key(&destination_item_key(
            "mapping-1",
            Path::new("/watch/confirmed.mkv"),
            "",
            ""
        )));
        assert!(!history.contains_key(&destination_item_key(
            "mapping-1",
            Path::new("/watch/pending.mkv"),
            "",
            ""
        )));
        let pending = load_pending_uploads(&database).expect("pending migration");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, "task-2");
        fs::remove_dir_all(root).expect("migration fixture cleanup");
    }

    #[test]
    fn sqlite_migrates_legacy_checkpoint_destination_from_item_json() {
        let root = std::env::temp_dir().join(format!(
            "guangya-checkpoint-migration-test-{}",
            Uuid::new_v4()
        ));
        let database = root.join("state.sqlite3");
        fs::create_dir_all(&root).expect("checkpoint migration directory");
        let file_path = root.join("video.mkv");
        fs::write(&file_path, b"video-content").expect("checkpoint migration file");
        let metadata = fs::metadata(&file_path).expect("checkpoint migration metadata");
        let item = UploadItem {
            mapping_id: "__manual__".into(),
            file_path: file_path.clone(),
            remote_parent_id: "parent-migrated".into(),
            remote_dir: "folder-migrated".into(),
            relative_path: String::new(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        };
        let checkpoint = OssUploadCheckpoint {
            task_id: "task-migrated".into(),
            object_path: "objects/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss.example.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-migrated".into(),
            part_size: 5,
            completed_parts: BTreeMap::new(),
        };
        let connection = open_database(&database).expect("legacy checkpoint database");
        connection
            .execute_batch(
                "CREATE TABLE upload_checkpoints (
                   mapping_id TEXT NOT NULL,
                   file_path TEXT NOT NULL,
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   item_json TEXT NOT NULL,
                   checkpoint_json TEXT NOT NULL,
                   uploaded_bytes INTEGER NOT NULL DEFAULT 0,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY (mapping_id, file_path)
                 );",
            )
            .expect("legacy checkpoint schema");
        connection
            .execute(
                "INSERT INTO upload_checkpoints
                   (mapping_id, file_path, size, modified_ms, item_json, checkpoint_json,
                    uploaded_bytes, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 5, 1)",
                params![
                    item.mapping_id,
                    item.file_path.to_string_lossy(),
                    item.size,
                    item.modified_ms.to_string(),
                    serde_json::to_string(&item).unwrap(),
                    serde_json::to_string(&checkpoint).unwrap()
                ],
            )
            .expect("legacy checkpoint row");
        drop(connection);

        init_database(&database).expect("legacy checkpoint database should migrate");
        assert_eq!(
            table_primary_key_columns(&open_database(&database).unwrap(), "upload_checkpoints")
                .unwrap(),
            vec!["mapping_id", "file_path", "remote_parent_id", "remote_dir"]
        );
        let restored = load_resumable_uploads(&database).expect("restored migrated checkpoint");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].remote_parent_id, "parent-migrated");
        assert_eq!(restored[0].remote_dir, "folder-migrated");
        assert_eq!(
            load_upload_checkpoint(&database, &item)
                .unwrap()
                .unwrap()
                .uploaded_bytes,
            5
        );
        fs::remove_dir_all(root).expect("checkpoint migration fixture cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn canonical_source_checks_reject_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("guangya-symlink-boundary-{}", Uuid::new_v4()));
        let watch = root.join("watch");
        let outside = root.join("outside");
        fs::create_dir_all(&watch).expect("watch directory");
        fs::create_dir_all(&outside).expect("outside directory");
        let outside_file = outside.join("secret.mkv");
        fs::write(&outside_file, b"secret").expect("outside source");
        let escaped = watch.join("escaped.mkv");
        symlink(&outside_file, &escaped).expect("source symlink");

        assert!(canonical_watch_path(&escaped, &watch).is_err());
        assert!(canonical_upload_source(&escaped, Some(&watch)).is_err());
        assert!(canonical_upload_source(&outside_file, Some(&watch)).is_err());

        let archive_alias = root.join("archive-alias");
        symlink(&watch, &archive_alias).expect("archive alias");
        let canonical_watch = canonical_existing_directory(&watch, "监控目录").unwrap();
        let canonical_archive = canonical_existing_directory(&archive_alias, "归档目录").unwrap();
        assert_eq!(canonical_archive, canonical_watch);

        let archive_root = root.join("archive");
        fs::create_dir_all(&archive_root).expect("archive root");
        let escaped_archive_parent = outside.join("must-not-be-created");
        symlink(&outside, archive_root.join("linked-parent")).expect("archive middle symlink");
        assert!(ensure_directory_tree_without_symlinks(
            &archive_root,
            Path::new("linked-parent/must-not-be-created/nested")
        )
        .is_err());
        assert!(!escaped_archive_parent.exists());
        fs::remove_dir_all(root).expect("symlink boundary fixture cleanup");
    }

    #[test]
    fn gcid_export_parser_accepts_numbers_and_strings() {
        let raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "commonPath": "H:/Media",
          "totalFilesCount": 2,
          "totalSize": "30",
          "files": [
            {
              "path": "Movies/Film.mkv",
              "size": 10,
              "gcid": "0123456789ABCDEF0123456789ABCDEF01234567"
            },
            {
              "path": "Shows\\Episode.mkv",
              "size": "20",
              "gcid": "89abcdef0123456789abcdef0123456789abcdef"
            }
          ]
        }"#;
        let (files, total_size, common_path) =
            parse_gcid_export(raw).expect("valid Guangya export");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].folder_path, "Movies");
        assert_eq!(files[0].name, "Film.mkv");
        assert_eq!(files[0].gcid, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(files[1].path, "Shows/Episode.mkv");
        assert_eq!(total_size, 30);
        assert_eq!(common_path, "H:/Media");
    }

    #[test]
    fn gcid_export_parser_rejects_unsafe_and_duplicate_paths() {
        let unsafe_raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "files": [{
            "path": "../secret.mkv",
            "size": 1,
            "gcid": "0123456789abcdef0123456789abcdef01234567"
          }]
        }"#;
        assert!(parse_gcid_export(unsafe_raw)
            .expect_err("parent traversal must be rejected")
            .contains("越界"));

        let duplicate_raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "files": [
            {
              "path": "Movies/Film.mkv",
              "size": 1,
              "gcid": "0123456789abcdef0123456789abcdef01234567"
            },
            {
              "path": "Movies\\Film.mkv",
              "size": 1,
              "gcid": "89abcdef0123456789abcdef0123456789abcdef"
            }
          ]
        }"#;
        assert!(parse_gcid_export(duplicate_raw)
            .expect_err("normalized duplicate paths must be rejected")
            .contains("重复路径"));
    }

    #[test]
    fn gcid_import_jobs_are_scoped_to_destination() {
        let raw = br#"{"source":"guangya"}"#;
        let first = gcid_import_job_id(raw, "", "Media Library");
        assert_eq!(first, gcid_import_job_id(raw, "", "Media Library"));
        assert_ne!(first, gcid_import_job_id(raw, "parent-1", "Media Library"));
        assert_ne!(first, gcid_import_job_id(raw, "", "Other Library"));
        assert_eq!(first.len(), 32);
        assert!(validate_gcid_destination("Media Library").is_ok());
        assert!(validate_gcid_destination("../Media Library").is_err());
    }

    #[test]
    fn completed_gcid_import_is_idempotent_when_prepared_again() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        let source = root.join("library.json");
        init_database(&database).expect("database should initialize");
        let raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "totalFilesCount": 1,
          "totalSize": 10,
          "files": [{
            "path": "Movies/Film.mkv",
            "size": 10,
            "gcid": "0123456789abcdef0123456789abcdef01234567"
          }]
        }"#;
        let job_id = prepare_gcid_import_database(&database, raw, &source, "", "Media Library")
            .expect("job should be prepared");
        let connection = open_database(&database).expect("database should reopen");
        connection
            .execute(
                "UPDATE gcid_import_files SET status = 'imported' WHERE job_id = ?1",
                params![job_id],
            )
            .expect("file should become imported");
        connection
            .execute(
                "UPDATE gcid_import_jobs SET status = 'completed' WHERE job_id = ?1",
                params![job_id],
            )
            .expect("job should become completed");
        drop(connection);

        assert_eq!(
            prepare_gcid_import_database(&database, raw, &source, "", "Media Library")
                .expect("same job should be reusable"),
            job_id
        );
        let status = load_gcid_import_status(&database, Some(&job_id))
            .expect("status should load")
            .expect("status should exist");
        assert_eq!(status.status, "completed");
        assert_eq!(status.counts.imported, 1);
        assert_eq!(status.counts.pending, 0);
        fs::remove_dir_all(root).expect("GCID fixture cleanup");
    }
}

fn main() {
    run();
}
