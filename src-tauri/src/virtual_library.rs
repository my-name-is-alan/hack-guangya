use super::*;
use axum::{
    body::Body,
    extract::{
        ws::Message as AxumWsMessage, FromRequestParts, Path as AxumPath, Query, State,
        WebSocketUpgrade,
    },
    http::{header::LOCATION, HeaderMap, Method, Request, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as UpstreamWsMessage};

const DEFAULT_STRM_PORT: u16 = 18_096;
const DEFAULT_EMBY_UPSTREAM: &str = "http://127.0.0.1:8096";
const LEGACY_REDIRECT_PORT: u16 = 19_091;
const DEFAULT_REFRESH_MINUTES: u64 = 15;
const MAX_REMOTE_ITEMS: usize = 100_000;
const MAX_REMOTE_DEPTH: usize = 64;
const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MANIFEST_NAME: &str = ".guangya-virtual-library.json";
const MAX_FILE_ID_CHARS: usize = 256;

const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "asf", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "rm",
    "rmvb", "ts", "vob", "webm", "wmv",
];
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "ac3", "aiff", "alac", "ape", "dff", "dsf", "dts", "flac", "m4a", "mp3", "ogg", "opus",
    "wav", "wma",
];
const METADATA_EXTENSIONS: &[&str] = &[
    "ass", "cue", "gif", "jpeg", "jpg", "lrc", "nfo", "png", "srt", "ssa", "sub", "sup", "vtt",
    "webp", "xml",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VirtualLibraryMapping {
    pub id: String,
    pub name: String,
    pub source_dir_id: String,
    pub source_path: String,
    pub local_path: String,
    /// 该目录在 Emby 看到的路径，用于同步后的增量刷新通知；留空不通知。
    #[serde(default)]
    pub emby_path: String,
    #[serde(default)]
    pub include_metadata: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VirtualLibraryOptions {
    #[serde(
        default = "default_strm_port",
        alias = "proxy_port",
        alias = "redirect_port"
    )]
    pub strm_port: u16,
    #[serde(default)]
    pub strm_base_url: String,
    #[serde(default = "default_emby_upstream")]
    pub emby_upstream: String,
    #[serde(default)]
    pub emby_api_key: String,
    #[serde(default = "default_refresh_minutes")]
    pub refresh_minutes: u64,
    #[serde(default)]
    pub mappings: Vec<VirtualLibraryMapping>,
}

