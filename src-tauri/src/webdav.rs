use super::*;
use axum::{
    body::Body,
    extract::State,
    http::{
        header::{
            ACCEPT_RANGES, ALLOW, AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION,
            CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_MATCH, IF_MODIFIED_SINCE,
            IF_NONE_MATCH, IF_UNMODIFIED_SINCE, LAST_MODIFIED, PRAGMA, RANGE, WWW_AUTHENTICATE,
        },
        HeaderMap, Method, Request, Response, StatusCode,
    },
    Router,
};
use percent_encoding::percent_decode_str;
use std::time::{SystemTime, UNIX_EPOCH};

// Native mounts already cache directory listings. A short server-side cache
// prevents changes made through another WebDAV/native mount from remaining
// invisible for minutes because two independent caches are stacked.
const DIRECTORY_CACHE_FRESH_SECS: u64 = 2;
const DIRECTORY_CACHE_STALE_SECS: u64 = 15;
const DIRECTORY_CACHE_MAX_ENTRIES: usize = 2_048;

#[derive(Clone)]
struct WebDavContext {
    app: tauri::AppHandle,
    state: SharedState,
    directory_cache: Arc<DirectoryCache>,
}

#[derive(Debug)]
struct DavError {
    status: StatusCode,
    message: String,
}

impl DavError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<String> for DavError {
    fn from(message: String) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }
}

type DavResult<T> = Result<T, DavError>;

#[derive(Debug, Clone)]
struct RemoteEntry {
    id: String,
    parent_id: String,
    name: String,
    is_directory: bool,
    size: u64,
    modified_ms: u64,
}

#[derive(Clone)]
struct CachedDirectory {
    entries: Vec<RemoteEntry>,
    fetched_at: Instant,
    accessed_at: Instant,
}

#[derive(Default)]
struct DirectoryCacheState {
    entries: HashMap<String, CachedDirectory>,
    generations: HashMap<String, u64>,
    scope: Option<Vec<u8>>,
}

#[derive(Default)]
struct DirectoryCache {
    state: Mutex<DirectoryCacheState>,
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl DirectoryCache {
    fn snapshot(&self, parent_id: &str) -> Option<(Vec<RemoteEntry>, Duration)> {
        let mut state = self.state.lock().ok()?;
        let cached = state.entries.get_mut(parent_id)?;
        cached.accessed_at = Instant::now();
        Some((cached.entries.clone(), cached.fetched_at.elapsed()))
    }

    fn generation(&self, parent_id: &str) -> u64 {
        let Ok(mut state) = self.state.lock() else {
            return 0;
        };
        *state.generations.entry(parent_id.to_string()).or_insert(0)
    }

    fn ensure_scope(&self, scope: &[u8]) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.scope.as_deref() == Some(scope) {
            return;
        }
        let keys = state
            .entries
            .keys()
            .chain(state.generations.keys())
            .cloned()
            .collect::<HashSet<_>>();
        state.entries.clear();
        for key in keys {
            let next = state
                .generations
                .get(&key)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            state.generations.insert(key, next);
        }
        state.scope = Some(scope.to_vec());
    }

    fn put_if_current(&self, parent_id: &str, generation: u64, entries: Vec<RemoteEntry>) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.generations.get(parent_id).copied().unwrap_or(0) != generation {
            return false;
        }

        let changed_child_directories = state
            .entries
            .get(parent_id)
            .map(|cached| changed_directory_ids(&cached.entries, &entries))
            .unwrap_or_default();
        for child_id in changed_child_directories {
            state.entries.remove(&child_id);
            let next = state
                .generations
                .get(&child_id)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            state.generations.insert(child_id, next);
        }

