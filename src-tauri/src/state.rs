//! 运行时状态、应用配置（AppConfig/Mapping）、快照与事件广播。

use crate::prelude::*;

pub(crate) type SharedState = Arc<Mutex<RuntimeState>>;


#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Mapping {
    pub(crate) id: String,
    pub(crate) local_path: String,
    pub(crate) remote_path: String,
    #[serde(default)]
    pub(crate) remote_parent_id: String,
    pub(crate) enabled: bool,
    #[serde(default = "default_source_policy")]
    pub(crate) source_policy: String,
    #[serde(default)]
    pub(crate) archive_path: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) scan_existing: bool,
    #[serde(default = "default_sync_types")]
    pub(crate) sync_types: Vec<String>,
    #[serde(default)]
    pub(crate) watch_error: Option<String>,
    #[serde(default = "default_monitor_mode")]
    pub(crate) monitor_mode: String,
    #[serde(default)]
    pub(crate) auto_share: bool,
    #[serde(default)]
    pub(crate) organizer_mapping_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedShare {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) url: String,
    pub(crate) created_at: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    #[serde(default)]
    pub(crate) mappings: Vec<Mapping>,
    #[serde(default)]
    pub(crate) saved_shares: Vec<SavedShare>,
    #[serde(default = "default_upload_concurrency")]
    pub(crate) upload_concurrency: usize,
    #[serde(default = "default_download_concurrency")]
    pub(crate) download_concurrency: usize,
    #[serde(default = "default_multipart_part_size")]
    pub(crate) multipart_part_size: String,
    #[serde(default = "default_true")]
    pub(crate) webdav_enabled: bool,
    #[serde(default = "default_webdav_port")]
    pub(crate) webdav_port: u16,
    #[serde(default = "default_webdav_username")]
    pub(crate) webdav_username: String,
    #[serde(default)]
    pub(crate) webdav_password: String,
    #[serde(default)]
    pub(crate) native_mount: NativeMountOptions,
    #[serde(default)]
    pub(crate) virtual_library: VirtualLibraryOptions,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mappings: Vec::new(),
            saved_shares: Vec::new(),
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
            multipart_part_size: default_multipart_part_size(),
            webdav_enabled: true,
            webdav_port: DEFAULT_WEBDAV_PORT,
            webdav_username: default_webdav_username(),
            webdav_password: String::new(),
            native_mount: NativeMountOptions::default(),
            virtual_library: VirtualLibraryOptions::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Snapshot {
    pub(crate) logged_in: bool,
    pub(crate) paused: bool,
    pub(crate) pending: usize,
    pub(crate) active_uploads: usize,
    pub(crate) upload_concurrency: usize,
    pub(crate) download_concurrency: usize,
    pub(crate) multipart_part_size: String,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) saved_shares: Vec<SavedShare>,
    pub(crate) hdhive: HdhivePublicConfig,
    pub(crate) auto_share_receipts: Vec<AutoShareReceipt>,
}

pub(crate) struct RuntimeState {
    pub(crate) token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) auth_account_scope: Option<String>,
    pub(crate) config_path: PathBuf,
    pub(crate) db_path: PathBuf,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) saved_shares: Vec<SavedShare>,
    pub(crate) queue: VecDeque<UploadItem>,
    pub(crate) flash_preflight_cache: HashMap<String, FlashPreflightCache>,
    pub(crate) waiting_files: HashMap<String, UploadItem>,
    pub(crate) history: HashMap<String, Stamp>,
    pub(crate) pending_cloud: HashMap<String, Stamp>,
    pub(crate) recovering_pending: HashSet<String>,
    pub(crate) inflight: HashMap<String, Stamp>,
    pub(crate) inflight_items: HashMap<String, UploadItem>,
    pub(crate) failed_uploads: HashMap<String, UploadItem>,
    pub(crate) cancelled_uploads: HashMap<String, Stamp>,
    pub(crate) paused_uploads: HashSet<String>,
    pub(crate) queue_pause_requests: HashSet<String>,
    pub(crate) remote_cache: HashMap<String, String>,
    /// 每个路径映射最近一次经远端确认的时间；超过
    /// `REMOTE_DIRECTORY_CACHE_FRESH_SECS` 后命中缓存必须先复核再使用。
    pub(crate) remote_cache_validated_at: HashMap<String, Instant>,
    pub(crate) remote_cache_generation: u64,
    pub(crate) remote_cache_gates: Arc<RemoteCacheGates>,
    pub(crate) upload_replacement_gates: Arc<RemoteCacheGates>,
    pub(crate) active_upload_replacements: HashSet<String>,
    pub(crate) watchers: HashMap<String, RecommendedWatcher>,
    pub(crate) event_tx: UnboundedSender<FsEvent>,
    pub(crate) paused: bool,
    pub(crate) active_uploads: usize,
    pub(crate) active_flash_preflights: usize,
    pub(crate) upload_concurrency: usize,
    pub(crate) download_concurrency: usize,
    pub(crate) multipart_part_size: String,
    pub(crate) cache_enabled: bool,
    pub(crate) cache_max_entries: usize,
    pub(crate) device_id: String,
    pub(crate) hdhive_enabled: bool,
    pub(crate) hdhive_base_url: String,
    pub(crate) hdhive_secret: String,
    pub(crate) hdhive_instance_id: String,
    pub(crate) auto_share_processing: HashSet<String>,
    pub(crate) gcid_import_running: HashSet<String>,
    pub(crate) developer_transfer_running: HashSet<String>,
    pub(crate) sms_verifications: HashMap<String, SmsVerificationSession>,
    pub(crate) webdav_enabled: bool,
    pub(crate) webdav_port: u16,
    pub(crate) webdav_username: String,
    pub(crate) webdav_password: String,
    pub(crate) webdav_running: bool,
    pub(crate) webdav_error: Option<String>,
    pub(crate) native_mount: NativeMountManager,
    pub(crate) virtual_library: VirtualLibraryManager,
    pub(crate) strm_sign_secret: String,
}