impl Default for VirtualLibraryOptions {
    fn default() -> Self {
        Self {
            strm_port: default_strm_port(),
            strm_base_url: String::new(),
            emby_upstream: default_emby_upstream(),
            emby_api_key: String::new(),
            refresh_minutes: default_refresh_minutes(),
            mappings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VirtualLibrarySyncStatus {
    pub running: bool,
    pub last_sync_at: Option<u64>,
    pub strm_files: usize,
    pub metadata_files: usize,
    pub skipped_files: usize,
    pub changed_files: usize,
    pub emby_notified: Option<usize>,
    pub emby_notify_error: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VirtualLibraryInfo {
    pub strm_endpoint: String,
    pub strm_base_url: String,
    pub strm_port: u16,
    pub strm_running: bool,
    pub strm_error: Option<String>,
    pub emby_upstream: String,
    pub emby_api_key_configured: bool,
    pub gateway_endpoint: String,
    pub refresh_minutes: u64,
    pub mappings: Vec<VirtualLibraryMapping>,
    pub statuses: HashMap<String, VirtualLibrarySyncStatus>,
}

pub struct VirtualLibraryManager {
    options: VirtualLibraryOptions,
    statuses: HashMap<String, VirtualLibrarySyncStatus>,
    strm_running: bool,
    strm_error: Option<String>,
}

impl VirtualLibraryManager {
    pub fn new(options: VirtualLibraryOptions) -> Self {
        let options = normalize_options(options).unwrap_or_default();
        Self {
            options,
            statuses: HashMap::new(),
            strm_running: false,
            strm_error: None,
        }
    }

    pub fn options(&self) -> VirtualLibraryOptions {
        self.options.clone()
    }

    /// STRM 内容使用的直链前缀：未显式配置时回落到本机 STRM 服务地址，
    /// 适合 Emby 与本软件在同一台机器的默认场景。
    pub fn effective_strm_base(&self) -> String {
        if self.options.strm_base_url.is_empty() {
            format!("http://127.0.0.1:{}", self.options.strm_port)
        } else {
            self.options.strm_base_url.clone()
        }
    }

    pub fn info(&self) -> VirtualLibraryInfo {
        VirtualLibraryInfo {
            strm_endpoint: format!("{}/strm/", self.effective_strm_base()),
            strm_base_url: self.options.strm_base_url.clone(),
            strm_port: self.options.strm_port,
            strm_running: self.strm_running,
            strm_error: self.strm_error.clone(),
            emby_upstream: self.options.emby_upstream.clone(),
            emby_api_key_configured: !self.options.emby_api_key.trim().is_empty(),
            gateway_endpoint: format!("{}/", self.effective_strm_base()),
            refresh_minutes: self.options.refresh_minutes,
            mappings: self.options.mappings.clone(),
            statuses: self.statuses.clone(),
        }
    }

    pub fn set_refresh_minutes(&mut self, value: u64) -> Result<(), String> {
        self.options.refresh_minutes = normalize_refresh_minutes(value)?;
        Ok(())
    }

    pub fn set_strm_base_url(&mut self, value: String) -> Result<(), String> {
        self.options.strm_base_url = normalize_strm_base_url(&value)?;
        Ok(())
    }

    pub fn set_emby_upstream(&mut self, value: String) -> Result<(), String> {
        self.options.emby_upstream = normalize_emby_upstream(&value)?;
        Ok(())
    }

    /// 空字符串保持不变的语义由调用方处理；这里直接落值（传 "" 即清除）。
    pub fn set_emby_api_key(&mut self, value: String) {
        self.options.emby_api_key = value.trim().to_string();
    }

    pub fn upsert_mapping(
        &mut self,
        mapping: VirtualLibraryMapping,
    ) -> Result<VirtualLibraryMapping, String> {
        let mapping = normalize_mapping(mapping)?;
        if self.options.mappings.iter().any(|current| {
            current.id != mapping.id
                && local_paths_overlap(
                    Path::new(&current.local_path),
                    Path::new(&mapping.local_path),
                )
        }) {
            return Err("虚拟库本地目录不能与其他配置相同或互相包含".to_string());
        }
        if let Some(current) = self
            .options
            .mappings
            .iter_mut()
            .find(|current| current.id == mapping.id)
        {
            *current = mapping.clone();
        } else {
            if self.options.mappings.len() >= 32 {
                return Err("虚拟库最多配置 32 个目录".to_string());
            }
            self.options.mappings.push(mapping.clone());
        }
        Ok(mapping)
    }

    pub fn remove_mapping(&mut self, id: &str) -> Result<(), String> {
        let before = self.options.mappings.len();
        self.options.mappings.retain(|mapping| mapping.id != id);
        if self.options.mappings.len() == before {
            return Err("虚拟库配置不存在".to_string());
        }
        self.statuses.remove(id);
        Ok(())
    }

    pub fn mapping(&self, id: &str) -> Option<VirtualLibraryMapping> {
        self.options
            .mappings
            .iter()
            .find(|mapping| mapping.id == id)
            .cloned()
    }

    pub fn begin_sync(&mut self, id: &str) -> Result<(), String> {
        let status = self.statuses.entry(id.to_string()).or_default();
        if status.running {
            return Err("该虚拟库正在同步".to_string());
        }
        status.running = true;
        status.error = None;
        Ok(())
    }

    pub fn finish_sync(&mut self, id: &str, result: Result<SyncSummary, String>) {
        let status = self.statuses.entry(id.to_string()).or_default();
        status.running = false;
        match result {
            Ok(summary) => {
                status.last_sync_at = Some(unix_timestamp());
                status.strm_files = summary.strm_files;
                status.metadata_files = summary.metadata_files;
                status.skipped_files = summary.skipped_files;
                status.changed_files = summary.changes.total;
                status.error = None;
            }
            Err(error) => status.error = Some(error),
        }
    }

    pub fn set_emby_notify_result(&mut self, id: &str, notified: Option<usize>, error: Option<String>) {
        let status = self.statuses.entry(id.to_string()).or_default();
        status.emby_notified = notified;
        status.emby_notify_error = error;
    }

    pub fn set_strm_status(&mut self, running: bool, error: Option<String>) {
        self.strm_running = running;
        self.strm_error = error;
    }
}

pub const MAX_CHANGE_PATHS: usize = 200;

#[derive(Debug, Clone, Default)]
pub struct SyncChanges {
    pub created: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub total: usize,
    pub truncated: bool,
}

impl SyncChanges {
    fn record(&mut self, bucket: ChangeKind, key: &str) {
        self.total += 1;
        if self.created.len() + self.modified.len() + self.removed.len() >= MAX_CHANGE_PATHS {
            self.truncated = true;
            return;
        }
        match bucket {
            ChangeKind::Created => self.created.push(key.to_string()),
            ChangeKind::Modified => self.modified.push(key.to_string()),
            ChangeKind::Removed => self.removed.push(key.to_string()),
        }
    }
}

#[derive(Clone, Copy)]
enum ChangeKind {
    Created,
    Modified,
    Removed,
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub strm_files: usize,
    pub metadata_files: usize,
    pub skipped_files: usize,
    pub changes: SyncChanges,
}

#[derive(Debug, Clone)]
struct RemoteEntry {
    id: String,
    name: String,
    is_directory: bool,
    size: u64,
    modified_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
    source_id: String,
    size: u64,
    modified_ms: u64,
    kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct VirtualLibraryManifest {
    version: u32,
    source_dir_id: String,
    entries: BTreeMap<String, ManifestEntry>,
}

pub const fn default_strm_port() -> u16 {
    DEFAULT_STRM_PORT
}

pub fn default_emby_upstream() -> String {
    DEFAULT_EMBY_UPSTREAM.to_string()
}

pub const fn default_refresh_minutes() -> u64 {
    DEFAULT_REFRESH_MINUTES
}

fn normalize_emby_upstream(value: &str) -> Result<String, String> {
    let raw = if value.trim().is_empty() {
        DEFAULT_EMBY_UPSTREAM
    } else {
        value.trim()
    };
    let parsed = reqwest::Url::parse(raw).map_err(|_| "Emby 原始服务地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err("Emby 原始服务地址必须是无账号、路径和查询参数的 HTTP(S) 地址".to_string());
    }
    Ok(parsed.origin().ascii_serialization())
}

fn normalize_refresh_minutes(value: u64) -> Result<u64, String> {
    if !(1..=24 * 60).contains(&value) {
        return Err("虚拟库刷新间隔必须为 1 到 1440 分钟".to_string());
    }
    Ok(value)
}

pub fn normalize_strm_base_url(value: &str) -> Result<String, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| "STRM 直链地址无效".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            "STRM 直链地址必须是不带账号和查询参数的 HTTP(S) 地址，例如 http://192.168.1.10:18096"
                .to_string(),
        );
    }
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path().trim_end_matches('/');
    Ok(format!("{origin}{path}"))
}

pub(crate) fn strm_signature(secret: &str, file_id: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .unwrap_or_else(|_| Hmac::<Sha256>::new_from_slice(b"guangya").expect("hmac"));
    mac.update(file_id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (l, r)| acc | (l ^ r))
        == 0
}

pub(crate) fn verify_strm_signature(secret: &str, file_id: &str, signature: &str) -> bool {
    if secret.trim().is_empty() || file_id.trim().is_empty() {
        return false;
    }
    let expected = strm_signature(secret, file_id);
    constant_time_eq(
        expected.as_bytes(),
        signature.trim().to_ascii_lowercase().as_bytes(),
    )
}

pub(crate) fn strm_url(base: &str, secret: &str, file_id: &str) -> String {
    format!(
        "{}/strm/{}?sign={}",
        base.trim_end_matches('/'),
        utf8_percent_encode(file_id, NON_ALPHANUMERIC),
        strm_signature(secret, file_id)
    )
}

fn valid_strm_file_id(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.chars().count() <= MAX_FILE_ID_CHARS
        && !value.chars().any(char::is_control)
        && !value.contains(['/', '\\'])
}

fn normalize_options(mut options: VirtualLibraryOptions) -> Result<VirtualLibraryOptions, String> {
    if options.strm_port == 0 || options.strm_port == LEGACY_REDIRECT_PORT {
        options.strm_port = default_strm_port();
    }
    if options.strm_port == DEFAULT_WEBDAV_PORT {
        return Err("STRM 直链端口不能与 WebDAV 端口相同".to_string());
    }
    options.strm_base_url = normalize_strm_base_url(&options.strm_base_url)?;
    options.emby_upstream = normalize_emby_upstream(&options.emby_upstream)?;
    options.refresh_minutes = normalize_refresh_minutes(options.refresh_minutes)?;
    options.mappings = options
        .mappings
        .into_iter()
        .map(normalize_mapping)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(options)
}

/// Emby 内路径：支持 Windows 与 POSIX 形式，去除尾部分隔符；留空表示不通知。
pub fn normalize_emby_path(value: &str) -> String {
    value.trim().trim_end_matches(['/', '\\']).to_string()
}

/// 云端路径子树是否重叠（相同或互为前缀）；'/' 视为整个云盘。
pub fn cloud_paths_overlap(left: &str, right: &str) -> bool {
    let normalize = |value: &str| -> String {
        let collapsed = value.trim().replace('\\', "/");
        let mut collapsed = collapsed;
        while collapsed.contains("//") {
            collapsed = collapsed.replace("//", "/");
        }
        if collapsed.is_empty() {
            return String::new();
        }
        if collapsed == "/" {
            return "/".to_string();
        }
        let stripped = collapsed.trim_end_matches('/');
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            format!("/{stripped}")
        }
    };
    let a = normalize(left);
    let b = normalize(right);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == "/" || b == "/" {
        return true;
    }
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

fn normalize_mapping(mut mapping: VirtualLibraryMapping) -> Result<VirtualLibraryMapping, String> {
    mapping.id = mapping.id.trim().to_string();
    if mapping.id.is_empty() {
        mapping.id = Uuid::new_v4().to_string();
    }
    mapping.name = mapping.name.trim().to_string();
    mapping.emby_path = normalize_emby_path(&mapping.emby_path);
    mapping.source_dir_id = normalize_api_id(&mapping.source_dir_id, "虚拟库云端目录 ID")?;
    mapping.source_path = mapping.source_path.trim().to_string();
    if mapping.source_path.is_empty() {
        return Err("虚拟库云端目录路径不能为空".to_string());
    }
    mapping.local_path = normalize_local_root(&mapping.local_path)?
        .to_string_lossy()
        .to_string();
    if mapping.name.is_empty() {
        mapping.name = mapping
            .source_path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or("虚拟库")
            .to_string();
    }
    if mapping.name.chars().count() > 80 {
        return Err("虚拟库名称不能超过 80 个字符".to_string());
    }
    Ok(mapping)
}

fn normalize_local_root(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err("虚拟库本地目录必须使用绝对路径".to_string());
    }
    let component_count = path.components().count();
    if component_count <= 2 || path.parent().is_none() {
        return Err("不能把磁盘根目录作为虚拟库目录".to_string());
    }
    Ok(path)
}

fn local_paths_overlap(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    let (left, right) = (
        PathBuf::from(left.to_string_lossy().to_lowercase()),
        PathBuf::from(right.to_string_lossy().to_lowercase()),
    );
    #[cfg(not(windows))]
    let (left, right) = (left.to_path_buf(), right.to_path_buf());
    left.starts_with(&right) || right.starts_with(&left)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_media(name: &str) -> bool {
    let extension = extension(name);
    VIDEO_EXTENSIONS.contains(&extension.as_str()) || AUDIO_EXTENSIONS.contains(&extension.as_str())
}

fn is_metadata(name: &str) -> bool {
    METADATA_EXTENSIONS.contains(&extension(name).as_str())
}

fn safe_component(value: &str) -> String {
    let mut safe = value
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
        .collect::<String>();
    safe = safe.trim().trim_end_matches(['.', ' ']).to_string();
    if safe.is_empty() || safe == "." || safe == ".." {
        safe = "未命名".to_string();
    }
    let stem = safe
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem[3..].parse::<u8>().is_ok())
    {
        safe.insert(0, '_');
    }
    safe
}

fn media_output_name(name: &str) -> String {
    let safe = safe_component(name);
    let stem = Path::new(&safe)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&safe);
    format!("{stem}.strm")
}

