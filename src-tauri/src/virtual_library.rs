use super::*;
use axum::{
    body::Body,
    extract::{ws::Message as AxumWsMessage, FromRequestParts, State, WebSocketUpgrade},
    http::{header::LOCATION, HeaderMap, Method, Request, Response, StatusCode},
    response::IntoResponse,
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

const DEFAULT_PROXY_PORT: u16 = 18_096;
const LEGACY_REDIRECT_PORT: u16 = 19_091;
const DEFAULT_EMBY_UPSTREAM: &str = "http://127.0.0.1:8096";
const DEFAULT_REFRESH_MINUTES: u64 = 15;
const MAX_REMOTE_ITEMS: usize = 100_000;
const MAX_REMOTE_DEPTH: usize = 64;
const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;
const MANIFEST_NAME: &str = ".guangya-virtual-library.json";

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
    #[serde(default)]
    pub include_metadata: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VirtualLibraryOptions {
    #[serde(default = "default_proxy_port", alias = "redirect_port")]
    pub proxy_port: u16,
    #[serde(default = "default_emby_upstream")]
    pub emby_upstream: String,
    #[serde(default = "default_refresh_minutes")]
    pub refresh_minutes: u64,
    #[serde(default)]
    pub mappings: Vec<VirtualLibraryMapping>,
}

impl Default for VirtualLibraryOptions {
    fn default() -> Self {
        Self {
            proxy_port: default_proxy_port(),
            emby_upstream: default_emby_upstream(),
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
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VirtualLibraryInfo {
    pub proxy_endpoint: String,
    pub proxy_port: u16,
    pub proxy_running: bool,
    pub proxy_error: Option<String>,
    pub emby_upstream: String,
    pub refresh_minutes: u64,
    pub mappings: Vec<VirtualLibraryMapping>,
    pub statuses: HashMap<String, VirtualLibrarySyncStatus>,
}

pub struct VirtualLibraryManager {
    options: VirtualLibraryOptions,
    statuses: HashMap<String, VirtualLibrarySyncStatus>,
    proxy_running: bool,
    proxy_error: Option<String>,
}

impl VirtualLibraryManager {
    pub fn new(options: VirtualLibraryOptions) -> Self {
        let options = normalize_options(options).unwrap_or_default();
        Self {
            options,
            statuses: HashMap::new(),
            proxy_running: false,
            proxy_error: None,
        }
    }

    pub fn options(&self) -> VirtualLibraryOptions {
        self.options.clone()
    }

    pub fn info(&self) -> VirtualLibraryInfo {
        VirtualLibraryInfo {
            proxy_endpoint: format!("http://127.0.0.1:{}/", self.options.proxy_port),
            proxy_port: self.options.proxy_port,
            proxy_running: self.proxy_running,
            proxy_error: self.proxy_error.clone(),
            emby_upstream: self.options.emby_upstream.clone(),
            refresh_minutes: self.options.refresh_minutes,
            mappings: self.options.mappings.clone(),
            statuses: self.statuses.clone(),
        }
    }

    pub fn set_refresh_minutes(&mut self, value: u64) -> Result<(), String> {
        self.options.refresh_minutes = normalize_refresh_minutes(value)?;
        Ok(())
    }

    pub fn set_emby_upstream(&mut self, value: String) -> Result<(), String> {
        self.options.emby_upstream = normalize_emby_upstream(&value)?;
        Ok(())
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
                status.error = None;
            }
            Err(error) => status.error = Some(error),
        }
    }

    pub fn set_proxy_status(&mut self, running: bool, error: Option<String>) {
        self.proxy_running = running;
        self.proxy_error = error;
    }
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub strm_files: usize,
    pub metadata_files: usize,
    pub skipped_files: usize,
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
    #[serde(default)]
    virtual_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct VirtualLibraryManifest {
    version: u32,
    source_dir_id: String,
    entries: BTreeMap<String, ManifestEntry>,
}

#[derive(Clone)]
struct ProxyContext {
    state: SharedState,
    client: reqwest::Client,
}

pub const fn default_proxy_port() -> u16 {
    DEFAULT_PROXY_PORT
}

pub fn default_emby_upstream() -> String {
    DEFAULT_EMBY_UPSTREAM.to_string()
}

pub const fn default_refresh_minutes() -> u64 {
    DEFAULT_REFRESH_MINUTES
}

fn normalize_refresh_minutes(value: u64) -> Result<u64, String> {
    if !(1..=24 * 60).contains(&value) {
        return Err("虚拟库刷新间隔必须为 1 到 1440 分钟".to_string());
    }
    Ok(value)
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

fn normalize_options(mut options: VirtualLibraryOptions) -> Result<VirtualLibraryOptions, String> {
    if options.proxy_port == 0 || options.proxy_port == LEGACY_REDIRECT_PORT {
        options.proxy_port = default_proxy_port();
    }
    if options.proxy_port == DEFAULT_WEBDAV_PORT {
        return Err("Emby 代理端口不能与 WebDAV 端口相同".to_string());
    }
    options.emby_upstream = normalize_emby_upstream(&options.emby_upstream)?;
    options.refresh_minutes = normalize_refresh_minutes(options.refresh_minutes)?;
    options.mappings = options
        .mappings
        .into_iter()
        .map(normalize_mapping)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(options)
}

fn normalize_mapping(mut mapping: VirtualLibraryMapping) -> Result<VirtualLibraryMapping, String> {
    mapping.id = mapping.id.trim().to_string();
    if mapping.id.is_empty() {
        mapping.id = Uuid::new_v4().to_string();
    }
    mapping.name = mapping.name.trim().to_string();
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

fn virtual_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        "未命名".to_string()
    } else {
        value.to_string()
    }
}

fn virtual_media_path(source_path: &str, segments: &[String], name: &str) -> String {
    let mut parts = source_path
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if !parts.starts_with('/') {
        parts.insert(0, '/');
    }
    for segment in segments
        .iter()
        .map(|value| virtual_component(value))
        .chain(std::iter::once(virtual_component(name)))
    {
        parts.push('/');
        parts.push_str(&segment);
    }
    while parts.contains("//") {
        parts = parts.replace("//", "/");
    }
    parts
}

fn strm_content(virtual_path: &str) -> String {
    format!("{}\n", virtual_path.trim())
}

fn normalized_virtual_path(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized.trim_end_matches('/').to_lowercase()
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

async fn download_url(token: &str, device_id: &str, file_id: &str) -> Result<String, String> {
    let response = api_post(
        token,
        device_id,
        "/userres/v1/get_res_download_url",
        json!({ "fileId": file_id }),
        &[],
    )
    .await?;
    response
        .data
        .as_ref()
        .and_then(|data| {
            data.get("downloadUrl")
                .or_else(|| data.get("downloadURL"))
                .or_else(|| data.get("signedURL"))
                .or_else(|| data.get("signedUrl"))
                .or_else(|| data.get("url"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "光鸭没有返回文件下载地址".to_string())
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
    let url = download_url(token, device_id, &entry.id).await?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载元数据失败（{}）：{error}", entry.name))?;
    if !response.status().is_success() {
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
) -> Result<(), String> {
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
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "清理过期虚拟文件失败（{}）：{error}",
                    target.display()
                ))
            }
        }
    }
    Ok(())
}

pub async fn sync_mapping(
    state: &SharedState,
    mapping: &VirtualLibraryMapping,
) -> Result<SyncSummary, String> {
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
        version: 2,
        source_dir_id: mapping.source_dir_id.clone(),
        entries: BTreeMap::new(),
    };
    let mut pending = VecDeque::from([(
        mapping.source_dir_id.clone(),
        PathBuf::new(),
        Vec::<String>::new(),
        0_usize,
    )]);
    let mut seen_outputs = HashSet::new();
    let mut scanned = 0_usize;
    let mut summary = SyncSummary {
        strm_files: 0,
        metadata_files: 0,
        skipped_files: 0,
    };

    while let Some((parent_id, relative_dir, virtual_segments, depth)) = pending.pop_front() {
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
                let mut child_virtual_segments = virtual_segments.clone();
                child_virtual_segments.push(entry.name);
                pending.push_back((entry.id, child_relative, child_virtual_segments, depth + 1));
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
            let virtual_path = if kind == "strm" {
                virtual_media_path(&mapping.source_path, &virtual_segments, &entry.name)
            } else {
                String::new()
            };
            let manifest_entry = ManifestEntry {
                source_id: entry.id.clone(),
                size: entry.size,
                modified_ms: entry.modified_ms,
                kind: kind.to_string(),
                virtual_path: virtual_path.clone(),
            };
            let unchanged = previous.entries.get(&key).is_some_and(|previous| {
                previous.source_id == manifest_entry.source_id
                    && previous.size == manifest_entry.size
                    && previous.modified_ms == manifest_entry.modified_ms
                    && previous.kind == manifest_entry.kind
                    && previous.virtual_path == manifest_entry.virtual_path
                    && target.is_file()
            });
            if kind == "strm" {
                let content = strm_content(&virtual_path);
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
                }
                summary.strm_files += 1;
            } else {
                if !unchanged {
                    download_metadata(&client, &token, &device_id, &entry, &target).await?;
                }
                summary.metadata_files += 1;
            }
            next.entries.insert(key, manifest_entry);
        }
    }

    remove_stale_files(&root, &previous, &next).await?;
    save_manifest(&root, &next).await?;
    Ok(summary)
}

