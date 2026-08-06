use crate::organizer_core::{
    parse_media_name, render_nfo, resolve_tmdb_match, sanitize_component, AnalyzedSidecar,
    AnalyzedVideo, CandidateAnalysis, GeneratorSpec, MatchResolution, MediaMetadata, MediaQuery,
    NativeSettings, RecognitionOverrides, TmdbCandidate, TmdbClient, NATIVE_ENGINE_VERSION,
    VIDEO_EXTENSIONS,
};
use crate::{
    api_post, finish_operation_response, hdhive_request, organizer_upload_bytes,
    poll_hdhive_receipt, save_auto_share_event, share_file_payload, share_id_for_hdhive,
    PendingAutoShare, SharedState,
};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio::time::sleep;
use uuid::Uuid;

const POLL_INTERVAL_SECONDS: u64 = 15;
const MAX_CLOUD_ITEMS: usize = 20_000;
const MAX_CLOUD_DEPTH: usize = 64;
const MAX_JOB_LIST: i64 = 100;
const MOVIE_PATH_TEMPLATE: &str = "{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}";
const TV_PATH_TEMPLATE: &str = "{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}";
const DEFAULT_SCRAPE_TYPES: &[&str] = &["movie_nfo", "tvshow_nfo", "poster", "fanart"];
const SCRAPE_TYPES: &[&str] = &[
    "movie_nfo",
    "tvshow_nfo",
    "episode_nfo",
    "poster",
    "fanart",
    "season_poster",
];

pub struct OrganizerRuntime {
    db_path: PathBuf,
    running_jobs: HashSet<String>,
    running_candidates: HashSet<String>,
}

pub type OrganizerSharedState = Arc<Mutex<OrganizerRuntime>>;

#[derive(Debug, Clone)]
struct OrganizerSecrets {
    api_key: String,
    native: NativeSettings,
    movie_path_template: String,
    tv_path_template: String,
    movie_category: String,
    tv_category: String,
    api_key_from_environment: bool,
    language_from_environment: bool,
    image_language_from_environment: bool,
    api_base: String,
    image_base: String,
    tmdb_proxy: String,
    category_rules: Vec<Value>,
    scrape_targets: Vec<Value>,
    default_scrape_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizerPublicSettings {
    provider: String,
    engine: String,
    configured: bool,
    api_key_configured: bool,
    api_key_managed_by_environment: bool,
    language_managed_by_environment: bool,
    image_language_managed_by_environment: bool,
    tmdb_api_base_managed_by_environment: bool,
    tmdb_image_base_managed_by_environment: bool,
    language: String,
    image_language: String,
    include_adult: bool,
    minimum_match_score: f64,
    movie_path_template: String,
    tv_path_template: String,
    movie_category: String,
    tv_category: String,
    tmdb_api_base: String,
    tmdb_image_base: String,
    category_rules: Vec<Value>,
    scrape_targets: Vec<Value>,
    default_scrape_types: Vec<String>,
    template_examples: Value,
    path_presets: Vec<Value>,
    scrape_type_options: Vec<Value>,
}

fn standard_template_examples(secrets: &OrganizerSecrets) -> Value {
    let mut movie = MediaMetadata::default();
    movie.media_type = "movie".to_string();
    movie.title = "示例电影".to_string();
    movie.original_title = "Example Movie".to_string();
    movie.year = Some(2024);
    movie.tmdb_id = 12345;
    movie.countries = vec!["US".to_string()];
    let mut movie_parsed = crate::organizer_core::ParsedMediaName::default();
    movie_parsed.quality = "1080p".to_string();
    let movie_category = resolve_media_category(&movie, secrets);
    let movie_context = template_context(&movie, &movie_parsed, "", &movie_category, "mkv");
    let movie_path = render_path_template(&secrets.movie_path_template, &movie_context)
        .unwrap_or_else(|_| format!("电影/US/2024/示例电影 (2024) [tmdb-12345]/示例电影 (2024).mkv"));

    let mut tv = MediaMetadata::default();
    tv.media_type = "tv".to_string();
    tv.title = "示例剧集".to_string();
    tv.original_title = "Example Series".to_string();
    tv.year = Some(2024);
    tv.tmdb_id = 67890;
    tv.countries = vec!["CN".to_string()];
    let mut tv_parsed = crate::organizer_core::ParsedMediaName::default();
    tv_parsed.season = Some(1);
    tv_parsed.episode = Some(2);
    tv_parsed.quality = "1080p".to_string();
    let tv_category = resolve_media_category(&tv, secrets);
    let tv_context = template_context(&tv, &tv_parsed, "第二集", &tv_category, "mkv");
    let tv_path = render_path_template(&secrets.tv_path_template, &tv_context)
        .unwrap_or_else(|_| format!("电视剧/CN/2024/示例剧集 (2024) [tmdb-67890]/Season 01/示例剧集.S01E02.mkv"));

    json!({
        "movie": {
            "input": "示例电影.2024.1080p.WEB-DL.mkv",
            "path": movie_path,
            "directory": path_parent(&movie_path),
            "filename": path_name(&movie_path)
        },
        "tv": {
            "input": "示例剧集.S01E02.1080p.WEB-DL.mkv",
            "path": tv_path,
            "directory": path_parent(&tv_path),
            "filename": path_name(&tv_path)
        }
    })
}

impl OrganizerSecrets {
    fn public(&self) -> OrganizerPublicSettings {
        OrganizerPublicSettings {
            provider: "tmdb".to_string(),
            engine: NATIVE_ENGINE_VERSION.to_string(),
            configured: !self.api_key.trim().is_empty(),
            api_key_configured: !self.api_key.trim().is_empty(),
            api_key_managed_by_environment: self.api_key_from_environment,
            language_managed_by_environment: self.language_from_environment,
            image_language_managed_by_environment: self.image_language_from_environment,
            tmdb_api_base_managed_by_environment: std::env::var("TMDB_API_BASE").ok().filter(|v| !v.trim().is_empty()).is_some(),
            tmdb_image_base_managed_by_environment: std::env::var("TMDB_IMAGE_BASE").ok().filter(|v| !v.trim().is_empty()).is_some(),
            language: self.native.language.clone(),
            image_language: self.native.image_language.clone(),
            include_adult: self.native.include_adult,
            minimum_match_score: self.native.minimum_match_score,
            movie_path_template: self.movie_path_template.clone(),
            tv_path_template: self.tv_path_template.clone(),
            movie_category: self.movie_category.clone(),
            tv_category: self.tv_category.clone(),
            tmdb_api_base: self.api_base.clone(),
            tmdb_image_base: self.image_base.clone(),
            category_rules: self.category_rules.clone(),
            scrape_targets: self.scrape_targets.clone(),
            default_scrape_types: self.default_scrape_types.clone(),
            template_examples: standard_template_examples(self),
            path_presets: vec![
                json!({ "id": "category-country-year", "name": "分类 / 国家 / 年份", "movie": MOVIE_PATH_TEMPLATE, "tv": TV_PATH_TEMPLATE }),
                json!({ "id": "media-server", "name": "媒体服务器常用", "movie": "{category}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}", "tv": "{category}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}" }),
                json!({ "id": "compact", "name": "精简目录", "movie": "{category}/{title} ({year})/{title}.{year}.{quality}.{ext}", "tv": "{category}/{title} ({year})/S{season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}" }),
            ],
            scrape_type_options: vec![
                json!({ "value": "movie_nfo", "label": "电影 NFO" }),
                json!({ "value": "tvshow_nfo", "label": "剧集 NFO" }),
                json!({ "value": "episode_nfo", "label": "单集 NFO" }),
                json!({ "value": "poster", "label": "海报" }),
                json!({ "value": "fanart", "label": "背景图" }),
                json!({ "value": "season_poster", "label": "季海报" }),
            ],
        }
    }