pub(crate) fn emit(app: &tauri::AppHandle, payload: impl Serialize + Clone) {
    let _ = app.emit("sync-event", payload);
}
pub(crate) fn status(app: &tauri::AppHandle, level: &str, message: impl Into<String>) {
    emit(
        app,
        json!({ "type": "status", "level": level, "message": message.into() }),
    );
}
pub(crate) fn snapshot(state: &RuntimeState) -> Snapshot {
    Snapshot {
        logged_in: state.token.is_some(),
        paused: state.paused,
        pending: state.queue.len() + state.waiting_files.len() + state.pending_cloud.len(),
        active_uploads: state.active_uploads,
        upload_concurrency: state.upload_concurrency,
        download_concurrency: state.download_concurrency,
        multipart_part_size: state.multipart_part_size.clone(),
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
pub(crate) fn default_source_policy() -> String {
    "keep".to_string()
}
pub(crate) fn default_upload_concurrency() -> usize {
    DEFAULT_UPLOAD_CONCURRENCY
}
pub(crate) fn default_download_concurrency() -> usize {
    DEFAULT_DOWNLOAD_CONCURRENCY
}
pub(crate) fn default_multipart_part_size() -> String {
    DEFAULT_MULTIPART_PART_SIZE.to_string()
}
pub(crate) fn default_webdav_port() -> u16 {
    DEFAULT_WEBDAV_PORT
}
pub(crate) fn default_webdav_username() -> String {
    DEFAULT_WEBDAV_USERNAME.to_string()
}
pub(crate) fn normalize_webdav_username(value: &str) -> Result<String, String> {
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
pub(crate) fn normalize_webdav_password(value: &str) -> Result<String, String> {
    if value.chars().count() < 12 || value.chars().count() > 256 {
        return Err("WebDAV 密码必须为 12 到 256 个字符".to_string());
    }
    Ok(value.to_string())
}

pub(crate) fn normalize_transfer_concurrency(value: usize, fallback: usize) -> usize {
    if (1..=MAX_TRANSFER_CONCURRENCY).contains(&value) {
        value
    } else {
        fallback
    }
}
pub(crate) fn validate_multipart_part_size(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if MULTIPART_PART_SIZE_OPTIONS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err("分片档位只支持 auto、4m、8m 或 16m".to_string())
    }
}
pub(crate) fn normalize_multipart_part_size(value: &str) -> String {
    validate_multipart_part_size(value).unwrap_or_else(|_| default_multipart_part_size())
}
pub(crate) fn default_true() -> bool {
    true
}
pub(crate) fn default_sync_types() -> Vec<String> {
    DEFAULT_MEDIA_EXTENSIONS
        .iter()
        .into_iter()
        .map(|value| (*value).to_string())
        .collect()
}
pub(crate) fn default_monitor_mode() -> String {
    "native".to_string()
}

pub(crate) fn normalize_monitor_mode(value: &str) -> String {
    if value.eq_ignore_ascii_case("polling") {
        "polling".to_string()
    } else {
        default_monitor_mode()
    }
}

pub(crate) fn normalize_sync_types(values: &[String]) -> Vec<String> {
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

pub(crate) fn emit_state(app: &tauri::AppHandle, state: &SharedState) {
    if let Ok(guard) = state.lock() {
        emit(app, json!({ "type": "state", "state": snapshot(&guard) }));
    }
}

pub(crate) fn load_config(path: &Path) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok())
        .unwrap_or_default()
}
pub(crate) fn save_config(state: &RuntimeState) {
    if let Some(parent) = state.config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let payload = json!({
        "mappings": state.mappings,
        "saved_shares": state.saved_shares,
        "upload_concurrency": state.upload_concurrency,
        "download_concurrency": state.download_concurrency,
        "multipart_part_size": state.multipart_part_size,
        "webdav_enabled": state.webdav_enabled,
        "webdav_port": state.webdav_port,
        "webdav_username": state.webdav_username,
        "webdav_password": state.webdav_password,
        "native_mount": state.native_mount.options(),
        "virtual_library": state.virtual_library.options()
    });
    let _ = fs::write(
        &state.config_path,
        serde_json::to_vec_pretty(&payload).unwrap_or_default(),
    );
}


#[tauri::command]
pub(crate) fn get_state(state: tauri::State<'_, SharedState>) -> Snapshot {
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