fn strm_content(url: &str) -> String {
    format!("{}\n", url.trim())
}

fn safe_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn relative_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_remote_entry(value: Value) -> Option<RemoteEntry> {
    let id = value
        .get("fileId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let name = value
        .get("fileName")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if id.is_empty() || name.is_empty() {
        return None;
    }
    let is_directory = value
        .get("resType")
        .or_else(|| value.get("type"))
        .and_then(Value::as_u64)
        .is_some_and(|value| value == 2)
        || value
            .get("isDirectory")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let size = value
        .get("fileSize")
        .or_else(|| value.get("size"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0);
    let modified_ms = ["utime", "updatedAt", "modifiedAt", "mtime", "ctime"]
        .into_iter()
        .find_map(|key| {
            value.get(key).and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
            })
        })
        .unwrap_or(0);
    Some(RemoteEntry {
        id,
        name,
        is_directory,
        size,
        modified_ms,
    })
}

async fn fetch_children(
    token: &str,
    device_id: &str,
    parent_id: &str,
) -> Result<Vec<RemoteEntry>, String> {
    let mut entries = Vec::new();
    for page in 0..1000_u64 {
        let response = api_post(
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
        let data = response.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_count = list.len();
        entries.extend(list.into_iter().filter_map(normalize_remote_entry));
        let total = data
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or(entries.len() as u64);
        if page_count == 0 || entries.len() as u64 >= total {
            break;
        }
    }
    Ok(entries)
}

async fn download_metadata(
    client: &reqwest::Client,
    token: &str,
    device_id: &str,
    entry: &RemoteEntry,
    target: &Path,
) -> Result<(), String> {
    if entry.size > MAX_METADATA_BYTES {
        return Err(format!("元数据文件超过 64 MB：{}", entry.name));
    }
    let url = cached_res_download_url(token, device_id, &entry.id, false).await?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载元数据失败（{}）：{error}", entry.name))?;
    if !response.status().is_success() {
        download_url_cache().invalidate(&entry.id);
        return Err(format!(
            "下载元数据失败（{}）：HTTP {}",
            entry.name,
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取元数据失败（{}）：{error}", entry.name))?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(format!("元数据文件超过 64 MB：{}", entry.name));
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("创建虚拟库目录失败：{error}"))?;
    }
    tokio::fs::write(target, bytes)
        .await
        .map_err(|error| format!("写入元数据失败（{}）：{error}", target.display()))
}

async fn load_manifest(root: &Path) -> VirtualLibraryManifest {
    let path = root.join(MANIFEST_NAME);
    tokio::fs::read(path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

async fn save_manifest(root: &Path, manifest: &VirtualLibraryManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("生成虚拟库清单失败：{error}"))?;
    tokio::fs::write(root.join(MANIFEST_NAME), bytes)
        .await
        .map_err(|error| format!("保存虚拟库清单失败：{error}"))
}

async fn remove_stale_files(
    root: &Path,
    previous: &VirtualLibraryManifest,
    next: &VirtualLibraryManifest,
) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    for key in previous.entries.keys() {
        if next.entries.contains_key(key) {
            continue;
        }
        let relative = PathBuf::from(key);
        if !safe_relative(&relative) {
            continue;
        }
        let target = root.join(relative);
        match tokio::fs::remove_file(&target).await {
            Ok(()) => removed.push(key.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => removed.push(key.clone()),
            Err(error) => {
                return Err(format!(
                    "清理过期虚拟文件失败（{}）：{error}",
                    target.display()
                ))
            }
        }
    }
    Ok(removed)
}

/// 自底向上删除 `root` 下的空目录，保留虚拟库根目录本身。
async fn prune_empty_directories(root: &Path) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    prune_empty_directories_inner(root, root, &mut removed).await?;
    Ok(removed)
}

fn prune_empty_directories_inner<'a>(
    root: &'a Path,
    dir: &'a Path,
    removed: &'a mut Vec<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!("读取虚拟库目录失败（{}）：{error}", dir.display()))
            }
        };
        let mut children = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("读取虚拟库目录失败（{}）：{error}", dir.display()))?
        {
            children.push(entry);
        }
        for entry in children {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| format!("读取虚拟库目录失败（{}）：{error}", dir.display()))?;
            if file_type.is_dir() {
                prune_empty_directories_inner(root, &entry.path(), removed).await?;
            }
        }
        if dir == root {
            return Ok(());
        }
        let mut remaining = match tokio::fs::read_dir(dir).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!("读取虚拟库目录失败（{}）：{error}", dir.display()))
            }
        };
        let mut empty = true;
        while let Some(entry) = remaining
            .next_entry()
            .await
            .map_err(|error| format!("读取虚拟库目录失败（{}）：{error}", dir.display()))?
        {
            if entry.file_name() != MANIFEST_NAME {
                empty = false;
                break;
            }
        }
        if empty {
            tokio::fs::remove_dir(dir).await.map_err(|error| {
                format!("清理空虚拟库目录失败（{}）：{error}", dir.display())
            })?;
            let key = dir
                .strip_prefix(root)
                .map(relative_key)
                .unwrap_or_else(|_| relative_key(dir));
            if !key.is_empty() {
                removed.push(key);
            }
        }
        Ok(())
    })
}