        let now = Instant::now();
        state.entries.insert(
            parent_id.to_string(),
            CachedDirectory {
                entries,
                fetched_at: now,
                accessed_at: now,
            },
        );
        while state.entries.len() > DIRECTORY_CACHE_MAX_ENTRIES {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, cached)| cached.accessed_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            state.entries.remove(&oldest);
        }
        true
    }

    fn invalidate(&self, parent_id: &str) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.entries.remove(parent_id);
        let next = state
            .generations
            .get(parent_id)
            .copied()
            .unwrap_or(0)
            .wrapping_add(1);
        state.generations.insert(parent_id.to_string(), next);
    }

    fn invalidate_subtree(&self, root_id: &str) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let mut pending = VecDeque::from([root_id.to_string()]);
        while let Some(parent_id) = pending.pop_front() {
            if let Some(cached) = state.entries.remove(&parent_id) {
                pending.extend(
                    cached
                        .entries
                        .into_iter()
                        .filter(|entry| entry.is_directory)
                        .map(|entry| entry.id),
                );
            }
            let next = state
                .generations
                .get(&parent_id)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            state.generations.insert(parent_id, next);
        }
    }

    fn invalidate_entries(&self, entry_ids: &[String]) -> (Vec<String>, bool) {
        let entry_ids = entry_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let (parents, matched) = self
            .state
            .lock()
            .ok()
            .map(|state| {
                let mut parents = HashSet::new();
                let mut matched = HashSet::new();
                for (parent_id, cached) in &state.entries {
                    for entry in &cached.entries {
                        if entry_ids.contains(entry.id.as_str()) {
                            parents.insert(parent_id.clone());
                            matched.insert(entry.id.as_str());
                        }
                    }
                }
                (parents, matched.len())
            })
            .unwrap_or_default();
        for parent_id in &parents {
            self.invalidate(parent_id);
        }
        for entry_id in &entry_ids {
            self.invalidate_subtree(entry_id);
        }
        (parents.into_iter().collect(), matched == entry_ids.len())
    }

    fn clear(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let keys = state
            .entries
            .keys()
            .chain(state.generations.keys())
            .cloned()
            .collect::<HashSet<_>>();
        state.entries.clear();
        for key in keys {
            let next = state
                .generations
                .get(&key)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            state.generations.insert(key, next);
        }
    }

    fn gate(&self, parent_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let Ok(mut gates) = self.gates.lock() else {
            return Arc::new(tokio::sync::Mutex::new(()));
        };
        if gates.len() > DIRECTORY_CACHE_MAX_ENTRIES * 2 {
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        }
        gates
            .entry(parent_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

fn shared_directory_cache() -> Arc<DirectoryCache> {
    static CACHE: OnceLock<Arc<DirectoryCache>> = OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(DirectoryCache::default()))
        .clone()
}

pub(crate) fn invalidate_directory_cache(parent_id: &str) {
    shared_directory_cache().invalidate(parent_id);
}

pub(crate) fn invalidate_directory_cache_entries(entry_ids: &[String]) -> (Vec<String>, bool) {
    shared_directory_cache().invalidate_entries(entry_ids)
}

pub(crate) fn invalidate_all_directory_cache() {
    shared_directory_cache().clear();
}

pub(crate) fn publish_directory_invalidation(
    app: &tauri::AppHandle,
    parent_ids: impl IntoIterator<Item = String>,
    all: bool,
    source: &str,
) {
    let mut parent_ids = parent_ids
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    parent_ids.sort();
    emit(
        app,
        json!({
            "type": "cloud-directory-invalidated",
            "parent_ids": parent_ids,
            "all": all,
            "source": source,
        }),
    );
}

fn changed_directory_ids(before: &[RemoteEntry], after: &[RemoteEntry]) -> Vec<String> {
    let next = after
        .iter()
        .filter(|entry| entry.is_directory)
        .map(|entry| {
            (
                entry.id.as_str(),
                (entry.name.as_str(), entry.modified_ms, entry.size),
            )
        })
        .collect::<HashMap<_, _>>();
    before
        .iter()
        .filter(|entry| entry.is_directory)
        .filter(|entry| {
            next.get(entry.id.as_str()).is_none_or(|fingerprint| {
                *fingerprint != (entry.name.as_str(), entry.modified_ms, entry.size)
            })
        })
        .map(|entry| entry.id.clone())
        .collect()
}

impl RemoteEntry {
    fn root() -> Self {
        Self {
            id: String::new(),
            parent_id: String::new(),
            name: "光鸭云盘".to_string(),
            is_directory: true,
            size: 0,
            modified_ms: now_ms(),
        }
    }

    fn etag(&self) -> String {
        format!(
            "\"gy-{}-{}-{}\"",
            if self.id.is_empty() { "root" } else { &self.id },
            self.modified_ms,
            self.size
        )
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn raw_timestamp_ms(value: &Value) -> u64 {
    for key in [
        "updatedAt",
        "updateTime",
        "modifiedAt",
        "modifyTime",
        "utime",
        "createdAt",
        "createTime",
        "ctime",
    ] {
        let Some(candidate) = value.get(key) else {
            continue;
        };
        let number = candidate
            .as_u64()
            .or_else(|| candidate.as_str().and_then(|item| item.parse().ok()))
            .unwrap_or(0);
        if number > 0 {
            return if number < 10_000_000_000 {
                number * 1000
            } else {
                number
            };
        }
    }
    now_ms()
}

fn normalize_entry(value: Value, parent_id: &str) -> Option<RemoteEntry> {
    let id = value
        .get("fileId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = value
        .get("fileName")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if id.is_empty() || name.is_empty() {
        return None;
    }
    let is_directory = value
        .get("resType")
        .or_else(|| value.get("type"))
        .and_then(Value::as_u64)
        .is_some_and(|item| item == 2)
        || value
            .get("isDirectory")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let size = value
        .get("fileSize")
        .or_else(|| value.get("size"))
        .and_then(|item| {
            item.as_u64()
                .or_else(|| item.as_str().and_then(|value| value.parse().ok()))
        })
        .unwrap_or(0);
    let modified_ms = raw_timestamp_ms(&value);
    Some(RemoteEntry {
        id,
        parent_id: parent_id.to_string(),
        name,
        is_directory,
        size,
        modified_ms,
    })
}

fn auth_context(state: &SharedState) -> DavResult<(String, String)> {
    let guard = state
        .lock()
        .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok((
        guard
            .token
            .clone()
            .ok_or_else(|| DavError::new(StatusCode::SERVICE_UNAVAILABLE, "请先登录光鸭云盘"))?,
        guard.device_id.clone(),
    ))
}

async fn fetch_children(context: &WebDavContext, parent_id: &str) -> DavResult<Vec<RemoteEntry>> {
    let (token, device_id) = auth_context(&context.state)?;
    let mut records = Vec::new();
    for page in 0..1000u64 {
        let response = api_post(
            &token,
            &device_id,
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
        .await
        .map_err(DavError::from)?;
        let data = response.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_count = list.len();
        records.extend(
            list.into_iter()
                .filter_map(|value| normalize_entry(value, parent_id)),
        );
        let total = data
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or(records.len() as u64);
        if page_count == 0 || records.len() as u64 >= total {
            break;
        }
    }
    Ok(records)
}

async fn list_children(
    context: &WebDavContext,
    parent_id: &str,
    force_refresh: bool,
) -> DavResult<Vec<RemoteEntry>> {
    let (token, _) = auth_context(&context.state)?;
    context
        .directory_cache
        .ensure_scope(Sha256::digest(token.as_bytes()).as_slice());
    let cached = context.directory_cache.snapshot(parent_id);
    if !force_refresh {
        if let Some((entries, age)) = &cached {
            if *age <= Duration::from_secs(DIRECTORY_CACHE_FRESH_SECS) {
                return Ok(entries.clone());
            }
        }

        let gate = context.directory_cache.gate(parent_id);
        if let Some((entries, age)) = cached {
            if age <= Duration::from_secs(DIRECTORY_CACHE_STALE_SECS) {
                if let Ok(guard) = gate.clone().try_lock_owned() {
                    let context = context.clone();
                    let parent_id = parent_id.to_string();
                    tokio::spawn(async move {
                        let _guard = guard;
                        let generation = context.directory_cache.generation(&parent_id);
                        if let Ok(entries) = fetch_children(&context, &parent_id).await {
                            context
                                .directory_cache
                                .put_if_current(&parent_id, generation, entries);
                        }
                    });
                }
                return Ok(entries);
            }
        }
    }

    let gate = context.directory_cache.gate(parent_id);
    let _guard = gate.lock().await;
    if !force_refresh {
        if let Some((entries, age)) = context.directory_cache.snapshot(parent_id) {
            if age <= Duration::from_secs(DIRECTORY_CACHE_FRESH_SECS) {
                return Ok(entries);
            }
        }
    }
    for _ in 0..3 {
        let generation = context.directory_cache.generation(parent_id);
        match fetch_children(context, parent_id).await {
            Ok(entries)
                if context.directory_cache.put_if_current(
                    parent_id,
                    generation,
                    entries.clone(),
                ) =>
            {
                return Ok(entries);
            }
            Ok(_) => continue,
            Err(error) if force_refresh => return Err(error),
            Err(error) => {
                return context
                    .directory_cache
                    .snapshot(parent_id)
                    .map(|(entries, _)| entries)
                    .ok_or(error);
            }
        }
    }
    Err(DavError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "云端目录正在变化，请重试当前操作",
    ))
}

async fn find_child(
    context: &WebDavContext,
    parent_id: &str,
    name: &str,
    force_refresh: bool,
) -> DavResult<Option<RemoteEntry>> {
    let children = list_children(context, parent_id, force_refresh).await?;
    if let Some(exact) = children.iter().find(|entry| entry.name == name) {
        return Ok(Some(exact.clone()));
    }
    let folded = children
        .into_iter()
        .filter(|entry| entry.name.to_lowercase() == name.to_lowercase())
        .collect::<Vec<_>>();
    Ok(if folded.len() == 1 {
        folded.into_iter().next()
    } else {
        None
    })
}

async fn resolve_entry(
    context: &WebDavContext,
    segments: &[String],
    force_refresh: bool,
) -> DavResult<RemoteEntry> {
    if segments.is_empty() {
        return Ok(RemoteEntry::root());
    }
    let mut parent_id = String::new();
    let mut entry = None;
    for (index, segment) in segments.iter().enumerate() {
        let current = find_child(context, &parent_id, segment, force_refresh)
            .await?
            .ok_or_else(|| {
                DavError::new(StatusCode::NOT_FOUND, format!("云端项目不存在：{segment}"))
            })?;
        if index + 1 < segments.len() && !current.is_directory {
            return Err(DavError::new(
                StatusCode::CONFLICT,
                format!("路径中包含文件：{segment}"),
            ));
        }
        parent_id = current.id.clone();
        entry = Some(current);
    }
    Ok(entry.unwrap_or_else(RemoteEntry::root))
}

async fn resolve_parent(
    context: &WebDavContext,
    segments: &[String],
    force_refresh: bool,
) -> DavResult<(String, String, Option<RemoteEntry>)> {
    if segments.is_empty() {
        return Err(DavError::new(
            StatusCode::FORBIDDEN,
            "不能修改 WebDAV 根目录",
        ));
    }
    let name = segments.last().cloned().unwrap_or_default();
    let parent_id = if segments.len() == 1 {
        String::new()
    } else {
        let parent = resolve_entry(context, &segments[..segments.len() - 1], force_refresh).await?;
        if !parent.is_directory {
            return Err(DavError::new(StatusCode::CONFLICT, "目标父路径不是目录"));
        }
        parent.id
    };
    let existing = find_child(context, &parent_id, &name, force_refresh).await?;
    Ok((parent_id, name, existing))
}

fn decode_path(path: &str) -> DavResult<Vec<String>> {
    let relative = path.trim_matches('/');
    if relative.is_empty() {
        return Ok(Vec::new());
    }
    relative
        .split('/')
        .map(|part| {
            let decoded = percent_decode_str(part)
                .decode_utf8()
                .map_err(|_| DavError::new(StatusCode::BAD_REQUEST, "WebDAV 路径编码无效"))?
                .into_owned();
            if decoded.is_empty()
                || decoded == "."
                || decoded == ".."
                || decoded.contains('/')
                || decoded.contains('\\')
                || decoded.contains('\0')
            {
                return Err(DavError::new(StatusCode::BAD_REQUEST, "WebDAV 路径无效"));
            }
            Ok(decoded)
        })
        .collect()
}

fn destination_path(headers: &HeaderMap) -> DavResult<Vec<String>> {
    let value = headers
        .get("destination")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| DavError::new(StatusCode::BAD_REQUEST, "缺少 Destination 请求头"))?;
    let path = if value.starts_with('/') {
        value.split(['?', '#']).next().unwrap_or(value).to_string()
    } else {
        reqwest::Url::parse(value)
            .map_err(|_| DavError::new(StatusCode::BAD_REQUEST, "Destination 地址无效"))?
            .path()
            .to_string()
    };
    decode_path(&path)
}

fn request_forces_directory_refresh(headers: &HeaderMap) -> bool {
    [CACHE_CONTROL, PRAGMA].iter().any(|name| {
        headers.get_all(name).iter().any(|value| {
            value.to_str().ok().is_some_and(|value| {
                let normalized = value.to_ascii_lowercase().replace([' ', '\t'], "");
                normalized
                    .split(',')
                    .any(|directive| matches!(directive, "no-cache" | "no-store" | "max-age=0"))
            })
        })
    })
}

fn authenticated(headers: &HeaderMap, state: &SharedState) -> bool {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic "));
    let Some(encoded) = authorization else {
        return false;
    };
    let Ok(decoded) = BASE64_STANDARD.decode(encoded) else {
        return false;
    };
    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };
    let Some((username, password)) = decoded.split_once(':') else {
        return false;
    };
    state
        .lock()
        .ok()
        .is_some_and(|guard| username == guard.webdav_username && password == guard.webdav_password)
}

fn plain_response(status: StatusCode, body: impl Into<String>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .body(Body::from(body.into()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("cache-control", "no-store")
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn encoded_href(segments: &[String], directory: bool) -> String {
    let suffix = segments
        .iter()
        .map(|segment| {
            percent_encoding::utf8_percent_encode(segment, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    let mut href = if suffix.is_empty() {
        "/".to_string()
    } else {
        format!("/{suffix}")
    };
    if directory && !href.ends_with('/') {
        href.push('/');
    }
    href
}

fn content_type(entry: &RemoteEntry) -> &'static str {
    if entry.is_directory {
        return "httpd/unix-directory";
    }
    match Path::new(&entry.name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn http_date(timestamp_ms: u64) -> String {
    httpdate::fmt_http_date(UNIX_EPOCH + std::time::Duration::from_millis(timestamp_ms))
}

fn property_response(segments: &[String], entry: &RemoteEntry) -> String {
    let resource_type = if entry.is_directory {
        "<D:collection/>"
    } else {
        ""
    };
    let length = if entry.is_directory {
        String::new()
    } else {
        format!("<D:getcontentlength>{}</D:getcontentlength>", entry.size)
    };
    format!(
        r#"<D:response>
<D:href>{}</D:href>
<D:propstat><D:prop>
<D:displayname>{}</D:displayname>
<D:resourcetype>{}</D:resourcetype>
{}
<D:getcontenttype>{}</D:getcontenttype>
<D:getlastmodified>{}</D:getlastmodified>
<D:creationdate>{}</D:creationdate>
<D:getetag>{}</D:getetag>
<D:supportedlock><D:lockentry><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockentry></D:supportedlock>
</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>
</D:response>"#,
        xml_escape(&encoded_href(segments, entry.is_directory)),
        xml_escape(&entry.name),
        resource_type,
        length,
        content_type(entry),
        http_date(entry.modified_ms),
        xml_escape(&http_date(entry.modified_ms)),
        xml_escape(&entry.etag())
    )
}

fn directory_index(segments: &[String], entry: &RemoteEntry, children: &[RemoteEntry]) -> String {
    let parent = if segments.is_empty() {
        String::new()
    } else {
        format!(
            r#"<li><a href="{}">../</a></li>"#,
            xml_escape(&encoded_href(&segments[..segments.len() - 1], true))
        )
    };
    let mut children = children.to_vec();
    children.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let items = children
        .into_iter()
        .map(|child| {
            let mut child_segments = segments.to_vec();
            child_segments.push(child.name.clone());
            format!(
                r#"<li><a href="{}">{}{}</a></li>"#,
                xml_escape(&encoded_href(&child_segments, child.is_directory)),
                xml_escape(&child.name),
                if child.is_directory { "/" } else { "" }
            )
        })
        .collect::<String>();
    let title = if entry.name.is_empty() {
        "光鸭云盘"
    } else {
        &entry.name
    };
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>{0}</title></head>
<body><h1>{0}</h1><ul>{1}{2}</ul></body>
</html>"#,
        xml_escape(title),
        parent,
        items
    )
}

async fn create_directory(context: &WebDavContext, parent_id: &str, name: &str) -> DavResult<()> {
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/create_dir",
        json!({ "parentId": parent_id, "dirName": name, "failIfNameExist": true }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map_err(DavError::from)?;
    context.directory_cache.invalidate(parent_id);
    Ok(())
}

async fn delete_entry(context: &WebDavContext, entry: &RemoteEntry) -> DavResult<()> {
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": [entry.id.clone()] }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id)
            .await
            .map_err(DavError::from)?;
    }
    context.directory_cache.invalidate(&entry.parent_id);
    if entry.is_directory {
        context.directory_cache.invalidate_subtree(&entry.id);
    }
    Ok(())
}

async fn rename_entry(context: &WebDavContext, entry_id: &str, new_name: &str) -> DavResult<()> {
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/rename",
        json!({ "fileId": entry_id, "newName": new_name }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map_err(DavError::from)?;
    Ok(())
}

async fn move_entry_to_parent(
    context: &WebDavContext,
    entry_id: &str,
    source_parent_id: &str,
    parent_id: &str,
) -> DavResult<()> {
    if source_parent_id == parent_id {
        return Ok(());
    }
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/move_file",
        json!({ "fileIds": [entry_id], "parentId": parent_id }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id)
            .await
            .map_err(DavError::from)?;
    }
    context.directory_cache.invalidate(source_parent_id);
    context.directory_cache.invalidate(parent_id);
    Ok(())
}

async fn move_entry(
    context: &WebDavContext,
    entry: &RemoteEntry,
    parent_id: &str,
    name: &str,
) -> DavResult<()> {
    let moved = entry.parent_id != parent_id;
    move_entry_to_parent(context, &entry.id, &entry.parent_id, parent_id).await?;
    if entry.name != name {
        if let Err(error) = rename_entry(context, &entry.id, name).await {
            if moved {
                if let Err(rollback_error) =
                    move_entry_to_parent(context, &entry.id, parent_id, &entry.parent_id).await
                {
                    return Err(DavError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "{}；恢复资源原目录也失败：{}",
                            error.message, rollback_error.message
                        ),
                    ));
                }
            }
            return Err(error);
        }
        context.directory_cache.invalidate(parent_id);
    }
    Ok(())
}

async fn copy_entry(
    context: &WebDavContext,
    entry: &RemoteEntry,
    parent_id: &str,
    name: &str,
) -> DavResult<()> {
    let before = list_children(context, parent_id, true).await?;
    if entry.name != name && before.iter().any(|item| item.name == entry.name) {
        return Err(DavError::new(
            StatusCode::CONFLICT,
            format!("目标目录中已有 {}，无法安全完成改名复制", entry.name),
        ));
    }
    let before_ids = before
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/copy_file",
        json!({ "fileIds": [entry.id.clone()], "parentId": parent_id }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    if let Some(task_id) = response
        .data
        .as_ref()
        .and_then(|data| data.get("taskId"))
        .and_then(Value::as_str)
    {
        wait_operation_task(&token, &device_id, task_id)
            .await
            .map_err(DavError::from)?;
    }
    context.directory_cache.invalidate(parent_id);
    if entry.name == name {
        return Ok(());
    }
    let copied = list_children(context, parent_id, true)
        .await?
        .into_iter()
        .find(|item| item.name == entry.name && !before_ids.contains(&item.id))
        .ok_or_else(|| {
            DavError::new(
                StatusCode::CONFLICT,
                "云端复制已完成，但无法定位副本进行重命名",
            )
        })?;
    if let Err(error) = rename_entry(context, &copied.id, name).await {
        let _ = delete_entry(context, &copied).await;
        return Err(error);
    }
    context.directory_cache.invalidate(parent_id);
    Ok(())
}

async fn put_file(
    context: &WebDavContext,
    request: Request<Body>,
    parent_id: &str,
    name: &str,
    existing: Option<&RemoteEntry>,
) -> DavResult<RemoteEntry> {
    let temporary_root = {
        let guard = context
            .state
            .lock()
            .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        guard
            .db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("webdav-uploads")
            .join(Uuid::new_v4().to_string())
    };
    let temporary_name = if existing.is_some() {
        format!(".__gy_dav_{}", Uuid::new_v4().simple())
    } else {
        name.to_string()
    };
    let temporary_file = temporary_root.join(&temporary_name);
    tokio::fs::create_dir_all(&temporary_root)
        .await
        .map_err(|error| {
            DavError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建 WebDAV 临时目录失败：{error}"),
            )
        })?;
    let result = async {
        let mut output = tokio::fs::File::create(&temporary_file)
            .await
            .map_err(|error| {
                DavError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建 WebDAV 临时文件失败：{error}"),
                )
            })?;
        let mut stream = request.into_body().into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                DavError::new(
                    StatusCode::BAD_REQUEST,
                    format!("读取 WebDAV 上传内容失败：{error}"),
                )
            })?;
            output.write_all(&chunk).await.map_err(|error| {
                DavError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("写入 WebDAV 临时文件失败：{error}"),
                )
            })?;
        }
        output.flush().await.map_err(|error| {
            DavError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("保存 WebDAV 临时文件失败：{error}"),
            )
        })?;
        drop(output);
        let metadata = tokio::fs::metadata(&temporary_file)
            .await
            .map_err(|error| {
                DavError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取 WebDAV 临时文件失败：{error}"),
                )
            })?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis())
            .unwrap_or(0);
        let item = UploadItem {
            mapping_id: "__manual__".to_string(),
            file_path: temporary_file.clone(),
            remote_parent_id: parent_id.to_string(),
            remote_dir: String::new(),
            relative_path: name.to_string(),
            change_kind: if existing.is_some() {
                "changed".to_string()
            } else {
                "added".to_string()
            },
            size: metadata.len(),
            modified_ms,
            replacement: None,
        };
        let uploaded = upload_item(&context.app, &context.state, &item)
            .await
            .map_err(|error| DavError::new(StatusCode::BAD_GATEWAY, error))?;
        let remote_id = uploaded.remote_file_id.ok_or_else(|| {
            DavError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "文件已上传，但云端暂未确认入库",
            )
        })?;
        let uploaded_entry = RemoteEntry {
            id: remote_id.clone(),
            parent_id: parent_id.to_string(),
            name: temporary_name.clone(),
            is_directory: false,
            size: metadata.len(),
            modified_ms: modified_ms as u64,
        };
        let backup = if let Some(existing) = existing {
            let backup = RemoteEntry {
                name: format!(".__gy_dav_backup_{}", Uuid::new_v4().simple()),
                parent_id: parent_id.to_string(),
                ..existing.clone()
            };
            if let Err(error) = move_entry(context, existing, parent_id, &backup.name).await {
                let _ = delete_entry(context, &uploaded_entry).await;
                return Err(error);
            }
            Some(backup)
        } else {
            None
        };
        if temporary_name != name {
            if let Err(error) = rename_entry(context, &remote_id, name).await {
                let rollback_error = if let Some(backup) = backup.as_ref() {
                    move_entry(context, backup, parent_id, name).await.err()
                } else {
                    None
                };
                let _ = delete_entry(context, &uploaded_entry).await;
                if let Some(rollback_error) = rollback_error {
                    return Err(DavError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "{}；恢复被覆盖文件也失败：{}",
                            error.message, rollback_error.message
                        ),
                    ));
                }
                return Err(error);
            }
        }
        if let Some(backup) = backup.as_ref() {
            delete_entry(context, backup).await?;
        }
        context.directory_cache.invalidate(parent_id);
        Ok(RemoteEntry {
            id: remote_id,
            parent_id: parent_id.to_string(),
            name: name.to_string(),
            is_directory: false,
            size: metadata.len(),
            modified_ms: modified_ms as u64,
        })
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&temporary_root).await;
    result
}