    fn client(&self) -> Result<TmdbClient, String> {
        TmdbClient::new(
            self.api_key.clone(),
            self.native.language.clone(),
            self.native.image_language.clone(),
            self.native.include_adult,
            self.api_base.clone(),
            self.image_base.clone(),
            Some(self.tmdb_proxy.clone()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizerMapping {
    id: String,
    source_path: String,
    target_path: String,
    source_dir_id: String,
    target_dir_id: String,
    enabled: bool,
    scan_existing: bool,
    monitor_mode: String,
    transfer_type: String,
    media_type: String,
    scrape: bool,
    scrape_types: Vec<String>,
    sync_extras: bool,
    conflict_policy: String,
    auto_execute: bool,
    share_after_organize: bool,
    share_risk_acknowledged: bool,
    settle_seconds: u64,
    watch_error: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PreviewSummary {
    total: usize,
    success: usize,
    failed: usize,
    warnings: usize,
    skipped: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CloudPreviewItem {
    success: bool,
    kind: String,
    source: Option<String>,
    source_id: Option<String>,
    source_parent_id: Option<String>,
    source_name: Option<String>,
    target: String,
    target_relative: String,
    target_parent_relative: String,
    target_name: String,
    operation: String,
    action: String,
    exists: bool,
    existing_id: Option<String>,
    renamed_for_conflict: bool,
    error_code: Option<String>,
    message: String,
    generator: Option<GeneratorSpec>,
    image_role: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CloudPreviewData {
    summary: PreviewSummary,
    items: Vec<CloudPreviewItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CloudPreview {
    success: bool,
    engine: String,
    mapping_signature: String,
    source_signature: String,
    query: MediaQuery,
    candidates: Vec<TmdbCandidate>,
    selected: Option<TmdbCandidate>,
    metadata: Option<MediaMetadata>,
    target_root: String,
    target_root_id: String,
    media_root: String,
    media_root_relative: String,
    share_relative_path: String,
    share_title: String,
    error_code: Option<String>,
    message: String,
    ignored_samples: Vec<String>,
    data: CloudPreviewData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OrganizerExecutionResult {
    success: bool,
    transferred: usize,
    skipped: usize,
    scraped: usize,
    warnings: Vec<String>,
    targets: Vec<String>,
    share: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizerJob {
    id: String,
    mapping_id: String,
    source_path: String,
    source_id: String,
    source_parent_id: String,
    source_size: i64,
    source_modified_ms: String,
    source_file_count: i64,
    source_signature: String,
    share_after_requested: bool,
    status: String,
    media_type: Option<String>,
    tmdb_id: Option<i64>,
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
    query_title: Option<String>,
    query_year: Option<i64>,
    preview: Option<CloudPreview>,
    result: Option<OrganizerExecutionResult>,
    error_code: Option<String>,
    message: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizerSnapshot {
    settings: OrganizerPublicSettings,
    mappings: Vec<OrganizerMapping>,
    jobs: Vec<OrganizerJob>,
    counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrganizerSettingsInput {
    api_key: Option<String>,
    #[serde(default)]
    clear_api_key: bool,
    language: Option<String>,
    image_language: Option<String>,
    include_adult: Option<bool>,
    minimum_match_score: Option<f64>,
    movie_path_template: Option<String>,
    tv_path_template: Option<String>,
    movie_category: Option<String>,
    tv_category: Option<String>,
    #[serde(alias = "tmdb_url_base", alias = "tmdb_api_proxy")]
    tmdb_api_base: Option<String>,
    #[serde(alias = "tmdb_image_url", alias = "tmdb_image_proxy")]
    tmdb_image_base: Option<String>,
    category_rules: Option<Vec<Value>>,
    scrape_targets: Option<Vec<Value>>,
    default_scrape_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrganizerMappingInput {
    source_path: Option<String>,
    target_path: Option<String>,
    source_dir_id: Option<String>,
    target_dir_id: Option<String>,
    enabled: Option<bool>,
    scan_existing: Option<bool>,
    transfer_type: Option<String>,
    media_type: Option<String>,
    scrape: Option<bool>,
    scrape_types: Option<Vec<String>>,
    sync_extras: Option<bool>,
    conflict_policy: Option<String>,
    auto_execute: Option<bool>,
    share_after_organize: Option<bool>,
    share_risk_acknowledged: Option<bool>,
    settle_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrganizerJobInput {
    media_type: Option<String>,
    tmdb_id: Option<i64>,
    title: Option<String>,
    year: Option<i64>,
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
    #[serde(default)]
    clear_tmdb_id: bool,
    #[serde(default)]
    clear_title: bool,
    #[serde(default)]
    clear_year: bool,
    #[serde(default)]
    clear_season: bool,
    #[serde(default)]
    clear_episode: bool,
    #[serde(default)]
    clear_episode_end: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SelectedScrapeFile {
    #[serde(alias = "file_id", alias = "fileId")]
    id: String,
    #[serde(alias = "parentId")]
    parent_id: String,
    #[serde(default, alias = "parentPath")]
    parent_path: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScrapeSelectedInput {
    #[serde(default)]
    files: Vec<SelectedScrapeFile>,
    #[serde(alias = "targetId")]
    target_id: Option<String>,
    #[serde(default)]
    transfer_type: Option<String>,
    #[serde(default)]
    media_type: Option<String>,
    #[serde(default)]
    scrape_types: Option<Vec<String>>,
    #[serde(default)]
    share_risk_acknowledged: bool,
}

#[derive(Debug, Clone, Default)]
struct CloudEntry {
    id: String,
    parent_id: String,
    name: String,
    logical_path: String,
    is_directory: bool,
    size: i64,
    modified_ms: String,
}

#[derive(Debug, Clone, Default)]
struct CandidateFingerprint {
    signature: String,
    size: i64,
    modified_ms: String,
    file_count: i64,
    video_count: i64,
}

#[derive(Debug, Clone)]
struct LoadedCandidate {
    candidate: CloudEntry,
    entries: Vec<CloudEntry>,
    fingerprint: CandidateFingerprint,
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建本地数据目录失败：{error}"))?;
    }
    let connection =
        Connection::open(path).map_err(|error| format!("打开 SQLite 失败：{error}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("设置 SQLite 等待时间失败：{error}"))?;
    Ok(connection)
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("读取 {table} 表结构失败：{error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("读取 {table} 表字段失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析 {table} 表字段失败：{error}"))?;
    if !columns.iter().any(|current| current == column) {
        connection
            .execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .map_err(|error| format!("迁移 {table}.{column} 失败：{error}"))?;
    }
    Ok(())
}

pub fn init_database(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS organizer_settings (
               id INTEGER PRIMARY KEY CHECK (id = 1), tmdb_api_key TEXT NOT NULL DEFAULT '',
               language TEXT NOT NULL DEFAULT 'zh-CN', image_language TEXT NOT NULL DEFAULT 'zh,null,en',
               include_adult INTEGER NOT NULL DEFAULT 0, minimum_match_score REAL NOT NULL DEFAULT 0.72,
               movie_path_template TEXT NOT NULL DEFAULT '', tv_path_template TEXT NOT NULL DEFAULT '',
               movie_category TEXT NOT NULL DEFAULT '电影', tv_category TEXT NOT NULL DEFAULT '电视剧',
               tmdb_api_base TEXT NOT NULL DEFAULT '', tmdb_image_base TEXT NOT NULL DEFAULT '',
               category_rules TEXT NOT NULL DEFAULT '[]', scrape_targets TEXT NOT NULL DEFAULT '[]',
               default_scrape_types TEXT NOT NULL DEFAULT '[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]', updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS organizer_mappings (
               id TEXT PRIMARY KEY, source_path TEXT NOT NULL, target_path TEXT NOT NULL DEFAULT '',
               source_dir_id TEXT NOT NULL DEFAULT '', target_dir_id TEXT NOT NULL DEFAULT '',
               enabled INTEGER NOT NULL DEFAULT 1, scan_existing INTEGER NOT NULL DEFAULT 1,
               monitor_mode TEXT NOT NULL DEFAULT 'cloud_polling', transfer_type TEXT NOT NULL DEFAULT 'copy',
               media_type TEXT NOT NULL DEFAULT '', scrape INTEGER NOT NULL DEFAULT 0,
               scrape_types TEXT NOT NULL DEFAULT '[]', sync_extras INTEGER NOT NULL DEFAULT 1,
               conflict_policy TEXT NOT NULL DEFAULT 'skip', auto_execute INTEGER NOT NULL DEFAULT 0,
               share_after_organize INTEGER NOT NULL DEFAULT 0, share_risk_acknowledged INTEGER NOT NULL DEFAULT 0,
               settle_seconds INTEGER NOT NULL DEFAULT 30, watch_error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS organizer_jobs (
               id TEXT PRIMARY KEY, mapping_id TEXT NOT NULL, source_path TEXT NOT NULL,
               source_id TEXT NOT NULL DEFAULT '', source_parent_id TEXT NOT NULL DEFAULT '',
               source_size INTEGER NOT NULL DEFAULT 0, source_modified_ms TEXT NOT NULL DEFAULT '0',
               source_file_count INTEGER NOT NULL DEFAULT 0, source_signature TEXT NOT NULL DEFAULT '',
               share_after_requested INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL, media_type TEXT,
               tmdb_id TEXT, season INTEGER, episode INTEGER, episode_end INTEGER, query_title TEXT,
               query_year INTEGER, preview_json TEXT, result_json TEXT, error_code TEXT, message TEXT,
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS organizer_jobs_mapping_status ON organizer_jobs(mapping_id, status, updated_at);
             CREATE INDEX IF NOT EXISTS organizer_jobs_source_id ON organizer_jobs(mapping_id, source_id, updated_at);",
        )
        .map_err(|error| format!("初始化云盘原生整理数据表失败：{error}"))?;
    for (column, definition) in [
        ("movie_path_template", "TEXT NOT NULL DEFAULT ''"),
        ("tv_path_template", "TEXT NOT NULL DEFAULT ''"),
        ("movie_category", "TEXT NOT NULL DEFAULT '电影'"),
        ("tv_category", "TEXT NOT NULL DEFAULT '电视剧'"),
        ("tmdb_api_base", "TEXT NOT NULL DEFAULT ''"),
        ("tmdb_image_base", "TEXT NOT NULL DEFAULT ''"),
        ("category_rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("scrape_targets", "TEXT NOT NULL DEFAULT '[]'"),
        ("default_scrape_types", "TEXT NOT NULL DEFAULT '[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]'"),
    ] {
        ensure_column(&connection, "organizer_settings", column, definition)?;
    }
    for (column, definition) in [
        ("source_dir_id", "TEXT NOT NULL DEFAULT ''"),
        ("target_dir_id", "TEXT NOT NULL DEFAULT ''"),
        ("scrape_types", "TEXT NOT NULL DEFAULT '[]'"),
        ("share_after_organize", "INTEGER NOT NULL DEFAULT 0"),
        ("share_risk_acknowledged", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column(&connection, "organizer_mappings", column, definition)?;
    }
    for (column, definition) in [
        ("source_id", "TEXT NOT NULL DEFAULT ''"),
        ("source_parent_id", "TEXT NOT NULL DEFAULT ''"),
        ("source_signature", "TEXT NOT NULL DEFAULT ''"),
        ("share_after_requested", "INTEGER NOT NULL DEFAULT 0"),
        ("episode", "INTEGER"),
        ("episode_end", "INTEGER"),
        ("query_title", "TEXT"),
        ("query_year", "INTEGER"),
        ("result_json", "TEXT"),
        ("error_code", "TEXT"),
    ] {
        ensure_column(&connection, "organizer_jobs", column, definition)?;
    }
    connection
        .execute(
            "UPDATE organizer_jobs SET status='failed', error_code='service_restarted', message='应用上次退出，任务可重新识别', updated_at=?1 WHERE status IN ('recognizing','running')",
            params![now_seconds()],
        )
        .map_err(|error| format!("恢复中断整理任务失败：{error}"))?;
    connection
        .execute(
            "UPDATE organizer_mappings SET enabled=0, scrape=0, watch_error='旧版本地整理配置已停用，请重新选择光鸭云盘 A/B 目录', updated_at=?1 WHERE source_dir_id='' OR target_dir_id=''",
            params![now_seconds()],
        )
        .map_err(|error| format!("迁移旧整理配置失败：{error}"))?;
    connection
        .execute(
            "UPDATE organizer_jobs SET status='needs_review', error_code='engine_migrated', message='整理引擎已切换为光鸭云盘内原生整理，请重新识别', updated_at=?1 WHERE status='ready' AND (preview_json IS NULL OR preview_json NOT LIKE ?2)",
            params![now_seconds(), format!("%\"engine\":\"{NATIVE_ENGINE_VERSION}\"%")],
        )
        .map_err(|error| format!("迁移旧整理预览失败：{error}"))?;
    Ok(())
}

fn clean(value: Option<&str>) -> String {
    value.unwrap_or_default().trim().to_string()
}

fn normalize_language(value: &str, fallback: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    };
    let expression = Regex::new(r"^[a-z]{2}(?:-[A-Z]{2})?$").expect("language regex");
    if !expression.is_match(value) {
        return Err("TMDB 语言格式不正确，例如 zh-CN 或 en-US".to_string());
    }
    Ok(value.to_string())
}

fn normalize_image_language(value: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        "zh,null,en"
    } else {
        value.trim()
    };
    let expression = Regex::new(r"^[a-z]{2}(?:-[A-Z]{2})?(?:,(?:[a-z]{2}(?:-[A-Z]{2})?|null))*$")
        .expect("image language regex");
    if !expression.is_match(value) {
        return Err("图片语言格式不正确，例如 zh-CN,null,en".to_string());
    }
    Ok(value.to_string())
}

fn normalize_match_score(value: f64) -> Result<f64, String> {
    if !(0.4..=0.98).contains(&value) {
        return Err("自动匹配阈值必须在 0.40 到 0.98 之间".to_string());
    }
    Ok((value * 100.0).round() / 100.0)
}

fn normalize_cloud_path(value: &str) -> String {
    let normalized = value.replace('\\', "/");
    let parts = normalized
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn normalize_category(value: &str, fallback: &str, label: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    };
    if value.chars().count() > 80 || value.contains(['/', '\\']) {
        return Err(format!("{label}不能包含路径分隔符且不能超过 80 个字符"));
    }
    Ok(value.to_string())
}

fn normalize_mirror_url(value: &str, fallback: &str, label: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() { fallback } else { value.trim() };
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(format!("{label}必须以 http:// 或 https:// 开头"));
    }
    if value.len() > 500 || value.contains(['?', '#']) {
        return Err(format!("{label}格式不正确或过长"));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn normalize_category_rules(value: Option<Vec<Value>>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for (index, rule) in value.unwrap_or_default().into_iter().take(100).enumerate() {
        let name = rule.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        if name.is_empty() || name.len() > 80 || name.contains(['/', '\\']) {
            return Err(format!("第 {} 条媒体分类名称无效", index + 1));
        }
        let genres = rule
            .get("genres")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|item| {
                if let Some(value) = item.as_str() {
                    value.split([',', '，', '\n']).map(|term| json!(term.trim().to_lowercase())).collect::<Vec<_>>()
                } else if let Some(value) = item.as_i64() {
                    vec![json!(value.to_string())]
                } else {
                    Vec::new()
                }
            })
            .filter(|item| item.as_str().is_some_and(|value| !value.is_empty()))
            .take(50)
            .collect::<Vec<_>>();
        if genres.is_empty() {
            return Err(format!("第 {} 条媒体分类至少配置一个 TMDB 类型", index + 1));
        }
        let media_type = rule.get("media_type").and_then(Value::as_str).unwrap_or("all").trim().to_lowercase();
        let media_type = if matches!(media_type.as_str(), "movie" | "tv" | "all") { media_type } else { "all".to_string() };
        let id = rule.get("id").and_then(Value::as_str).filter(|value| !value.trim().is_empty()).map(str::to_string)
            .unwrap_or_else(|| format!("category-{}", index + 1));
        result.push(json!({
            "id": id,
            "name": name,
            "media_type": media_type,
            "genres": genres,
            "enabled": rule.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        }));
    }
    Ok(result)
}

fn normalize_scrape_targets(value: Option<Vec<Value>>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for (index, target) in value.unwrap_or_default().into_iter().take(50).enumerate() {
        let name = target.get("name").and_then(Value::as_str).unwrap_or_default().trim();
        let dir_id = target.get("dir_id").or_else(|| target.get("target_dir_id"))
            .and_then(Value::as_str).unwrap_or_default().trim();
        if dir_id.is_empty() { return Err(format!("第 {} 个刮削目标未选择云盘目录", index + 1)); }
        let path = normalize_cloud_path(target.get("path").or_else(|| target.get("target_path")).and_then(Value::as_str).unwrap_or("/"));
        let id = target.get("id").and_then(Value::as_str).filter(|value| !value.trim().is_empty()).map(str::to_string)
            .unwrap_or_else(|| format!("target-{}", Uuid::new_v4()));
        let name = if name.is_empty() { format!("媒体库 {}", index + 1) } else { name.chars().take(80).collect() };
        result.push(json!({
            "id": id,
            "name": name,
            "dir_id": dir_id,
            "path": path,
        }));
    }
    Ok(result)
}

fn resolve_media_category(metadata: &MediaMetadata, secrets: &OrganizerSecrets) -> String {
    let media_type = if metadata.media_type == "tv" { "tv" } else { "movie" };
    let mut terms = metadata
        .genres
        .iter()
        .map(|value| value.trim().to_lowercase())
        .collect::<HashSet<_>>();
    terms.extend(metadata.genre_ids.iter().map(ToString::to_string));
    for rule in &secrets.category_rules {
        if rule.get("enabled").and_then(Value::as_bool) == Some(false) { continue; }
        let rule_type = rule.get("media_type").and_then(Value::as_str).unwrap_or("all");
        if rule_type != "all" && rule_type != media_type { continue; }
        let matches = rule.get("genres").and_then(Value::as_array).map(|items| items.iter().any(|item| {
            let term = item.as_str().map(str::to_lowercase).or_else(|| item.as_i64().map(|value| value.to_string()));
            term.map(|value| terms.contains(&value)).unwrap_or(false)
        })).unwrap_or(false);
        if matches {
            if let Some(name) = rule.get("name").and_then(Value::as_str).filter(|value| !value.trim().is_empty()) {
                return name.trim().to_string();
            }
        }
    }
    if media_type == "tv" { secrets.tv_category.clone() } else { secrets.movie_category.clone() }
}

fn normalize_transfer_type(value: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        "copy"
    } else {
        value.trim()
    };
    match value.to_lowercase().as_str() {
        "copy" => Ok("copy".to_string()),
        "move" => Ok("move".to_string()),
        _ => Err("云盘内整理方式必须是复制或移动".to_string()),
    }
}

fn normalize_media_type(value: &str) -> Result<String, String> {
    match value.trim().to_lowercase().as_str() {
        "" => Ok(String::new()),
        "movie" => Ok("movie".to_string()),
        "tv" => Ok("tv".to_string()),
        _ => Err("媒体类型必须是自动、电影或电视剧".to_string()),
    }
}

fn normalize_conflict_policy(value: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        "skip"
    } else {
        value.trim()
    };
    match value.to_lowercase().as_str() {
        "skip" | "overwrite" | "rename" => Ok(value.to_lowercase()),
        _ => Err("冲突策略必须是跳过、覆盖或保留两份".to_string()),
    }
}

fn normalize_scrape_types(values: &[String], enabled: bool) -> Result<Vec<String>, String> {
    if !enabled {
        return Ok(Vec::new());
    }
    let source = if values.is_empty() {
        DEFAULT_SCRAPE_TYPES
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    } else {
        values.to_vec()
    };
    let mut normalized = Vec::new();
    for value in source {
        let value = value.trim().to_lowercase();
        if value.is_empty() || normalized.contains(&value) {
            continue;
        }
        if !SCRAPE_TYPES.contains(&value.as_str()) {
            return Err(format!("不支持的刮削类型：{value}"));
        }
        normalized.push(value);
    }
    Ok(normalized)
}

fn path_parts(value: &str) -> Vec<&str> {
    value.split('/').filter(|part| !part.is_empty()).collect()
}

fn path_name(value: &str) -> String {
    path_parts(value)
        .last()
        .copied()
        .unwrap_or_default()
        .to_string()
}

fn path_parent(value: &str) -> String {
    let mut parts = path_parts(value);
    if !parts.is_empty() {
        parts.pop();
    }
    parts.join("/")
}

fn path_extension(value: &str) -> String {
    path_name(value)
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default()
}

fn path_stem(value: &str) -> String {
    let name = path_name(value);
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or(name)
}

fn join_relative(parts: &[&str]) -> String {
    parts
        .iter()
        .flat_map(|value| path_parts(value))
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_path_template(value: &str, fallback: &str, label: &str) -> Result<String, String> {
    let value = if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    };
    if value.chars().count() > 500 {
        return Err(format!("{label}不能超过 500 个字符"));
    }
    let lower = value.to_lowercase();
    if !lower.contains("{title}") {
        return Err(format!("{label}必须包含 {{title}}"));
    }
    let mut context = HashMap::new();
    for (key, value) in [
        ("category", "分类"),
        ("country", "CN"),
        ("year", "2026"),
        ("title", "示例"),
        ("original_title", "Example"),
        ("tmdb_id", "1"),
        ("season", "1"),
        ("episode", "1"),
        ("episode_end", ""),
        ("episode_title", "第一集"),
        ("edition", ""),
        ("quality", ""),
        ("part", ""),
        ("ext", "mkv"),
        ("season_tag", "S01"),
        ("episode_tag", "E01"),
    ] {
        context.insert(key.to_string(), value.to_string());
    }
    render_path_template(value, &context)?;
    Ok(value.to_string())
}

fn load_secrets(path: &Path) -> Result<OrganizerSecrets, String> {
    let connection = open_database(path)?;
    let stored = connection
        .query_row(
            "SELECT tmdb_api_key, language, image_language, include_adult, minimum_match_score,
                    movie_path_template, tv_path_template, movie_category, tv_category,
                    tmdb_api_base, tmdb_image_base, category_rules, scrape_targets, default_scrape_types
             FROM organizer_settings WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, f64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取整理设置失败：{error}"))?
        .unwrap_or_else(|| {
            (
                String::new(),
                "zh-CN".to_string(),
                "zh,null,en".to_string(),
                false,
                0.72,
                String::new(),
                String::new(),
                "电影".to_string(),
                "电视剧".to_string(),
                String::new(),
                String::new(),
                "[]".to_string(),
                "[]".to_string(),
                "[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]".to_string(),
            )
        });
    let environment_key = std::env::var("TMDB_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("TMDB_READ_ACCESS_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    let environment_language = std::env::var("TMDB_LANGUAGE")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let environment_image_language = std::env::var("TMDB_IMAGE_LANGUAGE")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let mut native = NativeSettings::default();
    native.language = normalize_language(
        environment_language.as_deref().unwrap_or(&stored.1),
        "zh-CN",
    )?;
    native.image_language =
        normalize_image_language(environment_image_language.as_deref().unwrap_or(&stored.2))?;
    native.include_adult = stored.3;
    native.minimum_match_score = normalize_match_score(stored.4)?;
    let stored_api_base = normalize_mirror_url(&stored.9, "https://api.themoviedb.org/3", "TMDB API 镜像")?;
    let stored_image_base = normalize_mirror_url(&stored.10, "https://image.tmdb.org/t/p", "TMDB 图片镜像")?;
    let category_rules = normalize_category_rules(parse_json::<Vec<Value>>(Some(stored.11.clone())))
        .unwrap_or_default();
    let scrape_targets = normalize_scrape_targets(parse_json::<Vec<Value>>(Some(stored.12.clone())))
        .unwrap_or_default();
    let stored_scrape_types = parse_json::<Vec<String>>(Some(stored.13.clone())).unwrap_or_default();
    let default_scrape_types = normalize_scrape_types(&stored_scrape_types, true)
        .unwrap_or_else(|_| DEFAULT_SCRAPE_TYPES.iter().map(|value| value.to_string()).collect());
    let tmdb_proxy = connection
        .query_row("SELECT value FROM app_state WHERE key='network_proxy_tmdb'", [], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|error| format!("读取 TMDB 代理设置失败：{error}"))?
        .unwrap_or_default();
    Ok(OrganizerSecrets {
        api_key: environment_key.clone().unwrap_or(stored.0),
        native,
        movie_path_template: if stored.5.trim().is_empty() {
            MOVIE_PATH_TEMPLATE.to_string()
        } else {
            stored.5
        },
        tv_path_template: if stored.6.trim().is_empty() {
            TV_PATH_TEMPLATE.to_string()
        } else {
            stored.6
        },
        movie_category: if stored.7.trim().is_empty() {
            "电影".to_string()
        } else {
            stored.7
        },
        tv_category: if stored.8.trim().is_empty() {
            "电视剧".to_string()
        } else {
            stored.8
        },
        api_key_from_environment: environment_key.is_some(),
        language_from_environment: environment_language.is_some(),
        image_language_from_environment: environment_image_language.is_some(),
        api_base: std::env::var("TMDB_API_BASE").ok().filter(|v| !v.trim().is_empty()).unwrap_or(stored_api_base),
        image_base: std::env::var("TMDB_IMAGE_BASE").ok().filter(|v| !v.trim().is_empty()).unwrap_or(stored_image_base),
        tmdb_proxy,
        category_rules,
        scrape_targets,
        default_scrape_types,
    })
}

fn parse_json<T: for<'de> Deserialize<'de>>(value: Option<String>) -> Option<T> {
    value.and_then(|value| serde_json::from_str(&value).ok())
}

fn row_mapping(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrganizerMapping> {
    let scrape_types = row
        .get::<_, String>(10)
        .ok()
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default();
    Ok(OrganizerMapping {
        id: row.get(0)?,
        source_path: row.get(1)?,
        target_path: row.get(2)?,
        source_dir_id: row.get(3)?,
        target_dir_id: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        scan_existing: row.get::<_, i64>(6)? != 0,
        monitor_mode: "cloud_polling".to_string(),
        transfer_type: row.get(8)?,
        media_type: row.get(9)?,
        scrape: row.get::<_, i64>(7)? != 0,
        scrape_types,
        sync_extras: row.get::<_, i64>(11)? != 0,
        conflict_policy: row.get(12)?,
        auto_execute: row.get::<_, i64>(13)? != 0,
        share_after_organize: row.get::<_, i64>(14)? != 0,
        share_risk_acknowledged: row.get::<_, i64>(15)? != 0,
        settle_seconds: row.get::<_, i64>(16)?.max(5) as u64,
        watch_error: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

const MAPPING_SELECT: &str = "SELECT id, source_path, target_path, source_dir_id, target_dir_id,
    enabled, scan_existing, scrape, transfer_type, media_type, scrape_types, sync_extras,
    conflict_policy, auto_execute, share_after_organize, share_risk_acknowledged, settle_seconds,
    watch_error, created_at, updated_at FROM organizer_mappings";

fn list_mappings(path: &Path) -> Result<Vec<OrganizerMapping>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(&format!("{MAPPING_SELECT} WHERE id NOT LIKE 'manual:%' ORDER BY created_at"))
        .map_err(|error| format!("读取整理监控失败：{error}"))?;
    let result = statement
        .query_map([], row_mapping)
        .map_err(|error| format!("读取整理监控失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析整理监控失败：{error}"));
    result
}

fn get_mapping(path: &Path, id: &str) -> Result<Option<OrganizerMapping>, String> {
    open_database(path)?
        .query_row(
            &format!("{MAPPING_SELECT} WHERE id=?1"),
            params![id],
            row_mapping,
        )
        .optional()
        .map_err(|error| format!("读取整理监控失败：{error}"))
}

const JOB_SELECT: &str = "SELECT id, mapping_id, source_path, source_id, source_parent_id,
    source_size, source_modified_ms, source_file_count, source_signature, share_after_requested,
    status, media_type, tmdb_id, season, episode, episode_end, query_title, query_year,
    preview_json, result_json, error_code, message, created_at, updated_at FROM organizer_jobs";

fn row_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrganizerJob> {
    let tmdb_raw = row.get::<_, Option<String>>(12)?.unwrap_or_default();
    Ok(OrganizerJob {
        id: row.get(0)?,
        mapping_id: row.get(1)?,
        source_path: row.get(2)?,
        source_id: row.get(3)?,
        source_parent_id: row.get(4)?,
        source_size: row.get(5)?,
        source_modified_ms: row.get(6)?,
        source_file_count: row.get(7)?,
        source_signature: row.get(8)?,
        share_after_requested: row.get::<_, i64>(9)? != 0,
        status: row.get(10)?,
        media_type: row.get(11)?,
        tmdb_id: tmdb_raw.parse().ok(),
        season: row.get(13)?,
        episode: row.get(14)?,
        episode_end: row.get(15)?,
        query_title: row.get(16)?,
        query_year: row.get(17)?,
        preview: parse_json(row.get(18)?),
        result: parse_json(row.get(19)?),
        error_code: row.get(20)?,
        message: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

fn list_jobs(path: &Path) -> Result<Vec<OrganizerJob>, String> {
    let connection = open_database(path)?;
    let mut statement = connection
        .prepare(&format!("{JOB_SELECT} ORDER BY updated_at DESC LIMIT ?1"))
        .map_err(|error| format!("读取整理任务失败：{error}"))?;
    let result = statement
        .query_map(params![MAX_JOB_LIST], row_job)
        .map_err(|error| format!("读取整理任务失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析整理任务失败：{error}"));
    result
}

fn get_job(path: &Path, id: &str) -> Result<Option<OrganizerJob>, String> {
    open_database(path)?
        .query_row(&format!("{JOB_SELECT} WHERE id=?1"), params![id], row_job)
        .optional()
        .map_err(|error| format!("读取整理任务失败：{error}"))
}

fn database_path(state: &OrganizerSharedState) -> Result<PathBuf, String> {
    state
        .lock()
        .map(|runtime| runtime.db_path.clone())
        .map_err(|error| error.to_string())
}

fn emit(app: &tauri::AppHandle, event: &str, detail: Value) {
    let mut payload = json!({ "type": "organizer", "event": event });
    if let (Some(target), Some(source)) = (payload.as_object_mut(), detail.as_object()) {
        target.extend(source.clone());
    }
    let _ = app.emit("sync-event", payload);
}

fn update_job_fields(path: &Path, id: &str, fields: &[(&str, Value)]) -> Result<(), String> {
    if fields.is_empty() {
        return Ok(());
    }
    let connection = open_database(path)?;
    let sql = format!(
        "UPDATE organizer_jobs SET {}, updated_at=?{} WHERE id=?{}",
        fields
            .iter()
            .enumerate()
            .map(|(index, (key, _))| format!("{key}=?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", "),
        fields.len() + 1,
        fields.len() + 2
    );
    let mut values = fields
        .iter()
        .map(|(_, value)| match value {
            Value::Null => rusqlite::types::Value::Null,
            Value::Bool(value) => rusqlite::types::Value::Integer(i64::from(*value)),
            Value::Number(value) => value
                .as_i64()
                .map(rusqlite::types::Value::Integer)
                .or_else(|| value.as_f64().map(rusqlite::types::Value::Real))
                .unwrap_or_else(|| rusqlite::types::Value::Text(value.to_string())),
            Value::String(value) => rusqlite::types::Value::Text(value.clone()),
            Value::Array(_) | Value::Object(_) => rusqlite::types::Value::Text(value.to_string()),
        })
        .collect::<Vec<_>>();
    values.push(rusqlite::types::Value::Integer(now_seconds()));
    values.push(rusqlite::types::Value::Text(id.to_string()));
    connection
        .execute(&sql, rusqlite::params_from_iter(values.iter()))
        .map_err(|error| format!("更新整理任务失败：{error}"))?;
    Ok(())
}

fn auth_context(app: &tauri::AppHandle) -> Result<(String, String), String> {
    let state = app.state::<SharedState>();
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok((
        guard
            .token
            .clone()
            .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
        guard.device_id.clone(),
    ))
}

fn cloud_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn value_string(value: Option<&Value>) -> String {
    value
        .and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .or_else(|| value.as_i64().map(|number| number.to_string()))
                .or_else(|| value.as_u64().map(|number| number.to_string()))
        })
        .unwrap_or_default()
}

fn normalize_cloud_entry(value: &Value, logical_path: Option<&str>) -> CloudEntry {
    let name = value_string(cloud_value(value, &["fileName", "name"]));
    let resource_type = cloud_value(value, &["resType", "type"]);
    let is_directory = resource_type.and_then(Value::as_i64) == Some(2)
        || resource_type
            .and_then(Value::as_str)
            .is_some_and(|value| value == "2" || value == "folder")
        || value
            .get("isDirectory")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || value
            .get("is_directory")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let modified = cloud_value(
        value,
        &[
            "updatedAt",
            "updateTime",
            "modifiedAt",
            "modifyTime",
            "utime",
            "createdAt",
            "createTime",
            "ctime",
            "modified_ms",
        ],
    );
    CloudEntry {
        id: value_string(cloud_value(value, &["fileId", "id"])),
        parent_id: value_string(cloud_value(value, &["parentId", "parent_id"])),
        name: name.clone(),
        logical_path: logical_path
            .unwrap_or_else(|| value.get("path").and_then(Value::as_str).unwrap_or(&name))
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string(),
        is_directory,
        size: cloud_value(value, &["fileSize", "size"])
            .and_then(Value::as_i64)
            .or_else(|| {
                cloud_value(value, &["fileSize", "size"])
                    .and_then(Value::as_u64)
                    .map(|value| value.min(i64::MAX as u64) as i64)
            })
            .unwrap_or(0),
        modified_ms: value_string(modified),
    }
}

async fn list_cloud_children(
    app: &tauri::AppHandle,
    parent_id: &str,
) -> Result<Vec<CloudEntry>, String> {
    let (token, device_id) = auth_context(app)?;
    let mut result = Vec::new();
    for page in 0..200_u64 {
        let response = api_post(
            &token,
            &device_id,
            "/userres/v1/file/get_file_list",
            json!({ "page": page, "pageSize": 100, "parentId": parent_id, "orderBy": 0, "sortType": 0 }),
            &[],
        )
        .await?;
        let data = response.data.unwrap_or_else(|| json!({ "list": [] }));
        let page_items = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page_len = page_items.len();
        result.extend(
            page_items
                .iter()
                .map(|value| normalize_cloud_entry(value, None)),
        );
        let total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        if page_len < 100 || (total > 0 && result.len() >= total as usize) {
            break;
        }
    }
    Ok(result)
}

async fn create_cloud_directory(
    app: &tauri::AppHandle,
    parent_id: &str,
    name: &str,
) -> Result<CloudEntry, String> {
    if name.trim().is_empty() || name.chars().any(|value| "\\/:*?\"<>|".contains(value)) {
        return Err(format!("无效的云端目录名称：{name}"));
    }
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/create_dir",
        json!({ "parentId": parent_id, "dirName": name, "failIfNameExist": true }),
        &[],
    )
    .await?;
    let mut created = response
        .data
        .as_ref()
        .map(|value| normalize_cloud_entry(value, None))
        .unwrap_or_default();
    if created.id.is_empty() {
        created = list_cloud_children(app, parent_id)
            .await?
            .into_iter()
            .find(|entry| entry.is_directory && entry.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("创建云端目录后无法定位：{name}"))?;
    }
    Ok(created)
}

async fn cloud_copy(app: &tauri::AppHandle, id: &str, parent_id: &str) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/copy_file",
        json!({ "fileIds": [id], "parentId": parent_id }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map(|_| ())
}

async fn cloud_move(app: &tauri::AppHandle, id: &str, parent_id: &str) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/move_file",
        json!({ "fileIds": [id], "parentId": parent_id }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map(|_| ())
}

async fn cloud_delete(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": [id] }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map(|_| ())
}

async fn cloud_rename(app: &tauri::AppHandle, id: &str, name: &str) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/rename",
        json!({ "fileId": id, "newName": name }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response)
        .await
        .map(|_| ())
}

fn video_extension(value: &str) -> bool {
    let extension = path_extension(value);
    VIDEO_EXTENSIONS.contains(&extension.as_str())
}

fn useful_candidate(entry: &CloudEntry) -> bool {
    !entry.id.is_empty()
        && !entry.name.is_empty()
        && !entry.name.starts_with('.')
        && !entry.name.starts_with("~$")
        && (entry.is_directory || video_extension(&entry.name))
}

fn candidate_fingerprint(candidate: &CloudEntry, entries: &[CloudEntry]) -> CandidateFingerprint {
    let mut normalized = vec![candidate.clone()];
    normalized.extend_from_slice(entries);
    normalized.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    let files = normalized
        .iter()
        .filter(|entry| !entry.is_directory)
        .collect::<Vec<_>>();
    let signature_source = normalized
        .iter()
        .map(|entry| {
            json!([
                entry.id,
                entry.parent_id,
                entry.logical_path,
                entry.size,
                entry.modified_ms,
                entry.is_directory
            ])
        })
        .collect::<Vec<_>>();
    let modified_ms = normalized
        .iter()
        .filter_map(|entry| entry.modified_ms.parse::<u128>().ok())
        .max()
        .unwrap_or(0)
        .to_string();
    CandidateFingerprint {
        signature: hex::encode(Sha256::digest(
            serde_json::to_vec(&signature_source).unwrap_or_default(),
        )),
        size: files.iter().map(|entry| entry.size).sum(),
        modified_ms,
        file_count: files.len() as i64,
        video_count: files
            .iter()
            .filter(|entry| video_extension(&entry.name))
            .count() as i64,
    }
}

async fn load_candidate(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    candidate_id: &str,
) -> Result<Option<LoadedCandidate>, String> {
    let roots = list_cloud_children(app, &mapping.source_dir_id).await?;
    let Some(mut root) = roots.into_iter().find(|entry| entry.id == candidate_id) else {
        return Ok(None);
    };
    root.parent_id = mapping.source_dir_id.clone();
    root.logical_path = root.name.clone();
    let mut entries = Vec::new();
    if root.is_directory {
        let mut pending = VecDeque::from([(root.id.clone(), root.name.clone(), 0_usize)]);
        while let Some((parent_id, logical_path, depth)) = pending.pop_front() {
            if depth >= MAX_CLOUD_DEPTH {
                return Err(format!("云盘目录层级超过 {MAX_CLOUD_DEPTH} 层，已停止扫描"));
            }
            for mut child in list_cloud_children(app, &parent_id).await? {
                child.parent_id = parent_id.clone();
                child.logical_path = join_relative(&[&logical_path, &child.name]);
                if child.is_directory {
                    pending.push_back((child.id.clone(), child.logical_path.clone(), depth + 1));
                }
                entries.push(child);
                if entries.len() > MAX_CLOUD_ITEMS {
                    return Err(format!(
                        "单个整理候选超过 {MAX_CLOUD_ITEMS} 项，请缩小 A 目录范围"
                    ));
                }
            }
        }
    }
    let fingerprint = candidate_fingerprint(&root, &entries);
    Ok(Some(LoadedCandidate {
        candidate: root,
        entries,
        fingerprint,
    }))
}

fn ignored_sample(value: &str) -> bool {
    value
        .split('/')
        .any(|part| part.eq_ignore_ascii_case("sample") || part.to_lowercase().contains("sample"))
}

fn extra_kind(value: &str) -> String {
    let lower = value.to_lowercase();
    if lower.contains("trailer") || lower.contains("预告") {
        "trailer".to_string()
    } else if lower.contains("extras")
        || lower.contains("featurette")
        || lower.contains("behind the scenes")
        || lower.contains("花絮")
    {
        "extra".to_string()
    } else {
        String::new()
    }
}

fn sidecar_kind(value: &str) -> Option<&'static str> {
    match path_extension(value).as_str() {
        "ass" | "idx" | "smi" | "srt" | "ssa" | "sub" | "sup" | "vtt" => Some("subtitle"),
        "aac" | "ac3" | "dts" | "eac3" | "flac" | "m4a" | "mka" | "mp3" | "ogg" | "opus"
        | "wav" => Some("audio"),
        _ => None,
    }
}

fn best_sidecar_video<'a>(
    sidecar: &CloudEntry,
    videos: &'a [CloudEntry],
) -> Option<&'a CloudEntry> {
    let stem = path_stem(&sidecar.logical_path).to_lowercase();
    videos.iter().max_by_key(|video| {
        let other = path_stem(&video.logical_path).to_lowercase();
        stem.chars()
            .zip(other.chars())
            .take_while(|(left, right)| left == right)
            .count()
    })
}

fn analyze_cloud_candidate(
    loaded: &LoadedCandidate,
    overrides: &RecognitionOverrides,
) -> Result<(CandidateAnalysis, HashMap<String, CloudEntry>), String> {
    let files = if loaded.candidate.is_directory {
        loaded
            .entries
            .iter()
            .filter(|entry| !entry.is_directory)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        vec![loaded.candidate.clone()]
    };
    let mut videos = files
        .iter()
        .filter(|entry| video_extension(&entry.name) && !ignored_sample(&entry.logical_path))
        .cloned()
        .collect::<Vec<_>>();
    videos.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    if videos.is_empty() {
        return Err("没有找到可整理的视频文件".to_string());
    }
    let candidate_name = if loaded.candidate.is_directory {
        loaded.candidate.name.clone()
    } else {
        path_stem(&loaded.candidate.name)
    };
    let group = parse_media_name(&candidate_name, overrides);
    let mut preliminary = videos
        .iter()
        .map(|entry| {
            let mut options = overrides.clone();
            if options.media_type.as_deref().unwrap_or_default().is_empty() {
                options.media_type = Some(group.media_type.clone());
            }
            let mut parsed = parse_media_name(&entry.logical_path, &options);
            if parsed.title.is_empty() {
                parsed.title = group.title.clone();
            }
            parsed
        })
        .collect::<Vec<_>>();
    let inferred_tv = preliminary
        .iter()
        .filter(|parsed| parsed.season.is_some() || parsed.episode.is_some())
        .count()
        * 2
        > preliminary.len();
    let media_type = overrides
        .media_type
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if inferred_tv {
                "tv".to_string()
            } else {
                group.media_type.clone()
            }
        });
    let title = overrides
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| (!group.title.is_empty()).then(|| group.title.clone()))
        .or_else(|| {
            preliminary
                .iter()
                .find(|item| !item.title.is_empty())
                .map(|item| item.title.clone())
        })
        .unwrap_or_default();
    let year = overrides
        .year
        .or(group.year)
        .or_else(|| preliminary.iter().find_map(|item| item.year));
    for (index, parsed) in preliminary.iter_mut().enumerate() {
        parsed.media_type = media_type.clone();
        if parsed.title.is_empty() {
            parsed.title = title.clone();
        }
        parsed.year = year.or(parsed.year);
        if videos.len() == 1 {
            parsed.season = overrides.season.or(parsed.season);
            parsed.episode = overrides.episode.or(parsed.episode);
            parsed.episode_end = overrides.episode_end.or(parsed.episode_end);
        }
        let _ = index;
    }
    let analyzed_videos = videos
        .iter()
        .zip(preliminary)
        .map(|(entry, parsed)| AnalyzedVideo {
            source: entry.logical_path.clone(),
            parsed,
            extra_kind: extra_kind(&entry.logical_path),
        })
        .collect::<Vec<_>>();
    let sidecars = files
        .iter()
        .filter_map(|entry| sidecar_kind(&entry.name).map(|kind| (entry, kind)))
        .filter_map(|(entry, kind)| {
            let video = best_sidecar_video(entry, &videos)?;
            Some(AnalyzedSidecar {
                source: entry.logical_path.clone(),
                kind: kind.to_string(),
                video_source: Some(video.logical_path.clone()),
            })
        })
        .collect::<Vec<_>>();
    let ignored_samples = files
        .iter()
        .filter(|entry| video_extension(&entry.name) && ignored_sample(&entry.logical_path))
        .map(|entry| entry.logical_path.clone())
        .collect::<Vec<_>>();
    let source_map = files
        .into_iter()
        .map(|entry| (entry.logical_path.clone(), entry))
        .collect();
    Ok((
        CandidateAnalysis {
            candidate_path: loaded.candidate.logical_path.clone(),
            candidate_type: if loaded.candidate.is_directory {
                "dir".to_string()
            } else {
                "file".to_string()
            },
            media_type: media_type.clone(),
            title: title.clone(),
            year,
            videos: analyzed_videos,
            sidecars,
            ignored_samples,
            query: MediaQuery {
                title,
                year,
                media_type,
            },
        },
        source_map,
    ))
}

fn render_path_template(
    template: &str,
    context: &HashMap<String, String>,
) -> Result<String, String> {
    // Keep the aliases accepted by the web engine, including the common
    // mixed-case forms users copy from MoviePilot-style templates.
    let aliases = Regex::new(r"(?i)\{catgroy\}")
        .expect("category alias regex")
        .replace_all(template, "{category}")
        .to_string();
    let aliases = Regex::new(r"(?i)\{tmdbid\}")
        .expect("tmdb alias regex")
        .replace_all(&aliases, "{tmdb_id}")
        .to_string();
    let aliases = Regex::new(r"(?i)\{season\s+x\}")
        .expect("season alias regex")
        .replace_all(&aliases, "{season_tag}")
        .to_string();
    let aliases = Regex::new(r"(?i)\{(?:episode|expose)\s+n\}")
        .expect("episode alias regex")
        .replace_all(&aliases, "{episode_tag}")
        .to_string();
    let token = Regex::new(r"\{([A-Za-z_]+)(?::(\d+))?\}").expect("template regex");
    let rendered = token
        .replace_all(&aliases, |captures: &regex::Captures| {
            let key = captures
                .get(1)
                .map(|value| value.as_str().to_lowercase())
                .unwrap_or_default();
            let value = context.get(&key).cloned().unwrap_or_default();
            captures
                .get(2)
                .and_then(|value| value.as_str().parse::<usize>().ok())
                .map(|width| format!("{value:0>width$}"))
                .unwrap_or(value)
        })
        .to_string();
    let normalized_rendered = rendered.replace('\\', "/");
    let raw_parts = normalized_rendered
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if raw_parts.iter().any(|part| *part == "." || *part == "..") {
        return Err("整理路径模板不能包含相对目录跳转".to_string());
    }
    let parts = raw_parts
        .into_iter()
        .map(|part| {
            sanitize_component(
                &part
                    .replace("()", "")
                    .replace("[]", "")
                    .replace("..", ".")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                "Unknown",
            )
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err("整理路径模板必须至少包含一个目录和一个文件名".to_string());
    }
    Ok(parts.join("/"))
}

fn template_context(
    metadata: &MediaMetadata,
    parsed: &crate::organizer_core::ParsedMediaName,
    episode_title: &str,
    category: &str,
    extension: &str,
) -> HashMap<String, String> {
    let season = parsed
        .season
        .map(|value| value.to_string())
        .unwrap_or_default();
    let episode = parsed
        .episode
        .map(|value| value.to_string())
        .unwrap_or_default();
    let episode_end = parsed
        .episode_end
        .filter(|value| Some(*value) != parsed.episode)
        .map(|value| format!("-E{value:02}"))
        .unwrap_or_default();
    let mut context = HashMap::new();
    context.insert("category".to_string(), category.to_string());
    context.insert("catgroy".to_string(), category.to_string());
    context.insert(
        "country".to_string(),
        metadata
            .countries
            .first()
            .cloned()
            .unwrap_or_else(|| "未知地区".to_string()),
    );
    context.insert(
        "year".to_string(),
        metadata
            .year
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    context.insert("title".to_string(), metadata.title.clone());
    context.insert(
        "original_title".to_string(),
        metadata.original_title.clone(),
    );
    context.insert("tmdb_id".to_string(), metadata.tmdb_id.to_string());
    context.insert("season".to_string(), season.clone());
    context.insert("episode".to_string(), episode.clone());
    context.insert("episode_end".to_string(), episode_end);
    context.insert("episode_title".to_string(), episode_title.to_string());
    context.insert(
        "edition".to_string(),
        if parsed.edition.is_empty() {
            String::new()
        } else {
            format!(" - {}", parsed.edition)
        },
    );
    context.insert(
        "quality".to_string(),
        if parsed.quality.is_empty() {
            String::new()
        } else {
            format!(" - {}", parsed.quality)
        },
    );
    context.insert(
        "part".to_string(),
        parsed
            .part
            .clone()
            .map(|value| format!(" - {value}"))
            .unwrap_or_default(),
    );
    context.insert(
        "ext".to_string(),
        extension.trim_start_matches('.').to_lowercase(),
    );
    context.insert(
        "season_tag".to_string(),
        parsed
            .season
            .map(|value| format!("S{value:02}"))
            .unwrap_or_default(),
    );
    context.insert(
        "episode_tag".to_string(),
        parsed
            .episode
            .map(|value| format!("E{value:02}"))
            .unwrap_or_default(),
    );
    context
}

fn cloud_target(mapping: &OrganizerMapping, relative: &str) -> String {
    join_relative(&[&mapping.target_path, relative])
}

fn target_key(value: &str) -> String {
    value.trim_matches('/').to_lowercase()
}

fn short_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))[..8].to_string()
}

fn common_target_directory(paths: &[String]) -> String {
    let parts = paths
        .iter()
        .map(|value| {
            path_parent(value)
                .split('/')
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return String::new();
    }
    let mut common = Vec::new();
    for index in 0..parts.iter().map(Vec::len).min().unwrap_or(0) {
        let value = &parts[0][index];
        if parts
            .iter()
            .all(|current| current.get(index) == Some(value))
        {
            common.push(value.clone());
        } else {
            break;
        }
    }
    common.join("/")
}

fn is_season_directory_name(value: &str, seasons: &[i64]) -> bool {
    let normalized = value
        .trim()
        .to_lowercase()
        .replace(['.', '_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    seasons.iter().any(|season| {
        let number = season.to_string();
        let padded = format!("{season:02}");
        normalized == number
            || normalized == padded
            || normalized == format!("s{number}")
            || normalized == format!("s{padded}")
            || normalized == format!("season {number}")
            || normalized == format!("season {padded}")
            || normalized == format!("第{number}季")
            || normalized == format!("第 {number} 季")
    })
}

fn media_root_for_cloud_targets(media_type: &str, items: &[CloudPreviewItem]) -> String {
    let common = common_target_directory(
        &items
            .iter()
            .map(|item| item.target_relative.clone())
            .collect::<Vec<_>>(),
    );
    if media_type != "tv" || common.is_empty() {
        return common;
    }
    let seasons = items
        .iter()
        .filter_map(|item| item.season)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let parts = path_parts(&common);
    if parts.len() > 1
        && is_season_directory_name(parts.last().copied().unwrap_or_default(), &seasons)
    {
        return parts[..parts.len() - 1].join("/");
    }
    common
}

fn season_directory_for_cloud_video(item: &CloudPreviewItem, media_root: &str) -> String {
    let directory = path_parent(&item.target_relative);
    if directory.is_empty() || directory == media_root {
        media_root.to_string()
    } else {
        directory
    }
}

async fn resolve_target(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
) -> Result<Option<CloudEntry>, String> {
    let mut parent_id = mapping.target_dir_id.clone();
    let mut current = None;
    for part in path_parts(relative) {
        let children = list_cloud_children(app, &parent_id).await?;
        let Some(entry) = children
            .into_iter()
            .find(|entry| entry.name == part || entry.name.eq_ignore_ascii_case(part))
        else {
            return Ok(None);
        };
        if current.is_some()
            && !current
                .as_ref()
                .is_some_and(|entry: &CloudEntry| entry.is_directory)
        {
            return Ok(None);
        }
        parent_id = entry.id.clone();
        current = Some(entry);
    }
    Ok(current)
}

async fn ensure_target_directory(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
) -> Result<String, String> {
    let mut parent_id = mapping.target_dir_id.clone();
    for part in path_parts(relative) {
        let children = list_cloud_children(app, &parent_id).await?;
        if let Some(entry) = children
            .into_iter()
            .find(|entry| entry.name == part || entry.name.eq_ignore_ascii_case(part))
        {
            if !entry.is_directory {
                return Err(format!("目标路径包含同名文件：{part}"));
            }
            parent_id = entry.id;
        } else {
            parent_id = create_cloud_directory(app, &parent_id, part).await?.id;
        }
    }
    Ok(parent_id)
}

async fn plan_target(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
    source_identity: &str,
    claimed: &mut HashSet<String>,
) -> Result<(String, String, bool, Option<String>, bool), String> {
    let normalized = relative.trim_matches('/').replace('\\', "/");
    let mut target_relative = normalized.clone();
    let key = target_key(&target_relative);
    let existing = if claimed.contains(&key) {
        None
    } else {
        resolve_target(app, mapping, &target_relative).await?
    };
    if !claimed.contains(&key) && existing.is_none() {
        claimed.insert(key);
        return Ok((target_relative, "create".to_string(), false, None, false));
    }
    if !claimed.contains(&key) && mapping.conflict_policy == "skip" {
        claimed.insert(key);
        return Ok((
            target_relative,
            "skip".to_string(),
            true,
            existing.map(|entry| entry.id),
            false,
        ));
    }
    if !claimed.contains(&key) && mapping.conflict_policy == "overwrite" {
        claimed.insert(key);
        return Ok((
            target_relative,
            "overwrite".to_string(),
            true,
            existing.map(|entry| entry.id),
            false,
        ));
    }
    let extension = path_extension(&target_relative);
    let stem = if extension.is_empty() {
        target_relative.clone()
    } else {
        target_relative[..target_relative.len().saturating_sub(extension.len() + 1)].to_string()
    };
    for index in 0..10_000_u32 {
        let suffix = if index == 0 {
            short_hash(source_identity)
        } else {
            format!("{}-{}", short_hash(source_identity), index + 1)
        };
        target_relative = if extension.is_empty() {
            format!("{stem} [{suffix}]")
        } else {
            format!("{stem} [{suffix}].{extension}")
        };
        let key = target_key(&target_relative);
        if claimed.contains(&key)
            || resolve_target(app, mapping, &target_relative)
                .await?
                .is_some()
        {
            continue;
        }
        claimed.insert(key);
        return Ok((target_relative, "create".to_string(), false, None, true));
    }
    Err("目标目录同名文件过多，无法生成安全名称".to_string())
}

fn add_preview_item(
    items: &mut Vec<CloudPreviewItem>,
    mapping: &OrganizerMapping,
    source: Option<String>,
    source_id: Option<String>,
    source_parent_id: Option<String>,
    source_name: Option<String>,
    kind: &str,
    _target_relative: String,
    operation: &str,
    planned: (String, String, bool, Option<String>, bool),
    generator: Option<GeneratorSpec>,
    image_role: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
    message: String,
) {
    let (target_relative, action, exists, existing_id, renamed_for_conflict) = planned;
    let target_name = path_name(&target_relative);
    let target_parent_relative = path_parent(&target_relative);
    items.push(CloudPreviewItem {
        success: true,
        kind: kind.to_string(),
        source,
        source_id,
        source_parent_id,
        source_name,
        target: cloud_target(mapping, &target_relative),
        target_relative,
        target_parent_relative,
        target_name,
        operation: operation.to_string(),
        action,
        exists,
        existing_id,
        renamed_for_conflict,
        error_code: None,
        message,
        generator,
        image_role,
        season,
        episode,
        episode_end,
    });
}

fn mapping_signature(mapping: &OrganizerMapping, secrets: &OrganizerSecrets) -> String {
    let payload = json!([
        mapping.source_dir_id,
        mapping.target_dir_id,
        mapping.transfer_type,
        mapping.media_type,
        mapping.scrape,
        mapping.scrape_types,
        mapping.sync_extras,
        mapping.conflict_policy,
        mapping.share_after_organize,
        secrets.native.language,
        secrets.native.image_language,
        secrets.movie_path_template,
        secrets.tv_path_template,
        secrets.movie_category,
        secrets.tv_category,
        secrets.api_base,
        secrets.image_base,
        secrets.category_rules,
        secrets.scrape_targets,
        secrets.default_scrape_types
    ]);
    hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).unwrap_or_default(),
    ))
}

fn episode_title(metadata: &MediaMetadata, season: Option<i64>, episode: Option<i64>) -> String {
    season
        .and_then(|season| metadata.seasons.get(&season.to_string()))
        .and_then(|season| {
            episode.and_then(|episode| {
                season
                    .episodes
                    .iter()
                    .find(|item| item.episode_number == episode)
            })
        })
        .map(|episode| episode.name.clone())
        .unwrap_or_default()
}

async fn build_preview(
    app: &tauri::AppHandle,
    loaded: &LoadedCandidate,
    analysis: &CandidateAnalysis,
    match_result: &MatchResolution,
    mapping: &OrganizerMapping,
    secrets: &OrganizerSecrets,
) -> Result<CloudPreview, String> {
    let mapping_signature = mapping_signature(mapping, secrets);
    if !match_result.ready || match_result.metadata.is_none() {
        return Ok(CloudPreview {
            success: false,
            engine: NATIVE_ENGINE_VERSION.to_string(),
            mapping_signature,
            source_signature: loaded.fingerprint.signature.clone(),
            query: match_result.query.clone(),
            candidates: match_result.candidates.clone(),
            selected: None,
            metadata: None,
            error_code: match_result.error_code.clone(),
            message: match_result.message.clone(),
            data: CloudPreviewData::default(),
            ..Default::default()
        });
    }
    let metadata = match_result.metadata.clone().expect("metadata checked");
    let template = if metadata.media_type == "tv" {
        &secrets.tv_path_template
    } else {
        &secrets.movie_path_template
    };
    let category = resolve_media_category(&metadata, secrets);
    let mut claimed = HashSet::new();
    let mut items = Vec::new();
    let mut video_targets = HashMap::new();
    for video in &analysis.videos {
        if metadata.media_type == "tv"
            && video.extra_kind.is_empty()
            && (video.parsed.season.is_none() || video.parsed.episode.is_none())
        {
            items.push(CloudPreviewItem {
                success: false,
                kind: "video".to_string(),
                source: Some(video.source.clone()),
                source_id: source_id_for(loaded, &video.source),
                target: String::new(),
                target_relative: String::new(),
                operation: mapping.transfer_type.clone(),
                action: "error".to_string(),
                error_code: Some("episode_required".to_string()),
                message: "未识别到季集号，请人工填写季号/集号或调整文件名".to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(source_entry) = loaded_entry(loaded, &video.source) else {
            continue;
        };
        let details = episode_title(&metadata, video.parsed.season, video.parsed.episode);
        let context = template_context(
            &metadata,
            &video.parsed,
            &details,
            &category,
            &path_extension(&source_entry.name),
        );
        let mut relative = render_path_template(template, &context)?;
        if !video.extra_kind.is_empty() {
            let extra_directory = if video.extra_kind == "trailer" {
                "trailers"
            } else {
                "extras"
            };
            relative = join_relative(&[
                &path_parent(&relative),
                extra_directory,
                &sanitize_component(&source_entry.name, "extra"),
            ]);
        }
        let planned = plan_target(app, mapping, &relative, &source_entry.id, &mut claimed).await?;
        let target_relative = planned.0.clone();
        add_preview_item(
            &mut items,
            mapping,
            Some(video.source.clone()),
            Some(source_entry.id.clone()),
            Some(source_entry.parent_id.clone()),
            Some(source_entry.name.clone()),
            if video.extra_kind.is_empty() {
                "video"
            } else {
                &video.extra_kind
            },
            target_relative.clone(),
            &mapping.transfer_type,
            planned,
            None,
            None,
            video.parsed.season,
            video.parsed.episode,
            video.parsed.episode_end,
            "可执行".to_string(),
        );
        video_targets.insert(video.source.clone(), target_relative);
    }
    if mapping.sync_extras {
        for sidecar in &analysis.sidecars {
            let Some(video_target) = sidecar
                .video_source
                .as_ref()
                .and_then(|source| video_targets.get(source))
            else {
                continue;
            };
            let Some(source_entry) = loaded_entry(loaded, &sidecar.source) else {
                continue;
            };
            let extension = path_extension(&source_entry.name);
            let target_stem = if let Some(dot) = video_target.rfind('.') {
                &video_target[..dot]
            } else {
                video_target
            };
            let relative = format!(
                "{}{}{}",
                target_stem,
                language_suffix(&source_entry.name),
                if extension.is_empty() {
                    String::new()
                } else {
                    format!(".{extension}")
                }
            );
            let planned =
                plan_target(app, mapping, &relative, &source_entry.id, &mut claimed).await?;
            add_preview_item(
                &mut items,
                mapping,
                Some(sidecar.source.clone()),
                Some(source_entry.id.clone()),
                Some(source_entry.parent_id.clone()),
                Some(source_entry.name.clone()),
                &sidecar.kind,
                planned.0.clone(),
                &mapping.transfer_type,
                planned,
                None,
                None,
                None,
                None,
                None,
                "跟随主视频整理".to_string(),
            );
        }
    }
    let main_videos = items
        .iter()
        .filter(|item| item.success && item.kind == "video")
        .cloned()
        .collect::<Vec<_>>();
    let media_root_relative = media_root_for_cloud_targets(&metadata.media_type, &main_videos);
    let scrape_types = if mapping.scrape {
        mapping.scrape_types.iter().cloned().collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let mut generated = Vec::new();
    if !scrape_types.is_empty() && !media_root_relative.is_empty() {
        if metadata.media_type == "movie" && scrape_types.contains("movie_nfo") {
            for video in &main_videos {
                let stem = video
                    .target_relative
                    .rfind('.')
                    .map(|index| video.target_relative[..index].to_string())
                    .unwrap_or_else(|| video.target_relative.clone());
                generated.push((
                    format!("{stem}.nfo"),
                    "nfo",
                    None,
                    Some(GeneratorSpec {
                        generator_type: "movie".to_string(),
                        season: None,
                        episode: None,
                    }),
                    None,
                    None,
                    None,
                    "生成电影 NFO",
                ));
            }
        }
        if metadata.media_type == "tv" && scrape_types.contains("tvshow_nfo") {
            generated.push((
                format!("{media_root_relative}/tvshow.nfo"),
                "nfo",
                None,
                Some(GeneratorSpec {
                    generator_type: "tvshow".to_string(),
                    season: None,
                    episode: None,
                }),
                None,
                None,
                None,
                "生成剧集 NFO",
            ));
        }
        if metadata.media_type == "tv" && scrape_types.contains("episode_nfo") {
            for video in &main_videos {
                let stem = video
                    .target_relative
                    .rfind('.')
                    .map(|index| video.target_relative[..index].to_string())
                    .unwrap_or_else(|| video.target_relative.clone());
                generated.push((
                    format!("{stem}.nfo"),
                    "nfo",
                    None,
                    Some(GeneratorSpec {
                        generator_type: "episode".to_string(),
                        season: video.season,
                        episode: video.episode,
                    }),
                    None,
                    video.season,
                    video.episode,
                    "生成单集 NFO",
                ));
            }
        }
        if scrape_types.contains("poster") && !metadata.poster_url.is_empty() {
            generated.push((
                format!("{media_root_relative}/poster.jpg"),
                "image",
                Some(metadata.poster_url.clone()),
                None,
                Some("poster".to_string()),
                None,
                None,
                "下载海报",
            ));
        }
        if scrape_types.contains("fanart") && !metadata.backdrop_url.is_empty() {
            generated.push((
                format!("{media_root_relative}/fanart.jpg"),
                "image",
                Some(metadata.backdrop_url.clone()),
                None,
                Some("fanart".to_string()),
                None,
                None,
                "下载背景图",
            ));
        }
        if metadata.media_type == "tv" && scrape_types.contains("season_poster") {
            for season in metadata.seasons.values() {
                if season.poster_url.is_empty() {
                    continue;
                }
                let season_root = main_videos
                    .iter()
                    .find(|item| item.season == Some(season.season_number))
                    .map(|item| season_directory_for_cloud_video(item, &media_root_relative))
                    .unwrap_or_else(|| media_root_relative.clone());
                generated.push((
                    format!("{season_root}/poster.jpg"),
                    "image",
                    Some(season.poster_url.clone()),
                    None,
                    Some("season-poster".to_string()),
                    Some(season.season_number),
                    None,
                    "下载季海报",
                ));
            }
        }
    }
    for (relative, kind, source, generator, image_role, season, episode, message) in generated {
        let planned = plan_target(
            app,
            mapping,
            &relative,
            source.as_deref().unwrap_or(&relative),
            &mut claimed,
        )
        .await?;
        add_preview_item(
            &mut items,
            mapping,
            source.clone(),
            None,
            None,
            Some(path_name(&relative)),
            kind,
            planned.0.clone(),
            if kind == "nfo" {
                "generate"
            } else {
                "download"
            },
            planned,
            generator,
            image_role,
            season,
            episode,
            None,
            message.to_string(),
        );
    }
    let skipped = items.iter().filter(|item| item.action == "skip").count();
    let failed_items = items.iter().filter(|item| !item.success).count();
    let warnings = skipped + analysis.ignored_samples.len();
    let share_title = format!(
        "{}{}",
        metadata.title,
        metadata
            .year
            .map(|value| format!(" ({value})"))
            .unwrap_or_default()
    );
    Ok(CloudPreview {
        success: failed_items == 0 && !main_videos.is_empty(),
        engine: NATIVE_ENGINE_VERSION.to_string(),
        mapping_signature,
        source_signature: loaded.fingerprint.signature.clone(),
        query: match_result.query.clone(),
        candidates: match_result.candidates.clone(),
        selected: match_result.selected.clone(),
        metadata: Some(metadata),
        target_root: mapping.target_path.clone(),
        target_root_id: mapping.target_dir_id.clone(),
        media_root: cloud_target(mapping, &media_root_relative),
        media_root_relative: media_root_relative.clone(),
        share_relative_path: media_root_relative,
        share_title,
        error_code: if failed_items > 0 {
            Some("preview_failed".to_string())
        } else {
            None
        },
        message: if failed_items > 0 {
            format!("有 {failed_items} 项无法生成目标，请人工修正")
        } else {
            format!(
                "已生成 {} 项云端整理预览{}",
                items.len(),
                if warnings > 0 {
                    format!("，{warnings} 项提示")
                } else {
                    String::new()
                }
            )
        },
        ignored_samples: analysis.ignored_samples.clone(),
        data: CloudPreviewData {
            summary: PreviewSummary {
                total: items.len(),
                success: items.len().saturating_sub(failed_items),
                failed: failed_items,
                warnings,
                skipped,
            },
            items,
        },
    })
}

fn loaded_entry<'a>(loaded: &'a LoadedCandidate, source: &str) -> Option<&'a CloudEntry> {
    if loaded.candidate.logical_path == source {
        Some(&loaded.candidate)
    } else {
        loaded
            .entries
            .iter()
            .find(|entry| entry.logical_path == source)
    }
}

fn source_id_for(loaded: &LoadedCandidate, source: &str) -> Option<String> {
    loaded_entry(loaded, source).map(|entry| entry.id.clone())
}

fn language_suffix(value: &str) -> String {
    let stem = path_stem(value);
    let tokens = stem
        .split(['.', ' ', '_', '-'])
        .filter(|token| {
            let lower = token.to_lowercase();
            lower.len() >= 2
                && lower.len() <= 8
                && (lower == "chs"
                    || lower == "cht"
                    || lower == "eng"
                    || lower == "jpn"
                    || lower == "kor"
                    || lower == "zh"
                    || lower == "en"
                    || lower == "cn"
                    || lower == "tw")
        })
        .last()
        .map(|token| format!(".{token}"))
        .unwrap_or_default();
    tokens
}

#[derive(Debug, Clone)]
struct TransferStep {
    operation: String,
    created_id: String,
    source_parent_id: String,
    source_name: String,
    target_name: String,
    backup: Option<(String, String)>,
}

async fn locate_transferred(
    app: &tauri::AppHandle,
    parent_id: &str,
    source_id: &str,
    source_name: &str,
    before: &HashSet<String>,
    operation: &str,
) -> Result<CloudEntry, String> {
    let children = list_cloud_children(app, parent_id).await?;
    let found = if operation == "move" {
        children.into_iter().find(|entry| entry.id == source_id)
    } else {
        children
            .into_iter()
            .find(|entry| entry.name == source_name && !before.contains(&entry.id))
    };
    found.ok_or_else(|| {
        format!(
            "云端{}已完成，但无法定位目标资源",
            if operation == "move" {
                "移动"
            } else {
                "复制"
            }
        )
    })
}

async fn rollback_transfers(app: &tauri::AppHandle, steps: &[TransferStep]) -> Vec<String> {
    let mut warnings = Vec::new();
    for step in steps.iter().rev() {
        let result = if step.operation == "copy" {
            cloud_delete(app, &step.created_id).await
        } else {
            match cloud_move(app, &step.created_id, &step.source_parent_id).await {
                Ok(()) if step.target_name != step.source_name => {
                    cloud_rename(app, &step.created_id, &step.source_name).await
                }
                Ok(()) => Ok(()),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            warnings.push(format!("{} 回滚失败：{error}", step.source_name));
        }
        if let Some((backup_id, original_name)) = &step.backup {
            if let Err(error) = cloud_rename(app, backup_id, original_name).await {
                warnings.push(format!("恢复覆盖备份 {} 失败：{error}", original_name));
            }
        }
    }
    warnings
}

async fn execute_transfers(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    preview: &CloudPreview,
) -> Result<(usize, usize, Vec<String>), String> {
    let mut transaction = Vec::new();
    let mut transferred = 0usize;
    let mut skipped = 0usize;
    let candidates = preview
        .data
        .items
        .iter()
        .filter(|item| {
            item.success
                && item.source_id.is_some()
                && matches!(
                    item.kind.as_str(),
                    "video" | "subtitle" | "audio" | "trailer" | "extra"
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    for item in candidates {
        let source_id = item.source_id.clone().unwrap_or_default();
        let target_parent_id =
            match ensure_target_directory(app, mapping, &item.target_parent_relative).await {
                Ok(value) => value,
                Err(error) => {
                    let rollback = rollback_transfers(app, &transaction).await;
                    return Err(format_with_rollback(error, rollback));
                }
            };
        let existing = match resolve_target(app, mapping, &item.target_relative).await {
            Ok(value) => value,
            Err(error) => {
                let rollback = rollback_transfers(app, &transaction).await;
                return Err(format_with_rollback(error, rollback));
            }
        };
        if item.action == "skip" && existing.is_some() {
            skipped += 1;
            continue;
        }
        if item.action == "create" && existing.is_some() {
            let rollback = rollback_transfers(app, &transaction).await;
            return Err(format_with_rollback(
                format!("预览后目标已出现同名项目：{}", item.target),
                rollback,
            ));
        }
        let mut backup = None;
        if let Some(existing) = existing {
            let backup_name = format!(".__gy_org_backup_{}", Uuid::new_v4().simple());
            if let Err(error) = cloud_rename(app, &existing.id, &backup_name).await {
                let rollback = rollback_transfers(app, &transaction).await;
                return Err(format_with_rollback(
                    format!("覆盖前备份已有目标失败：{error}"),
                    rollback,
                ));
            }
            backup = Some((existing.id, existing.name));
        }
        let before = list_cloud_children(app, &target_parent_id)
            .await
            .map(|children| {
                children
                    .into_iter()
                    .map(|entry| entry.id)
                    .collect::<HashSet<_>>()
            });
        let before = match before {
            Ok(value) => value,
            Err(error) => {
                let rollback = rollback_transfers(app, &transaction).await;
                return Err(format_with_rollback(error, rollback));
            }
        };
        let operation = mapping.transfer_type.as_str();
        let operation_result = if operation == "move" {
            cloud_move(app, &source_id, &target_parent_id).await
        } else {
            cloud_copy(app, &source_id, &target_parent_id).await
        };
        if let Err(error) = operation_result {
            let rollback = rollback_transfers(app, &transaction).await;
            return Err(format_with_rollback(
                format!(
                    "{}失败：{error}",
                    if operation == "move" {
                        "移动云端文件"
                    } else {
                        "复制云端文件"
                    }
                ),
                rollback,
            ));
        }
        let created = match locate_transferred(
            app,
            &target_parent_id,
            &source_id,
            &item
                .source_name
                .clone()
                .unwrap_or_else(|| path_name(item.source.as_deref().unwrap_or_default())),
            &before,
            operation,
        )
        .await
        {
            Ok(value) => value,
            Err(error) => {
                let rollback = rollback_transfers(app, &transaction).await;
                return Err(format_with_rollback(error, rollback));
            }
        };
        if created.name != item.target_name {
            if let Err(error) = cloud_rename(app, &created.id, &item.target_name).await {
                let rollback = rollback_transfers(app, &transaction).await;
                return Err(format_with_rollback(
                    format!("重命名整理目标失败：{error}"),
                    rollback,
                ));
            }
        }
        transaction.push(TransferStep {
            operation: operation.to_string(),
            created_id: created.id.clone(),
            source_parent_id: item.source_parent_id.clone().unwrap_or_default(),
            source_name: item.source_name.clone().unwrap_or_default(),
            target_name: item.target_name.clone(),
            backup,
        });
        transferred += 1;
    }
    for step in &transaction {
        if let Some((backup_id, _)) = &step.backup {
            if let Err(error) = cloud_delete(app, backup_id).await {
                return Err(format!("新文件已落库，但清理覆盖备份失败：{error}"));
            }
        }
    }
    Ok((
        transferred,
        skipped,
        transaction
            .iter()
            .map(|step| step.created_id.clone())
            .collect(),
    ))
}

fn format_with_rollback(error: String, warnings: Vec<String>) -> String {
    if warnings.is_empty() {
        error
    } else {
        format!("{error}；{}", warnings.join("；"))
    }
}

async fn download_bytes(url: &str, proxy: &str) -> Result<Vec<u8>, String> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if !proxy.trim().is_empty() {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy.trim())
                .map_err(|error| format!("初始化刮削代理失败：{error}"))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| format!("创建刮削下载客户端失败：{error}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载刮削图片失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载刮削图片失败（HTTP {}）", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > 25 * 1024 * 1024)
    {
        return Err("刮削图片超过 25 MB，已跳过".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取刮削图片失败：{error}"))?;
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("刮削图片超过 25 MB，已跳过".to_string());
    }
    Ok(bytes.to_vec())
}

async fn execute_scrape(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    preview: &CloudPreview,
    proxy: &str,
) -> (usize, usize, Vec<String>) {
    let Some(metadata) = preview.metadata.as_ref() else {
        return (0, 0, vec!["没有可用的 TMDB 元数据，已跳过刮削".to_string()]);
    };
    let generated = preview
        .data
        .items
        .iter()
        .filter(|item| {
            item.success && item.source_id.is_none() && (item.kind == "nfo" || item.kind == "image")
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut scraped = 0usize;
    let mut skipped = 0usize;
    let mut warnings = Vec::new();
    for item in generated {
        let parent_id =
            match ensure_target_directory(app, mapping, &item.target_parent_relative).await {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(format!("{}：{error}", item.target));
                    continue;
                }
            };
        let existing = match resolve_target(app, mapping, &item.target_relative).await {
            Ok(value) => value,
            Err(error) => {
                warnings.push(format!("{}：{error}", item.target));
                continue;
            }
        };
        if item.action == "skip" && existing.is_some() {
            skipped += 1;
            continue;
        }
        let mut backup = None;
        if let Some(existing) = existing {
            let backup_name = format!(".__gy_org_meta_{}", Uuid::new_v4().simple());
            if let Err(error) = cloud_rename(app, &existing.id, &backup_name).await {
                warnings.push(format!("{}：覆盖前备份失败：{error}", item.target));
                continue;
            }
            backup = Some((existing.id, existing.name));
        }
        let bytes = if item.kind == "nfo" {
            match item.generator.as_ref() {
                Some(generator) => render_nfo(generator, metadata).into_bytes(),
                None => {
                    warnings.push(format!("{}：NFO 生成器缺失", item.target));
                    continue;
                }
            }
        } else {
            let Some(source) = item.source.as_deref() else {
                if let Some((backup_id, original_name)) = backup {
                    let _ = cloud_rename(app, &backup_id, &original_name).await;
                }
                warnings.push(format!("{}：刮削资源地址为空", item.target));
                continue;
            };
            match download_bytes(source, proxy).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    if let Some((backup_id, original_name)) = backup {
                        let _ = cloud_rename(app, &backup_id, &original_name).await;
                    }
                    warnings.push(format!("{}：{error}", item.target));
                    continue;
                }
            }
        };
        if let Err(error) = organizer_upload_bytes(app, &parent_id, &item.target_name, &bytes).await
        {
            if let Some((backup_id, original_name)) = backup {
                let _ = cloud_rename(app, &backup_id, &original_name).await;
            }
            warnings.push(format!("{}：上传刮削元数据失败：{error}", item.target));
            continue;
        }
        if let Some((backup_id, _)) = backup {
            let _ = cloud_delete(app, &backup_id).await;
        }
        scraped += 1;
    }
    (scraped, skipped, warnings)
}

async fn create_fresh_organizer_share(
    app: tauri::AppHandle,
    mapping_id: &str,
    target_id: &str,
    title: &str,
) -> Result<Value, String> {
    let state = app.state::<SharedState>();
    let (token, device_id, hdhive_enabled, base_url, secret, instance_id, db_path) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard
                .token
                .clone()
                .ok_or_else(|| "尚未登录光鸭云盘".to_string())?,
            guard.device_id.clone(),
            guard.hdhive_enabled,
            guard.hdhive_base_url.clone(),
            guard.hdhive_secret.clone(),
            guard.hdhive_instance_id.clone(),
            guard.db_path.clone(),
        )
    };
    let data = api_post(
        &token,
        &device_id,
        "/userres/v1/share_file",
        share_file_payload(&[target_id.to_string()], title, 0, "", false),
        &[],
    )
    .await?
    .data
    .ok_or_else(|| "光鸭没有返回分享信息".to_string())?;
    let share_url = ["shareUrl", "shareURL", "share_url", "url"]
        .iter()
        .find_map(|key| data.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let share_id = share_id_for_hdhive(&data, &share_url);
    if share_url.is_empty() || share_id.is_empty() {
        return Err("光鸭没有返回完整分享链接".to_string());
    }
    if !hdhive_enabled {
        return Ok(json!({
            "share_url": share_url,
            "share_id": share_id,
            "hdhive_status": "disabled",
            "message": "HDHive 已关闭，仅创建 B 目录新分享"
        }));
    }
    let event_id = Uuid::new_v4().to_string();
    let payload = json!({
        "event_id": event_id,
        "mapping_id": mapping_id,
        "target_key": title,
        "target_type": "folder",
        "remote_target_id": target_id,
        "share_id": share_id,
        "share_url": share_url,
        "title": title,
        "intent": "new",
        "change_hint": { "added": [], "changed": [], "removed": [] }
    });
    let _ = save_auto_share_event(
        &db_path,
        &event_id,
        mapping_id,
        title,
        Some(&share_url),
        "sending",
        None,
        Some("整理完成后已从 B 目录创建新分享，正在提交影巢"),
        None,
        &payload,
    );
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
            let status = accepted
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("accepted")
                .to_string();
            let _ = save_auto_share_event(
                &db_path,
                &event_id,
                mapping_id,
                title,
                Some(&share_url),
                &status,
                None,
                Some("B 目录新分享已提交影巢"),
                None,
                &payload,
            );
            let pending = PendingAutoShare {
                mapping_id: mapping_id.to_string(),
                target_key: title.to_string(),
                target_type: "folder".to_string(),
                title: title.to_string(),
                remote_target_id: target_id.to_string(),
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
                payload,
            ));
            Ok(json!({ "share_url": share_url, "share_id": share_id, "hdhive_status": status }))
        }
        Err(error) => {
            let _ = save_auto_share_event(
                &db_path,
                &event_id,
                mapping_id,
                title,
                Some(&share_url),
                "delivery_failed",
                None,
                Some(&error),
                None,
                &payload,
            );
            Err(format!("B 目录新分享已创建，但提交影巢失败：{error}"))
        }
    }
}

fn public_settings(path: &Path) -> Result<OrganizerPublicSettings, String> {
    Ok(load_secrets(path)?.public())
}

fn update_settings(
    path: &Path,
    input: OrganizerSettingsInput,
) -> Result<OrganizerPublicSettings, String> {
    let current = load_secrets(path)?;
    let api_key = if input.clear_api_key {
        String::new()
    } else {
        input
            .api_key
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| current.api_key.clone())
    };
    if api_key.trim().is_empty() && !current.api_key_from_environment {
        return Err("请填写 TMDB API Key 或 Read Access Token".to_string());
    }
    let language = normalize_language(
        input
            .language
            .as_deref()
            .unwrap_or(&current.native.language),
        "zh-CN",
    )?;
    let image_language = normalize_image_language(
        input
            .image_language
            .as_deref()
            .unwrap_or(&current.native.image_language),
    )?;
    let minimum_match_score = normalize_match_score(
        input
            .minimum_match_score
            .unwrap_or(current.native.minimum_match_score),
    )?;
    let movie_path_template = normalize_path_template(
        input
            .movie_path_template
            .as_deref()
            .unwrap_or(&current.movie_path_template),
        MOVIE_PATH_TEMPLATE,
        "电影路径模板",
    )?;
    let tv_path_template = normalize_path_template(
        input
            .tv_path_template
            .as_deref()
            .unwrap_or(&current.tv_path_template),
        TV_PATH_TEMPLATE,
        "电视剧路径模板",
    )?;
    let movie_category = normalize_category(
        input
            .movie_category
            .as_deref()
            .unwrap_or(&current.movie_category),
        "电影",
        "电影分类名",
    )?;
    let tv_category = normalize_category(
        input.tv_category.as_deref().unwrap_or(&current.tv_category),
        "电视剧",
        "电视剧分类名",
    )?;
    let api_base = normalize_mirror_url(
        input.tmdb_api_base.as_deref().unwrap_or(&current.api_base),
        "https://api.themoviedb.org/3",
        "TMDB API 镜像",
    )?;
    let image_base = normalize_mirror_url(
        input.tmdb_image_base.as_deref().unwrap_or(&current.image_base),
        "https://image.tmdb.org/t/p",
        "TMDB 图片镜像",
    )?;
    let category_rules = normalize_category_rules(input.category_rules.or_else(|| Some(current.category_rules.clone())))?;
    let scrape_targets = normalize_scrape_targets(input.scrape_targets.or_else(|| Some(current.scrape_targets.clone())))?;
    let default_scrape_types = normalize_scrape_types(
        input.default_scrape_types.as_deref().unwrap_or(&current.default_scrape_types),
        true,
    )?;
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO organizer_settings
             (id, tmdb_api_key, language, image_language, include_adult, minimum_match_score,
              movie_path_template, tv_path_template, movie_category, tv_category,
              tmdb_api_base, tmdb_image_base, category_rules, scrape_targets, default_scrape_types, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET tmdb_api_key=excluded.tmdb_api_key,
              language=excluded.language, image_language=excluded.image_language,
              include_adult=excluded.include_adult, minimum_match_score=excluded.minimum_match_score,
              movie_path_template=excluded.movie_path_template, tv_path_template=excluded.tv_path_template,
               movie_category=excluded.movie_category, tv_category=excluded.tv_category,
               tmdb_api_base=excluded.tmdb_api_base, tmdb_image_base=excluded.tmdb_image_base,
               category_rules=excluded.category_rules, scrape_targets=excluded.scrape_targets,
               default_scrape_types=excluded.default_scrape_types,
              updated_at=excluded.updated_at",
            params![
                api_key,
                language,
                image_language,
                i64::from(input.include_adult.unwrap_or(current.native.include_adult)),
                minimum_match_score,
                movie_path_template,
                tv_path_template,
                movie_category,
                tv_category,
                api_base,
                image_base,
                serde_json::to_string(&category_rules).map_err(|error| format!("序列化媒体分类失败：{error}"))?,
                serde_json::to_string(&scrape_targets).map_err(|error| format!("序列化刮削目标失败：{error}"))?,
                serde_json::to_string(&default_scrape_types).map_err(|error| format!("序列化默认刮削类型失败：{error}"))?,
                now_seconds()
            ],
        )
        .map_err(|error| format!("保存整理设置失败：{error}"))?;
    public_settings(path)
}

fn normalize_mapping_input(
    input: OrganizerMappingInput,
    current: Option<&OrganizerMapping>,
    default_scrape_types: &[String],
) -> Result<OrganizerMapping, String> {
    let source_dir_id = clean(
        input
            .source_dir_id
            .as_deref()
            .or_else(|| current.map(|item| item.source_dir_id.as_str())),
    );
    let target_dir_id = clean(
        input
            .target_dir_id
            .as_deref()
            .or_else(|| current.map(|item| item.target_dir_id.as_str())),
    );
    if source_dir_id.is_empty() || target_dir_id.is_empty() {
        return Err("请选择光鸭云盘来源 A 目录和目标 B 目录（不允许使用云盘根目录）".to_string());
    }
    if source_dir_id == target_dir_id {
        return Err("来源 A 目录与目标 B 目录不能相同".to_string());
    }
    let source_path = normalize_cloud_path(
        input
            .source_path
            .as_deref()
            .or_else(|| current.map(|item| item.source_path.as_str()))
            .unwrap_or("/"),
    );
    let target_path = normalize_cloud_path(
        input
            .target_path
            .as_deref()
            .or_else(|| current.map(|item| item.target_path.as_str()))
            .unwrap_or("/"),
    );
    if source_path == target_path
        || source_path.starts_with(&format!("{target_path}/"))
        || target_path.starts_with(&format!("{source_path}/"))
    {
        return Err("来源 A 与目标 B 目录不能互相包含，避免循环整理".to_string());
    }
    let transfer_type = normalize_transfer_type(
        input
            .transfer_type
            .as_deref()
            .or_else(|| current.map(|item| item.transfer_type.as_str()))
            .unwrap_or("copy"),
    )?;
    let conflict_policy = normalize_conflict_policy(
        input
            .conflict_policy
            .as_deref()
            .or_else(|| current.map(|item| item.conflict_policy.as_str()))
            .unwrap_or("skip"),
    )?;
    let risk = input
        .share_risk_acknowledged
        .or_else(|| current.map(|item| item.share_risk_acknowledged))
        .unwrap_or(false);
    if (transfer_type == "move" || conflict_policy == "overwrite") && !risk {
        return Err("移动或覆盖可能使已有分享失效，请先确认分享失效风险".to_string());
    }
    let scrape = input
        .scrape
        .or_else(|| current.map(|item| item.scrape))
        .unwrap_or(false);
    let scrape_types = normalize_scrape_types(
        input
            .scrape_types
            .as_deref()
            .or_else(|| current.map(|item| item.scrape_types.as_slice()))
            .unwrap_or(default_scrape_types),
        scrape,
    )?;
    let settle_seconds = input
        .settle_seconds
        .or_else(|| current.map(|item| item.settle_seconds))
        .unwrap_or(30);
    if !(5..=3600).contains(&settle_seconds) {
        return Err("静默等待必须是 5 到 3600 秒之间的整数".to_string());
    }
    let now = now_seconds();
    Ok(OrganizerMapping {
        id: current.map(|item| item.id.clone()).unwrap_or_default(),
        source_path,
        target_path,
        source_dir_id,
        target_dir_id,
        enabled: input
            .enabled
            .or_else(|| current.map(|item| item.enabled))
            .unwrap_or(true),
        scan_existing: input
            .scan_existing
            .or_else(|| current.map(|item| item.scan_existing))
            .unwrap_or(true),
        monitor_mode: "cloud_polling".to_string(),
        transfer_type,
        media_type: normalize_media_type(
            input
                .media_type
                .as_deref()
                .or_else(|| current.map(|item| item.media_type.as_str()))
                .unwrap_or(""),
        )?,
        scrape,
        scrape_types,
        sync_extras: input
            .sync_extras
            .or_else(|| current.map(|item| item.sync_extras))
            .unwrap_or(true),
        conflict_policy,
        auto_execute: input
            .auto_execute
            .or_else(|| current.map(|item| item.auto_execute))
            .unwrap_or(false),
        share_after_organize: input
            .share_after_organize
            .or_else(|| current.map(|item| item.share_after_organize))
            .unwrap_or(false),
        share_risk_acknowledged: risk,
        settle_seconds,
        watch_error: None,
        created_at: current.map(|item| item.created_at).unwrap_or(now),
        updated_at: now,
    })
}

fn save_mapping(path: &Path, mapping: &OrganizerMapping) -> Result<(), String> {
    open_database(path)?
        .execute(
            "INSERT INTO organizer_mappings
             (id, source_path, target_path, source_dir_id, target_dir_id, enabled, scan_existing,
              monitor_mode, transfer_type, media_type, scrape, scrape_types, sync_extras,
              conflict_policy, auto_execute, share_after_organize, share_risk_acknowledged,
              settle_seconds, watch_error, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'cloud_polling', ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, NULL, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET source_path=excluded.source_path, target_path=excluded.target_path,
              source_dir_id=excluded.source_dir_id, target_dir_id=excluded.target_dir_id,
              enabled=excluded.enabled, scan_existing=excluded.scan_existing, monitor_mode='cloud_polling',
              transfer_type=excluded.transfer_type, media_type=excluded.media_type, scrape=excluded.scrape,
              scrape_types=excluded.scrape_types, sync_extras=excluded.sync_extras,
              conflict_policy=excluded.conflict_policy, auto_execute=excluded.auto_execute,
              share_after_organize=excluded.share_after_organize,
              share_risk_acknowledged=excluded.share_risk_acknowledged,
              settle_seconds=excluded.settle_seconds, watch_error=NULL, updated_at=excluded.updated_at",
            params![
                mapping.id,
                mapping.source_path,
                mapping.target_path,
                mapping.source_dir_id,
                mapping.target_dir_id,
                i64::from(mapping.enabled),
                i64::from(mapping.scan_existing),
                mapping.transfer_type,
                mapping.media_type,
                i64::from(mapping.scrape),
                serde_json::to_string(&mapping.scrape_types).unwrap_or_else(|_| "[]".to_string()),
                i64::from(mapping.sync_extras),
                mapping.conflict_policy,
                i64::from(mapping.auto_execute),
                i64::from(mapping.share_after_organize),
                i64::from(mapping.share_risk_acknowledged),
                mapping.settle_seconds as i64,
                mapping.created_at,
                mapping.updated_at
            ],
        )
        .map_err(|error| format!("保存整理监控失败：{error}"))?;
    Ok(())
}

fn mapping_idle(path: &Path, id: &str) -> Result<(), String> {
    let running = open_database(path)?
        .query_row(
            "SELECT id FROM organizer_jobs WHERE mapping_id=?1 AND status IN ('recognizing','running') LIMIT 1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("检查整理任务状态失败：{error}"))?;
    if running.is_some() {
        return Err("该云盘目录正在识别或整理，完成后才能修改或删除".to_string());
    }
    Ok(())
}

fn resolved_overrides(
    job: &OrganizerJob,
    mapping: &OrganizerMapping,
    input: &OrganizerJobInput,
) -> RecognitionOverrides {
    RecognitionOverrides {
        media_type: if input.media_type.is_some() {
            input.media_type.clone()
        } else {
            job.media_type
                .clone()
                .or_else(|| (!mapping.media_type.is_empty()).then(|| mapping.media_type.clone()))
        },
        tmdb_id: if input.clear_tmdb_id {
            None
        } else {
            input.tmdb_id.or(job.tmdb_id)
        },
        title: if input.clear_title {
            None
        } else {
            input.title.clone().or_else(|| job.query_title.clone())
        },
        year: if input.clear_year {
            None
        } else {
            input.year.or(job.query_year)
        },
        season: if input.clear_season {
            None
        } else {
            input.season.or(job.season)
        },
        episode: if input.clear_episode {
            None
        } else {
            input.episode.or(job.episode)
        },
        episode_end: if input.clear_episode_end {
            None
        } else {
            input.episode_end.or(job.episode_end)
        },
    }
}

fn insert_job(
    path: &Path,
    mapping: &OrganizerMapping,
    candidate: &CloudEntry,
    fingerprint: &CandidateFingerprint,
    share_after: bool,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let display_path = join_relative(&[&mapping.source_path, &candidate.name]);
    open_database(path)?
        .execute(
            "INSERT INTO organizer_jobs
             (id, mapping_id, source_path, source_id, source_parent_id, source_size, source_modified_ms,
              source_file_count, source_signature, share_after_requested, status, media_type, message,
              created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'recognizing', ?11, ?12, ?13, ?13)",
            params![
                id,
                mapping.id,
                display_path,
                candidate.id,
                mapping.source_dir_id,
                fingerprint.size,
                fingerprint.modified_ms,
                fingerprint.file_count,
                fingerprint.signature,
                i64::from(share_after),
                if mapping.media_type.is_empty() { None::<String> } else { Some(mapping.media_type.clone()) },
                "等待光鸭云盘原生识别",
                now_seconds()
            ],
        )
        .map_err(|error| format!("创建整理任务失败：{error}"))?;
    Ok(id)
}

async fn recognize_job(
    app: &tauri::AppHandle,
    state: &OrganizerSharedState,
    id: &str,
    input: OrganizerJobInput,
    execute_after: bool,
) -> Result<OrganizerJob, String> {
    let path = database_path(state)?;
    let job = get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    let mapping =
        get_mapping(&path, &job.mapping_id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let loaded = load_candidate(app, &mapping, &job.source_id)
        .await?
        .ok_or_else(|| "待整理云端项目已经不存在".to_string())?;
    let overrides = resolved_overrides(&job, &mapping, &input);
    update_job_fields(
        &path,
        id,
        &[
            ("status", json!("recognizing")),
            ("source_size", json!(loaded.fingerprint.size)),
            ("source_modified_ms", json!(loaded.fingerprint.modified_ms)),
            ("source_file_count", json!(loaded.fingerprint.file_count)),
            ("source_signature", json!(loaded.fingerprint.signature)),
            ("media_type", json!(overrides.media_type.clone())),
            (
                "tmdb_id",
                json!(overrides.tmdb_id.map(|value| value.to_string())),
            ),
            ("season", json!(overrides.season)),
            ("episode", json!(overrides.episode)),
            ("episode_end", json!(overrides.episode_end)),
            ("query_title", json!(overrides.title.clone())),
            ("query_year", json!(overrides.year)),
            ("preview_json", Value::Null),
            ("result_json", Value::Null),
            ("error_code", Value::Null),
            ("message", json!("光鸭正在解析云盘文件名并匹配 TMDB")),
        ],
    )?;
    emit(
        app,
        "job-updated",
        json!({ "job_id": id, "mapping_id": mapping.id, "status": "recognizing" }),
    );
    let secrets = load_secrets(&path)?;
    if secrets.api_key.trim().is_empty() {
        update_job_fields(
            &path,
            id,
            &[
                ("status", json!("needs_review")),
                ("error_code", json!("tmdb_not_configured")),
                ("message", json!("请先配置 TMDB API Key")),
            ],
        )?;
        return get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string());
    }
    let recognition = async {
        let (analysis, _) = analyze_cloud_candidate(&loaded, &overrides)?;
        let mut resolved = overrides.clone();
        if resolved.title.is_none() {
            resolved.title = Some(analysis.title.clone());
        }
        if resolved.year.is_none() {
            resolved.year = analysis.year;
        }
        if resolved.media_type.is_none() {
            resolved.media_type = Some(analysis.media_type.clone());
        }
        let match_result =
            resolve_tmdb_match(&analysis, &secrets.client()?, &secrets.native, &resolved).await?;
        let preview = build_preview(app, &loaded, &analysis, &match_result, &mapping, &secrets).await?;
        Ok::<_, String>((match_result, preview))
    }
    .await;
    let (match_result, preview) = match recognition {
        Ok(value) => value,
        Err(error) => {
            let needs_review = error.contains("没有找到可整理的视频")
                || error.contains("无法从文件名提取媒体名称");
            let error_code = if needs_review {
                if error.contains("视频") { "video_required" } else { "title_required" }
            } else if error.contains("TMDB") {
                "tmdb_unavailable"
            } else {
                "recognition_failed"
            };
            let status = if needs_review { "needs_review" } else { "failed" };
            update_job_fields(
                &path,
                id,
                &[
                    ("status", json!(status)),
                    ("error_code", json!(error_code)),
                    ("message", json!(error.clone())),
                ],
            )?;
            emit(
                app,
                "job-updated",
                json!({ "job_id": id, "mapping_id": mapping.id, "status": status, "message": error }),
            );
            return get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string());
        }
    };
    let ready = preview.success && preview.data.summary.failed == 0;
    let status = if ready { "ready" } else { "needs_review" };
    let error_code = if ready {
        None
    } else {
        preview
            .error_code
            .clone()
            .or_else(|| match_result.error_code.clone())
    };
    update_job_fields(
        &path,
        id,
        &[
            ("status", json!(status)),
            (
                "media_type",
                json!(Some(match_result.query.media_type.clone())),
            ),
            (
                "tmdb_id",
                json!(match_result
                    .selected
                    .as_ref()
                    .map(|value| value.tmdb_id.to_string())),
            ),
            ("query_title", json!(Some(match_result.query.title.clone()))),
            ("query_year", json!(match_result.query.year)),
            (
                "preview_json",
                json!(serde_json::to_string(&preview).unwrap_or_default()),
            ),
            ("error_code", json!(error_code)),
            ("message", json!(preview.message.clone())),
        ],
    )?;
    emit(
        app,
        "job-updated",
        json!({ "job_id": id, "mapping_id": mapping.id, "status": status }),
    );
    let result = get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if ready && execute_after && mapping.enabled {
        return execute_job(app, state, id).await;
    }
    Ok(result)
}

fn schedule_candidate(
    app: &tauri::AppHandle,
    state: &OrganizerSharedState,
    mapping: OrganizerMapping,
    candidate_id: String,
    signature: String,
    immediate: bool,
    share_after: Option<bool>,
) {
    let delay = if immediate { 0 } else { mapping.settle_seconds };
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        if delay > 0 {
            sleep(Duration::from_secs(delay)).await;
        }
        let key = format!("{}::{candidate_id}", mapping.id);
        let should_run = state
            .lock()
            .ok()
            .is_some_and(|mut runtime| runtime.running_candidates.insert(key.clone()));
        if !should_run {
            return;
        }
        let path = match database_path(&state) {
            Ok(path) => path,
            Err(_) => {
                if let Ok(mut runtime) = state.lock() {
                    runtime.running_candidates.remove(&key);
                }
                return;
            }
        };
        let result = async {
            let loaded = load_candidate(&app, &mapping, &candidate_id).await?;
            let Some(loaded) = loaded else { return Ok::<(), String>(()); };
            if loaded.fingerprint.signature != signature {
                return Ok(());
            }
            let duplicate = open_database(&path)?
                .query_row(
                    "SELECT id FROM organizer_jobs WHERE mapping_id=?1 AND source_id=?2 AND source_signature=?3 AND status IN ('recognizing','ready','running','completed','completed_warning','needs_review') ORDER BY updated_at DESC LIMIT 1",
                    params![mapping.id, candidate_id, signature],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("读取重复整理任务失败：{error}"))?;
            let job_id = if let Some(job_id) = duplicate {
                job_id
            } else {
                insert_job(&path, &mapping, &loaded.candidate, &loaded.fingerprint, share_after.unwrap_or(mapping.share_after_organize))?
            };
            if let Some(true) = share_after {
                update_job_fields(&path, &job_id, &[("share_after_requested", json!(1))])?;
            }
            let _ = recognize_job(&app, &state, &job_id, OrganizerJobInput::default(), mapping.auto_execute).await?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            let _ = open_database(&path).and_then(|connection| {
                connection
                    .execute(
                        "UPDATE organizer_mappings SET watch_error=?1, updated_at=?2 WHERE id=?3",
                        params![error, now_seconds(), mapping.id],
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
            emit(
                &app,
                "job-error",
                json!({ "mapping_id": mapping.id, "source_id": candidate_id, "message": error.to_string() }),
            );
        }
        if let Ok(mut runtime) = state.lock() {
            runtime.running_candidates.remove(&key);
        }
    });
}

async fn execute_job(
    app: &tauri::AppHandle,
    state: &OrganizerSharedState,
    id: &str,
) -> Result<OrganizerJob, String> {
    let path = database_path(state)?;
    let job = get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if matches!(job.status.as_str(), "completed" | "completed_warning") {
        return Err("该任务已经整理完成".to_string());
    }
    let mapping =
        get_mapping(&path, &job.mapping_id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let preview = job
        .preview
        .clone()
        .ok_or_else(|| "当前任务没有可执行预览，请先重新识别".to_string())?;
    if !preview.success || preview.data.summary.failed > 0 {
        return Err("当前任务没有可执行预览，请先重新识别".to_string());
    }
    let secrets = load_secrets(&path)?;
    if preview.mapping_signature != mapping_signature(&mapping, &secrets) {
        return Err("整理配置在预览后发生变化，请先重新识别".to_string());
    }
    let loaded = load_candidate(app, &mapping, &job.source_id)
        .await?
        .ok_or_else(|| "待整理云端项目已经不存在".to_string())?;
    if loaded.fingerprint.signature != preview.source_signature {
        return Err("待整理云端内容在预览后发生变化，请先重新识别".to_string());
    }
    {
        let mut runtime = state.lock().map_err(|error| error.to_string())?;
        if !runtime.running_jobs.insert(id.to_string()) {
            return Err("该任务正在整理，请勿重复执行".to_string());
        }
    }
    update_job_fields(
        &path,
        id,
        &[
            ("status", json!("running")),
            ("error_code", Value::Null),
            ("message", json!("光鸭正在执行云盘 A → B 原生整理")),
        ],
    )?;
    emit(
        app,
        "job-updated",
        json!({ "job_id": id, "mapping_id": mapping.id, "status": "running" }),
    );
    let outcome = async {
        let (transferred, skipped, targets) = execute_transfers(app, &mapping, &preview).await?;
        let (scraped, scrape_skipped, mut warnings) =
            execute_scrape(app, &mapping, &preview, &secrets.tmdb_proxy).await;
        if mapping.transfer_type == "move" {
            warnings.push("云端移动会使来源资源的已有分享失效；光鸭没有复用来源分享".to_string());
        }
        let mut share = None;
        if job.share_after_requested || mapping.share_after_organize {
            match ensure_target_directory(app, &mapping, &preview.share_relative_path).await {
                Ok(target_id) => match create_fresh_organizer_share(
                    app.clone(),
                    &mapping.id,
                    &target_id,
                    &preview.share_title,
                )
                .await
                {
                    Ok(value) => share = Some(value),
                    Err(error) => warnings.push(format!(
                        "整理已完成，但创建 B 目录新分享失败：{error}"
                    )),
                },
                Err(error) => warnings.push(format!(
                    "整理已完成，但无法定位 B 目录分享目标：{error}"
                )),
            }
        }
        Ok::<OrganizerExecutionResult, String>(OrganizerExecutionResult {
            success: true,
            transferred,
            skipped: skipped + scrape_skipped,
            scraped,
            warnings,
            targets,
            share,
        })
    }
    .await;
    let result = match outcome {
        Ok(result) => result,
        Err(error) => {
            update_job_fields(
                &path,
                id,
                &[
                    ("status", json!("failed")),
                    ("error_code", json!("transfer_failed")),
                    ("message", json!(error.clone())),
                ],
            )?;
            if let Ok(mut runtime) = state.lock() {
                runtime.running_jobs.remove(id);
            }
            emit(
                app,
                "job-updated",
                json!({ "job_id": id, "mapping_id": mapping.id, "status": "failed", "message": error }),
            );
            return get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string());
        }
    };
    let status = if result.warnings.is_empty() {
        "completed"
    } else {
        "completed_warning"
    };
    let message = format!(
        "云盘整理完成：转移 {} 项，刮削 {} 项{}{}",
        result.transferred,
        result.scraped,
        if result.share.is_some() {
            "，已从 B 目录重新分享"
        } else {
            ""
        },
        if result.warnings.is_empty() {
            String::new()
        } else {
            format!("；{} 项提示", result.warnings.len())
        }
    );
    update_job_fields(
        &path,
        id,
        &[
            ("status", json!(status)),
            (
                "error_code",
                if result.warnings.is_empty() {
                    Value::Null
                } else {
                    json!("completed_warning")
                },
            ),
            (
                "result_json",
                json!(serde_json::to_string(&result).unwrap_or_default()),
            ),
            ("message", json!(message)),
        ],
    )?;
    if let Ok(mut runtime) = state.lock() {
        runtime.running_jobs.remove(id);
    }
    emit(
        app,
        "job-updated",
        json!({ "job_id": id, "mapping_id": mapping.id, "status": status }),
    );
    get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string())
}

fn snapshot(path: &Path) -> Result<OrganizerSnapshot, String> {
    let settings = public_settings(path)?;
    let mappings = list_mappings(path)?;
    let jobs = list_jobs(path)?;
    let mut counts = HashMap::new();
    for job in &jobs {
        *counts.entry(job.status.clone()).or_insert(0) += 1;
    }
    Ok(OrganizerSnapshot {
        settings,
        mappings,
        jobs,
        counts,
    })
}

async fn scan_mapping_inner(
    app: &tauri::AppHandle,
    state: &OrganizerSharedState,
    id: &str,
    immediate: bool,
    candidate_id: Option<&str>,
    candidate_name: Option<&str>,
    share_after: Option<bool>,
) -> Result<usize, String> {
    let path = database_path(state)?;
    let mapping = get_mapping(&path, id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    if !mapping.enabled {
        return Err("请先启用整理监控".to_string());
    }
    if auth_context(app).is_err() {
        return Err("请先登录光鸭云盘".to_string());
    }
    let roots = list_cloud_children(app, &mapping.source_dir_id).await?;
    let mut queued = 0usize;
    for candidate in roots {
        if !useful_candidate(&candidate) {
            continue;
        }
        if candidate_id.is_some_and(|value| value != candidate.id) {
            continue;
        }
        if candidate_name.is_some_and(|value| value != candidate.name) {
            continue;
        }
        let Some(loaded) = load_candidate(app, &mapping, &candidate.id).await? else {
            continue;
        };
        if loaded.fingerprint.video_count < 1 {
            continue;
        }
        schedule_candidate(
            app,
            state,
            mapping.clone(),
            candidate.id,
            loaded.fingerprint.signature,
            immediate,
            share_after,
        );
        queued += 1;
    }
    open_database(&path)?
        .execute(
            "UPDATE organizer_mappings SET watch_error=NULL, updated_at=?1 WHERE id=?2",
            params![now_seconds(), id],
        )
        .map_err(|error| format!("更新整理监控状态失败：{error}"))?;
    emit(
        app,
        "scan-started",
        json!({ "mapping_id": id, "queued": queued }),
    );
    Ok(queued)
}

async fn polling_loop(app: tauri::AppHandle, state: OrganizerSharedState) {
    loop {
        sleep(Duration::from_secs(POLL_INTERVAL_SECONDS)).await;
        let mappings = match database_path(&state).and_then(|path| list_mappings(&path)) {
            Ok(value) => value
                .into_iter()
                .filter(|mapping| mapping.enabled)
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        for mapping in mappings {
            let _ = scan_mapping_inner(&app, &state, &mapping.id, false, None, None, None).await;
        }
    }
}

pub fn initialize(
    _app: tauri::AppHandle,
    db_path: PathBuf,
) -> Result<OrganizerSharedState, String> {
    init_database(&db_path)?;
    Ok(Arc::new(Mutex::new(OrganizerRuntime {
        db_path,
        running_jobs: HashSet::new(),
        running_candidates: HashSet::new(),
    })))
}

pub fn start(app: tauri::AppHandle, state: OrganizerSharedState) {
    let initial_app = app.clone();
    let initial_state = state.clone();
    tauri::async_runtime::spawn(async move {
        let path = match database_path(&initial_state) {
            Ok(path) => path,
            Err(_) => return,
        };
        if let Ok(mappings) = list_mappings(&path) {
            for mapping in mappings
                .into_iter()
                .filter(|mapping| mapping.enabled && mapping.scan_existing)
            {
                let _ = scan_mapping_inner(
                    &initial_app,
                    &initial_state,
                    &mapping.id,
                    true,
                    None,
                    None,
                    None,
                )
                .await;
            }
        }
        polling_loop(initial_app, initial_state).await;
    });
}

pub fn validate_backup_mapping_link(
    app: &tauri::AppHandle,
    organizer_mapping_id: &str,
    remote_parent_id: &str,
) -> Result<(), String> {
    if organizer_mapping_id.trim().is_empty() {
        return Ok(());
    }
    let state = app.state::<OrganizerSharedState>();
    let path = database_path(state.inner())?;
    let mapping = get_mapping(&path, organizer_mapping_id)?
        .ok_or_else(|| "关联的云盘整理监控不存在".to_string())?;
    if !mapping.enabled {
        return Err("关联的云盘整理监控已停用".to_string());
    }
    if mapping.source_dir_id != remote_parent_id {
        return Err("上传目录必须与整理监控的 A 目录完全一致，避免上传后找错媒体".to_string());
    }
    Ok(())
}

pub async fn notify_upload(
    app: tauri::AppHandle,
    organizer_mapping_id: String,
    _remote_file_id: String,
    relative_path: String,
    share_after: bool,
) -> Result<Value, String> {
    let state = app.state::<OrganizerSharedState>();
    let path = database_path(state.inner())?;
    let mapping = get_mapping(&path, &organizer_mapping_id)?
        .ok_or_else(|| "上传任务关联的云盘整理监控不存在".to_string())?;
    if !mapping.enabled {
        return Err("上传任务关联的云盘整理监控已停用".to_string());
    }
    let candidate_name = path_parts(&relative_path)
        .first()
        .copied()
        .unwrap_or_default()
        .to_string();
    let queued = scan_mapping_inner(
        &app,
        state.inner(),
        &mapping.id,
        false,
        None,
        if candidate_name.is_empty() {
            None
        } else {
            Some(candidate_name.as_str())
        },
        Some(share_after),
    )
    .await?;
    if queued == 0 {
        return Err("上传已入库，但尚未在整理 A 目录定位到对应媒体项目".to_string());
    }
    Ok(json!({ "queued": queued, "mapping_id": mapping.id }))
}

#[tauri::command]
pub fn get_organizer_state(
    state: tauri::State<'_, OrganizerSharedState>,
) -> Result<OrganizerSnapshot, String> {
    snapshot(&database_path(state.inner())?)
}

#[tauri::command]
pub fn update_organizer_settings(
    state: tauri::State<'_, OrganizerSharedState>,
    input: OrganizerSettingsInput,
) -> Result<OrganizerPublicSettings, String> {
    update_settings(&database_path(state.inner())?, input)
}

#[tauri::command]
pub async fn test_organizer_connection(
    state: tauri::State<'_, OrganizerSharedState>,
    input: OrganizerSettingsInput,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let current = load_secrets(&path)?;
    let api_key = input
        .api_key
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(current.api_key);
    if api_key.trim().is_empty() {
        return Err("请填写 TMDB API Key 或 Read Access Token".to_string());
    }
    let mut settings = current.native.clone();
    if let Some(language) = input.language {
        settings.language = normalize_language(&language, "zh-CN")?;
    }
    if let Some(image_language) = input.image_language {
        settings.image_language = normalize_image_language(&image_language)?;
    }
    let api_base = normalize_mirror_url(
        input.tmdb_api_base.as_deref().unwrap_or(&current.api_base),
        "https://api.themoviedb.org/3",
        "TMDB API 镜像",
    )?;
    let image_base = normalize_mirror_url(
        input.tmdb_image_base.as_deref().unwrap_or(&current.image_base),
        "https://image.tmdb.org/t/p",
        "TMDB 图片镜像",
    )?;
    let client = TmdbClient::new(
        api_key,
        settings.language,
        settings.image_language,
        input.include_adult.unwrap_or(settings.include_adult),
        api_base,
        image_base,
        Some(current.tmdb_proxy),
    )?;
    client.test().await?;
    Ok(json!({ "success": true, "message": "TMDB 连接成功" }))
}

#[tauri::command]
pub async fn scrape_selected_files(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    input: ScrapeSelectedInput,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let secrets = load_secrets(&path)?;
    if !secrets.public().configured {
        return Err("请先配置 TMDB API Key".to_string());
    }
    if input.files.is_empty() {
        return Err("请先选择至少一个视频文件或目录".to_string());
    }
    let targets = &secrets.scrape_targets;
    let target = targets.iter().find(|item| {
        input.target_id.as_deref().is_some_and(|id| item.get("id").and_then(Value::as_str) == Some(id))
    }).or_else(|| (targets.len() == 1).then(|| &targets[0]))
        .ok_or_else(|| if targets.is_empty() { "请先在设置 > 整理 > 刮削偏好中配置媒体库目标" } else { "请选择一个已配置的刮削目标目录" })?;
    let target_dir_id = target.get("dir_id").or_else(|| target.get("target_dir_id")).and_then(Value::as_str).unwrap_or_default().to_string();
    let target_path = target.get("path").or_else(|| target.get("target_path")).and_then(Value::as_str).unwrap_or("/").to_string();
    if target_dir_id.trim().is_empty() { return Err("刮削目标目录配置无效".to_string()); }
    let default_scrape_types = secrets.default_scrape_types.clone();
    let scrape_types = normalize_scrape_types(
        input.scrape_types.as_deref().unwrap_or(&default_scrape_types),
        true,
    )?;
    let transfer_type_input = input.transfer_type.clone();
    let media_type_input = input.media_type.clone();
    let share_risk_acknowledged = input.share_risk_acknowledged;
    let mut jobs = Vec::new();
    let mut failures = Vec::new();
    for source in input.files.into_iter().take(100) {
        let result = async {
            let source_id = source.id.trim().to_string();
            let source_parent_id = source.parent_id.trim().to_string();
            if source_id.is_empty() || source_parent_id.is_empty() { return Err("选中项缺少文件 ID 或来源目录".to_string()); }
            let source_path = normalize_cloud_path(source.parent_path.as_deref().or(source.path.as_deref()).unwrap_or("/"));
            let transfer_type = normalize_transfer_type(transfer_type_input.as_deref().unwrap_or("copy"))?;
            let risk = share_risk_acknowledged;
            if transfer_type == "move" && !risk { return Err("移动可能使已有分享失效，请先确认风险".to_string()); }
            let mapping = OrganizerMapping {
                id: format!("manual:{}", Uuid::new_v4()),
                source_path,
                target_path: normalize_cloud_path(&target_path),
                source_dir_id: source_parent_id,
                target_dir_id: target_dir_id.clone(),
                enabled: true,
                scan_existing: false,
                monitor_mode: "manual".to_string(),
                transfer_type,
                media_type: normalize_media_type(media_type_input.as_deref().or(source.media_type.as_deref()).unwrap_or(""))?,
                scrape: true,
                scrape_types: scrape_types.clone(),
                sync_extras: true,
                conflict_policy: "skip".to_string(),
                auto_execute: false,
                share_after_organize: false,
                share_risk_acknowledged: risk,
                settle_seconds: 5,
                watch_error: None,
                created_at: now_seconds(),
                updated_at: now_seconds(),
            };
            let loaded = load_candidate(&app, &mapping, &source_id).await?.ok_or_else(|| "待整理云端项目不存在或不在来源目录中".to_string())?;
            if loaded.fingerprint.video_count < 1 { return Err("选中项中没有可识别的视频文件".to_string()); }
            save_mapping(&path, &mapping)?;
            let job_id = insert_job(&path, &mapping, &loaded.candidate, &loaded.fingerprint, false)?;
            let job = recognize_job(&app, state.inner(), &job_id, OrganizerJobInput::default(), true).await?;
            Ok::<OrganizerJob, String>(job)
        }.await;
        match result {
            Ok(job) => jobs.push(job),
            Err(error) => failures.push(json!({ "id": source.id, "message": error })),
        }
    }
    Ok(json!({ "jobs": jobs, "failures": failures }))
}

#[tauri::command]
pub async fn add_organizer_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    input: OrganizerMappingInput,
) -> Result<OrganizerMapping, String> {
    let path = database_path(state.inner())?;
    if !public_settings(&path)?.configured {
        return Err("请先配置 TMDB API Key".to_string());
    }
    let secrets = load_secrets(&path)?;
    let mut mapping = normalize_mapping_input(input, None, &secrets.default_scrape_types)?;
    if list_mappings(&path)?
        .iter()
        .any(|current| current.source_dir_id == mapping.source_dir_id)
    {
        return Err("该云盘 A 目录已经存在整理监控".to_string());
    }
    if list_cloud_children(&app, &mapping.source_dir_id)
        .await
        .is_err()
        || list_cloud_children(&app, &mapping.target_dir_id)
            .await
            .is_err()
    {
        return Err("无法读取云盘 A/B 目录，请确认登录态和目录权限".to_string());
    }
    mapping.id = Uuid::new_v4().to_string();
    save_mapping(&path, &mapping)?;
    if mapping.enabled && mapping.scan_existing {
        let _ =
            scan_mapping_inner(&app, state.inner(), &mapping.id, true, None, None, None).await?;
    }
    emit(&app, "mapping-added", json!({ "mapping_id": mapping.id }));
    get_mapping(&path, &mapping.id)?.ok_or_else(|| "整理监控保存后无法读取".to_string())
}

#[tauri::command]
pub async fn update_organizer_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: OrganizerMappingInput,
) -> Result<OrganizerMapping, String> {
    let path = database_path(state.inner())?;
    mapping_idle(&path, &id)?;
    let current = get_mapping(&path, &id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let secrets = load_secrets(&path)?;
    let mapping = normalize_mapping_input(input, Some(&current), &secrets.default_scrape_types)?;
    if mapping.enabled && !public_settings(&path)?.configured {
        return Err("请先配置 TMDB API Key".to_string());
    }
    if list_mappings(&path)?
        .iter()
        .any(|item| item.id != id && item.source_dir_id == mapping.source_dir_id)
    {
        return Err("该云盘 A 目录已经存在整理监控".to_string());
    }
    list_cloud_children(&app, &mapping.source_dir_id).await?;
    list_cloud_children(&app, &mapping.target_dir_id).await?;
    let mapping = OrganizerMapping { id, ..mapping };
    save_mapping(&path, &mapping)?;
    open_database(&path)?
        .execute("DELETE FROM organizer_jobs WHERE mapping_id=?1 AND status IN ('recognizing','ready','needs_review')", params![mapping.id])
        .map_err(|error| format!("清理旧整理预览失败：{error}"))?;
    emit(&app, "mapping-updated", json!({ "mapping_id": mapping.id }));
    get_mapping(&path, &mapping.id)?.ok_or_else(|| "整理监控保存后无法读取".to_string())
}

#[tauri::command]
pub fn remove_organizer_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    mapping_idle(&path, &id)?;
    let connection = open_database(&path)?;
    connection
        .execute(
            "DELETE FROM organizer_jobs WHERE mapping_id=?1",
            params![id],
        )
        .map_err(|error| format!("删除整理历史失败：{error}"))?;
    connection
        .execute("DELETE FROM organizer_mappings WHERE id=?1", params![id])
        .map_err(|error| format!("删除整理监控失败：{error}"))?;
    emit(&app, "mapping-removed", json!({ "mapping_id": id }));
    Ok(json!({}))
}

#[tauri::command]
pub fn remove_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if job.status == "running" {
        return Err("任务正在整理，不能删除".to_string());
    }
    let connection = open_database(&path)?;
    connection
        .execute("DELETE FROM organizer_jobs WHERE id=?1", params![id])
        .map_err(|error| format!("删除整理任务失败：{error}"))?;
    if job.mapping_id.starts_with("manual:") {
        connection.execute(
            "DELETE FROM organizer_mappings WHERE id=?1 AND NOT EXISTS (SELECT 1 FROM organizer_jobs WHERE mapping_id=?1)",
            params![job.mapping_id],
        ).map_err(|error| format!("清理一次性整理配置失败：{error}"))?;
    }
    emit(
        &app,
        "job-removed",
        json!({ "job_id": id, "mapping_id": job.mapping_id }),
    );
    Ok(json!({}))
}

#[tauri::command]
pub async fn scan_organizer_mapping(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
) -> Result<Value, String> {
    let queued = scan_mapping_inner(&app, state.inner(), &id, true, None, None, None).await?;
    Ok(json!({ "queued": queued }))
}

#[tauri::command]
pub async fn run_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: OrganizerJobInput,
) -> Result<OrganizerJob, String> {
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    let has_overrides = input.media_type.is_some()
        || input.tmdb_id.is_some()
        || input.title.is_some()
        || input.year.is_some()
        || input.season.is_some()
        || input.episode.is_some()
        || input.episode_end.is_some()
        || input.clear_tmdb_id
        || input.clear_title
        || input.clear_year
        || input.clear_season
        || input.clear_episode
        || input.clear_episode_end;
    if has_overrides || job.preview.is_none() || job.status == "needs_review" {
        recognize_job(&app, state.inner(), &id, input, true).await
    } else {
        execute_job(&app, state.inner(), &id).await
    }
}

#[tauri::command]
pub async fn retry_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: OrganizerJobInput,
) -> Result<OrganizerJob, String> {
    recognize_job(&app, state.inner(), &id, input, false).await
}