pub async fn sync_mapping(
    state: &SharedState,
    mapping: &VirtualLibraryMapping,
) -> Result<SyncSummary, String> {
    let (token, device_id, strm_base, sign_secret) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.virtual_library.effective_strm_base(),
            guard.strm_sign_secret.clone(),
        )
    };
    if sign_secret.trim().is_empty() {
        return Err("STRM 签名密钥未初始化，请重启应用".to_string());
    }
    let root = normalize_local_root(&mapping.local_path)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| format!("创建虚拟库目录失败：{error}"))?;
    let previous = load_manifest(&root).await;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| format!("创建元数据下载客户端失败：{error}"))?;
    let mut next = VirtualLibraryManifest {
        version: 3,
        source_dir_id: mapping.source_dir_id.clone(),
        entries: BTreeMap::new(),
    };
    let mut pending = VecDeque::from([(mapping.source_dir_id.clone(), PathBuf::new(), 0_usize)]);
    let mut seen_outputs = HashSet::new();
    let mut scanned = 0_usize;
    let mut summary = SyncSummary {
        strm_files: 0,
        metadata_files: 0,
        skipped_files: 0,
        changes: SyncChanges::default(),
    };

    while let Some((parent_id, relative_dir, depth)) = pending.pop_front() {
        if depth > MAX_REMOTE_DEPTH {
            return Err(format!("云端目录超过 {MAX_REMOTE_DEPTH} 层，已停止同步"));
        }
        for entry in fetch_children(&token, &device_id, &parent_id).await? {
            scanned += 1;
            if scanned > MAX_REMOTE_ITEMS {
                return Err(format!("单个虚拟库超过 {MAX_REMOTE_ITEMS} 项，已停止同步"));
            }
            if entry.is_directory {
                let child_relative = relative_dir.join(safe_component(&entry.name));
                tokio::fs::create_dir_all(root.join(&child_relative))
                    .await
                    .map_err(|error| format!("创建虚拟库目录失败：{error}"))?;
                pending.push_back((entry.id, child_relative, depth + 1));
                continue;
            }

            let (relative_file, kind) = if is_media(&entry.name) {
                (relative_dir.join(media_output_name(&entry.name)), "strm")
            } else if mapping.include_metadata && is_metadata(&entry.name) {
                (relative_dir.join(safe_component(&entry.name)), "metadata")
            } else {
                summary.skipped_files += 1;
                continue;
            };
            let key = relative_key(&relative_file);
            let collision_key = if cfg!(windows) {
                key.to_lowercase()
            } else {
                key.clone()
            };
            if !seen_outputs.insert(collision_key) {
                return Err(format!("多个云端文件会生成同一本地文件：{key}"));
            }
            let target = root.join(&relative_file);
            let manifest_entry = ManifestEntry {
                source_id: entry.id.clone(),
                size: entry.size,
                modified_ms: entry.modified_ms,
                kind: kind.to_string(),
            };
            let unchanged = previous.entries.get(&key).is_some_and(|previous| {
                previous.source_id == manifest_entry.source_id
                    && previous.size == manifest_entry.size
                    && previous.modified_ms == manifest_entry.modified_ms
                    && previous.kind == manifest_entry.kind
                    && target.is_file()
            });
            let existed_before = previous.entries.contains_key(&key);
            if kind == "strm" {
                let content = strm_content(&strm_url(&strm_base, &sign_secret, &entry.id));
                if !unchanged
                    || tokio::fs::read_to_string(&target).await.ok().as_deref()
                        != Some(content.as_str())
                {
                    if let Some(parent) = target.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|error| format!("创建虚拟库目录失败：{error}"))?;
                    }
                    tokio::fs::write(&target, content).await.map_err(|error| {
                        format!("写入 STRM 失败（{}）：{error}", target.display())
                    })?;
                    summary.changes.record(
                        if existed_before {
                            ChangeKind::Modified
                        } else {
                            ChangeKind::Created
                        },
                        &key,
                    );
                }
                summary.strm_files += 1;
            } else {
                if !unchanged {
                    download_metadata(&client, &token, &device_id, &entry, &target).await?;
                    summary.changes.record(
                        if existed_before {
                            ChangeKind::Modified
                        } else {
                            ChangeKind::Created
                        },
                        &key,
                    );
                }
                summary.metadata_files += 1;
            }
            next.entries.insert(key, manifest_entry);
        }
    }

    let removed_keys = remove_stale_files(&root, &previous, &next).await?;
    for key in &removed_keys {
        summary.changes.record(ChangeKind::Removed, key);
    }
    // 过期 STRM/元数据删掉后，自底向上收回变空的电影/剧集文件夹。
    for key in prune_empty_directories(&root).await? {
        summary.changes.record(ChangeKind::Removed, &key);
    }
    save_manifest(&root, &next).await?;
    Ok(summary)
}