fn response(status: StatusCode, body: impl Into<Body>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .header("access-control-allow-origin", "*")
        .body(body.into())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

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

async fn source_id_for_virtual_path(
    mappings: &[VirtualLibraryMapping],
    value: &str,
) -> Option<String> {
    let expected = normalized_virtual_path(value);
    if expected.is_empty() {
        return None;
    }
    for mapping in mappings.iter().filter(|mapping| mapping.enabled) {
        let manifest = load_manifest(Path::new(&mapping.local_path)).await;
        if let Some(entry) = manifest.entries.values().find(|entry| {
            entry.kind == "strm" && normalized_virtual_path(&entry.virtual_path) == expected
        }) {
            return Some(entry.source_id.clone());
        }
    }
    None
}

fn emby_request_url(upstream: &str, uri: &axum::http::Uri) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(upstream).map_err(|error| error.to_string())?;
    url.set_path(uri.path());
    url.set_query(uri.query());
    Ok(url)
}

async fn playback_source_id(
    context: &ProxyContext,
    method: &Method,
    uri: &axum::http::Uri,
    headers: &HeaderMap,
) -> Option<String> {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return None;
    }
    let item_id = playback_item_id(uri.path())?;
    let (upstream, mappings) = {
        let guard = context.state.lock().ok()?;
        (
            guard.virtual_library.options().emby_upstream,
            guard.virtual_library.options().mappings,
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
        if let Some(source_id) = source_id_for_virtual_path(&mappings, path).await {
            return Some(source_id);
        }
    }
    None
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

async fn proxy_http(context: &ProxyContext, request: Request<Body>) -> Response<Body> {
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
        .unwrap_or_else(|_| response(StatusCode::BAD_GATEWAY, "构造 Emby 代理响应失败"))
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

async fn proxy_request(
    State(context): State<ProxyContext>,
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
    let source_id =
        playback_source_id(&context, request.method(), request.uri(), request.headers()).await;
    if let Some(source_id) = source_id {
        let (token, device_id) = match context.state.lock() {
            Ok(guard) => match guard.token.clone() {
                Some(token) => (token, guard.device_id.clone()),
                None => return response(StatusCode::SERVICE_UNAVAILABLE, "请先登录光鸭云盘"),
            },
            Err(error) => return response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        };
        return match download_url(&token, &device_id, &source_id).await {
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

pub async fn serve_proxy(app: tauri::AppHandle, state: SharedState, port: u16) {
    let address = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format!("Emby 代理端口监听 {address} 失败：{error}");
            if let Ok(mut guard) = state.lock() {
                guard
                    .virtual_library
                    .set_proxy_status(false, Some(message.clone()));
            }
            status(&app, "error", message);
            return;
        }
    };
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            status(&app, "error", format!("创建 Emby 代理客户端失败：{error}"));
            return;
        }
    };
    if let Ok(mut guard) = state.lock() {
        guard.virtual_library.set_proxy_status(true, None);
    }
    status(
        &app,
        "success",
        format!("虚拟库 Emby 代理已启动：http://{address}/"),
    );
    let router = Router::new()
        .fallback(proxy_request)
        .with_state(ProxyContext {
            state: state.clone(),
            client,
        });
    if let Err(error) = axum::serve(listener, router).await {
        let message = format!("虚拟库播放重定向服务异常退出：{error}");
        if let Ok(mut guard) = state.lock() {
            guard
                .virtual_library
                .set_proxy_status(false, Some(message.clone()));
        }
        status(&app, "error", message);
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
    fn strm_contains_only_the_cloud_virtual_path() {
        let path = virtual_media_path("/电影", &["子目录".to_string()], "示例 电影.mkv");
        assert_eq!(strm_content(&path), "/电影/子目录/示例 电影.mkv\n");
        assert!(!strm_content(&path).contains("http"));
    }

    #[test]
    fn legacy_redirect_settings_migrate_to_the_emby_proxy_defaults() {
        let options: VirtualLibraryOptions = serde_json::from_value(serde_json::json!({
            "redirect_port": LEGACY_REDIRECT_PORT,
            "refresh_minutes": 15,
            "mappings": []
        }))
        .expect("legacy virtual-library settings should deserialize");
        let normalized = normalize_options(options).expect("legacy settings should normalize");
        assert_eq!(normalized.proxy_port, DEFAULT_PROXY_PORT);
        assert_eq!(normalized.emby_upstream, DEFAULT_EMBY_UPSTREAM);
    }

    #[test]
    fn emby_proxy_only_classifies_original_stream_routes() {
        assert_eq!(
            playback_item_id("/emby/Videos/movie-id/stream.mkv").as_deref(),
            Some("movie-id")
        );
        assert_eq!(
            playback_item_id("/Audio/song-id/stream.mp3").as_deref(),
            Some("song-id")
        );
        assert_eq!(
            playback_item_id("/Items/movie-id/File").as_deref(),
            Some("movie-id")
        );
        assert!(playback_item_id("/Videos/movie-id/master.m3u8").is_none());
        assert!(playback_item_id("/System/Info").is_none());
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
            })
            .expect_err("overlapping targets must be rejected");
        assert!(error.contains("互相包含"));
    }
}