fn webdav_etag_matches(value: &str, etag: &str) -> bool {
    let expected = etag.strip_prefix("W/").unwrap_or(etag);
    value.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == expected
    })
}

fn webdav_condition_response(status: StatusCode, entry: &RemoteEntry) -> DavResult<Response<Body>> {
    Response::builder()
        .status(status)
        .header(ETAG, entry.etag())
        .header(LAST_MODIFIED, http_date(entry.modified_ms))
        .body(Body::empty())
        .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn request_http_date(headers: &HeaderMap, name: &axum::http::HeaderName) -> Option<u64> {
    let value = headers.get(name)?.to_str().ok()?;
    httpdate::parse_http_date(value)
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

async fn read_file(
    context: &WebDavContext,
    request_headers: &HeaderMap,
    entry: &RemoteEntry,
    head_only: bool,
) -> DavResult<Response<Body>> {
    let etag = entry.etag();
    if let Some(value) = request_headers
        .get(IF_MATCH)
        .and_then(|value| value.to_str().ok())
    {
        if !webdav_etag_matches(value, &etag) {
            return webdav_condition_response(StatusCode::PRECONDITION_FAILED, entry);
        }
    } else if request_http_date(request_headers, &IF_UNMODIFIED_SINCE)
        .is_some_and(|value| entry.modified_ms > value.saturating_add(999))
    {
        return webdav_condition_response(StatusCode::PRECONDITION_FAILED, entry);
    }
    if let Some(value) = request_headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    {
        if webdav_etag_matches(value, &etag) {
            return webdav_condition_response(StatusCode::NOT_MODIFIED, entry);
        }
    } else if request_http_date(request_headers, &IF_MODIFIED_SINCE)
        .is_some_and(|value| entry.modified_ms <= value.saturating_add(999))
    {
        return webdav_condition_response(StatusCode::NOT_MODIFIED, entry);
    }
    let (token, device_id) = auth_context(&context.state)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_download_url",
        json!({ "fileId": entry.id }),
        &[],
    )
    .await
    .map_err(DavError::from)?;
    let data = response.data.unwrap_or_default();
    let download_url = data
        .get("signedURL")
        .or_else(|| data.get("signedUrl"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DavError::new(StatusCode::BAD_GATEWAY, "光鸭没有返回文件下载地址"))?;
    let client = reqwest::Client::new();
    let mut upstream_request = client.get(download_url);
    if let Some(value) = request_headers.get(RANGE) {
        upstream_request = upstream_request.header(RANGE.as_str(), value.as_bytes());
    }
    let upstream = upstream_request
        .send()
        .await
        .map_err(|error| DavError::new(StatusCode::BAD_GATEWAY, error.to_string()))?;
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !upstream.status().is_success()
        && upstream.status().as_u16() != 304
        && upstream.status().as_u16() != 416
    {
        return Err(DavError::new(
            if upstream.status().as_u16() == 404 {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            },
            format!("云端文件读取失败（HTTP {}）", upstream.status()),
        ));
    }
    let mut builder = Response::builder().status(status);
    for header in [
        ACCEPT_RANGES,
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_RANGE,
        CONTENT_DISPOSITION,
    ] {
        if let Some(value) = upstream.headers().get(header.as_str()) {
            builder = builder.header(header, value.as_bytes());
        }
    }
    if upstream.headers().get(ACCEPT_RANGES.as_str()).is_none() {
        builder = builder.header(ACCEPT_RANGES, "bytes");
    }
    builder = builder.header(ETAG, etag);
    builder = builder.header(LAST_MODIFIED, http_date(entry.modified_ms));
    let body = if head_only {
        Body::empty()
    } else {
        Body::from_stream(upstream.bytes_stream())
    };
    builder
        .body(body)
        .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

async fn handle_request(
    State(context): State<WebDavContext>,
    request: Request<Body>,
) -> Response<Body> {
    if !authenticated(request.headers(), &context.state) {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(
                WWW_AUTHENTICATE,
                r#"Basic realm="Guangya WebDAV", charset="UTF-8""#,
            )
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty()));
    }
    let result = handle_authenticated(&context, request).await;
    match result {
        Ok(response) => response,
        Err(error) => plain_response(error.status, error.message),
    }
}

async fn handle_authenticated(
    context: &WebDavContext,
    request: Request<Body>,
) -> DavResult<Response<Body>> {
    let method = request.method().clone();
    let segments = decode_path(request.uri().path())?;
    if method == Method::OPTIONS {
        return Ok(Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(
                ALLOW,
                "OPTIONS, PROPFIND, PROPPATCH, GET, HEAD, PUT, MKCOL, DELETE, MOVE, COPY",
            )
            .header("dav", "1")
            .header("ms-author-via", "DAV")
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty())));
    }
    if method.as_str() == "PROPFIND" {
        let force_refresh = request_forces_directory_refresh(request.headers());
        let depth = request
            .headers()
            .get("depth")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("1");
        if !matches!(depth, "0" | "1") {
            return Err(DavError::new(StatusCode::FORBIDDEN, "仅支持 Depth: 0 或 1"));
        }
        let entry = resolve_entry(context, &segments, force_refresh).await?;
        let mut responses = vec![property_response(&segments, &entry)];
        if depth == "1" && entry.is_directory {
            for child in list_children(context, &entry.id, force_refresh).await? {
                let mut child_segments = segments.clone();
                child_segments.push(child.name.clone());
                responses.push(property_response(&child_segments, &child));
            }
        }
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">{}</D:multistatus>"#,
            responses.join("")
        );
        return Response::builder()
            .status(StatusCode::MULTI_STATUS)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("dav", "1")
            .body(Body::from(body))
            .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()));
    }
    if method.as_str() == "PROPPATCH" {
        let entry = resolve_entry(
            context,
            &segments,
            request_forces_directory_refresh(request.headers()),
        )
        .await?;
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">{}</D:multistatus>"#,
            property_response(&segments, &entry)
        );
        return Response::builder()
            .status(StatusCode::MULTI_STATUS)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("dav", "1")
            .body(Body::from(body))
            .map_err(|error| DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()));
    }
    if method == Method::GET || method == Method::HEAD {
        let force_refresh = request_forces_directory_refresh(request.headers());
        let entry = resolve_entry(context, &segments, force_refresh).await?;
        if entry.is_directory {
            let children = list_children(context, &entry.id, force_refresh).await?;
            let body = directory_index(&segments, &entry, &children);
            return Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/html; charset=utf-8")
                .header(CONTENT_LENGTH, body.len().to_string())
                .header("cache-control", "no-store")
                .body(if method == Method::HEAD {
                    Body::empty()
                } else {
                    Body::from(body)
                })
                .map_err(|error| {
                    DavError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
                });
        }
        let headers = request.headers().clone();
        return read_file(context, &headers, &entry, method == Method::HEAD).await;
    }
    if method == Method::PUT {
        let (parent_id, name, existing) = resolve_parent(context, &segments, true).await?;
        if existing.as_ref().is_some_and(|entry| entry.is_directory) {
            return Err(DavError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "不能用文件覆盖目录",
            ));
        }
        let created = existing.is_none();
        let uploaded = put_file(context, request, &parent_id, &name, existing.as_ref()).await?;
        publish_directory_invalidation(&context.app, [parent_id], false, "webdav-put");
        return Ok(Response::builder()
            .status(if created {
                StatusCode::CREATED
            } else {
                StatusCode::NO_CONTENT
            })
            .header(ETAG, uploaded.etag())
            .body(Body::empty())
            .unwrap_or_else(|_| Response::new(Body::empty())));
    }
    if method.as_str() == "MKCOL" {
        let (parent_id, name, existing) = resolve_parent(context, &segments, true).await?;
        if existing.is_some() {
            return Err(DavError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "目标已经存在",
            ));
        }
        create_directory(context, &parent_id, &name).await?;
        invalidate_remote_directory_cache(&context.state);
        publish_directory_invalidation(&context.app, [parent_id], false, "webdav-mkcol");
        return Ok(empty_response(StatusCode::CREATED));
    }
    if method == Method::DELETE {
        let entry = resolve_entry(context, &segments, true).await?;
        if entry.id.is_empty() {
            return Err(DavError::new(
                StatusCode::FORBIDDEN,
                "不能删除 WebDAV 根目录",
            ));
        }
        let parent_id = entry.parent_id.clone();
        delete_entry(context, &entry).await?;
        invalidate_remote_directory_cache(&context.state);
        publish_directory_invalidation(&context.app, [parent_id], false, "webdav-delete");
        return Ok(empty_response(StatusCode::NO_CONTENT));
    }
    if matches!(method.as_str(), "MOVE" | "COPY") {
        let entry = resolve_entry(context, &segments, true).await?;
        if entry.id.is_empty() {
            return Err(DavError::new(
                StatusCode::FORBIDDEN,
                "不能移动或复制 WebDAV 根目录",
            ));
        }
        let destination = destination_path(request.headers())?;
        let source_parent_id = entry.parent_id.clone();
        let (parent_id, name, existing) = resolve_parent(context, &destination, true).await?;
        let overwrite = request
            .headers()
            .get("overwrite")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("T")
            .to_uppercase()
            != "F";
        if existing.as_ref().is_some_and(|item| item.id == entry.id) {
            if method.as_str() == "MOVE" {
                return Ok(empty_response(StatusCode::NO_CONTENT));
            }
            return Err(DavError::new(StatusCode::FORBIDDEN, "不能把资源复制到自身"));
        }
        if existing.is_some() && !overwrite {
            return Err(DavError::new(
                StatusCode::PRECONDITION_FAILED,
                "目标已经存在",
            ));
        }
        let replaced = existing.is_some();
        let backup = if let Some(existing) = existing {
            let backup = RemoteEntry {
                parent_id: parent_id.clone(),
                name: format!(".__gy_dav_backup_{}", Uuid::new_v4().simple()),
                ..existing.clone()
            };
            move_entry(context, &existing, &parent_id, &backup.name).await?;
            Some(backup)
        } else {
            None
        };
        let operation = if method.as_str() == "MOVE" {
            move_entry(context, &entry, &parent_id, &name).await
        } else {
            copy_entry(context, &entry, &parent_id, &name).await
        };
        if let Err(error) = operation {
            if let Some(backup) = backup.as_ref() {
                if let Err(rollback) = move_entry(context, backup, &parent_id, &name).await {
                    return Err(DavError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "{}；恢复被覆盖目标也失败：{}",
                            error.message, rollback.message
                        ),
                    ));
                }
            }
            return Err(error);
        }
        if let Some(backup) = backup {
            delete_entry(context, &backup).await?;
        }
        invalidate_remote_directory_cache(&context.state);
        publish_directory_invalidation(
            &context.app,
            [source_parent_id, parent_id],
            false,
            if method.as_str() == "MOVE" {
                "webdav-move"
            } else {
                "webdav-copy"
            },
        );
        return Ok(empty_response(if replaced {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::CREATED
        }));
    }
    Err(DavError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        format!("不支持 WebDAV 方法：{method}"),
    ))
}