/// 同步完成后按变更清单调用 Emby 的增量扫描接口；未配置 API Key 或
/// Emby 内路径时返回 None（不通知）。
pub async fn notify_emby(
    upstream: &str,
    api_key: &str,
    emby_path: &str,
    changes: &SyncChanges,
) -> Result<Option<usize>, String> {
    let emby_path = normalize_emby_path(emby_path);
    if api_key.trim().is_empty() || emby_path.is_empty() || changes.total == 0 {
        return Ok(None);
    }
    let windows_style = emby_path.contains('\\');
    let to_emby_path = |key: &str| -> String {
        if windows_style {
            format!("{emby_path}\\{}", key.replace('/', "\\"))
        } else {
            format!("{emby_path}/{key}")
        }
    };
    let updates: Vec<Value> = if changes.truncated {
        vec![json!({ "Path": emby_path, "UpdateType": "Modified" })]
    } else {
        changes
            .created
            .iter()
            .map(|key| json!({ "Path": to_emby_path(key), "UpdateType": "Created" }))
            .chain(
                changes
                    .modified
                    .iter()
                    .map(|key| json!({ "Path": to_emby_path(key), "UpdateType": "Modified" })),
            )
            .chain(
                changes
                    .removed
                    .iter()
                    .map(|key| json!({ "Path": to_emby_path(key), "UpdateType": "Deleted" })),
            )
            .collect()
    };
    if updates.is_empty() {
        return Ok(None);
    }
    let count = updates.len();
    let target = format!(
        "{}/emby/Library/Media/Updated",
        upstream.trim_end_matches('/')
    );
    let client = http_client()?;
    let response = client
        .post(target)
        .header("x-emby-token", api_key.trim())
        .json(&json!({ "Updates": updates }))
        .send()
        .await
        .map_err(|error| format!("Emby 刷新通知失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("Emby 刷新通知失败：HTTP {}", response.status()));
    }
    Ok(Some(count))
}

fn response(status: StatusCode, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .header("access-control-allow-origin", "*")
        .body(body.into())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[derive(Clone)]
struct StrmContext {
    state: SharedState,
    client: reqwest::Client,
}

/// 从 Emby MediaSources 的 Path 中解析本服务签发的 STRM 直链，
/// 返回 (fileId, 签名)。非本服务直链返回 None。
fn strm_url_credentials(value: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let mut segments = parsed.path_segments()?;
    if segments.next()? != "strm" {
        return None;
    }
    let file_id = segments.next()?.to_string();
    if segments.next().is_some() {
        return None;
    }
    let file_id = percent_encoding::percent_decode_str(&file_id)
        .decode_utf8()
        .ok()?
        .to_string();
    if !valid_strm_file_id(&file_id) {
        return None;
    }
    let signature = parsed
        .query_pairs()
        .find(|(name, _)| name == "sign")
        .map(|(_, value)| value.into_owned())?;
    Some((file_id, signature))
}

/// 只匹配原画播放路由（stream/original/Items File）；HLS 转码等仍交给 Emby。
fn playback_item_id(path: &str) -> Option<String> {
    let mut segments = path
        .trim_matches('/')
        .split('/')
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if segments
        .first()
        .is_some_and(|value| value.eq_ignore_ascii_case("emby"))
    {
        segments.remove(0);
    }
    let raw_id = if segments.len() == 3
        && (segments[0].eq_ignore_ascii_case("videos") || segments[0].eq_ignore_ascii_case("audio"))
        && (segments[2].eq_ignore_ascii_case("stream")
            || segments[2].to_ascii_lowercase().starts_with("stream.")
            || segments[2].eq_ignore_ascii_case("original")
            || segments[2].to_ascii_lowercase().starts_with("original."))
    {
        segments[1]
    } else if segments.len() == 3
        && segments[0].eq_ignore_ascii_case("items")
        && segments[2].eq_ignore_ascii_case("file")
    {
        segments[1]
    } else {
        return None;
    };
    let decoded = percent_encoding::percent_decode_str(raw_id)
        .decode_utf8()
        .ok()?;
    normalize_api_id(&decoded, "Emby 媒体 ID").ok()
}

fn emby_request_url(upstream: &str, uri: &axum::http::Uri) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(upstream).map_err(|error| error.to_string())?;
    url.set_path(uri.path());
    url.set_query(uri.query());
    Ok(url)
}

/// 对命中的播放路由调用上游 PlaybackInfo，找出 Path 为本服务签发
/// STRM 直链的媒体源并校验签名，返回云端 fileId。
async fn playback_file_id(
    context: &StrmContext,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
) -> Option<String> {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return None;
    }
    let item_id = playback_item_id(uri.path())?;
    let (upstream, secret) = {
        let guard = context.state.lock().ok()?;
        (
            guard.virtual_library.options().emby_upstream,
            guard.strm_sign_secret.clone(),
        )
    };
    let prefix = if uri.path().to_ascii_lowercase().starts_with("/emby/") {
        "/emby"
    } else {
        ""
    };
    let mut playback_url = reqwest::Url::parse(&upstream).ok()?;
    playback_url.set_path(&format!("{prefix}/Items/{item_id}/PlaybackInfo"));
    if let Some(query) = uri.query() {
        let request_url = reqwest::Url::parse(&format!("http://localhost/?{query}")).ok()?;
        for (name, value) in request_url.query_pairs() {
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "api_key" | "x-emby-token" | "userid"
            ) {
                playback_url.query_pairs_mut().append_pair(&name, &value);
            }
        }
    }
    let mut request = context.client.get(playback_url);
    for name in [
        "authorization",
        "x-emby-authorization",
        "x-emby-token",
        "user-agent",
    ] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    let payload = request
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    let sources = payload
        .get("MediaSources")
        .or_else(|| payload.get("mediaSources"))
        .and_then(Value::as_array)?;
    let request_url = reqwest::Url::parse(&format!(
        "http://localhost{}",
        uri.path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/")
    ))
    .ok()?;
    let requested_source_id = request_url
        .query_pairs()
        .find(|(name, _)| name.eq_ignore_ascii_case("MediaSourceId"))
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let mut ordered = sources.iter().collect::<Vec<_>>();
    if !requested_source_id.is_empty() {
        ordered.sort_by_key(|source| {
            source
                .get("Id")
                .or_else(|| source.get("id"))
                .and_then(Value::as_str)
                .is_none_or(|value| value != requested_source_id)
        });
    }
    for source in ordered {
        let path = source
            .get("Path")
            .or_else(|| source.get("path"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some((file_id, signature)) = strm_url_credentials(path) {
            if verify_strm_signature(&secret, &file_id, &signature) {
                return Some(file_id);
            }
        }
    }
    None
}

/// 浏览器（Emby Web）里的 JS fetch 读取跨域数据需要 CORS 头，而云盘 CDN
/// 不返回；对浏览器 UA 的播放请求改为网关中转并注入 CORS 头，
/// App 播放器仍然 302 直连 CDN。
pub(crate) fn is_browser_user_agent(value: &str) -> bool {
    let agent = value.to_ascii_lowercase();
    agent.contains("mozilla/")
        && (agent.contains("chrome/")
            || agent.contains("safari/")
            || agent.contains("firefox/")
            || agent.contains("edg/"))
}

/// 浏览器播放中转：拉取 CDN 数据流式转发，透传 Range，补 CORS 头；
/// 缓存直链过期（403/410）时强制刷新一次。
async fn proxy_cdn_stream(
    context: &StrmContext,
    token: &str,
    device_id: &str,
    file_id: &str,
    request_headers: &HeaderMap,
) -> Response<Body> {
    let send = |url: String| {
        let mut outgoing = context.client.get(url);
        for name in ["range", "if-range", "accept-encoding"] {
            if let Some(value) = request_headers.get(name) {
                outgoing = outgoing.header(name, value);
            }
        }
        outgoing.send()
    };
    let url = match cached_res_download_url(token, device_id, file_id, false).await {
        Ok(url) => url,
        Err(error) => return response(StatusCode::BAD_GATEWAY, error),
    };
    let mut upstream = match send(url).await {
        Ok(upstream) => upstream,
        Err(error) => return response(StatusCode::BAD_GATEWAY, error.to_string()),
    };
    if matches!(upstream.status().as_u16(), 403 | 410) {
        let refreshed = match cached_res_download_url(token, device_id, file_id, true).await {
            Ok(url) => url,
            Err(error) => return response(StatusCode::BAD_GATEWAY, error),
        };
        upstream = match send(refreshed).await {
            Ok(upstream) => upstream,
            Err(error) => return response(StatusCode::BAD_GATEWAY, error.to_string()),
        };
    }
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for name in [
        "content-type",
        "content-length",
        "content-range",
        "accept-ranges",
        "etag",
        "last-modified",
    ] {
        if let Some(value) = upstream.headers().get(name) {
            builder = builder.header(name, value.as_bytes());
        }
    }
    builder = builder
        .header("cache-control", "no-store")
        .header("access-control-allow-origin", "*")
        .header(
            "access-control-expose-headers",
            "Content-Range, Content-Length, Accept-Ranges, ETag, Last-Modified",
        );
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| response(StatusCode::BAD_GATEWAY, "构造播放中转响应失败"))
}

fn hop_by_hop_header(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

async fn proxy_http(context: &StrmContext, request: Request<Body>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let upstream = match context.state.lock() {
        Ok(guard) => guard.virtual_library.options().emby_upstream,
        Err(error) => return response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let url = match emby_request_url(&upstream, &parts.uri) {
        Ok(url) => url,
        Err(error) => return response(StatusCode::BAD_GATEWAY, error),
    };
    let client_host = parts.headers.get("host").cloned();
    let mut outgoing = context.client.request(parts.method, url);
    for (name, value) in &parts.headers {
        if name.as_str() != "host" && !hop_by_hop_header(name) {
            outgoing = outgoing.header(name, value);
        }
    }
    if let Some(host) = &client_host {
        outgoing = outgoing.header("x-forwarded-host", host);
    }
    outgoing = outgoing
        .header("x-forwarded-proto", "http")
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));
    let upstream_response = match outgoing.send().await {
        Ok(response) => response,
        Err(error) => {
            return response(
                StatusCode::BAD_GATEWAY,
                format!("Emby 原始服务连接失败：{error}"),
            )
        }
    };
    let status = upstream_response.status();
    let mut headers = upstream_response.headers().clone();
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
    if let (Some(location), Some(host)) = (headers.get(LOCATION).cloned(), client_host) {
        if let (Ok(location), Ok(host)) = (location.to_str(), host.to_str()) {
            if location.starts_with(&upstream) {
                if let Ok(value) = format!("http://{host}{}", &location[upstream.len()..]).parse() {
                    headers.insert(LOCATION, value);
                }
            }
        }
    }
    let mut builder = Response::builder().status(status);
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(upstream_response.bytes_stream()))
        .unwrap_or_else(|_| response(StatusCode::BAD_GATEWAY, "构造 Emby 网关响应失败"))
}

fn websocket_url(upstream: &str, uri: &axum::http::Uri) -> Result<String, String> {
    let mut url = emby_request_url(upstream, uri)?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| "Emby WebSocket 地址无效".to_string())?;
    Ok(url.to_string())
}

async fn relay_websocket(
    downstream: axum::extract::ws::WebSocket,
    upstream: String,
    headers: HeaderMap,
) {
    let mut request = match upstream.into_client_request() {
        Ok(request) => request,
        Err(_) => return,
    };
    for name in [
        "authorization",
        "x-emby-authorization",
        "x-emby-token",
        "origin",
        "user-agent",
        "sec-websocket-protocol",
    ] {
        if let Some(value) = headers.get(name) {
            request.headers_mut().insert(name, value.clone());
        }
    }
    let upstream = match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => socket,
        Err(_) => return,
    };
    let (mut downstream_tx, mut downstream_rx) = downstream.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    loop {
        tokio::select! {
            message = downstream_rx.next() => match message {
                Some(Ok(AxumWsMessage::Text(value))) => {
                    if upstream_tx.send(UpstreamWsMessage::Text(value.to_string().into())).await.is_err() { break; }
                }
                Some(Ok(AxumWsMessage::Binary(value))) => {
                    if upstream_tx.send(UpstreamWsMessage::Binary(value)).await.is_err() { break; }
                }
                Some(Ok(AxumWsMessage::Ping(value))) => {
                    if upstream_tx.send(UpstreamWsMessage::Ping(value)).await.is_err() { break; }
                }
                Some(Ok(AxumWsMessage::Pong(value))) => {
                    if upstream_tx.send(UpstreamWsMessage::Pong(value)).await.is_err() { break; }
                }
                Some(Ok(AxumWsMessage::Close(_))) | Some(Err(_)) | None => break,
            },
            message = upstream_rx.next() => match message {
                Some(Ok(UpstreamWsMessage::Text(value))) => {
                    if downstream_tx.send(AxumWsMessage::Text(value.to_string().into())).await.is_err() { break; }
                }
                Some(Ok(UpstreamWsMessage::Binary(value))) => {
                    if downstream_tx.send(AxumWsMessage::Binary(value)).await.is_err() { break; }
                }
                Some(Ok(UpstreamWsMessage::Ping(value))) => {
                    if downstream_tx.send(AxumWsMessage::Ping(value)).await.is_err() { break; }
                }
                Some(Ok(UpstreamWsMessage::Pong(value))) => {
                    if downstream_tx.send(AxumWsMessage::Pong(value)).await.is_err() { break; }
                }
                Some(Ok(UpstreamWsMessage::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(UpstreamWsMessage::Frame(_))) => {}
            }
        }
    }
}

/// Emby 兼容网关：客户端把本服务地址当 Emby 用。普通请求（浏览、搜索、
/// 图片、WebSocket）完整转发到 Emby 原始服务；命中 STRM 直链媒体源的原画
/// 播放请求直接 302 到云盘 CDN，数据不经过 Emby 或本机。
async fn gateway_request(
    State(context): State<StrmContext>,
    request: Request<Body>,
) -> Response<Body> {
    if request
        .headers()
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    {
        let upstream = match context.state.lock() {
            Ok(guard) => websocket_url(
                &guard.virtual_library.options().emby_upstream,
                request.uri(),
            ),
            Err(error) => Err(error.to_string()),
        };
        let upstream = match upstream {
            Ok(value) => value,
            Err(error) => return response(StatusCode::BAD_GATEWAY, error),
        };
        let headers = request.headers().clone();
        let (mut parts, _) = request.into_parts();
        return match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(upgrade) => upgrade
                .on_upgrade(move |socket| relay_websocket(socket, upstream, headers))
                .into_response(),
            Err(error) => response(StatusCode::BAD_REQUEST, error.to_string()),
        };
    }
    let file_id =
        playback_file_id(&context, request.method(), request.uri(), request.headers()).await;
    if let Some(file_id) = file_id {
        let (token, device_id) = match context.state.lock() {
            Ok(guard) => match guard.token.clone() {
                Some(token) => (token, guard.device_id.clone()),
                None => return response(StatusCode::SERVICE_UNAVAILABLE, "请先登录光鸭云盘"),
            },
            Err(error) => return response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        let user_agent = request
            .headers()
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if is_browser_user_agent(user_agent) {
            return proxy_cdn_stream(&context, &token, &device_id, &file_id, request.headers())
                .await;
        }
        return match cached_res_download_url(&token, &device_id, &file_id, false).await {
            Ok(url) => Response::builder()
                .status(StatusCode::FOUND)
                .header(LOCATION, url)
                .header("cache-control", "no-store")
                .header("access-control-allow-origin", "*")
                .body(Body::empty())
                .unwrap_or_else(|_| {
                    response(StatusCode::INTERNAL_SERVER_ERROR, "构造播放重定向失败")
                }),
            Err(error) => response(StatusCode::BAD_GATEWAY, error),
        };
    }
    proxy_http(&context, request).await
}