pub async fn serve(app: tauri::AppHandle, state: SharedState, port: u16) {
    invalidate_all_directory_cache();
    let address = format!("127.0.0.1:{port}");
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(listener) => listener,
        Err(error) => {
            if let Ok(mut guard) = state.lock() {
                guard.webdav_running = false;
                guard.webdav_error = Some(format!("WebDAV 监听 {address} 失败：{error}"));
            }
            status(
                &app,
                "error",
                format!("本地挂载服务启动失败（{address}）：{error}"),
            );
            return;
        }
    };
    if let Ok(mut guard) = state.lock() {
        guard.webdav_running = true;
        guard.webdav_error = None;
    }
    status(
        &app,
        "success",
        format!("本地挂载服务已启动：http://{address}/"),
    );
    let router = Router::new()
        .fallback(handle_request)
        .with_state(WebDavContext {
            app: app.clone(),
            state: state.clone(),
            directory_cache: shared_directory_cache(),
        });
    if let Err(error) = axum::serve(listener, router).await {
        if let Ok(mut guard) = state.lock() {
            guard.webdav_running = false;
            guard.webdav_error = Some(format!("WebDAV 服务异常退出：{error}"));
        }
        status(&app, "error", format!("本地挂载服务异常退出：{error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webdav_path_decoding_preserves_unicode_and_rejects_encoded_separators() {
        assert_eq!(decode_path("/").unwrap(), Vec::<String>::new());
        assert_eq!(
            decode_path("/%E5%BD%B1%E7%89%87/demo.mp4").unwrap(),
            vec!["影片".to_string(), "demo.mp4".to_string()]
        );
        assert!(decode_path("/safe%2Fescape").is_err());
        assert!(decode_path("/..").is_err());
    }

    #[test]
    fn webdav_properties_describe_files_and_directories() {
        let directory = RemoteEntry {
            id: "folder".to_string(),
            parent_id: String::new(),
            name: "资料".to_string(),
            is_directory: true,
            size: 0,
            modified_ms: 1_700_000_000_000,
        };
        let file = RemoteEntry {
            id: "file".to_string(),
            parent_id: directory.id.clone(),
            name: "readme.txt".to_string(),
            is_directory: false,
            size: 12,
            modified_ms: 1_700_000_000_000,
        };
        assert!(property_response(&["资料".to_string()], &directory).contains("<D:collection/>"));
        let file_xml = property_response(&["资料".to_string(), "readme.txt".to_string()], &file);
        assert!(file_xml.contains("<D:getcontentlength>12</D:getcontentlength>"));
        assert!(file_xml.contains("text/plain"));
        assert!(file_xml.contains("/%E8%B5%84%E6%96%99/readme%2Etxt"));
    }

    #[test]
    fn webdav_conditional_etags_accept_weak_lists_and_wildcards() {
        let etag = r#""gy-file-12-1700000000000""#;
        assert!(webdav_etag_matches("*", etag));
        assert!(webdav_etag_matches(
            r#""other", W/"gy-file-12-1700000000000""#,
            etag
        ));
        assert!(!webdav_etag_matches(r#""other""#, etag));
    }

    #[test]
    fn directory_get_index_links_children_and_parent_without_downloading_the_directory() {
        let directory = RemoteEntry {
            id: "folder".to_string(),
            parent_id: String::new(),
            name: "资料".to_string(),
            is_directory: true,
            size: 0,
            modified_ms: 1_700_000_000_000,
        };
        let child = RemoteEntry {
            id: "file".to_string(),
            parent_id: directory.id.clone(),
            name: "说明.txt".to_string(),
            is_directory: false,
            size: 12,
            modified_ms: 1_700_000_000_000,
        };
        let body = directory_index(&["资料".to_string()], &directory, &[child]);
        assert!(body.contains(r#"href="/">../</a>"#));
        assert!(body.contains(r#"href="/%E8%B5%84%E6%96%99/%E8%AF%B4%E6%98%8E%2Etxt""#));
        assert!(!body.contains("目录不能直接下载"));
    }

    #[test]
    fn directory_cache_invalidates_changed_child_listings_and_rejects_stale_refreshes() {
        let cache = DirectoryCache::default();
        cache.ensure_scope(b"account-a");
        let child = RemoteEntry {
            id: "folder".to_string(),
            parent_id: String::new(),
            name: "资料".to_string(),
            is_directory: true,
            size: 0,
            modified_ms: 100,
        };
        assert!(cache.put_if_current("", 0, vec![child.clone()]));
        assert!(cache.put_if_current("folder", 0, Vec::new()));

        let mut changed = child;
        changed.modified_ms = 101;
        assert!(cache.put_if_current("", 0, vec![changed]));
        assert!(cache.snapshot("folder").is_none());

        let generation = cache.generation("");
        cache.invalidate("");
        assert!(!cache.put_if_current("", generation, Vec::new()));

        assert!(cache.put_if_current("", cache.generation(""), Vec::new()));
        cache.ensure_scope(b"account-b");
        assert!(cache.snapshot("").is_none());
    }

    #[test]
    fn manual_refresh_headers_force_directory_revalidation() {
        let mut headers = HeaderMap::new();
        assert!(!request_forces_directory_refresh(&headers));

        headers.insert(CACHE_CONTROL, "public, max-age = 0".parse().unwrap());
        assert!(request_forces_directory_refresh(&headers));

        headers.clear();
        headers.insert(PRAGMA, "no-cache".parse().unwrap());
        assert!(request_forces_directory_refresh(&headers));

        headers.clear();
        headers.insert(CACHE_CONTROL, "max-age=60".parse().unwrap());
        assert!(!request_forces_directory_refresh(&headers));
    }

    #[test]
    fn shared_directory_cache_invalidates_cached_entry_parents_and_unknowns_safely() {
        let first = shared_directory_cache();
        let second = shared_directory_cache();
        assert!(Arc::ptr_eq(&first, &second));
        first.clear();
        first.ensure_scope(b"shared-account");
        let child = RemoteEntry {
            id: "folder".to_string(),
            parent_id: String::new(),
            name: "资料".to_string(),
            is_directory: true,
            size: 0,
            modified_ms: 100,
        };
        assert!(first.put_if_current("", first.generation(""), vec![child]));
        assert!(first.put_if_current("folder", first.generation("folder"), Vec::new()));

        let (parents, all_found) = invalidate_directory_cache_entries(&["folder".to_string()]);
        assert!(all_found);
        assert_eq!(parents, vec![String::new()]);
        assert!(first.snapshot("").is_none());
        assert!(first.snapshot("folder").is_none());

        let (_, all_found) = invalidate_directory_cache_entries(&["missing".to_string()]);
        assert!(!all_found);
    }
}