/// `/strm/<fileId>?sign=<hmac>`：STRM 文件里的播放直链。
/// 免登录，仅校验 per-instance HMAC 签名，校验通过后 302 到云盘 CDN 直链。
async fn strm_request(
    State(context): State<StrmContext>,
    AxumPath(file_id): AxumPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response<Body> {
    let file_id = file_id.trim().to_string();
    if !valid_strm_file_id(&file_id) {
        return response(StatusCode::NOT_FOUND, "not found");
    }
    let signature = query.get("sign").map(String::as_str).unwrap_or_default();
    let (secret, token, device_id) = {
        let Ok(guard) = context.state.lock() else {
            return response(StatusCode::INTERNAL_SERVER_ERROR, "内部状态不可用");
        };
        (
            guard.strm_sign_secret.clone(),
            guard.token.clone(),
            guard.device_id.clone(),
        )
    };
    if !verify_strm_signature(&secret, &file_id, signature) {
        return response(StatusCode::FORBIDDEN, "STRM 签名无效");
    }
    let Some(token) = token else {
        return response(StatusCode::SERVICE_UNAVAILABLE, "请先登录光鸭云盘");
    };
    match cached_res_download_url(&token, &device_id, &file_id, false).await {
        Ok(url) => Response::builder()
            .status(StatusCode::FOUND)
            .header(LOCATION, url)
            .header("cache-control", "no-store")
            .header("access-control-allow-origin", "*")
            .body(Body::empty())
            .unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, "构造播放重定向失败")),
        Err(error) => response(StatusCode::BAD_GATEWAY, format!("获取云盘直链失败：{error}")),
    }
}

/// `/guangya/webhooks/emby?token=<secret>`：Emby webhook 推送入口，转交
/// Telegram 渠道解析并通知；免管理登录，仅校验 webhook secret。
async fn emby_webhook_request(
    State(context): State<StrmContext>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response<Body> {
    let Ok(db_path) = context.state.lock().map(|guard| guard.db_path.clone()) else {
        return response(StatusCode::INTERNAL_SERVER_ERROR, "内部状态不可用");
    };
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let token = query
        .get("token")
        .or_else(|| query.get("secret"))
        .map(String::as_str);
    let (status_code, payload) =
        crate::telegram::handle_emby_webhook(&db_path, token, &content_type, &body);
    Response::builder()
        .status(StatusCode::from_u16(status_code).unwrap_or(StatusCode::BAD_REQUEST))
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Body::from(payload.to_string()))
        .unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, "构造响应失败"))
}

/// 默认只监听本机；显式配置非回环直链地址（Emby 在 Docker 容器或其他
/// 设备上）时监听所有网卡，端点本身仍要求 HMAC 签名。
pub(crate) fn strm_bind_host(base_url: &str) -> &'static str {
    let host = reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
    match host.as_deref() {
        None | Some("127.0.0.1") | Some("localhost") | Some("::1") | Some("[::1]") => "127.0.0.1",
        Some(_) => "0.0.0.0",
    }
}

pub async fn serve_strm(
    app: tauri::AppHandle,
    state: SharedState,
    mut rebind: watch::Receiver<u64>,
) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            status(&app, "error", format!("创建 Emby 网关客户端失败：{error}"));
            return;
        }
    };
    loop {
        let (port, base) = match state.lock() {
            Ok(guard) => {
                let options = guard.virtual_library.options();
                (options.strm_port, options.strm_base_url)
            }
            Err(_) => return,
        };
        let bind_host = strm_bind_host(&base);
        let address = format!("{bind_host}:{port}");
        let listener = match tokio::net::TcpListener::bind(&address).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!("STRM 直链端口监听 {address} 失败：{error}");
                if let Ok(mut guard) = state.lock() {
                    guard
                        .virtual_library
                        .set_strm_status(false, Some(message.clone()));
                }
                status(&app, "error", message);
                // 等待设置变更后再重试，避免空转。
                if rebind.changed().await.is_err() {
                    return;
                }
                continue;
            }
        };
        if let Ok(mut guard) = state.lock() {
            guard.virtual_library.set_strm_status(true, None);
        }
        let scope = if bind_host == "0.0.0.0" {
            "所有网卡"
        } else {
            "仅本机"
        };
        status(
            &app,
            "success",
            format!("STRM 直链与 Emby 兼容网关已启动（{scope}）：http://{address}/"),
        );
        let router = Router::new()
            .route("/strm/{file_id}", get(strm_request))
            .route(
                "/guangya/webhooks/emby",
                axum::routing::post(emby_webhook_request)
                    .route_layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
            )
            .fallback(gateway_request)
            .with_state(StrmContext {
                state: state.clone(),
                client: client.clone(),
            });
        let mut shutdown_signal = rebind.clone();
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_signal.changed().await;
            })
            .await;
        // 消费本轮变更标记，下一轮循环按最新设置重新决定监听范围。
        let _ = rebind.borrow_and_update();
        if let Err(error) = served {
            let message = format!("STRM 直链服务异常退出：{error}");
            if let Ok(mut guard) = state.lock() {
                guard
                    .virtual_library
                    .set_strm_status(false, Some(message.clone()));
            }
            status(&app, "error", message);
            sleep(Duration::from_secs(3)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_and_metadata_classification_matches_emby_library_files() {
        assert!(is_media("Movie.2026.mkv"));
        assert!(is_media("Album.flac"));
        assert_eq!(media_output_name("Movie.2026.mkv"), "Movie.2026.strm");
        assert!(is_metadata("movie.nfo"));
        assert!(is_metadata("poster.jpg"));
        assert!(is_metadata("Movie.zh-CN.ass"));
        assert!(!is_metadata("archive.zip"));
    }

    #[test]
    fn strm_contains_a_signed_direct_link() {
        let url = strm_url("http://127.0.0.1:18096", "secret", "file-1");
        assert_eq!(
            url,
            format!(
                "http://127.0.0.1:18096/strm/file%2D1?sign={}",
                strm_signature("secret", "file-1")
            )
        );
        assert_eq!(strm_content(&url), format!("{url}\n"));
        assert!(verify_strm_signature(
            "secret",
            "file-1",
            &strm_signature("secret", "file-1")
        ));
        assert!(!verify_strm_signature(
            "secret",
            "file-1",
            &strm_signature("secret", "file-2")
        ));
        assert!(!verify_strm_signature("", "file-1", ""));
    }

    #[test]
    fn strm_base_url_normalization_accepts_prefix_paths_and_rejects_credentials() {
        assert_eq!(normalize_strm_base_url("").unwrap(), "");
        assert_eq!(
            normalize_strm_base_url("http://192.168.1.10:18096/").unwrap(),
            "http://192.168.1.10:18096"
        );
        assert_eq!(
            normalize_strm_base_url("https://nas.example.com/guangya/").unwrap(),
            "https://nas.example.com/guangya"
        );
        assert!(normalize_strm_base_url("ftp://192.168.1.10").is_err());
        assert!(normalize_strm_base_url("http://user:pass@192.168.1.10").is_err());
        assert!(normalize_strm_base_url("http://192.168.1.10?x=1").is_err());
    }

    #[test]
    fn legacy_proxy_settings_migrate_to_the_strm_service_defaults() {
        let options: VirtualLibraryOptions = serde_json::from_value(serde_json::json!({
            "proxy_port": LEGACY_REDIRECT_PORT,
            "emby_upstream": "http://127.0.0.1:8096",
            "refresh_minutes": 15,
            "mappings": []
        }))
        .expect("legacy virtual-library settings should deserialize");
        let normalized = normalize_options(options).expect("legacy settings should normalize");
        assert_eq!(normalized.strm_port, DEFAULT_STRM_PORT);
        assert_eq!(normalized.strm_base_url, "");
    }

    #[test]
    fn gateway_only_intercepts_original_stream_routes() {
        assert_eq!(
            playback_item_id("/emby/Videos/movie-id/stream.mkv").as_deref(),
            Some("movie-id")
        );
        assert_eq!(
            playback_item_id("/Audio/song-id/stream.mp3").as_deref(),
            Some("song-id")
        );
        assert_eq!(
            playback_item_id("/Videos/movie-id/original.mp4").as_deref(),
            Some("movie-id")
        );
        assert_eq!(
            playback_item_id("/Items/movie-id/File").as_deref(),
            Some("movie-id")
        );
        assert!(playback_item_id("/Videos/movie-id/master.m3u8").is_none());
        assert!(playback_item_id("/System/Info").is_none());
    }

    #[test]
    fn gateway_matches_only_own_signed_strm_media_sources() {
        let url = strm_url("http://192.168.2.223:18096", "secret", "1933793525808189509");
        let (file_id, signature) = strm_url_credentials(&url).expect("own strm url should parse");
        assert_eq!(file_id, "1933793525808189509");
        assert!(verify_strm_signature("secret", &file_id, &signature));
        assert!(!verify_strm_signature("other-secret", &file_id, &signature));

        assert!(strm_url_credentials("/visual_media/movie.mkv").is_none());
        assert!(strm_url_credentials("http://192.168.2.223:18096/strm/a/b?sign=x").is_none());
        assert!(strm_url_credentials("http://192.168.2.223:18096/other/x?sign=x").is_none());
        assert!(strm_url_credentials("http://192.168.2.223:18096/strm/id").is_none());
    }

    #[test]
    fn browser_user_agents_are_proxied_instead_of_redirected() {
        assert!(is_browser_user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
        ));
        assert!(is_browser_user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15"
        ));
        assert!(!is_browser_user_agent("Fileball/1.3.20"));
        assert!(!is_browser_user_agent("ExoPlayerLib/2.19.1"));
        assert!(!is_browser_user_agent("AppleCoreMedia/1.0.0.21F90"));
        assert!(!is_browser_user_agent(""));
    }

    #[test]
    fn emby_upstream_normalization_keeps_origin_only() {
        assert_eq!(
            normalize_emby_upstream("").unwrap(),
            "http://127.0.0.1:8096"
        );
        assert_eq!(
            normalize_emby_upstream("http://127.0.0.1:8096/").unwrap(),
            "http://127.0.0.1:8096"
        );
        assert!(normalize_emby_upstream("http://127.0.0.1:8096/emby").is_err());
        assert!(normalize_emby_upstream("ftp://127.0.0.1").is_err());
    }

    #[test]
    fn strm_bind_host_widens_only_for_non_loopback_bases() {
        assert_eq!(strm_bind_host(""), "127.0.0.1");
        assert_eq!(strm_bind_host("http://127.0.0.1:18096"), "127.0.0.1");
        assert_eq!(strm_bind_host("http://localhost:18096"), "127.0.0.1");
        assert_eq!(strm_bind_host("http://[::1]:18096"), "127.0.0.1");
        assert_eq!(strm_bind_host("http://192.168.2.223:18096"), "0.0.0.0");
        assert_eq!(strm_bind_host("http://host.docker.internal:18096"), "0.0.0.0");
        assert_eq!(strm_bind_host("https://nas.example.com/guangya"), "0.0.0.0");
    }

    #[test]
    fn strm_file_id_validation_rejects_traversal_and_separators() {
        assert!(valid_strm_file_id("file-1"));
        assert!(valid_strm_file_id("file:1"));
        assert!(!valid_strm_file_id(""));
        assert!(!valid_strm_file_id(".."));
        assert!(!valid_strm_file_id("a/b"));
        assert!(!valid_strm_file_id("a\\b"));
    }

    #[test]
    fn local_target_rejects_roots_and_relative_paths() {
        assert!(normalize_local_root("relative/library").is_err());
        #[cfg(windows)]
        assert!(normalize_local_root("C:\\").is_err());
        #[cfg(not(windows))]
        assert!(normalize_local_root("/").is_err());
    }

    #[test]
    fn overlapping_local_targets_are_rejected() {
        #[cfg(windows)]
        let root = "C:\\VirtualLibraries\\Movies";
        #[cfg(not(windows))]
        let root = "/tmp/virtual-libraries/movies";
        let mut manager = VirtualLibraryManager::new(VirtualLibraryOptions::default());
        manager
            .upsert_mapping(VirtualLibraryMapping {
                id: "movies".to_string(),
                name: "电影".to_string(),
                source_dir_id: "cloud-movies".to_string(),
                source_path: "/电影".to_string(),
                local_path: root.to_string(),
                include_metadata: false,
                enabled: true,
                emby_path: String::new(),
            })
            .expect("first mapping should be accepted");
        let error = manager
            .upsert_mapping(VirtualLibraryMapping {
                id: "nested".to_string(),
                name: "嵌套".to_string(),
                source_dir_id: "cloud-nested".to_string(),
                source_path: "/电影/嵌套".to_string(),
                local_path: Path::new(root).join("nested").to_string_lossy().to_string(),
                include_metadata: false,
                enabled: true,
                emby_path: String::new(),
            })
            .expect_err("overlapping targets must be rejected");
        assert!(error.contains("互相包含"));
    }

    #[tokio::test]
    async fn prune_empty_directories_removes_nested_empty_movie_folders() {
        let root = std::env::temp_dir().join(format!(
            "guangya-vl-prune-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let movie_dir = root.join("电影").join("阿凡达");
        tokio::fs::create_dir_all(&movie_dir)
            .await
            .expect("create nested dirs");
        tokio::fs::write(root.join("kept.strm"), "keep\n")
            .await
            .expect("write kept file");
        let removed = prune_empty_directories(&root)
            .await
            .expect("prune should succeed");
        assert!(removed.iter().any(|key| key == "电影/阿凡达" || key == "电影"));
        assert!(!movie_dir.exists());
        assert!(!root.join("电影").exists());
        assert!(root.join("kept.strm").exists());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
