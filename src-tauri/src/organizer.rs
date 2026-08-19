use crate::organizer_core::{
    parse_media_name_with_settings, render_nfo, resolve_tmdb_match, sanitize_component,
    validate_auxiliary_rule_block, AnalyzedSidecar, AnalyzedVideo, CandidateAnalysis,
    GeneratorSpec, MatchResolution, MediaMetadata, MediaQuery, NativeSettings,
    RecognitionOverrides, TmdbCandidate, TmdbClient, NATIVE_ENGINE_VERSION, VIDEO_EXTENSIONS,
};
use crate::{
    api_post, finish_operation_response, hdhive_request, load_global_network_proxy,
    organizer_upload_bytes, poll_hdhive_receipt, publish_cloud_mutation, save_auto_share_event,
    share_file_payload, share_id_for_hdhive, PendingAutoShare, SharedState,
};
use futures_util::{stream, StreamExt};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio::{
    process::Command,
    time::{sleep, timeout},
};
use uuid::Uuid;

const POLL_INTERVAL_SECONDS: u64 = 15;
const MAX_CLOUD_ITEMS: usize = 20_000;
const MAX_CLOUD_DEPTH: usize = 64;
const MAX_SCRAPE_CANDIDATES: usize = 1_000;
const SCRAPE_RECOGNITION_CONCURRENCY: usize = 4;
const SCRAPE_EXECUTION_CONCURRENCY: usize = 3;
// 云端 rename 接口对并发敏感（并发过高会触发业务码 120 限流，msg 为“缺少clientId”），
// 串行执行并在 cloud_rename 里做退避重试兜底。
const ORGANIZER_RENAME_CONCURRENCY: usize = 1;
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
    include_media_info: bool,
    upgrade_criteria: Vec<String>,
    upgrade_release_groups: String,
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
    word_segment_search: bool,
    similarity_match: bool,
    recognition_words: String,
    release_groups: String,
    render_words: String,
    capture_groups: String,
    include_media_info: bool,
    movie_path_template: String,
    tv_path_template: String,
    movie_category: String,
    tv_category: String,
    tmdb_api_base: String,
    tmdb_image_base: String,
    category_rules: Vec<Value>,
    scrape_targets: Vec<Value>,
    default_scrape_types: Vec<String>,
    upgrade_criteria: Vec<String>,
    upgrade_release_groups: String,
    upgrade_criteria_options: Vec<Value>,
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
    movie_parsed.video_format = "1080p".to_string();
    movie_parsed.resource_type = "WEB-DL".to_string();
    movie_parsed.video_codec = "HEVC".to_string();
    movie_parsed.audio_codec = "DDP".to_string();
    movie_parsed.audio_info = "5.1".to_string();
    movie_parsed.release_group = "Example".to_string();
    movie_parsed.media_probed = true;
    let movie_category = resolve_media_category(&movie, secrets);
    let movie_context = template_context(&movie, &movie_parsed, "", &movie_category, "mkv");
    let movie_path = render_path_template(&secrets.movie_path_template, &movie_context)
        .map(|path| append_media_info_suffix(path, &secrets.movie_path_template, &movie_context, secrets.include_media_info))
        .unwrap_or_else(|_| "电影/美国/2024/示例电影 (2024) [tmdb-12345]/示例电影 (2024).1080p.WEB-DL.5.1.HEVC.DDP-Example.mkv".to_string());

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
    tv_parsed.video_format = "2160p".to_string();
    tv_parsed.source = "Netflix".to_string();
    tv_parsed.release_type = "WEB-DL".to_string();
    tv_parsed.dynamic_range = "HDR".to_string();
    tv_parsed.frame_rate = "60fps".to_string();
    tv_parsed.color_depth = "10bit".to_string();
    tv_parsed.video_codec = "HEVC".to_string();
    tv_parsed.audio_codec = "DDP".to_string();
    tv_parsed.release_group = "Example".to_string();
    tv_parsed.media_probed = true;
    let tv_category = resolve_media_category(&tv, secrets);
    let tv_context = template_context(&tv, &tv_parsed, "第二集", &tv_category, "mkv");
    let tv_path = render_path_template(&secrets.tv_path_template, &tv_context)
        .map(|path| append_media_info_suffix(path, &secrets.tv_path_template, &tv_context, secrets.include_media_info))
        .unwrap_or_else(|_| "电视剧/中国/2024/示例剧集 (2024) [tmdb-67890]/Season 01/示例剧集.S01E02.2160p.Netflix.WEB-DL.HDR.60fps.10bit.HEVC.DDP-Example.mkv".to_string());

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
            tmdb_api_base_managed_by_environment: std::env::var("TMDB_API_BASE")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .is_some(),
            tmdb_image_base_managed_by_environment: std::env::var("TMDB_IMAGE_BASE")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .is_some(),
            language: self.native.language.clone(),
            image_language: self.native.image_language.clone(),
            include_adult: self.native.include_adult,
            minimum_match_score: self.native.minimum_match_score,
            word_segment_search: self.native.word_segment_search,
            similarity_match: self.native.similarity_match,
            recognition_words: self.native.recognition_words.clone(),
            release_groups: self.native.release_groups.clone(),
            render_words: self.native.render_words.clone(),
            capture_groups: self.native.capture_groups.clone(),
            include_media_info: self.include_media_info,
            movie_path_template: self.movie_path_template.clone(),
            tv_path_template: self.tv_path_template.clone(),
            movie_category: self.movie_category.clone(),
            tv_category: self.tv_category.clone(),
            tmdb_api_base: self.api_base.clone(),
            tmdb_image_base: self.image_base.clone(),
            category_rules: self.category_rules.clone(),
            scrape_targets: self.scrape_targets.clone(),
            default_scrape_types: self.default_scrape_types.clone(),
            upgrade_criteria: self.upgrade_criteria.clone(),
            upgrade_release_groups: self.upgrade_release_groups.clone(),
            upgrade_criteria_options: crate::organizer_core::UPGRADE_CRITERIA
                .iter()
                .map(|value| {
                    json!({ "value": value, "label": crate::organizer_core::upgrade_criterion_label(value) })
                })
                .collect(),
            template_examples: standard_template_examples(self),
            path_presets: vec![
                json!({ "id": "reference-media-info", "name": "参考完整命名（媒体信息后缀）", "movie": "{category}/{country}/{title} ({year}) {tmdb-{tmdbid}}/{en_title}.{year}.{videoFormat}.{resourceType}.{effect}.{audioInfo}.{videoCodec}.{audioCodec}-{releaseGroup}{fileExt}", "tv": "{category}/{country}/{title} ({year}) {tmdb-{tmdbid}}/Season {season}/{en_title}.{year}.{season_episode}.{videoFormat}.{source}.{release_type}.{high_quality}.{dolby_vision}.{dynamic_range}.{frame_rate}.{color_depth}.{video_codec}.{audioCodec}-{releaseGroup}{fileExt}" }),
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
    #[serde(default)]
    upgraded: usize,
    #[serde(default)]
    suppressed: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ReplacedFile {
    id: String,
    name: String,
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
    #[serde(default)]
    replaces: Vec<ReplacedFile>,
    #[serde(default)]
    upgraded_by: Option<String>,
    #[serde(default)]
    suppressed: bool,
    #[serde(default)]
    suppressed_by: Option<String>,
    #[serde(default)]
    suppressed_existing: Option<String>,
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
    #[serde(default)]
    media_probe_warnings: Vec<String>,
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
    /// 本次执行创建的全部云端产物（转移文件 + 刮削元数据），
    /// 供“重新归档”清理上一次的落位结果。
    #[serde(default)]
    created_items: Vec<CreatedOutputItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CreatedOutputItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    target_relative: String,
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
    episode_offset: Option<i64>,
    recognition_words: Option<String>,
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
    word_segment_search: Option<bool>,
    similarity_match: Option<bool>,
    recognition_words: Option<String>,
    release_groups: Option<String>,
    render_words: Option<String>,
    capture_groups: Option<String>,
    include_media_info: Option<bool>,
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
    upgrade_criteria: Option<Vec<String>>,
    upgrade_release_groups: Option<String>,
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
    episode_offset: Option<i64>,
    recognition_words: Option<String>,
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
    #[serde(default)]
    clear_episode_offset: bool,
    #[serde(default)]
    clear_recognition_words: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OrganizerJobDeleteInput {
    #[serde(default)]
    delete_source: bool,
    #[serde(default)]
    delete_target: bool,
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

#[derive(Debug, Clone)]
struct CloudScrapeCandidate {
    entry: CloudEntry,
    suggested_media_type: String,
    suggested_title: String,
    video_count: usize,
    reason: &'static str,
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
               word_segment_search INTEGER NOT NULL DEFAULT 1, similarity_match INTEGER NOT NULL DEFAULT 1,
               recognition_words TEXT NOT NULL DEFAULT '', release_groups TEXT NOT NULL DEFAULT '',
               render_words TEXT NOT NULL DEFAULT '', capture_groups TEXT NOT NULL DEFAULT '',
               include_media_info INTEGER NOT NULL DEFAULT 1,
               movie_path_template TEXT NOT NULL DEFAULT '', tv_path_template TEXT NOT NULL DEFAULT '',
               movie_category TEXT NOT NULL DEFAULT '电影', tv_category TEXT NOT NULL DEFAULT '电视剧',
               tmdb_api_base TEXT NOT NULL DEFAULT '', tmdb_image_base TEXT NOT NULL DEFAULT '',
               category_rules TEXT NOT NULL DEFAULT '[]', scrape_targets TEXT NOT NULL DEFAULT '[]',
               default_scrape_types TEXT NOT NULL DEFAULT '[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]',
               upgrade_criteria TEXT NOT NULL DEFAULT '[\"resolution\",\"dynamic_range\",\"release_group\",\"size\"]',
               upgrade_release_groups TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL
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
               episode_offset INTEGER, recognition_words TEXT,
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS organizer_jobs_mapping_status ON organizer_jobs(mapping_id, status, updated_at);
             CREATE INDEX IF NOT EXISTS organizer_jobs_source_id ON organizer_jobs(mapping_id, source_id, updated_at);",
        )
        .map_err(|error| format!("初始化云盘原生整理数据表失败：{error}"))?;
    for (column, definition) in [
        ("word_segment_search", "INTEGER NOT NULL DEFAULT 1"),
        ("similarity_match", "INTEGER NOT NULL DEFAULT 1"),
        ("recognition_words", "TEXT NOT NULL DEFAULT ''"),
        ("release_groups", "TEXT NOT NULL DEFAULT ''"),
        ("render_words", "TEXT NOT NULL DEFAULT ''"),
        ("capture_groups", "TEXT NOT NULL DEFAULT ''"),
        ("include_media_info", "INTEGER NOT NULL DEFAULT 1"),
        ("movie_path_template", "TEXT NOT NULL DEFAULT ''"),
        ("tv_path_template", "TEXT NOT NULL DEFAULT ''"),
        ("movie_category", "TEXT NOT NULL DEFAULT '电影'"),
        ("tv_category", "TEXT NOT NULL DEFAULT '电视剧'"),
        ("tmdb_api_base", "TEXT NOT NULL DEFAULT ''"),
        ("tmdb_image_base", "TEXT NOT NULL DEFAULT ''"),
        ("category_rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("scrape_targets", "TEXT NOT NULL DEFAULT '[]'"),
        (
            "default_scrape_types",
            "TEXT NOT NULL DEFAULT '[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]'",
        ),
        (
            "upgrade_criteria",
            "TEXT NOT NULL DEFAULT '[\"resolution\",\"dynamic_range\",\"release_group\",\"size\"]'",
        ),
        ("upgrade_release_groups", "TEXT NOT NULL DEFAULT ''"),
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
        ("episode_offset", "INTEGER"),
        ("recognition_words", "TEXT"),
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

fn normalize_rule_text(value: &str, label: &str) -> Result<String, String> {
    let normalized = value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string();
    if normalized.chars().count() > 100_000 {
        return Err(format!("{label}不能超过 100000 个字符"));
    }
    if normalized.lines().count() > 2_000 {
        return Err(format!("{label}不能超过 2000 行"));
    }
    Ok(normalized)
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
    let value = if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    };
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(format!("{label}必须以 http:// 或 https:// 开头"));
    }
    if value.len() > 500 || value.contains(['?', '#']) {
        return Err(format!("{label}格式不正确或过长"));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn category_rule_terms(rule: &Value, keys: &[&str]) -> Vec<Value> {
    let source = keys
        .iter()
        .find_map(|key| rule.get(*key))
        .cloned()
        .unwrap_or(Value::Null);
    let values = source.as_array().cloned().unwrap_or_else(|| vec![source]);
    let mut seen = HashSet::new();
    values
        .into_iter()
        .flat_map(|item| {
            if let Some(value) = item.as_str() {
                value
                    .split([',', '，', '\n'])
                    .map(|term| term.trim().to_lowercase())
                    .collect::<Vec<_>>()
            } else if let Some(value) = item.as_i64() {
                vec![value.to_string()]
            } else {
                Vec::new()
            }
        })
        .filter(|term| !term.is_empty() && seen.insert(term.clone()))
        .take(80)
        .map(Value::String)
        .collect()
}

fn normalize_category_rules(value: Option<Vec<Value>>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for (index, rule) in value.unwrap_or_default().into_iter().take(100).enumerate() {
        let raw_name = rule
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .replace('\\', "/");
        let name_parts = raw_name
            .split('/')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if name_parts.is_empty()
            || name_parts.len() > 8
            || name_parts
                .iter()
                .any(|part| matches!(*part, "." | "..") || part.chars().count() > 80)
        {
            return Err(format!("第 {} 条媒体分类名称无效", index + 1));
        }
        let name = name_parts.join("/");
        let genres = category_rule_terms(&rule, &["genres", "genre_ids", "genre_text"]);
        let original_languages =
            category_rule_terms(&rule, &["original_languages", "original_language"]);
        let origin_countries = category_rule_terms(&rule, &["origin_countries", "origin_country"]);
        if genres.is_empty() && original_languages.is_empty() && origin_countries.is_empty() {
            return Err(format!(
                "第 {} 条媒体分类至少配置一个类型、原始语言或来源地区",
                index + 1
            ));
        }
        let media_type = rule
            .get("media_type")
            .and_then(Value::as_str)
            .unwrap_or("all")
            .trim()
            .to_lowercase();
        let media_type = if matches!(media_type.as_str(), "movie" | "tv" | "all") {
            media_type
        } else {
            "all".to_string()
        };
        let id = rule
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("category-{}", index + 1));
        result.push(json!({
            "id": id,
            "name": name,
            "media_type": media_type,
            "genres": genres,
            "original_languages": original_languages,
            "origin_countries": origin_countries,
            "enabled": rule.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        }));
    }
    Ok(result)
}

fn normalize_scrape_targets(value: Option<Vec<Value>>) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for (index, target) in value.unwrap_or_default().into_iter().take(50).enumerate() {
        let name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let dir_id = target
            .get("dir_id")
            .or_else(|| target.get("target_dir_id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if dir_id.is_empty() {
            return Err(format!("第 {} 个刮削目标未选择云盘目录", index + 1));
        }
        let path = normalize_cloud_path(
            target
                .get("path")
                .or_else(|| target.get("target_path"))
                .and_then(Value::as_str)
                .unwrap_or("/"),
        );
        let id = target
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("target-{}", Uuid::new_v4()));
        let name = if name.is_empty() {
            format!("媒体库 {}", index + 1)
        } else {
            name.chars().take(80).collect()
        };
        result.push(json!({
            "id": id,
            "name": name,
            "dir_id": dir_id,
            "path": path,
        }));
    }
    Ok(result)
}

fn configured_output_target<'a>(targets: &'a [Value], target_dir_id: &str) -> Option<&'a Value> {
    targets.iter().find(|target| {
        target
            .get("dir_id")
            .or_else(|| target.get("target_dir_id"))
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == target_dir_id.trim())
    })
}

fn bind_configured_output_target(
    mut mapping: OrganizerMapping,
    targets: &[Value],
    allow_legacy: bool,
) -> Result<OrganizerMapping, String> {
    let Some(target) = configured_output_target(targets, &mapping.target_dir_id) else {
        if allow_legacy {
            return Ok(mapping);
        }
        return Err("目标 B 目录必须从“刮削输出”中已配置的媒体库目标选择".to_string());
    };
    mapping.target_dir_id = target
        .get("dir_id")
        .or_else(|| target.get("target_dir_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    mapping.target_path = normalize_cloud_path(
        target
            .get("path")
            .or_else(|| target.get("target_path"))
            .and_then(Value::as_str)
            .unwrap_or("/"),
    );
    Ok(mapping)
}

#[cfg(test)]
mod configured_output_target_tests {
    use super::*;

    #[test]
    fn mapping_output_is_bound_to_the_global_scrape_target() {
        let mapping = OrganizerMapping {
            id: "mapping".to_string(),
            source_path: "/A".to_string(),
            target_path: "/stale".to_string(),
            source_dir_id: "a".to_string(),
            target_dir_id: "b".to_string(),
            enabled: true,
            scan_existing: true,
            monitor_mode: "cloud_polling".to_string(),
            transfer_type: "copy".to_string(),
            media_type: String::new(),
            scrape: false,
            scrape_types: Vec::new(),
            sync_extras: true,
            conflict_policy: "skip".to_string(),
            auto_execute: false,
            share_after_organize: false,
            share_risk_acknowledged: false,
            settle_seconds: 30,
            watch_error: None,
            created_at: 1,
            updated_at: 1,
        };
        let targets =
            vec![json!({ "id": "library", "name": "媒体库", "dir_id": "b", "path": "/刮削输出" })];
        let bound = bind_configured_output_target(mapping.clone(), &targets, false).unwrap();
        assert_eq!(bound.target_path, "/刮削输出");
        assert!(bind_configured_output_target(mapping, &[], false).is_err());
    }
}

fn resolve_media_category(metadata: &MediaMetadata, secrets: &OrganizerSecrets) -> String {
    let media_type = if metadata.media_type == "tv" {
        "tv"
    } else {
        "movie"
    };
    let genre_names = metadata
        .genres
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let genre_ids = metadata
        .genre_ids
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    for rule in &secrets.category_rules {
        if rule.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let rule_type = rule
            .get("media_type")
            .and_then(Value::as_str)
            .unwrap_or("all");
        if rule_type != "all" && rule_type != media_type {
            continue;
        }
        let genre_matches = rule
            .get("genres")
            .and_then(Value::as_array)
            .map(|items| {
                items.is_empty()
                    || items.iter().any(|item| {
                        let term = item
                            .as_str()
                            .map(str::to_lowercase)
                            .or_else(|| item.as_i64().map(|value| value.to_string()));
                        term.map(|value| {
                            genre_ids.contains(&value)
                                || genre_names.iter().any(|name| {
                                    name == &value || name.contains(&value) || value.contains(name)
                                })
                        })
                        .unwrap_or(false)
                    })
            })
            .unwrap_or(true);
        let language_matches = rule
            .get("original_languages")
            .and_then(Value::as_array)
            .map(|items| {
                items.is_empty()
                    || items.iter().any(|item| {
                        item.as_str().is_some_and(|value| {
                            value.eq_ignore_ascii_case(&metadata.original_language)
                        })
                    })
            })
            .unwrap_or(true);
        let countries = metadata
            .origin_countries
            .iter()
            .chain(metadata.countries.iter())
            .map(|value| value.to_lowercase())
            .collect::<HashSet<_>>();
        let country_matches = rule
            .get("origin_countries")
            .and_then(Value::as_array)
            .map(|items| {
                items.is_empty()
                    || items.iter().any(|item| {
                        item.as_str()
                            .is_some_and(|value| countries.contains(&value.to_lowercase()))
                    })
            })
            .unwrap_or(true);
        if genre_matches && language_matches && country_matches {
            if let Some(name) = rule
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return name.trim().to_string();
            }
        }
    }
    if media_type == "tv" {
        secrets.tv_category.clone()
    } else {
        secrets.movie_category.clone()
    }
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
        "skip" | "overwrite" | "rename" | "upgrade" => Ok(value.to_lowercase()),
        _ => Err("冲突策略必须是跳过、覆盖、保留两份或洗版".to_string()),
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
        ("fileext", ".mkv"),
        ("season_tag", "S01"),
        ("episode_tag", "E01"),
        ("season_episode", "S01E01"),
        ("video_format", "2160p"),
        ("videoformat", "2160p"),
        ("resource_type", "WEB-DL"),
        ("resourcetype", "WEB-DL"),
        ("video_codec", "HEVC"),
        ("videocodec", "HEVC"),
        ("audio_codec", "DDP"),
        ("audiocodec", "DDP"),
        ("release_group", "Example"),
        ("releasegroup", "Example"),
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
                    word_segment_search, similarity_match, recognition_words, release_groups, render_words, capture_groups,
                    movie_path_template, tv_path_template, movie_category, tv_category,
                    tmdb_api_base, tmdb_image_base, category_rules, scrape_targets, default_scrape_types, include_media_info,
                    upgrade_criteria, upgrade_release_groups
             FROM organizer_settings WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)? != 0,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, String>(19)?,
                    row.get::<_, i64>(20)? != 0,
                    row.get::<_, String>(21)?,
                    row.get::<_, String>(22)?,
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
                true,
                true,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                "电影".to_string(),
                "电视剧".to_string(),
                String::new(),
                String::new(),
                "[]".to_string(),
                "[]".to_string(),
                "[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]".to_string(),
                true,
                "[\"resolution\",\"dynamic_range\",\"release_group\",\"size\"]".to_string(),
                String::new(),
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
    native.word_segment_search = stored.5;
    native.similarity_match = stored.6;
    native.recognition_words = normalize_rule_text(&stored.7, "自定义识别词")?;
    native.release_groups = normalize_rule_text(&stored.8, "自定义制作组")?;
    native.render_words = normalize_rule_text(&stored.9, "自定义渲染词")?;
    native.capture_groups = normalize_rule_text(&stored.10, "自定义捕获组")?;
    let stored_api_base =
        normalize_mirror_url(&stored.15, "https://api.themoviedb.org/3", "TMDB API 镜像")?;
    let stored_image_base =
        normalize_mirror_url(&stored.16, "https://image.tmdb.org/t/p", "TMDB 图片镜像")?;
    let category_rules =
        normalize_category_rules(parse_json::<Vec<Value>>(Some(stored.17.clone())))
            .unwrap_or_default();
    let scrape_targets =
        normalize_scrape_targets(parse_json::<Vec<Value>>(Some(stored.18.clone())))
            .unwrap_or_default();
    let stored_scrape_types =
        parse_json::<Vec<String>>(Some(stored.19.clone())).unwrap_or_default();
    let default_scrape_types =
        normalize_scrape_types(&stored_scrape_types, true).unwrap_or_else(|_| {
            DEFAULT_SCRAPE_TYPES
                .iter()
                .map(|value| value.to_string())
                .collect()
        });
    let tmdb_proxy = crate::load_global_network_proxy(path)?;
    Ok(OrganizerSecrets {
        api_key: environment_key.clone().unwrap_or(stored.0),
        native,
        movie_path_template: if stored.11.trim().is_empty() {
            MOVIE_PATH_TEMPLATE.to_string()
        } else {
            stored.11
        },
        tv_path_template: if stored.12.trim().is_empty() {
            TV_PATH_TEMPLATE.to_string()
        } else {
            stored.12
        },
        movie_category: if stored.13.trim().is_empty() {
            "电影".to_string()
        } else {
            stored.13
        },
        tv_category: if stored.14.trim().is_empty() {
            "电视剧".to_string()
        } else {
            stored.14
        },
        api_key_from_environment: environment_key.is_some(),
        language_from_environment: environment_language.is_some(),
        image_language_from_environment: environment_image_language.is_some(),
        api_base: std::env::var("TMDB_API_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(stored_api_base),
        image_base: std::env::var("TMDB_IMAGE_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(stored_image_base),
        tmdb_proxy,
        category_rules,
        scrape_targets,
        default_scrape_types,
        include_media_info: stored.20,
        upgrade_criteria: crate::organizer_core::normalize_upgrade_criteria(
            &parse_json::<Vec<String>>(Some(stored.21.clone())).unwrap_or_default(),
        ),
        upgrade_release_groups: normalize_rule_text(&stored.22, "洗版制作组优先级")?,
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
        .prepare(&format!(
            "{MAPPING_SELECT} WHERE id NOT LIKE 'manual:%' ORDER BY created_at"
        ))
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
    preview_json, result_json, error_code, message, created_at, updated_at,
    episode_offset, recognition_words FROM organizer_jobs";

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
        episode_offset: row.get(24)?,
        recognition_words: row.get::<_, Option<String>>(25)?.filter(|value| !value.trim().is_empty()),
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
    // Telegram 渠道观察整理事件：job-updated 的完成/失败状态会触发通知。
    crate::telegram::observe_event(&payload);
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

async fn cloud_download_url(app: &tauri::AppHandle, file_id: &str) -> Result<String, String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/get_res_download_url",
        json!({ "fileId": file_id }),
        &[],
    )
    .await?;
    let data = response.data.unwrap_or(Value::Null);
    [
        "signedURL",
        "signedUrl",
        "downloadUrl",
        "downloadURL",
        "url",
    ]
    .iter()
    .find_map(|key| data.get(*key).and_then(Value::as_str))
    .map(str::to_string)
    .filter(|value| !value.is_empty())
    .ok_or_else(|| "光鸭没有返回媒体读取地址".to_string())
}

fn ffprobe_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let executable = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let mut candidates = Vec::new();
    if let Ok(value) = std::env::var("FFPROBE_PATH") {
        if !value.trim().is_empty() {
            candidates.push(PathBuf::from(value));
        }
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources").join(executable));
        candidates.push(resource_dir.join(executable));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(executable),
    );
    candidates.push(PathBuf::from(executable));
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn probe_string<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn normalized_probe_video_format(height: i64, width: i64) -> String {
    if height >= 4000 || width >= 7600 {
        "4320p".to_string()
    } else if height >= 2000 || width >= 3800 {
        "2160p".to_string()
    } else if height >= 1350 {
        "1440p".to_string()
    } else if height >= 1000 {
        "1080p".to_string()
    } else if height >= 700 {
        "720p".to_string()
    } else if height >= 540 {
        "576p".to_string()
    } else if height >= 460 {
        "480p".to_string()
    } else if height > 0 {
        format!("{height}p")
    } else {
        String::new()
    }
}

fn normalized_probe_video_codec(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "h264" | "avc" => "AVC".to_string(),
        "hevc" | "h265" => "HEVC".to_string(),
        "av1" => "AV1".to_string(),
        "vp9" => "VP9".to_string(),
        "mpeg2video" => "MPEG2".to_string(),
        other => other.to_uppercase(),
    }
}

fn normalized_probe_audio_codec(value: &str, profile: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "eac3" => "DDP".to_string(),
        "ac3" => "DD".to_string(),
        "truehd" => "TrueHD".to_string(),
        "dts" if profile.to_lowercase().contains("master") => "DTS-HD MA".to_string(),
        "dts" => "DTS".to_string(),
        "aac" => "AAC".to_string(),
        "flac" => "FLAC".to_string(),
        "opus" => "OPUS".to_string(),
        value if value.starts_with("pcm_") => "LPCM".to_string(),
        other => other.to_uppercase(),
    }
}

fn normalized_probe_frame_rate(value: &str) -> String {
    let mut parts = value.split('/');
    let numerator = parts
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);
    let denominator = parts
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0);
    if numerator <= 0.0 || denominator <= 0.0 {
        return String::new();
    }
    let rate = numerator / denominator;
    let common = [
        23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 120.0,
    ]
    .into_iter()
    .find(|candidate| (candidate - rate).abs() < 0.02)
    .unwrap_or(rate);
    if common.fract().abs() < 0.001 {
        format!("{}fps", common as i64)
    } else {
        let formatted = format!("{common:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
        format!("{formatted}fps")
    }
}

fn normalized_probe_audio_layout(audio: &Value) -> String {
    let layout = probe_string(audio, "channel_layout").to_lowercase();
    let channels = audio.get("channels").and_then(Value::as_i64).unwrap_or(0);
    if layout.contains("7.1") || channels == 8 {
        "7.1"
    } else if layout.contains("6.1") || channels == 7 {
        "6.1"
    } else if layout.contains("5.1") || channels == 6 {
        "5.1"
    } else if layout.contains("stereo") || channels == 2 {
        "2.0"
    } else if layout.contains("mono") || channels == 1 {
        "1.0"
    } else {
        return if channels > 0 {
            format!("{channels}.0")
        } else {
            String::new()
        };
    }
    .to_string()
}

fn parse_ffprobe_technical_data(payload: &Value) -> HashMap<String, String> {
    let streams = payload
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let video = streams
        .iter()
        .find(|stream| probe_string(stream, "codec_type") == "video")
        .cloned()
        .unwrap_or(Value::Null);
    let audio = streams
        .iter()
        .find(|stream| probe_string(stream, "codec_type") == "audio")
        .cloned()
        .unwrap_or(Value::Null);
    let video_text = serde_json::to_string(&video)
        .unwrap_or_default()
        .to_lowercase();
    let audio_text = serde_json::to_string(&audio)
        .unwrap_or_default()
        .to_lowercase();
    let dolby_vision = if video_text.contains("dolby vision") || video_text.contains("dovi") {
        "DV"
    } else {
        ""
    };
    let transfer = probe_string(&video, "color_transfer").to_lowercase();
    let dynamic_range = if video_text.contains("hdr10+") || video_text.contains("smpte2094") {
        "HDR10+"
    } else if transfer == "smpte2084" {
        "HDR10"
    } else if transfer == "arib-std-b67" {
        "HLG"
    } else {
        ""
    };
    let explicit_depth = video
        .get("bits_per_raw_sample")
        .or_else(|| video.get("bits_per_sample"))
        .and_then(|value| {
            value
                .as_str()
                .and_then(|value| value.parse::<i64>().ok())
                .or_else(|| value.as_i64())
        })
        .unwrap_or(0);
    let color_depth = if explicit_depth >= 8 {
        format!("{explicit_depth}bit")
    } else {
        Regex::new(r"(?i)(?:p|le|be)(10|12|16)(?:le|be)?$")
            .expect("pixel depth regex")
            .captures(probe_string(&video, "pix_fmt"))
            .and_then(|capture| capture.get(1))
            .map(|value| format!("{}bit", value.as_str()))
            .unwrap_or_default()
    };
    let audio_layout = normalized_probe_audio_layout(&audio);
    let audio_info = [
        if audio_text.contains("atmos") || audio_text.contains("joc") {
            "Atmos"
        } else {
            ""
        },
        &audio_layout,
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let effect = [dolby_vision, dynamic_range]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut result = HashMap::new();
    for (key, value) in [
        (
            "video_format",
            normalized_probe_video_format(
                video.get("height").and_then(Value::as_i64).unwrap_or(0),
                video.get("width").and_then(Value::as_i64).unwrap_or(0),
            ),
        ),
        (
            "video_codec",
            normalized_probe_video_codec(probe_string(&video, "codec_name")),
        ),
        (
            "frame_rate",
            normalized_probe_frame_rate(if probe_string(&video, "avg_frame_rate").is_empty() {
                probe_string(&video, "r_frame_rate")
            } else {
                probe_string(&video, "avg_frame_rate")
            }),
        ),
        ("color_depth", color_depth),
        ("dolby_vision", dolby_vision.to_string()),
        ("dynamic_range", dynamic_range.to_string()),
        ("effect", effect),
        (
            "audio_codec",
            normalized_probe_audio_codec(
                probe_string(&audio, "codec_name"),
                probe_string(&audio, "profile"),
            ),
        ),
        ("audio_info", audio_info),
    ] {
        if !value.is_empty() {
            result.insert(key.to_string(), value);
        }
    }
    result
}

async fn probe_media_url(
    app: &tauri::AppHandle,
    input: &str,
) -> Result<HashMap<String, String>, String> {
    let mut available = false;
    let mut timed_out = false;
    for executable in ffprobe_candidates(app) {
        let has_explicit_parent = executable
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
        if has_explicit_parent && !executable.is_file() {
            continue;
        }
        let mut command = Command::new(&executable);
        command.args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
            input,
        ]);
        command.kill_on_drop(true);
        // 识别时探测媒体信息不能弹出控制台窗口
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        match timeout(Duration::from_secs(45), command.output()).await {
            Ok(Ok(output)) => {
                available = true;
                if !output.status.success() {
                    continue;
                }
                let payload: Value = serde_json::from_slice(&output.stdout)
                    .map_err(|_| "FFprobe 返回的媒体信息无效".to_string())?;
                return Ok(parse_ffprobe_technical_data(&payload));
            }
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(Err(_)) => available = true,
            Err(_) => {
                available = true;
                timed_out = true;
            }
        }
    }
    if timed_out {
        Err("FFprobe 获取媒体信息超时".to_string())
    } else if available {
        Err("FFprobe 无法读取该媒体文件".to_string())
    } else {
        Err("未找到 FFprobe，请重新执行安装或打包准备".to_string())
    }
}

fn apply_probe_metadata(
    parsed: &mut crate::organizer_core::ParsedMediaName,
    values: HashMap<String, String>,
) {
    if let Some(value) = values.get("video_format") {
        parsed.video_format = value.clone();
    }
    if let Some(value) = values.get("video_codec") {
        parsed.video_codec = value.clone();
    }
    if let Some(value) = values.get("frame_rate") {
        parsed.frame_rate = value.clone();
    }
    if let Some(value) = values.get("color_depth") {
        parsed.color_depth = value.clone();
    }
    if let Some(value) = values.get("dolby_vision") {
        parsed.dolby_vision = value.clone();
    }
    if let Some(value) = values.get("dynamic_range") {
        parsed.dynamic_range = value.clone();
    }
    if let Some(value) = values.get("effect") {
        parsed.effect = value.clone();
    }
    if let Some(value) = values.get("audio_codec") {
        parsed.audio_codec = value.clone();
    }
    if let Some(value) = values.get("audio_info") {
        parsed.audio_info = value.clone();
    }
    parsed.media_probed = true;
}

/// ffprobe 探测并发上限：串行探测时每个视频最长 45 秒超时，24 集剧集在
/// CDN 不通时会把任务钉在 recognizing 十几分钟。
const MEDIA_PROBE_CONCURRENCY: usize = 4;

async fn enrich_analysis_with_media_info(
    app: &tauri::AppHandle,
    loaded: &LoadedCandidate,
    analysis: &mut CandidateAnalysis,
) -> Vec<String> {
    let probes = analysis
        .videos
        .iter()
        .enumerate()
        .filter_map(|(index, video)| {
            loaded_entry(loaded, &video.source)
                .map(|entry| (index, entry.id.clone(), entry.name.clone()))
        })
        .collect::<Vec<_>>();
    let mut outcomes = stream::iter(probes.into_iter().map(|(index, entry_id, name)| {
        let app = app.clone();
        async move {
            let result = async {
                let url = cloud_download_url(&app, &entry_id).await?;
                probe_media_url(&app, &url).await
            }
            .await;
            (index, name, result)
        }
    }))
    .buffer_unordered(MEDIA_PROBE_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    // 结果按视频原始顺序回填，保证告警顺序稳定。
    outcomes.sort_by_key(|(index, ..)| *index);
    let mut warnings = Vec::new();
    for (index, name, result) in outcomes {
        match result {
            Ok(values) => {
                if let Some(video) = analysis.videos.get_mut(index) {
                    apply_probe_metadata(&mut video.parsed, values);
                }
            }
            Err(error) => warnings.push(format!("{name}：{error}")),
        }
    }
    warnings
}

#[cfg(test)]
mod media_probe_tests {
    use super::{country_name_zh, parse_ffprobe_technical_data};
    use serde_json::json;

    #[test]
    fn ffprobe_streams_fill_real_technical_suffix_fields() {
        let parsed = parse_ffprobe_technical_data(&json!({ "streams": [
            { "codec_type": "video", "codec_name": "hevc", "width": 3840, "height": 2160, "avg_frame_rate": "60000/1001", "pix_fmt": "yuv420p10le", "color_transfer": "smpte2084" },
            { "codec_type": "audio", "codec_name": "eac3", "channels": 6, "channel_layout": "5.1(side)", "tags": { "title": "Dolby Atmos JOC" } }
        ] }));
        assert_eq!(
            parsed.get("video_format").map(String::as_str),
            Some("2160p")
        );
        assert_eq!(parsed.get("video_codec").map(String::as_str), Some("HEVC"));
        assert_eq!(
            parsed.get("frame_rate").map(String::as_str),
            Some("59.94fps")
        );
        assert_eq!(parsed.get("color_depth").map(String::as_str), Some("10bit"));
        assert_eq!(
            parsed.get("audio_info").map(String::as_str),
            Some("Atmos 5.1")
        );
        assert_eq!(country_name_zh("US"), "美国");
        assert_eq!(country_name_zh("CN"), "中国");
    }
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
    let data = finish_operation_response(&token, &device_id, response).await?;
    let mut created = normalize_cloud_entry(&data, None);
    if created.id.is_empty() {
        created = list_cloud_children(app, parent_id)
            .await?
            .into_iter()
            .find(|entry| entry.is_directory && entry.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("创建云端目录后无法定位：{name}"))?;
    }
    publish_cloud_mutation(
        app,
        app.state::<SharedState>().inner(),
        [parent_id.to_string()],
        &[],
        false,
        "organizer-create-folder",
    );
    Ok(created)
}

async fn cloud_copy_many(
    app: &tauri::AppHandle,
    ids: &[String],
    parent_id: &str,
) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/copy_file",
        json!({ "fileIds": ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        app,
        app.state::<SharedState>().inner(),
        [parent_id.to_string()],
        &[],
        false,
        "organizer-copy",
    );
    Ok(())
}

async fn cloud_move(app: &tauri::AppHandle, id: &str, parent_id: &str) -> Result<(), String> {
    cloud_move_many(app, &[id.to_string()], parent_id).await
}

async fn cloud_move_many(
    app: &tauri::AppHandle,
    ids: &[String],
    parent_id: &str,
) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = api_post(
        &token,
        &device_id,
        "/userres/v1/file/move_file",
        json!({ "fileIds": ids, "parentId": parent_id }),
        &[],
    )
    .await?;
    finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        app,
        app.state::<SharedState>().inner(),
        [parent_id.to_string()],
        ids,
        true,
        "organizer-move",
    );
    Ok(())
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
    finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        app,
        app.state::<SharedState>().inner(),
        Vec::new(),
        &[id.to_string()],
        true,
        "organizer-delete",
    );
    Ok(())
}

async fn cloud_rename(app: &tauri::AppHandle, id: &str, name: &str) -> Result<(), String> {
    let (token, device_id) = auth_context(app)?;
    let response = crate::files::rename_remote_response(&token, &device_id, id, name).await?;
    finish_operation_response(&token, &device_id, response).await?;
    publish_cloud_mutation(
        app,
        app.state::<SharedState>().inner(),
        Vec::new(),
        &[id.to_string()],
        true,
        "organizer-rename",
    );
    Ok(())
}

async fn cloud_rename_many(
    app: &tauri::AppHandle,
    renames: Vec<(String, String)>,
) -> Result<(), String> {
    let results = stream::iter(renames)
        .map(|(id, name)| async move {
            cloud_rename(app, &id, &name)
                .await
                .map_err(|error| format!("{name}：{error}"))
        })
        .buffer_unordered(ORGANIZER_RENAME_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let errors = results
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("并发重命名失败：{}", errors.join("；")))
    }
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

fn season_container_name(value: &str) -> bool {
    Regex::new(r"(?i)^(?:season|s)[ ._\-]?\d{1,3}$")
        .expect("season container regex")
        .is_match(value.trim())
        || Regex::new(r"^第\s*\d{1,3}\s*季$")
            .expect("Chinese season container regex")
            .is_match(value.trim())
}

fn technical_container_name(value: &str) -> bool {
    Regex::new(r"(?i)^(?:bdmv|stream|video(?:_ts)?|disc|disk|cd|part)[ ._\-]?\d*$")
        .expect("technical container regex")
        .is_match(value.trim())
}

fn useful_context_title(value: &str, settings: &NativeSettings) -> String {
    let generic = [
        "download",
        "downloads",
        "media",
        "library",
        "movies",
        "movie",
        "tv",
        "shows",
        "series",
        "下载",
        "下载目录",
        "媒体",
        "媒体库",
        "电影",
        "电视剧",
        "剧集",
        "视频",
    ];
    for segment in value.replace('\\', "/").split('/').rev() {
        if segment.trim().is_empty()
            || season_container_name(segment)
            || technical_container_name(segment)
        {
            continue;
        }
        let normalized = crate::organizer_core::normalize_search_title(segment);
        if normalized.is_empty() || generic.contains(&normalized.as_str()) {
            continue;
        }
        let parsed = parse_media_name_with_settings(
            segment,
            &RecognitionOverrides {
                media_type: Some("tv".to_string()),
                ..Default::default()
            },
            settings,
        );
        if !parsed.title.trim().is_empty() {
            return parsed.title;
        }
    }
    String::new()
}

fn scrape_video_count(
    node: &CloudEntry,
    nodes: &HashMap<String, CloudEntry>,
    children: &HashMap<String, Vec<String>>,
    memo: &mut HashMap<String, usize>,
) -> usize {
    if let Some(count) = memo.get(&node.id) {
        return *count;
    }
    let count = if node.is_directory {
        children
            .get(&node.id)
            .into_iter()
            .flatten()
            .filter_map(|id| nodes.get(id))
            .map(|child| scrape_video_count(child, nodes, children, memo))
            .sum()
    } else if video_extension(&node.name) && !ignored_sample(&node.logical_path) {
        1
    } else {
        0
    };
    memo.insert(node.id.clone(), count);
    count
}

fn push_scrape_candidate(
    output: &mut Vec<CloudScrapeCandidate>,
    node: &CloudEntry,
    reason: &'static str,
    media_type: &str,
    video_count: usize,
    settings: &NativeSettings,
) {
    output.push(CloudScrapeCandidate {
        entry: node.clone(),
        suggested_media_type: if media_type.is_empty() && season_container_name(&node.name) {
            "tv".to_string()
        } else {
            media_type.to_string()
        },
        suggested_title: if season_container_name(&node.name) {
            useful_context_title(&path_parent(&node.logical_path), settings)
        } else {
            String::new()
        },
        video_count,
        reason,
    });
}

fn plan_cloud_scrape_node(
    node: &CloudEntry,
    nodes: &HashMap<String, CloudEntry>,
    children: &HashMap<String, Vec<String>>,
    counts: &mut HashMap<String, usize>,
    settings: &NativeSettings,
    output: &mut Vec<CloudScrapeCandidate>,
) {
    let nested_count = scrape_video_count(node, nodes, children, counts);
    if nested_count == 0 {
        return;
    }
    if !node.is_directory {
        push_scrape_candidate(output, node, "video-file", "", nested_count, settings);
        return;
    }
    let direct = children.get(&node.id).cloned().unwrap_or_default();
    let direct_videos = direct
        .iter()
        .filter_map(|id| nodes.get(id))
        .filter(|entry| {
            !entry.is_directory && scrape_video_count(entry, nodes, children, counts) > 0
        })
        .cloned()
        .collect::<Vec<_>>();
    let media_directories = direct
        .iter()
        .filter_map(|id| nodes.get(id))
        .filter(|entry| {
            entry.is_directory && scrape_video_count(entry, nodes, children, counts) > 0
        })
        .cloned()
        .collect::<Vec<_>>();
    let parsed = direct_videos
        .iter()
        .map(|entry| {
            parse_media_name_with_settings(
                &entry.name,
                &RecognitionOverrides {
                    media_type: Some("tv".to_string()),
                    ..Default::default()
                },
                settings,
            )
        })
        .collect::<Vec<_>>();
    let episodic = parsed
        .iter()
        .filter(|item| item.episode.is_some() || item.season.is_some())
        .collect::<Vec<_>>();
    let identities = episodic
        .iter()
        .map(|item| crate::organizer_core::normalize_search_title(&item.title))
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let all_episodic = !direct_videos.is_empty() && episodic.len() == direct_videos.len();
    let season_directories = media_directories
        .iter()
        .filter(|entry| season_container_name(&entry.name))
        .count();

    if season_container_name(&node.name) {
        push_scrape_candidate(output, node, "season-folder", "tv", nested_count, settings);
    } else if !media_directories.is_empty()
        && season_directories == media_directories.len()
        && (direct_videos.is_empty() || (all_episodic && identities.len() <= 1))
    {
        push_scrape_candidate(
            output,
            node,
            "series-with-seasons",
            "tv",
            nested_count,
            settings,
        );
    } else if media_directories.is_empty() {
        if direct_videos.len() == 1 {
            push_scrape_candidate(
                output,
                node,
                "single-video-folder",
                "",
                nested_count,
                settings,
            );
        } else if all_episodic && identities.len() <= 1 {
            push_scrape_candidate(output, node, "episode-folder", "tv", nested_count, settings);
        } else {
            for entry in &direct_videos {
                push_scrape_candidate(output, entry, "loose-video", "", 1, settings);
            }
        }
    } else if direct_videos.is_empty()
        && media_directories.len() == 1
        && (season_container_name(&media_directories[0].name)
            || technical_container_name(&media_directories[0].name))
    {
        push_scrape_candidate(output, node, "media-container", "", nested_count, settings);
    } else {
        for entry in &direct_videos {
            push_scrape_candidate(output, entry, "mixed-loose-video", "", 1, settings);
        }
        for directory in &media_directories {
            plan_cloud_scrape_node(directory, nodes, children, counts, settings, output);
        }
    }
}

fn plan_cloud_scrape_candidates(
    loaded: &LoadedCandidate,
    settings: &NativeSettings,
) -> Vec<CloudScrapeCandidate> {
    let mut nodes = HashMap::from([(loaded.candidate.id.clone(), loaded.candidate.clone())]);
    for entry in &loaded.entries {
        nodes.insert(entry.id.clone(), entry.clone());
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for entry in nodes.values() {
        if entry.id != loaded.candidate.id {
            children
                .entry(entry.parent_id.clone())
                .or_default()
                .push(entry.id.clone());
        }
    }
    for ids in children.values_mut() {
        ids.sort_by(|left, right| {
            let left_path = nodes
                .get(left)
                .map(|entry| entry.logical_path.as_str())
                .unwrap_or("");
            let right_path = nodes
                .get(right)
                .map(|entry| entry.logical_path.as_str())
                .unwrap_or("");
            left_path.cmp(right_path)
        });
    }
    let mut counts = HashMap::new();
    let mut output = Vec::new();
    plan_cloud_scrape_node(
        &loaded.candidate,
        &nodes,
        &children,
        &mut counts,
        settings,
        &mut output,
    );
    output
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

/// 样片判定（对齐 Node 端 `isSamplePath`）：目录段必须整段是 sample/samples，
/// 文件名主干中的 sample 必须以分隔符为界。禁止无边界子串匹配——
/// 否则 `The Sampler (2019).mkv` 或名为 `Samples Collection` 的父目录会让
/// 整批正片被当成样片丢弃，直接报"没有找到可整理的视频文件"。
fn ignored_sample(value: &str) -> bool {
    static DIRECTORY: OnceLock<Regex> = OnceLock::new();
    static STEM: OnceLock<Regex> = OnceLock::new();
    let directory = DIRECTORY.get_or_init(|| {
        Regex::new(r"(?:^|/)(?:sample|samples)(?:/|$)").expect("sample directory regex")
    });
    let stem = STEM.get_or_init(|| {
        Regex::new(r"(?:^|[ ._\-])sample(?:[ ._\-]|$)").expect("sample stem regex")
    });
    let normalized = value.replace('\\', "/").to_lowercase();
    directory.is_match(&normalized) || stem.is_match(&path_stem(&normalized))
}

/// 附加内容判定（对齐 Node 端 `extraKind`）：trailer 必须是文件名主干中以
/// 分隔符为界的独立词，或位于 trailers/预告 专用目录内；extra 必须位于
/// extras/featurettes 等专用目录内。禁止全路径子串匹配——否则剧名
/// `Trailer Park Boys` 会让整季被判成预告片、跳过全部刮削。
fn extra_kind(value: &str) -> String {
    static TRAILER_STEM: OnceLock<Regex> = OnceLock::new();
    static TRAILER_DIRECTORY: OnceLock<Regex> = OnceLock::new();
    static EXTRA_DIRECTORY: OnceLock<Regex> = OnceLock::new();
    let trailer_stem = TRAILER_STEM.get_or_init(|| {
        Regex::new(r"(?:^|[ ._\-])trailer(?:[ ._\-]|$)").expect("trailer stem regex")
    });
    let trailer_directory = TRAILER_DIRECTORY.get_or_init(|| {
        Regex::new(r"(?:^|/)(?:trailers?|预告|预告片)/").expect("trailer directory regex")
    });
    let extra_directory = EXTRA_DIRECTORY.get_or_init(|| {
        Regex::new(
            r"(?:^|/)(?:extras?|featurettes?|behind the scenes|deleted scenes|interviews?|花絮|幕后)/",
        )
        .expect("extra directory regex")
    });
    let normalized = value.replace('\\', "/").to_lowercase();
    let stem = path_stem(&normalized);
    if trailer_stem.is_match(&stem)
        || trailer_directory.is_match(&normalized)
        || matches!(stem.trim(), "预告" | "预告片")
    {
        "trailer".to_string()
    } else if extra_directory.is_match(&normalized) || matches!(stem.trim(), "花絮" | "幕后") {
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

fn cloud_parent_season(file_path: &str, candidate_path: &str) -> Option<i64> {
    let directory = path_parent(file_path);
    let relative = directory
        .strip_prefix(candidate_path.trim_matches('/'))
        .unwrap_or(&directory)
        .trim_matches('/');
    Regex::new(r"(?i)(?:^|/)(?:Season|S)[ ._\-]?(\d{1,3})(?:$|/)")
        .expect("cloud season folder regex")
        .captures(relative)
        .or_else(|| {
            Regex::new(r"(?:^|/)第\s*(\d{1,3})\s*季(?:$|/)")
                .expect("cloud Chinese season folder regex")
                .captures(relative)
        })
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<i64>().ok())
}

fn most_useful_cloud_title(
    parsed: &[crate::organizer_core::ParsedMediaName],
    fallback: &str,
) -> String {
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    for item in parsed {
        let title = item.title.trim();
        if title.is_empty() {
            continue;
        }
        let key = crate::organizer_core::normalize_search_title(title);
        let entry = counts.entry(key).or_insert((title.to_string(), 0));
        entry.1 += 1;
        if title.chars().count() > entry.0.chars().count() {
            entry.0 = title.to_string();
        }
    }
    counts
        .into_values()
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| left.0.chars().count().cmp(&right.0.chars().count()))
        })
        .map(|value| value.0)
        .unwrap_or_else(|| fallback.to_string())
}

fn best_sidecar_video<'a>(
    sidecar: &CloudEntry,
    videos: &'a [CloudEntry],
    analyzed_videos: &[AnalyzedVideo],
    settings: &NativeSettings,
) -> Option<&'a CloudEntry> {
    if videos.is_empty() {
        return None;
    }
    let sidecar_name = path_stem(&sidecar.logical_path);
    let stripped = Regex::new(
        r"(?i)(?:chs|cht|chi|eng|jpn|kor|zh[-_.]?(?:cn|tw)|简体|繁体|繁體|字幕|forced|sdh|default)",
    )
    .expect("sidecar language regex")
    .replace_all(&sidecar_name, "");
    let sidecar_stem = crate::organizer_core::normalize_search_title(&stripped);
    if let Some(index) = videos.iter().position(|video| {
        sidecar_stem
            == crate::organizer_core::normalize_search_title(&path_stem(&video.logical_path))
    }) {
        return videos.get(index);
    }
    let parsed = parse_media_name_with_settings(
        &sidecar.logical_path,
        &RecognitionOverrides {
            media_type: Some("tv".to_string()),
            ..Default::default()
        },
        settings,
    );
    if let Some(episode) = parsed.episode {
        if let Some(index) = analyzed_videos.iter().position(|video| {
            video.parsed.episode == Some(episode)
                && (parsed.season.is_none() || video.parsed.season == parsed.season)
        }) {
            return videos.get(index);
        }
    }
    let sidecar_parent = path_parent(&sidecar.logical_path);
    let same_directory = videos
        .iter()
        .enumerate()
        .filter(|(_, video)| path_parent(&video.logical_path) == sidecar_parent)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if same_directory.len() == 1 {
        return videos.get(same_directory[0]);
    }
    (videos.len() == 1).then(|| &videos[0])
}

#[cfg(test)]
mod cloud_sidecar_tests {
    use super::*;

    #[test]
    fn ambiguous_sidecar_is_not_attached_to_an_unrelated_video() {
        let videos = vec![
            CloudEntry {
                name: "Show.S01E01.mkv".to_string(),
                logical_path: "Library/Show.S01E01.mkv".to_string(),
                ..Default::default()
            },
            CloudEntry {
                name: "Other.S01E02.mkv".to_string(),
                logical_path: "Library/Other.S01E02.mkv".to_string(),
                ..Default::default()
            },
        ];
        let analyzed = videos
            .iter()
            .map(|video| AnalyzedVideo {
                source: video.logical_path.clone(),
                parsed: crate::organizer_core::ParsedMediaName::default(),
                extra_kind: String::new(),
            })
            .collect::<Vec<_>>();
        let unrelated = CloudEntry {
            name: "Unrelated.srt".to_string(),
            logical_path: "Library/Unrelated.srt".to_string(),
            ..Default::default()
        };
        assert!(
            best_sidecar_video(&unrelated, &videos, &analyzed, &NativeSettings::default())
                .is_none()
        );

        let exact = CloudEntry {
            name: "Show.S01E01.zh-CN.srt".to_string(),
            logical_path: "Library/Show.S01E01.zh-CN.srt".to_string(),
            ..Default::default()
        };
        assert_eq!(
            best_sidecar_video(&exact, &videos, &analyzed, &NativeSettings::default())
                .map(|video| video.name.as_str()),
            Some("Show.S01E01.mkv")
        );
    }

    #[test]
    fn cloud_analysis_uses_season_folder_and_video_titles_instead_of_generic_root() {
        let root = CloudEntry {
            id: "root".to_string(),
            name: "Unhelpful.Collection".to_string(),
            logical_path: "Root/Unhelpful.Collection".to_string(),
            is_directory: true,
            ..Default::default()
        };
        let entries = [1, 2]
            .into_iter()
            .map(|episode| CloudEntry {
                id: format!("video-{episode}"),
                parent_id: "season-2".to_string(),
                name: format!("Actual.Show.E{episode:02}.mkv"),
                logical_path: format!(
                    "Root/Unhelpful.Collection/Season 02/Actual.Show.E{episode:02}.mkv"
                ),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let loaded = LoadedCandidate {
            candidate: root,
            entries,
            fingerprint: CandidateFingerprint::default(),
        };
        let (analysis, _) = analyze_cloud_candidate(
            &loaded,
            &RecognitionOverrides {
                media_type: Some("tv".to_string()),
                ..Default::default()
            },
            &NativeSettings::default(),
        )
        .expect("cloud analysis");
        assert_eq!(analysis.title, "Actual Show");
        assert!(analysis
            .videos
            .iter()
            .all(|video| video.parsed.season == Some(2)));
        assert_eq!(analysis.videos[0].parsed.episode, Some(1));
        assert_eq!(analysis.videos[1].parsed.episode, Some(2));
    }

    #[test]
    fn mixed_large_folder_is_split_without_breaking_a_whole_series() {
        let root = CloudEntry {
            id: "root".to_string(),
            name: "Downloads".to_string(),
            logical_path: "Downloads".to_string(),
            is_directory: true,
            ..Default::default()
        };
        let entry =
            |id: &str, parent: &str, name: &str, logical_path: &str, is_directory| CloudEntry {
                id: id.to_string(),
                parent_id: parent.to_string(),
                name: name.to_string(),
                logical_path: logical_path.to_string(),
                is_directory,
                ..Default::default()
            };
        let entries = vec![
            entry("show", "root", "Foundation", "Downloads/Foundation", true),
            entry(
                "season",
                "show",
                "Season 1",
                "Downloads/Foundation/Season 1",
                true,
            ),
            entry(
                "show-e1",
                "season",
                "E01.mkv",
                "Downloads/Foundation/Season 1/E01.mkv",
                false,
            ),
            entry(
                "show-e2",
                "season",
                "E02.mkv",
                "Downloads/Foundation/Season 1/E02.mkv",
                false,
            ),
            entry(
                "movie-a",
                "root",
                "The.Matrix.1999",
                "Downloads/The.Matrix.1999",
                true,
            ),
            entry(
                "movie-a-file",
                "movie-a",
                "The.Matrix.1999.mkv",
                "Downloads/The.Matrix.1999/The.Matrix.1999.mkv",
                false,
            ),
            entry(
                "movie-b",
                "root",
                "Arrival.2016",
                "Downloads/Arrival.2016",
                true,
            ),
            entry(
                "movie-b-file",
                "movie-b",
                "Arrival.2016.mkv",
                "Downloads/Arrival.2016/Arrival.2016.mkv",
                false,
            ),
            entry(
                "loose-a",
                "root",
                "Alpha.S01E01.mkv",
                "Downloads/Alpha.S01E01.mkv",
                false,
            ),
            entry(
                "loose-b",
                "root",
                "Beta.S01E02.mkv",
                "Downloads/Beta.S01E02.mkv",
                false,
            ),
        ];
        let loaded = LoadedCandidate {
            candidate: root.clone(),
            fingerprint: candidate_fingerprint(&root, &entries),
            entries,
        };
        let mut ids = plan_cloud_scrape_candidates(&loaded, &NativeSettings::default())
            .into_iter()
            .map(|item| item.entry.id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(
            ids,
            vec!["loose-a", "loose-b", "movie-a", "movie-b", "show"]
        );
    }

    #[test]
    fn sample_detection_requires_token_boundaries() {
        // 目录整段或分隔符界定的 sample 才是样片。
        assert!(ignored_sample("Downloads/Sample/movie.mkv"));
        assert!(ignored_sample("Downloads/samples/movie.mkv"));
        assert!(ignored_sample("Downloads/Movie.2020.sample.mkv"));
        assert!(ignored_sample("Downloads/Movie sample.mkv"));
        // 标题里含 sample 子串的正片绝不能被丢弃。
        assert!(!ignored_sample("Downloads/The.Sampler.2019.1080p.mkv"));
        assert!(!ignored_sample("Downloads/Samples Collection/A.Movie.2020.mkv"));
        assert!(!ignored_sample("Downloads/Free Samples (2012)/Free.Samples.2012.mkv"));
    }

    #[test]
    fn extra_detection_requires_dedicated_directories_or_tokens() {
        assert_eq!(extra_kind("Show/trailers/clip.mkv"), "trailer");
        assert_eq!(extra_kind("Show/Movie.Trailer.mkv"), "trailer");
        assert_eq!(extra_kind("Show/预告/片段.mkv"), "trailer");
        assert_eq!(extra_kind("Show/extras/bonus.mkv"), "extra");
        assert_eq!(extra_kind("Show/Featurettes/making.mkv"), "extra");
        assert_eq!(extra_kind("Show/花絮/片段.mkv"), "extra");
        // 剧名含 trailer/extras 子串不能把正片判成附加内容。
        assert_eq!(extra_kind("Trailer Park Boys/Season 1/S01E01.mkv"), "");
        assert_eq!(extra_kind("Extras.UK.S01E01.mkv"), "");
    }

    #[test]
    fn subtitle_language_suffix_emits_bcp47_with_forced_and_sdh() {
        assert_eq!(language_suffix("Show.S01E01.chs.srt"), ".zh-CN");
        assert_eq!(language_suffix("Show.S01E01.cht.srt"), ".zh-TW");
        assert_eq!(language_suffix("Show.S01E01.简体.srt"), ".zh-CN");
        assert_eq!(language_suffix("Show.S01E01.eng.srt"), ".en");
        assert_eq!(
            language_suffix("Show.S01E01.chs.forced.srt"),
            ".zh-CN.forced"
        );
        assert_eq!(language_suffix("Show.S01E01.en.sdh.srt"), ".en.sdh");
        assert_eq!(language_suffix("Show.S01E01.srt"), "");
    }

    #[test]
    fn path_template_renders_empty_values_without_zero_padding() {
        let mut context = HashMap::new();
        context.insert("title".to_string(), "剧名".to_string());
        context.insert("season".to_string(), String::new());
        context.insert("ext".to_string(), "mkv".to_string());
        // 缺季时 {season:02} 渲染为空，而不是补零成 "00"。
        let rendered =
            render_path_template("剧目/Season {season:02}/{title}.{ext}", &context).unwrap();
        assert_eq!(rendered, "剧目/Season/剧名.mkv");
        // 空值残留的悬空 " - " 会被清理。
        context.insert("episode_title".to_string(), String::new());
        let rendered =
            render_path_template("剧目/{title} - {episode_title} - .{ext}", &context).unwrap();
        assert!(!rendered.contains("- -"), "{rendered}");
    }
}

fn analyze_cloud_candidate(
    loaded: &LoadedCandidate,
    overrides: &RecognitionOverrides,
    settings: &NativeSettings,
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
    let group = parse_media_name_with_settings(&candidate_name, overrides, settings);
    let preliminary = videos
        .iter()
        .map(|entry| {
            let mut options = overrides.clone();
            if options.media_type.as_deref().unwrap_or_default().is_empty() {
                options.media_type = Some(group.media_type.clone());
            }
            options.season = overrides.season.or_else(|| {
                cloud_parent_season(&entry.logical_path, &loaded.candidate.logical_path)
            });
            let mut parsed =
                parse_media_name_with_settings(&entry.logical_path, &options, settings);
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
        .unwrap_or_else(|| most_useful_cloud_title(&preliminary, &group.title));
    let year = overrides
        .year
        .or(group.year)
        .or_else(|| preliminary.iter().find_map(|item| item.year));
    // 用最终确定的 media_type 对每个视频完整重新解析一遍（对齐 Node 端的
    // 两遍解析）。季集提取强依赖 media_type 提示：只有 tv 提示才会启用
    // "仅集号"（第5集 / EP05）和番剧 "- 05" 破折号集号分支。旧实现只事后
    // 覆盖 media_type 字段、不重算季集，导致 "Show - 05.mkv" 这类文件
    // 解析不出集号而报 episode_required。
    let preliminary = videos
        .iter()
        .map(|entry| {
            let mut options = overrides.clone();
            options.media_type = Some(media_type.clone());
            options.season = overrides.season.or_else(|| {
                cloud_parent_season(&entry.logical_path, &loaded.candidate.logical_path)
            });
            if videos.len() != 1 {
                options.episode = None;
                options.episode_end = None;
            }
            let mut parsed =
                parse_media_name_with_settings(&entry.logical_path, &options, settings);
            parsed.media_type = media_type.clone();
            if parsed.title.is_empty() {
                parsed.title = title.clone();
            }
            parsed.year = year.or(parsed.year);
            parsed
        })
        .collect::<Vec<_>>();
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
            let video = best_sidecar_video(entry, &videos, &analyzed_videos, settings)?;
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
    // 集偏移：识别出的集号统一加偏移量（可为负），用于源命名与 TMDB 集数错位的剧集
    let mut analyzed_videos = analyzed_videos;
    if let Some(offset) = overrides.episode_offset.filter(|value| *value != 0) {
        for video in &mut analyzed_videos {
            if let Some(episode) = video.parsed.episode {
                video.parsed.episode = Some((episode + offset).max(0));
            }
            if let Some(episode_end) = video.parsed.episode_end {
                video.parsed.episode_end = Some((episode_end + offset).max(0));
            }
        }
    }
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
            title_candidates: crate::organizer_core::title_candidates_from(
                &analyzed_videos
                    .iter()
                    .map(|item| item.parsed.clone())
                    .collect::<Vec<_>>(),
                &group,
            ),
            year,
            tmdb_id: group
                .tmdb_id
                .or_else(|| analyzed_videos.iter().find_map(|item| item.parsed.tmdb_id)),
            videos: analyzed_videos,
            sidecars,
            ignored_samples,
            query: MediaQuery {
                title,
                year,
                media_type,
                tmdb_id: group.tmdb_id,
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
    let conditional_pattern =
        Regex::new(r"(?is)\{\{@if@\}\}(.*?)\{\{@endif@\}\}").expect("conditional template regex");
    let conditional_variable =
        Regex::new(r"(?i)\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}").expect("conditional variable regex");
    let conditional = conditional_pattern
        .replace_all(template, |captures: &regex::Captures| {
            let body = captures
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let keys = conditional_variable
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str().to_lowercase()))
                .collect::<Vec<_>>();
            if !keys.is_empty()
                && keys
                    .iter()
                    .all(|key| context.get(key).is_some_and(|value| !value.is_empty()))
            {
                body.to_string()
            } else {
                String::new()
            }
        })
        .to_string();
    let aliases = Regex::new(r"(?i)\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}")
        .expect("double brace template regex")
        .replace_all(&conditional, "{$1}")
        .to_string();
    let aliases = Regex::new(r"(?i)\{catgroy\}")
        .expect("category alias regex")
        .replace_all(&aliases, "{category}")
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
            // 空值渲染为空串（对齐 Node 端）：不允许 {season:02} 在缺季时
            // 补零成 "00"，否则剧集附加内容会落进 "Season 00/" 目录。
            if value.is_empty() {
                return String::new();
            }
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
    // 对齐 Node cleanRenderedSegment：清理空值残留的悬空连接符，
    // 避免剧集标题为空时产出 "剧名 - .mkv"、空字段产出 "WEB-DL..59fps"
    // 或空制作组产出 "AAC-.mkv" 这类文件名。
    let double_dash = Regex::new(r"\s+-\s+-\s+").expect("double dash regex");
    let trailing_dash = Regex::new(r"(?:\s+-\s*)+$").expect("trailing dash regex");
    let repeated_dots = Regex::new(r"\.{2,}").expect("repeated dots regex");
    let dash_dot = Regex::new(r"-+\.").expect("dash dot regex");
    let parts = raw_parts
        .into_iter()
        .map(|part| {
            let collapsed = part
                .replace("()", "")
                .replace("[]", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let collapsed = repeated_dots.replace_all(&collapsed, ".").to_string();
            let collapsed = dash_dot.replace_all(&collapsed, ".").to_string();
            let collapsed = double_dash.replace_all(&collapsed, " - ").to_string();
            let collapsed = trailing_dash.replace_all(&collapsed, "").to_string();
            sanitize_component(&collapsed, "Unknown")
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
    let season_tag = parsed
        .season
        .map(|value| format!("S{value:02}"))
        .unwrap_or_default();
    let episode_tag = parsed
        .episode
        .map(|value| format!("E{value:02}"))
        .unwrap_or_default();
    let mut country_codes = Vec::new();
    let mut seen_countries = HashSet::new();
    for code in metadata
        .countries
        .iter()
        .chain(metadata.origin_countries.iter())
    {
        let normalized = code.trim().to_uppercase();
        if !normalized.is_empty() && seen_countries.insert(normalized.clone()) {
            country_codes.push(normalized);
        }
    }
    let country_names = country_codes
        .iter()
        .map(|code| country_name_zh(code))
        .collect::<Vec<_>>();
    let source_platform = [parsed.resource_type.clone(), parsed.source.clone()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let effect_version = [parsed.effect.clone(), parsed.edition.clone()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let video_codec_frame_rate_high_quality = [
        parsed.video_codec.clone(),
        parsed.frame_rate.clone(),
        parsed.high_quality.clone(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let media_info = compose_media_info(&metadata.media_type, parsed);
    let mut context = HashMap::new();
    context.insert("category".to_string(), category.to_string());
    context.insert("catgroy".to_string(), category.to_string());
    context.insert(
        "country".to_string(),
        country_names
            .first()
            .cloned()
            .unwrap_or_else(|| "未知地区".to_string()),
    );
    context.insert(
        "country_code".to_string(),
        country_codes.first().cloned().unwrap_or_default(),
    );
    context.insert(
        "release_country".to_string(),
        if country_names.is_empty() {
            "未知地区".to_string()
        } else {
            country_names.join("、")
        },
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
        "en_title".to_string(),
        if metadata.original_title.is_empty() {
            metadata.title.clone()
        } else {
            metadata.original_title.clone()
        },
    );
    context.insert(
        "original_title".to_string(),
        metadata.original_title.clone(),
    );
    context.insert("original_filename".to_string(), parsed.original.clone());
    context.insert("original_name".to_string(), parsed.original.clone());
    context.insert(
        "segment".to_string(),
        parsed.part.clone().unwrap_or_default(),
    );
    context.insert(
        "season_year".to_string(),
        metadata
            .year
            .map(|value| value.to_string())
            .unwrap_or_default(),
    );
    context.insert("tmdb_id".to_string(), metadata.tmdb_id.to_string());
    context.insert("tmdbid".to_string(), metadata.tmdb_id.to_string());
    context.insert("season".to_string(), season.clone());
    context.insert("episode".to_string(), episode.clone());
    context.insert("episode_end".to_string(), episode_end.clone());
    context.insert(
        "season_episode".to_string(),
        format!("{season_tag}{episode_tag}{episode_end}"),
    );
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
        "fileext".to_string(),
        if extension.is_empty() {
            String::new()
        } else {
            format!(".{}", extension.trim_start_matches('.').to_lowercase())
        },
    );
    context.insert("season_tag".to_string(), season_tag);
    context.insert("episode_tag".to_string(), episode_tag);
    for (key, value) in [
        ("video_format", parsed.video_format.clone()),
        ("videoformat", parsed.video_format.clone()),
        ("resource_type", parsed.resource_type.clone()),
        ("resourcetype", parsed.resource_type.clone()),
        ("source", parsed.source.clone()),
        ("effect", parsed.effect.clone()),
        ("audio_info", parsed.audio_info.clone()),
        ("audioinfo", parsed.audio_info.clone()),
        ("video_codec", parsed.video_codec.clone()),
        ("videocodec", parsed.video_codec.clone()),
        ("audio_codec", parsed.audio_codec.clone()),
        ("audiocodec", parsed.audio_codec.clone()),
        ("release_group", parsed.release_group.clone()),
        ("releasegroup", parsed.release_group.clone()),
        ("release_type", parsed.release_type.clone()),
        ("high_quality", parsed.high_quality.clone()),
        ("dolby_vision", parsed.dolby_vision.clone()),
        ("dynamic_range", parsed.dynamic_range.clone()),
        ("frame_rate", parsed.frame_rate.clone()),
        ("color_depth", parsed.color_depth.clone()),
        ("source_platform", source_platform),
        ("version", parsed.edition.clone()),
        ("effect_version", effect_version),
        (
            "remux",
            if parsed.resource_type.eq_ignore_ascii_case("remux")
                || parsed.release_type.eq_ignore_ascii_case("remux")
            {
                "REMUX".to_string()
            } else {
                String::new()
            },
        ),
        ("version_number", parsed.part.clone().unwrap_or_default()),
        (
            "video_codec_frame_rate_high_quality",
            video_codec_frame_rate_high_quality,
        ),
        ("media_info", media_info),
        (
            "media_probed",
            if parsed.media_probed {
                "1".to_string()
            } else {
                String::new()
            },
        ),
    ] {
        context.insert(key.to_string(), value);
    }
    context
}

fn country_name_zh(code: &str) -> String {
    static COUNTRY_NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
    let names = COUNTRY_NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../../shared/countries-zh.json")).unwrap_or_default()
    });
    names
        .get(&code.trim().to_uppercase())
        .cloned()
        .unwrap_or_else(|| code.trim().to_uppercase())
}

fn compose_media_info(media_type: &str, parsed: &crate::organizer_core::ParsedMediaName) -> String {
    let values = if media_type == "tv" {
        vec![
            parsed.video_format.clone(),
            parsed.source.clone(),
            if parsed.release_type.is_empty() {
                parsed.resource_type.clone()
            } else {
                parsed.release_type.clone()
            },
            parsed.high_quality.clone(),
            parsed.dolby_vision.clone(),
            parsed.dynamic_range.clone(),
            parsed.frame_rate.clone(),
            parsed.color_depth.clone(),
            parsed.video_codec.clone(),
            parsed.audio_codec.clone(),
        ]
    } else {
        vec![
            parsed.video_format.clone(),
            if parsed.resource_type.is_empty() {
                parsed.release_type.clone()
            } else {
                parsed.resource_type.clone()
            },
            parsed.effect.clone(),
            parsed.audio_info.clone(),
            parsed.video_codec.clone(),
            parsed.audio_codec.clone(),
        ]
    };
    let mut body = Vec::new();
    for value in values.into_iter().filter(|value| !value.trim().is_empty()) {
        if body.last() != Some(&value) {
            body.push(value);
        }
    }
    let body = body.join(".");
    if parsed.release_group.is_empty() {
        body
    } else if body.is_empty() {
        parsed.release_group.clone()
    } else {
        format!("{body}-{}", parsed.release_group)
    }
}

fn append_media_info_suffix(
    relative: String,
    template: &str,
    context: &HashMap<String, String>,
    enabled: bool,
) -> String {
    let token = Regex::new(r"(?i)\{\{?\s*(?:media_info|video_?format|resource_?type|source|effect|audio_?info|video_?codec|audio_?codec|release_?group|release_?type|high_?quality|dolby_?vision|dynamic_?range|frame_?rate|color_?depth)\s*\}?\}").expect("technical template token regex");
    let media_info = context.get("media_info").cloned().unwrap_or_default();
    let probed = context
        .get("media_probed")
        .is_some_and(|value| !value.is_empty());
    if !enabled || !probed || media_info.is_empty() || token.is_match(template) {
        return relative;
    }
    match relative
        .rfind('.')
        .filter(|index| *index > relative.rfind('/').unwrap_or(0))
    {
        Some(index) => format!(
            "{}.{}{}",
            &relative[..index],
            media_info,
            &relative[index..]
        ),
        None => format!("{relative}.{media_info}"),
    }
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

/// 单次整理任务生命周期内的目标目录列表缓存（对齐 Node 端
/// `createTargetResolver`）。`resolve_target` / `ensure_target_directory`
/// 逐段解析路径，没有缓存时一个 24 集季的预览就要发数百次云端列目录请求，
/// 极易触发限流并把任务长时间钉在 recognizing/running。
struct TargetResolver {
    cache: HashMap<String, Vec<CloudEntry>>,
}

impl TargetResolver {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    async fn list(
        &mut self,
        app: &tauri::AppHandle,
        parent_id: &str,
        force: bool,
    ) -> Result<Vec<CloudEntry>, String> {
        if !force {
            if let Some(children) = self.cache.get(parent_id) {
                return Ok(children.clone());
            }
        }
        let children = list_cloud_children(app, parent_id).await?;
        self.cache.insert(parent_id.to_string(), children.clone());
        Ok(children)
    }

    fn invalidate(&mut self, parent_id: &str) {
        self.cache.remove(parent_id);
    }
}

async fn resolve_target(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
    resolver: &mut TargetResolver,
) -> Result<Option<CloudEntry>, String> {
    let mut parent_id = mapping.target_dir_id.clone();
    let mut current = None;
    for part in path_parts(relative) {
        let children = resolver.list(app, &parent_id, false).await?;
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
    resolver: &mut TargetResolver,
) -> Result<String, String> {
    let mut parent_id = mapping.target_dir_id.clone();
    for part in path_parts(relative) {
        let children = resolver.list(app, &parent_id, false).await?;
        if let Some(entry) = children
            .into_iter()
            .find(|entry| entry.name == part || entry.name.eq_ignore_ascii_case(part))
        {
            if !entry.is_directory {
                return Err(format!("目标路径包含同名文件：{part}"));
            }
            parent_id = entry.id;
        } else {
            let create_result = create_cloud_directory(app, &parent_id, part).await;
            resolver.invalidate(&parent_id);
            let refreshed = resolver.list(app, &parent_id, true).await?;
            if let Some(entry) = refreshed
                .into_iter()
                .find(|entry| entry.name == part || entry.name.eq_ignore_ascii_case(part))
            {
                if !entry.is_directory {
                    return Err(format!("目标路径包含同名文件：{part}"));
                }
                parent_id = entry.id;
            } else {
                parent_id = create_result?.id;
            }
        }
    }
    Ok(parent_id)
}

async fn plan_target(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
    source_identity: &str,
    conflict_policy: &str,
    claimed: &mut HashSet<String>,
    resolver: &mut TargetResolver,
) -> Result<(String, String, bool, Option<String>, bool), String> {
    let normalized = relative.trim_matches('/').replace('\\', "/");
    let mut target_relative = normalized.clone();
    let key = target_key(&target_relative);
    let existing = if claimed.contains(&key) {
        None
    } else {
        resolve_target(app, mapping, &target_relative, resolver).await?
    };
    if !claimed.contains(&key) && existing.is_none() {
        claimed.insert(key);
        return Ok((target_relative, "create".to_string(), false, None, false));
    }
    if !claimed.contains(&key) && conflict_policy == "skip" {
        claimed.insert(key);
        return Ok((
            target_relative,
            "skip".to_string(),
            true,
            existing.map(|entry| entry.id),
            false,
        ));
    }
    if !claimed.contains(&key) && conflict_policy == "overwrite" {
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
            || resolve_target(app, mapping, &target_relative, resolver)
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

/// 列出目标相对路径对应目录的现有子项；任一层级不存在时返回空列表（不创建目录）。
async fn list_target_children(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    parent_relative: &str,
    resolver: &mut TargetResolver,
) -> Result<Vec<CloudEntry>, String> {
    let mut parent_id = mapping.target_dir_id.clone();
    for part in path_parts(parent_relative) {
        let children = resolver.list(app, &parent_id, false).await?;
        let Some(entry) = children
            .into_iter()
            .find(|entry| entry.is_directory && (entry.name == part || entry.name.eq_ignore_ascii_case(part)))
        else {
            return Ok(Vec::new());
        };
        parent_id = entry.id;
    }
    resolver.list(app, &parent_id, false).await
}

fn file_stem_lower(name: &str) -> String {
    let extension = path_extension(name);
    let stem = if extension.is_empty() {
        name
    } else {
        &name[..name.len().saturating_sub(extension.len() + 1)]
    };
    stem.to_lowercase()
}

/// 在目标目录的既有条目中找出与新文件“同一内容”的旧版本视频：
/// 电影 = 同 part 的视频；剧集 = 同季同集。
fn find_existing_cloud_versions(
    media_type: &str,
    parsed: &crate::organizer_core::ParsedMediaName,
    entries: &[CloudEntry],
    settings: &NativeSettings,
) -> Vec<(CloudEntry, crate::organizer_core::ParsedMediaName)> {
    let overrides = crate::organizer_core::RecognitionOverrides {
        media_type: Some(media_type.to_string()),
        ..Default::default()
    };
    let mut versions = Vec::new();
    for entry in entries {
        if entry.is_directory || !video_extension(&entry.name) {
            continue;
        }
        let entry_parsed = crate::organizer_core::parse_media_name_with_settings(
            &entry.name,
            &overrides,
            settings,
        );
        if media_type == "tv" {
            if entry_parsed.season != parsed.season || entry_parsed.episode != parsed.episode {
                continue;
            }
        } else {
            let entry_part = entry_parsed.part.as_deref().unwrap_or("").trim().to_lowercase();
            let next_part = parsed.part.as_deref().unwrap_or("").trim().to_lowercase();
            if entry_part != next_part {
                continue;
            }
        }
        versions.push((entry.clone(), entry_parsed));
    }
    versions
}

/// 被替换旧版本连同其同名前缀的字幕/NFO 等伴随文件一起列入清理清单。
fn collect_replaced_cloud_files(
    versions: &[(CloudEntry, crate::organizer_core::ParsedMediaName)],
    entries: &[CloudEntry],
) -> Vec<ReplacedFile> {
    let mut seen = HashSet::new();
    let mut replaces = Vec::new();
    for (version, _) in versions {
        let stem = file_stem_lower(&version.name);
        for sibling in entries {
            if sibling.is_directory || sibling.id.is_empty() || seen.contains(&sibling.id) {
                continue;
            }
            let is_version_file = sibling.id == version.id;
            let is_sidecar =
                !video_extension(&sibling.name) && file_stem_lower(&sibling.name).starts_with(&stem);
            if !is_version_file && !is_sidecar {
                continue;
            }
            seen.insert(sibling.id.clone());
            replaces.push(ReplacedFile {
                id: sibling.id.clone(),
                name: sibling.name.clone(),
            });
        }
    }
    replaces
}

#[cfg(test)]
mod upgrade_version_tests {
    use super::*;

    fn entry(id: &str, name: &str, is_directory: bool, size: i64) -> CloudEntry {
        CloudEntry {
            id: id.to_string(),
            name: name.to_string(),
            is_directory,
            size,
            ..Default::default()
        }
    }

    #[test]
    fn movie_versions_match_by_part_and_take_sidecars_into_replacement_list() {
        let settings = NativeSettings::default();
        let entries = vec![
            entry("old-video", "Movie (2020) - 1080p BluRay x264.mkv", false, 100),
            entry("old-sub", "Movie (2020) - 1080p BluRay x264.chs.srt", false, 1),
            entry("old-nfo", "Movie (2020) - 1080p BluRay x264.nfo", false, 1),
            entry("poster", "poster.jpg", false, 1),
            entry("cd2", "Movie (2020) - 1080p BluRay x264 - CD2.mkv", false, 100),
            entry("dir", "extras", true, 0),
        ];
        let parsed = crate::organizer_core::parse_media_name(
            "Movie.2020.2160p.WEB-DL.mkv",
            &crate::organizer_core::RecognitionOverrides::default(),
        );
        let versions = find_existing_cloud_versions("movie", &parsed, &entries, &settings);
        assert_eq!(
            versions.iter().map(|(entry, _)| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["old-video"]
        );
        let mut replaced = collect_replaced_cloud_files(&versions, &entries)
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        replaced.sort();
        assert_eq!(replaced, vec!["old-nfo", "old-sub", "old-video"]);
    }

    #[test]
    fn tv_versions_match_by_season_and_episode() {
        let settings = NativeSettings::default();
        let entries = vec![
            entry("e1", "Show - S01E01 - 1080p.mkv", false, 10),
            entry("e2", "Show - S01E02 - 1080p.mkv", false, 10),
        ];
        let parsed = crate::organizer_core::parse_media_name(
            "Show.S01E02.2160p.mkv",
            &crate::organizer_core::RecognitionOverrides::default(),
        );
        let versions = find_existing_cloud_versions("tv", &parsed, &entries, &settings);
        assert_eq!(
            versions.iter().map(|(entry, _)| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["e2"]
        );
    }
}

#[derive(Debug, Clone, Default)]
struct PlannedUpgrade {
    planned: (String, String, bool, Option<String>, bool),
    replaces: Vec<ReplacedFile>,
    upgraded_by: Option<String>,
    suppressed: bool,
    suppressed_by: Option<String>,
    suppressed_existing: Option<String>,
}

/// 洗版目标解析：在目标目录中找同一内容的旧版本，按优先级比较后决定
/// 替换（action=upgrade，附带 replaces 清单）或压制跳过（suppressed）。
#[allow(clippy::too_many_arguments)]
async fn plan_upgrade_target(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
    source_identity: &str,
    media_type: &str,
    parsed: &crate::organizer_core::ParsedMediaName,
    size: i64,
    secrets: &OrganizerSecrets,
    claimed: &mut HashSet<String>,
    resolver: &mut TargetResolver,
) -> Result<PlannedUpgrade, String> {
    let normalized = relative.trim_matches('/').replace('\\', "/");
    let key = target_key(&normalized);
    if claimed.contains(&key) {
        // 同一批内多个来源渲染出同一目标：退化为“保留两份”，避免相互覆盖。
        let planned = plan_target(app, mapping, &normalized, source_identity, "rename", claimed, resolver).await?;
        return Ok(PlannedUpgrade { planned, ..Default::default() });
    }
    let parent_relative = path_parent(&normalized);
    let siblings = list_target_children(app, mapping, &parent_relative, resolver).await?;
    let versions = find_existing_cloud_versions(media_type, parsed, &siblings, &secrets.native);
    if versions.is_empty() {
        let planned = plan_target(app, mapping, &normalized, source_identity, "skip", claimed, resolver).await?;
        return Ok(PlannedUpgrade { planned, ..Default::default() });
    }
    let mut upgraded_by: Option<String> = None;
    for (entry, entry_parsed) in &versions {
        let verdict = crate::organizer_core::compare_media_versions(
            (parsed, size),
            (entry_parsed, entry.size),
            &secrets.upgrade_criteria,
            &secrets.upgrade_release_groups,
        );
        match verdict {
            crate::organizer_core::UpgradeVerdict::NextWins(criterion) => {
                if upgraded_by.is_none() {
                    upgraded_by = Some(criterion.to_string());
                }
            }
            crate::organizer_core::UpgradeVerdict::ExistingWins(criterion) => {
                claimed.insert(key);
                return Ok(PlannedUpgrade {
                    planned: (
                        normalized,
                        "skip".to_string(),
                        true,
                        Some(entry.id.clone()).filter(|id| !id.is_empty()),
                        false,
                    ),
                    suppressed: true,
                    suppressed_by: Some(criterion.to_string()),
                    suppressed_existing: Some(entry.name.clone()),
                    ..Default::default()
                });
            }
            crate::organizer_core::UpgradeVerdict::Tie => {
                claimed.insert(key);
                return Ok(PlannedUpgrade {
                    planned: (
                        normalized,
                        "skip".to_string(),
                        true,
                        Some(entry.id.clone()).filter(|id| !id.is_empty()),
                        false,
                    ),
                    suppressed: true,
                    suppressed_existing: Some(entry.name.clone()),
                    ..Default::default()
                });
            }
        }
    }
    claimed.insert(key);
    let replaces = collect_replaced_cloud_files(&versions, &siblings);
    let target_name_key = target_key(&path_name(&normalized));
    let exists = versions
        .iter()
        .any(|(entry, _)| target_key(&entry.name) == target_name_key);
    Ok(PlannedUpgrade {
        planned: (normalized, "upgrade".to_string(), exists, None, false),
        replaces,
        upgraded_by,
        ..Default::default()
    })
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
        ..Default::default()
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
        secrets.native.word_segment_search,
        secrets.native.similarity_match,
        secrets.native.recognition_words,
        secrets.native.release_groups,
        secrets.native.render_words,
        secrets.native.capture_groups,
        secrets.include_media_info,
        secrets.movie_path_template,
        secrets.tv_path_template,
        secrets.movie_category,
        secrets.tv_category,
        secrets.api_base,
        secrets.image_base,
        secrets.category_rules,
        secrets.scrape_targets,
        secrets.default_scrape_types,
        secrets.upgrade_criteria,
        secrets.upgrade_release_groups
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
    media_probe_warnings: &[String],
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
    // 预览阶段共享一份目标目录缓存，避免每个条目都从 B 根重新逐级列目录。
    let mut resolver = TargetResolver::new();
    let mut items = Vec::new();
    let mut video_targets = HashMap::new();
    // 洗版策略只作用于主视频；字幕/刮削产物遇同名默认跳过。
    let upgrade_policy = mapping.conflict_policy == "upgrade";
    let generated_conflict_policy = if upgrade_policy {
        "skip"
    } else {
        mapping.conflict_policy.as_str()
    };
    // 主视频洗版结论（suppressed / upgraded），字幕等伴随文件跟随。
    let mut video_flags: HashMap<String, (bool, bool)> = HashMap::new();
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
        relative =
            append_media_info_suffix(relative, template, &context, secrets.include_media_info);
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
        let upgrade_meta = if upgrade_policy && video.extra_kind.is_empty() {
            Some(
                plan_upgrade_target(
                    app,
                    mapping,
                    &relative,
                    &source_entry.id,
                    &metadata.media_type,
                    &video.parsed,
                    source_entry.size,
                    secrets,
                    &mut claimed,
                    &mut resolver,
                )
                .await?,
            )
        } else {
            None
        };
        let planned = match &upgrade_meta {
            Some(meta) => meta.planned.clone(),
            None => {
                let policy = if video.extra_kind.is_empty() {
                    mapping.conflict_policy.as_str()
                } else {
                    generated_conflict_policy
                };
                plan_target(app, mapping, &relative, &source_entry.id, policy, &mut claimed, &mut resolver).await?
            }
        };
        let message = match &upgrade_meta {
            Some(meta) if planned.1 == "upgrade" => format!(
                "洗版：{}更优，将替换 {}",
                crate::organizer_core::upgrade_criterion_label(
                    meta.upgraded_by.as_deref().unwrap_or("")
                ),
                meta.replaces
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            Some(meta) if meta.suppressed => match &meta.suppressed_by {
                Some(criterion) => format!(
                    "现有版本{}更优，已跳过（{}）",
                    crate::organizer_core::upgrade_criterion_label(criterion),
                    meta.suppressed_existing.as_deref().unwrap_or("")
                ),
                None => format!(
                    "已存在相同版本，已跳过（{}）",
                    meta.suppressed_existing.as_deref().unwrap_or("")
                ),
            },
            _ => "可执行".to_string(),
        };
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
            message,
        );
        if let Some(meta) = upgrade_meta {
            video_flags.insert(
                video.source.clone(),
                (meta.suppressed, meta.planned.1 == "upgrade"),
            );
            if let Some(last) = items.last_mut() {
                last.replaces = meta.replaces;
                last.upgraded_by = meta.upgraded_by;
                last.suppressed = meta.suppressed;
                last.suppressed_by = meta.suppressed_by;
                last.suppressed_existing = meta.suppressed_existing;
            }
        }
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
            let (follow_suppressed, follow_upgraded) = sidecar
                .video_source
                .as_ref()
                .and_then(|source| video_flags.get(source))
                .copied()
                .unwrap_or((false, false));
            let planned = if upgrade_policy && follow_suppressed {
                let normalized = relative.trim_matches('/').replace('\\', "/");
                claimed.insert(target_key(&normalized));
                (normalized, "skip".to_string(), false, None, false)
            } else if upgrade_policy && follow_upgraded {
                // 旧版本的伴随文件已随主视频列入替换清单；同名残留由执行期备份交换兜底。
                let normalized = relative.trim_matches('/').replace('\\', "/");
                claimed.insert(target_key(&normalized));
                (normalized, "upgrade".to_string(), false, None, false)
            } else {
                plan_target(
                    app,
                    mapping,
                    &relative,
                    &source_entry.id,
                    generated_conflict_policy,
                    &mut claimed,
                    &mut resolver,
                )
                .await?
            };
            let message = if upgrade_policy && follow_suppressed {
                "主视频被现有版本压制，跟随跳过"
            } else if planned.1 == "upgrade" {
                "跟随主视频洗版"
            } else {
                "跟随主视频整理"
            };
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
                message.to_string(),
            );
            if upgrade_policy && follow_suppressed {
                if let Some(last) = items.last_mut() {
                    last.suppressed = true;
                }
            }
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
                // 只为本次实际整理到的季生成季海报。TMDB 会返回剧集的全部季，
                // 找不到对应季视频时旧逻辑回落到剧集根目录 poster.jpg，与主
                // 海报撞名——冲突去重会产出 poster [hash].jpg 垃圾文件。
                let Some(season_root) = main_videos
                    .iter()
                    .find(|item| item.season == Some(season.season_number))
                    .map(|item| season_directory_for_cloud_video(item, &media_root_relative))
                else {
                    continue;
                };
                if season_root == media_root_relative {
                    continue;
                }
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
            generated_conflict_policy,
            &mut claimed,
            &mut resolver,
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
    let upgraded = items
        .iter()
        .filter(|item| item.action == "upgrade" && item.kind == "video")
        .count();
    let suppressed = items.iter().filter(|item| item.suppressed).count();
    let warnings = skipped + analysis.ignored_samples.len() + media_probe_warnings.len();
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
                "已生成 {} 项云端整理预览{}{}",
                items.len(),
                if upgraded > 0 {
                    format!("，洗版替换 {upgraded} 项")
                } else {
                    String::new()
                },
                if warnings > 0 {
                    format!("，{warnings} 项提示")
                } else {
                    String::new()
                }
            )
        },
        ignored_samples: analysis.ignored_samples.clone(),
        media_probe_warnings: media_probe_warnings.to_vec(),
        data: CloudPreviewData {
            summary: PreviewSummary {
                total: items.len(),
                success: items.len().saturating_sub(failed_items),
                failed: failed_items,
                warnings,
                skipped,
                upgraded,
                suppressed,
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

/// 字幕/音轨语言后缀（对齐 Node 端 `languageSuffix`）：输出 BCP-47 标记
/// （Emby/Jellyfin 才能识别），并保留 forced/sdh 标记，避免
/// `Show.chs.srt` 和 `Show.chs.forced.srt` 渲染成同名目标互相覆盖。
fn language_suffix(value: &str) -> String {
    static ZH_CN: OnceLock<Regex> = OnceLock::new();
    static ZH_TW: OnceLock<Regex> = OnceLock::new();
    static EN: OnceLock<Regex> = OnceLock::new();
    static JA: OnceLock<Regex> = OnceLock::new();
    static KO: OnceLock<Regex> = OnceLock::new();
    static FORCED: OnceLock<Regex> = OnceLock::new();
    static SDH: OnceLock<Regex> = OnceLock::new();
    let name = path_stem(value).to_lowercase();
    let language = if ZH_CN
        .get_or_init(|| {
            Regex::new(r"(?i)(?:zh[-_. ]?(?:cn|hans)|chs|(?:^|[._ -])sc(?:[._ -]|$)|简体|簡體|简中)")
                .expect("zh-cn subtitle regex")
        })
        .is_match(&name)
    {
        "zh-CN"
    } else if ZH_TW
        .get_or_init(|| {
            Regex::new(r"(?i)(?:zh[-_. ]?(?:tw|hant)|cht|(?:^|[._ -])tc(?:[._ -]|$)|繁体|繁體|繁中)")
                .expect("zh-tw subtitle regex")
        })
        .is_match(&name)
    {
        "zh-TW"
    } else if EN
        .get_or_init(|| {
            Regex::new(r"(?i)(?:^|[._ -])(?:eng|en)(?:[._ -]|$)|英文").expect("en subtitle regex")
        })
        .is_match(&name)
    {
        "en"
    } else if JA
        .get_or_init(|| {
            Regex::new(r"(?i)(?:^|[._ -])(?:jpn|ja|jp)(?:[._ -]|$)|日文|日语|日語")
                .expect("ja subtitle regex")
        })
        .is_match(&name)
    {
        "ja"
    } else if KO
        .get_or_init(|| {
            Regex::new(r"(?i)(?:^|[._ -])(?:kor|ko|kr)(?:[._ -]|$)|韩文|韓文|韩语|韓語")
                .expect("ko subtitle regex")
        })
        .is_match(&name)
    {
        "ko"
    } else {
        ""
    };
    let forced = FORCED
        .get_or_init(|| Regex::new(r"(?i)(?:^|[._ -])forced(?:[._ -]|$)").expect("forced regex"))
        .is_match(&name);
    let sdh = SDH
        .get_or_init(|| Regex::new(r"(?i)(?:^|[._ -])(?:sdh|hi)(?:[._ -]|$)").expect("sdh regex"))
        .is_match(&name);
    format!(
        "{}{}{}",
        if language.is_empty() {
            String::new()
        } else {
            format!(".{language}")
        },
        if forced { ".forced" } else { "" },
        if sdh { ".sdh" } else { "" }
    )
}

#[derive(Debug, Clone)]
struct TransferStep {
    operation: String,
    created_id: String,
    source_parent_id: String,
    source_name: String,
    target_name: String,
    kind: String,
    target_relative: String,
    backup: Option<(String, String)>,
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
    resolver: &mut TargetResolver,
) -> Result<(usize, usize, Vec<CreatedOutputItem>), String> {
    let mut transaction = Vec::new();
    let mut unattached_backups: Vec<(String, String)> = Vec::new();
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
    let mut groups: HashMap<String, Vec<CloudPreviewItem>> = HashMap::new();
    for item in candidates {
        groups
            .entry(item.target_parent_relative.clone())
            .or_default()
            .push(item);
    }
    let operation = mapping.transfer_type.as_str();
    let outcome = async {
        for (target_parent_relative, items) in groups {
            let target_parent_id =
                ensure_target_directory(app, mapping, &target_parent_relative, resolver).await?;
            let mut prepared = Vec::new();
            for item in items {
                if item.action == "skip" && item.suppressed {
                    skipped += 1;
                    continue;
                }
                // 洗版：转移前把被替换的旧版本及其伴随文件移入回收站。
                if item.action == "upgrade" && !item.replaces.is_empty() {
                    for replaced in &item.replaces {
                        cloud_delete(app, &replaced.id).await.map_err(|error| {
                            format!("洗版删除旧版本失败（{}）：{error}", replaced.name)
                        })?;
                    }
                    resolver.invalidate(&target_parent_id);
                }
                let existing = resolve_target(app, mapping, &item.target_relative, resolver).await?;
                if item.action == "skip" && existing.is_some() {
                    skipped += 1;
                    continue;
                }
                if item.action == "create" && existing.is_some() {
                    return Err(format!("预览后目标已出现同名项目：{}", item.target));
                }
                let backup = if let Some(existing) = existing {
                    let backup_name = format!(".__gy_org_backup_{}", Uuid::new_v4().simple());
                    cloud_rename(app, &existing.id, &backup_name).await?;
                    resolver.invalidate(&target_parent_id);
                    let backup = (existing.id, existing.name);
                    unattached_backups.push(backup.clone());
                    Some(backup)
                } else {
                    None
                };
                prepared.push((item, backup));
            }
            if prepared.is_empty() {
                continue;
            }
            let before = resolver
                .list(app, &target_parent_id, true)
                .await?
                .into_iter()
                .map(|entry| entry.id)
                .collect::<HashSet<_>>();
            let source_ids = prepared
                .iter()
                .filter_map(|(item, _)| item.source_id.clone())
                .collect::<Vec<_>>();
            if operation == "move" {
                cloud_move_many(app, &source_ids, &target_parent_id).await?;
            } else {
                cloud_copy_many(app, &source_ids, &target_parent_id).await?;
            }
            let after = resolver.list(app, &target_parent_id, true).await?;
            let mut available_copies = after
                .iter()
                .filter(|entry| !before.contains(&entry.id))
                .cloned()
                .collect::<Vec<_>>();
            let mut renames = Vec::new();
            for (item, backup) in prepared {
                let source_id = item.source_id.clone().unwrap_or_default();
                let source_name = item
                    .source_name
                    .clone()
                    .unwrap_or_else(|| path_name(item.source.as_deref().unwrap_or_default()));
                let created = if operation == "move" {
                    after.iter().find(|entry| entry.id == source_id).cloned()
                } else {
                    available_copies
                        .iter()
                        .position(|entry| entry.name == source_name)
                        .map(|index| available_copies.remove(index))
                }
                .ok_or_else(|| {
                    format!(
                        "云端{}已完成，但无法定位目标资源：{source_name}",
                        if operation == "move" {
                            "移动"
                        } else {
                            "复制"
                        }
                    )
                })?;
                if created.name != item.target_name {
                    renames.push((created.id.clone(), item.target_name.clone()));
                }
                if let Some((backup_id, _)) = &backup {
                    unattached_backups.retain(|(id, _)| id != backup_id);
                }
                transaction.push(TransferStep {
                    operation: operation.to_string(),
                    created_id: created.id,
                    source_parent_id: item.source_parent_id.unwrap_or_default(),
                    source_name,
                    target_name: item.target_name,
                    kind: item.kind.clone(),
                    target_relative: item.target_relative.clone(),
                    backup,
                });
            }
            cloud_rename_many(app, renames).await?;
            // 移动/复制 + 批量改名都改变了目标目录内容，之后的组必须重新列取。
            resolver.invalidate(&target_parent_id);
            transferred += source_ids.len();
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = outcome {
        let mut warnings = rollback_transfers(app, &transaction).await;
        for (backup_id, original_name) in unattached_backups {
            if let Err(rollback_error) = cloud_rename(app, &backup_id, &original_name).await {
                warnings.push(format!(
                    "恢复覆盖备份 {original_name} 失败：{rollback_error}"
                ));
            }
        }
        return Err(format_with_rollback(error, warnings));
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
            .map(|step| CreatedOutputItem {
                id: step.created_id.clone(),
                name: step.target_name.clone(),
                kind: step.kind.clone(),
                target_relative: step.target_relative.clone(),
            })
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
    resolver: &mut TargetResolver,
) -> (usize, usize, Vec<String>, Vec<CreatedOutputItem>) {
    let Some(metadata) = preview.metadata.as_ref() else {
        return (
            0,
            0,
            vec!["没有可用的 TMDB 元数据，已跳过刮削".to_string()],
            Vec::new(),
        );
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
    let mut created = Vec::new();
    for item in generated {
        let parent_id =
            match ensure_target_directory(app, mapping, &item.target_parent_relative, resolver)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(format!("{}：{error}", item.target));
                    continue;
                }
            };
        let existing = match resolve_target(app, mapping, &item.target_relative, resolver).await {
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
        // 预览后目标出现了同名文件：不静默覆盖别的进程刚写入的内容
        // （对齐 Node 端行为）。
        if item.action == "create" && existing.is_some() {
            warnings.push(format!("{}：预览后目标已出现同名文件，已跳过", item.target));
            continue;
        }
        let mut backup = None;
        if let Some(existing) = existing {
            let backup_name = format!(".__gy_org_meta_{}", Uuid::new_v4().simple());
            if let Err(error) = cloud_rename(app, &existing.id, &backup_name).await {
                warnings.push(format!("{}：覆盖前备份失败：{error}", item.target));
                continue;
            }
            resolver.invalidate(&parent_id);
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
            resolver.invalidate(&parent_id);
            warnings.push(format!("{}：上传刮削元数据失败：{error}", item.target));
            continue;
        }
        if let Some((backup_id, _)) = backup {
            // 备份清理失败要留痕：静默忽略会在 B 目录残留 .__gy_org_meta_* 孤儿文件。
            if let Err(error) = cloud_delete(app, &backup_id).await {
                warnings.push(format!(
                    "{}：已写入新元数据，但清理覆盖备份失败：{error}",
                    item.target
                ));
            }
        }
        resolver.invalidate(&parent_id);
        scraped += 1;
        created.push(CreatedOutputItem {
            id: String::new(),
            name: item.target_name.clone(),
            kind: item.kind.clone(),
            target_relative: item.target_relative.clone(),
        });
    }
    (scraped, skipped, warnings, created)
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
    let proxy = load_global_network_proxy(&db_path)?;
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
        &proxy,
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
    let word_segment_search = input
        .word_segment_search
        .unwrap_or(current.native.word_segment_search);
    let similarity_match = input
        .similarity_match
        .unwrap_or(current.native.similarity_match);
    let recognition_words = normalize_rule_text(
        input
            .recognition_words
            .as_deref()
            .unwrap_or(&current.native.recognition_words),
        "自定义识别词",
    )?;
    let release_groups = normalize_rule_text(
        input
            .release_groups
            .as_deref()
            .unwrap_or(&current.native.release_groups),
        "自定义制作组",
    )?;
    let render_words = normalize_rule_text(
        input
            .render_words
            .as_deref()
            .unwrap_or(&current.native.render_words),
        "自定义渲染词",
    )?;
    let capture_groups = normalize_rule_text(
        input
            .capture_groups
            .as_deref()
            .unwrap_or(&current.native.capture_groups),
        "自定义捕获组",
    )?;
    let include_media_info = input
        .include_media_info
        .unwrap_or(current.include_media_info);
    validate_auxiliary_rule_block(&recognition_words, "自定义识别词", true)?;
    validate_auxiliary_rule_block(&render_words, "自定义渲染词", true)?;
    validate_auxiliary_rule_block(&capture_groups, "自定义捕获组", false)?;
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
        input
            .tmdb_image_base
            .as_deref()
            .unwrap_or(&current.image_base),
        "https://image.tmdb.org/t/p",
        "TMDB 图片镜像",
    )?;
    let category_rules = normalize_category_rules(
        input
            .category_rules
            .or_else(|| Some(current.category_rules.clone())),
    )?;
    let scrape_targets = normalize_scrape_targets(
        input
            .scrape_targets
            .or_else(|| Some(current.scrape_targets.clone())),
    )?;
    let default_scrape_types = normalize_scrape_types(
        input
            .default_scrape_types
            .as_deref()
            .unwrap_or(&current.default_scrape_types),
        true,
    )?;
    let upgrade_criteria = crate::organizer_core::normalize_upgrade_criteria(
        input
            .upgrade_criteria
            .as_deref()
            .unwrap_or(&current.upgrade_criteria),
    );
    let upgrade_release_groups = normalize_rule_text(
        input
            .upgrade_release_groups
            .as_deref()
            .unwrap_or(&current.upgrade_release_groups),
        "洗版制作组优先级",
    )?;
    let previous_target_ids = current
        .scrape_targets
        .iter()
        .filter_map(|target| target.get("dir_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let next_target_ids = scrape_targets
        .iter()
        .filter_map(|target| target.get("dir_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<HashSet<_>>();
    let removed_target_ids = previous_target_ids
        .difference(&next_target_ids)
        .cloned()
        .collect::<HashSet<_>>();
    if let Some(mapping) = list_mappings(path)?
        .into_iter()
        .find(|mapping| removed_target_ids.contains(&mapping.target_dir_id))
    {
        return Err(format!(
            "刮削输出“{}”仍被整理监控使用，请先修改对应监控的输出目标",
            mapping.target_path
        ));
    }
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO organizer_settings
             (id, tmdb_api_key, language, image_language, include_adult, minimum_match_score,
              word_segment_search, similarity_match, recognition_words, release_groups, render_words, capture_groups,
              movie_path_template, tv_path_template, movie_category, tv_category,
              tmdb_api_base, tmdb_image_base, category_rules, scrape_targets, default_scrape_types, include_media_info,
              upgrade_criteria, upgrade_release_groups, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
             ON CONFLICT(id) DO UPDATE SET tmdb_api_key=excluded.tmdb_api_key,
              language=excluded.language, image_language=excluded.image_language,
              include_adult=excluded.include_adult, minimum_match_score=excluded.minimum_match_score,
              word_segment_search=excluded.word_segment_search, similarity_match=excluded.similarity_match,
              recognition_words=excluded.recognition_words, release_groups=excluded.release_groups,
              render_words=excluded.render_words, capture_groups=excluded.capture_groups,
              movie_path_template=excluded.movie_path_template, tv_path_template=excluded.tv_path_template,
               movie_category=excluded.movie_category, tv_category=excluded.tv_category,
               tmdb_api_base=excluded.tmdb_api_base, tmdb_image_base=excluded.tmdb_image_base,
               category_rules=excluded.category_rules, scrape_targets=excluded.scrape_targets,
               default_scrape_types=excluded.default_scrape_types, include_media_info=excluded.include_media_info,
              upgrade_criteria=excluded.upgrade_criteria, upgrade_release_groups=excluded.upgrade_release_groups,
              updated_at=excluded.updated_at",
            params![
                api_key,
                language,
                image_language,
                i64::from(input.include_adult.unwrap_or(current.native.include_adult)),
                minimum_match_score,
                i64::from(word_segment_search),
                i64::from(similarity_match),
                recognition_words,
                release_groups,
                render_words,
                capture_groups,
                movie_path_template,
                tv_path_template,
                movie_category,
                tv_category,
                api_base,
                image_base,
                serde_json::to_string(&category_rules).map_err(|error| format!("序列化媒体分类失败：{error}"))?,
                serde_json::to_string(&scrape_targets).map_err(|error| format!("序列化刮削目标失败：{error}"))?,
                serde_json::to_string(&default_scrape_types).map_err(|error| format!("序列化默认刮削类型失败：{error}"))?,
                i64::from(include_media_info),
                serde_json::to_string(&upgrade_criteria).map_err(|error| format!("序列化洗版优先级失败：{error}"))?,
                upgrade_release_groups,
                now_seconds()
            ],
        )
        .map_err(|error| format!("保存整理设置失败：{error}"))?;
    for target in &scrape_targets {
        let Some(dir_id) = target.get("dir_id").and_then(Value::as_str) else {
            continue;
        };
        let target_path = target
            .get("path")
            .and_then(Value::as_str)
            .map(normalize_cloud_path)
            .unwrap_or_else(|| "/".to_string());
        connection
            .execute(
                "UPDATE organizer_mappings SET target_path=?1, updated_at=?2 WHERE target_dir_id=?3",
                params![target_path, now_seconds(), dir_id],
            )
            .map_err(|error| format!("同步整理监控输出路径失败：{error}"))?;
    }
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
    if (transfer_type == "move" || conflict_policy == "overwrite" || conflict_policy == "upgrade")
        && !risk
    {
        return Err("移动、覆盖或洗版可能使已有分享失效，请先确认分享失效风险".to_string());
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
    // recognizing 状态加时间兜底：识别正常几分钟内就会结束，超过 10 分钟
    // 未更新的 recognizing 视为历史残留（例如进程中断），不再永久锁住
    // 监控配置的修改与删除。
    let stale_before = now_seconds() - 600;
    let running = open_database(path)?
        .query_row(
            "SELECT id FROM organizer_jobs WHERE mapping_id=?1
             AND (status='running' OR (status='recognizing' AND updated_at>=?2)) LIMIT 1",
            params![id, stale_before],
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
        episode_offset: if input.clear_episode_offset {
            None
        } else {
            input.episode_offset.or(job.episode_offset)
        },
        recognition_words: if input.clear_recognition_words {
            None
        } else {
            input
                .recognition_words
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| job.recognition_words.clone())
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
    // 识别前置读取失败必须落库为 failed：任务初始状态就是 recognizing，
    // 若此处直接返回错误，任务会永久停在 recognizing——既不会被轮询重试
    // （去重查询包含 recognizing），还会让 mapping_idle 永久拒绝修改监控。
    let loaded = match load_candidate(app, &mapping, &job.source_id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => {
            update_job_fields(
                &path,
                id,
                &[
                    ("status", json!("failed")),
                    ("error_code", json!("source_missing")),
                    ("message", json!("待整理云端项目已经不存在")),
                ],
            )?;
            emit(
                app,
                "job-updated",
                json!({ "job_id": id, "mapping_id": mapping.id, "status": "failed" }),
            );
            return get_job(&path, id)?.ok_or_else(|| "整理任务不存在".to_string());
        }
        Err(error) => {
            update_job_fields(
                &path,
                id,
                &[
                    ("status", json!("failed")),
                    ("error_code", json!("source_unavailable")),
                    ("message", json!(format!("读取待整理云端内容失败：{error}"))),
                ],
            )?;
            emit(
                app,
                "job-updated",
                json!({ "job_id": id, "mapping_id": mapping.id, "status": "failed", "message": error }),
            );
            return Err(error);
        }
    };
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
            ("episode_offset", json!(overrides.episode_offset)),
            ("recognition_words", json!(overrides.recognition_words.clone())),
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
    let mut secrets = load_secrets(&path)?;
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
    // 临时识别词只对本任务生效，且优先于全局规则执行
    if let Some(words) = overrides
        .recognition_words
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        secrets.native.recognition_words = if secrets.native.recognition_words.trim().is_empty() {
            words.to_string()
        } else {
            format!("{words}\n{}", secrets.native.recognition_words)
        };
    }
    let secrets = secrets;
    let recognition = async {
        let (mut analysis, _) = analyze_cloud_candidate(&loaded, &overrides, &secrets.native)?;
        let media_probe_warnings = if secrets.include_media_info {
            enrich_analysis_with_media_info(app, &loaded, &mut analysis).await
        } else {
            Vec::new()
        };
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
        if resolved.tmdb_id.is_none() {
            resolved.tmdb_id = analysis.tmdb_id;
        }
        let match_result =
            resolve_tmdb_match(&analysis, &secrets.client()?, &secrets.native, &resolved).await?;
        let preview = build_preview(
            app,
            &loaded,
            &analysis,
            &match_result,
            &mapping,
            &secrets,
            &media_probe_warnings,
        )
        .await?;
        Ok::<_, String>((match_result, preview))
    }
    .await;
    let (match_result, preview) = match recognition {
        Ok(value) => value,
        Err(error) => {
            let needs_review = error.contains("没有找到可整理的视频")
                || error.contains("无法从文件名提取媒体名称");
            let error_code = if needs_review {
                if error.contains("视频") {
                    "video_required"
                } else {
                    "title_required"
                }
            } else if error.contains("TMDB") {
                "tmdb_unavailable"
            } else {
                "recognition_failed"
            };
            let status = if needs_review {
                "needs_review"
            } else {
                "failed"
            };
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
            if let Some(job_id) = duplicate {
                // 命中同签名的已有任务：绝不重新识别或执行（对齐 Node 端）。
                // 复制模式下 A 目录源文件长期存在，每轮轮询都会命中同一签名；
                // 旧逻辑会把 completed 任务打回 recognizing 并重复执行——
                // "保留两份"策略下每 15 秒生成一份重复文件，开启自动分享时
                // 每轮创建一个新分享。
                if let Some(true) = share_after {
                    update_job_fields(&path, &job_id, &[("share_after_requested", json!(1))])?;
                }
                return Ok(());
            }
            let job_id = insert_job(&path, &mapping, &loaded.candidate, &loaded.fingerprint, share_after.unwrap_or(mapping.share_after_organize))?;
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
        // 源内容在预览后发生变化（常见于上传仍在进行）：自动按最新内容重新识别
        // 并执行，而不是报错卡在待执行。相互递归需要 Box::pin 打断 future 循环。
        return Box::pin(recognize_job(
            app,
            state,
            id,
            OrganizerJobInput::default(),
            true,
        ))
        .await;
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
        // 转移、刮削与分享共享同一份目标目录缓存，写操作后按父目录精确失效。
        let mut resolver = TargetResolver::new();
        let (transferred, skipped, transferred_items) =
            execute_transfers(app, &mapping, &preview, &mut resolver).await?;
        let (scraped, scrape_skipped, mut warnings, scraped_items) =
            execute_scrape(app, &mapping, &preview, &secrets.tmdb_proxy, &mut resolver).await;
        let targets = transferred_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let created_items = transferred_items
            .into_iter()
            .chain(scraped_items)
            .collect::<Vec<_>>();
        let mut share = None;
        if job.share_after_requested || mapping.share_after_organize {
            match ensure_target_directory(app, &mapping, &preview.share_relative_path, &mut resolver)
                .await
            {
                Ok(target_id) => match create_fresh_organizer_share(
                    app.clone(),
                    &mapping.id,
                    &target_id,
                    &preview.share_title,
                )
                .await
                {
                    Ok(value) => share = Some(value),
                    Err(error) => {
                        warnings.push(format!("整理已完成，但创建 B 目录新分享失败：{error}"))
                    }
                },
                Err(error) => {
                    warnings.push(format!("整理已完成，但无法定位 B 目录分享目标：{error}"))
                }
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
            created_items,
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
    // 移动模式的固定说明只进 message，不进 warnings：它不是异常，
    // 不应把所有移动任务都渲染成 completed_warning，把真正的刮削失败
    // 淹没在同一个提示堆里。
    let message = format!(
        "云盘整理完成：转移 {} 项，刮削 {} 项{}{}{}",
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
        },
        if mapping.transfer_type == "move" {
            "；提醒：云端移动会使来源资源的已有分享失效"
        } else {
            ""
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
    // 通知虚拟库同步覆盖 B 目录的 STRM 并刷新 Emby；失败不影响整理结果。
    if result.transferred > 0 {
        let shared = app.state::<SharedState>();
        let _ = crate::mounts::sync_virtual_libraries_for_cloud_target(
            app,
            shared.inner(),
            &mapping.target_dir_id,
            &mapping.target_path,
        );
    }
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
            if let Err(error) =
                scan_mapping_inner(&app, &state, &mapping.id, false, None, None, None).await
            {
                // 轮询失败要落库并通知前端，否则登录态失效/目录被删时监控
                // 看起来一切正常，只是永远不产出任务。
                if let Ok(path) = database_path(&state) {
                    let _ = open_database(&path).and_then(|connection| {
                        connection
                            .execute(
                                "UPDATE organizer_mappings SET watch_error=?1, updated_at=?2 WHERE id=?3",
                                params![error, now_seconds(), mapping.id],
                            )
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    });
                }
                emit(
                    &app,
                    "mapping-error",
                    json!({ "mapping_id": mapping.id, "message": error }),
                );
            }
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
        input
            .tmdb_image_base
            .as_deref()
            .unwrap_or(&current.image_base),
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
    let target = targets
        .iter()
        .find(|item| {
            input
                .target_id
                .as_deref()
                .is_some_and(|id| item.get("id").and_then(Value::as_str) == Some(id))
        })
        .or_else(|| (targets.len() == 1).then(|| &targets[0]))
        .ok_or_else(|| {
            if targets.is_empty() {
                "请先在设置 > 整理 > 刮削偏好中配置媒体库目标"
            } else {
                "请选择一个已配置的刮削目标目录"
            }
        })?;
    let target_dir_id = target
        .get("dir_id")
        .or_else(|| target.get("target_dir_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target_path = target
        .get("path")
        .or_else(|| target.get("target_path"))
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_string();
    if target_dir_id.trim().is_empty() {
        return Err("刮削目标目录配置无效".to_string());
    }
    let default_scrape_types = secrets.default_scrape_types.clone();
    let scrape_types = normalize_scrape_types(
        input
            .scrape_types
            .as_deref()
            .unwrap_or(&default_scrape_types),
        true,
    )?;
    let transfer_type_input = input.transfer_type.clone();
    let media_type_input = input.media_type.clone();
    let share_risk_acknowledged = input.share_risk_acknowledged;
    let mut jobs = Vec::new();
    let mut failures = Vec::new();
    let mut queued = Vec::new();
    let mut candidate_ids = HashSet::new();
    let selected_count = input.files.len();
    let _ = list_cloud_children(&app, &target_dir_id).await?;
    for source in input.files.into_iter().take(100) {
        let submitted_id = source.id.clone();
        let source_id = source.id.trim().to_string();
        let source_parent_id = source.parent_id.trim().to_string();
        if source_id.is_empty() || source_parent_id.is_empty() {
            failures.push(json!({ "id": submitted_id, "message": "选中项缺少文件 ID 或来源目录" }));
            continue;
        }
        let source_path = normalize_cloud_path(
            source
                .parent_path
                .as_deref()
                .or(source.path.as_deref())
                .unwrap_or("/"),
        );
        let transfer_type =
            normalize_transfer_type(transfer_type_input.as_deref().unwrap_or("copy"))?;
        if transfer_type == "move" && !share_risk_acknowledged {
            return Err("移动可能使已有分享失效，请先确认风险".to_string());
        }
        let explicit_media_type = normalize_media_type(
            media_type_input
                .as_deref()
                .or(source.media_type.as_deref())
                .unwrap_or(""),
        )?;
        let base_mapping = OrganizerMapping {
            id: format!("manual:{}", Uuid::new_v4()),
            source_path: source_path.clone(),
            target_path: normalize_cloud_path(&target_path),
            source_dir_id: source_parent_id,
            target_dir_id: target_dir_id.clone(),
            enabled: true,
            scan_existing: false,
            monitor_mode: "manual".to_string(),
            transfer_type: transfer_type.clone(),
            media_type: explicit_media_type.clone(),
            scrape: true,
            scrape_types: scrape_types.clone(),
            sync_extras: true,
            conflict_policy: "skip".to_string(),
            auto_execute: false,
            share_after_organize: false,
            share_risk_acknowledged,
            settle_seconds: 5,
            watch_error: None,
            created_at: now_seconds(),
            updated_at: now_seconds(),
        };
        let loaded = match load_candidate(&app, &base_mapping, &source_id).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                failures.push(json!({ "id": submitted_id, "message": "选中的云端项目已经不存在" }));
                continue;
            }
            Err(error) => {
                failures.push(json!({ "id": submitted_id, "message": error }));
                continue;
            }
        };
        let planned = plan_cloud_scrape_candidates(&loaded, &secrets.native);
        if planned.is_empty() {
            failures.push(json!({ "id": submitted_id, "message": "没有找到可刮削的视频文件" }));
            continue;
        }
        for candidate in planned {
            if candidate_ids.contains(&candidate.entry.id) {
                continue;
            }
            if candidate_ids.len() >= MAX_SCRAPE_CANDIDATES {
                failures.push(json!({
                    "id": candidate.entry.id,
                    "message": format!("拆分后超过 {MAX_SCRAPE_CANDIDATES} 个媒体候选，请缩小刮削范围")
                }));
                break;
            }
            candidate_ids.insert(candidate.entry.id.clone());
            let parent_path = if candidate.entry.id == loaded.candidate.id {
                source_path.clone()
            } else {
                let parent_logical = path_parent(&candidate.entry.logical_path);
                let relative_parent = parent_logical
                    .strip_prefix(loaded.candidate.logical_path.trim_matches('/'))
                    .unwrap_or(&parent_logical)
                    .trim_matches('/');
                normalize_cloud_path(&join_relative(&[
                    &source_path,
                    &loaded.candidate.name,
                    relative_parent,
                ]))
            };
            let mut suggested_title = candidate.suggested_title.clone();
            if suggested_title.is_empty()
                && candidate.reason == "season-folder"
                && candidate.entry.id == loaded.candidate.id
            {
                suggested_title = useful_context_title(&source_path, &secrets.native);
            }
            let mut mapping = base_mapping.clone();
            mapping.id = format!("manual:{}", Uuid::new_v4());
            mapping.source_path = parent_path;
            mapping.source_dir_id = candidate.entry.parent_id.clone();
            if mapping.media_type.is_empty() {
                mapping.media_type = candidate.suggested_media_type.clone();
            }
            mapping.created_at = now_seconds();
            mapping.updated_at = mapping.created_at;
            if let Err(error) = save_mapping(&path, &mapping) {
                failures.push(json!({ "id": candidate.entry.id, "name": candidate.entry.name, "message": error }));
                continue;
            }
            let job_id = Uuid::new_v4().to_string();
            let source_display_path = join_relative(&[&mapping.source_path, &candidate.entry.name]);
            let timestamp = now_seconds();
            let insert = open_database(&path)?.execute(
                "INSERT INTO organizer_jobs
                 (id, mapping_id, source_path, source_id, source_parent_id, source_size,
                  source_modified_ms, source_file_count, source_signature, share_after_requested,
                  status, media_type, query_title, message, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, '0', ?6, '', 0, 'recognizing', ?7, ?8,
                         '刮削已提交，等待后台识别', ?9, ?9)",
                params![
                    job_id,
                    mapping.id,
                    source_display_path,
                    candidate.entry.id,
                    mapping.source_dir_id,
                    candidate.video_count as i64,
                    if mapping.media_type.is_empty() {
                        None::<String>
                    } else {
                        Some(mapping.media_type.clone())
                    },
                    if suggested_title.is_empty() {
                        None::<String>
                    } else {
                        Some(suggested_title)
                    },
                    timestamp,
                ],
            );
            if let Err(error) = insert {
                failures.push(json!({ "id": candidate.entry.id, "name": candidate.entry.name, "message": format!("创建刮削任务失败：{error}") }));
                continue;
            }
            emit(
                &app,
                "job-updated",
                json!({ "job_id": job_id.clone(), "mapping_id": mapping.id.clone(), "status": "recognizing" }),
            );
            match get_job(&path, &job_id)? {
                Some(job) => {
                    jobs.push(job);
                    queued.push((job_id, mapping.id));
                }
                None => failures.push(json!({ "id": candidate.entry.id, "name": candidate.entry.name, "message": "刮削任务创建后无法读取" })),
            }
        }
    }
    if !queued.is_empty() {
        let task_app = app.clone();
        let task_state = state.inner().clone();
        let task_path = path.clone();
        tauri::async_runtime::spawn(async move {
            let recognized = stream::iter(queued)
                .map(|(job_id, mapping_id)| {
                    let worker_app = task_app.clone();
                    let worker_state = task_state.clone();
                    let worker_path = task_path.clone();
                    async move {
                        match recognize_job(
                            &worker_app,
                            &worker_state,
                            &job_id,
                            OrganizerJobInput::default(),
                            false,
                        )
                        .await
                        {
                            Ok(job) if job.status == "ready" => Some(job),
                            Ok(_) => None,
                            Err(error) => {
                                let _ = update_job_fields(
                                    &worker_path,
                                    &job_id,
                                    &[
                                        ("status", json!("failed")),
                                        ("error_code", json!("scrape_failed")),
                                        ("message", json!(error.clone())),
                                    ],
                                );
                                emit(
                                    &worker_app,
                                    "job-updated",
                                    json!({
                                        "job_id": job_id,
                                        "mapping_id": mapping_id,
                                        "status": "failed",
                                        "message": error,
                                    }),
                                );
                                None
                            }
                        }
                    }
                })
                .buffer_unordered(SCRAPE_RECOGNITION_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
            let mut target_groups: HashMap<String, Vec<String>> = HashMap::new();
            for job in recognized.into_iter().flatten() {
                let key = job
                    .preview
                    .as_ref()
                    .map(|preview| {
                        format!(
                            "{}::{}",
                            preview.target_root_id, preview.media_root_relative
                        )
                    })
                    .unwrap_or_else(|| job.id.clone());
                target_groups.entry(key).or_default().push(job.id);
            }
            stream::iter(target_groups.into_values())
                .map(|group| {
                    let worker_app = task_app.clone();
                    let worker_state = task_state.clone();
                    let worker_path = task_path.clone();
                    async move {
                        for job_id in group {
                            if let Err(error) = execute_job(&worker_app, &worker_state, &job_id).await {
                                let mapping_id = get_job(&worker_path, &job_id)
                                    .ok()
                                    .flatten()
                                    .map(|job| job.mapping_id)
                                    .unwrap_or_default();
                                let _ = update_job_fields(
                                    &worker_path,
                                    &job_id,
                                    &[
                                        ("status", json!("failed")),
                                        ("error_code", json!("scrape_failed")),
                                        ("message", json!(error.clone())),
                                    ],
                                );
                                emit(
                                    &worker_app,
                                    "job-updated",
                                    json!({ "job_id": job_id, "mapping_id": mapping_id, "status": "failed", "message": error }),
                                );
                            }
                        }
                    }
                })
                .buffer_unordered(SCRAPE_EXECUTION_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
        });
    }
    Ok(
        json!({ "jobs": jobs, "failures": failures, "planned": jobs.len(), "selected": selected_count }),
    )
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
    let mut mapping = bind_configured_output_target(
        normalize_mapping_input(input, None, &secrets.default_scrape_types)?,
        &secrets.scrape_targets,
        false,
    )?;
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
    let target_was_submitted = input.target_dir_id.is_some() || input.target_path.is_some();
    let mapping = bind_configured_output_target(
        normalize_mapping_input(input, Some(&current), &secrets.default_scrape_types)?,
        &secrets.scrape_targets,
        !target_was_submitted,
    )?;
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
    // 只把旧预览打回待复核，不删除任务：needs_review 队列是用户的人工复核
    // 工作区，调一次模板就整队清空的旧行为会丢掉所有待处理任务。
    open_database(&path)?
        .execute(
            "UPDATE organizer_jobs SET status='needs_review', preview_json=NULL,
             error_code='config_changed', message='整理配置已变更，请重新识别'
             WHERE mapping_id=?1 AND status IN ('recognizing','ready')",
            params![mapping.id],
        )
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
pub async fn remove_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: Option<OrganizerJobDeleteInput>,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if matches!(job.status.as_str(), "recognizing" | "running") {
        return Err("任务正在整理，不能删除".to_string());
    }
    let input = input.unwrap_or_default();
    let mut deleted_source = 0usize;
    let mut deleted_target = 0usize;
    let mut warnings = Vec::new();
    if input.delete_target {
        let target_ids = job
            .result
            .as_ref()
            .map(|result| {
                result
                    .targets
                    .iter()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        for target_id in &target_ids {
            cloud_delete(&app, target_id)
                .await
                .map_err(|error| format!("删除媒体库文件失败：{error}"))?;
            deleted_target += 1;
        }
        if target_ids.is_empty() {
            warnings.push("该记录没有可安全定位的媒体库文件，未删除媒体库内容".to_string());
        }
    }
    if input.delete_source {
        let source = list_cloud_children(&app, &job.source_parent_id)
            .await?
            .into_iter()
            .find(|entry| entry.id == job.source_id);
        if let Some(source) = source {
            cloud_delete(&app, &source.id)
                .await
                .map_err(|error| format!("删除源文件失败：{error}"))?;
            deleted_source = 1;
        } else {
            warnings.push("源文件已移动或不存在，未重复删除".to_string());
        }
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
        json!({ "job_id": id, "mapping_id": job.mapping_id, "deleted_source": deleted_source, "deleted_target": deleted_target }),
    );
    Ok(
        json!({ "deleted_source": deleted_source, "deleted_target": deleted_target, "warnings": warnings }),
    )
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
pub async fn share_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if !matches!(job.status.as_str(), "completed" | "completed_warning") {
        return Err("只有已完成的整理任务才能创建分享".to_string());
    }
    let mapping =
        get_mapping(&path, &job.mapping_id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let preview = job
        .preview
        .clone()
        .ok_or_else(|| "整理预览缺少最终媒体目录，无法创建分享".to_string())?;
    if preview.share_relative_path.trim().is_empty() {
        return Err("整理预览缺少最终媒体目录，无法创建分享".to_string());
    }
    let mut resolver = TargetResolver::new();
    let target = resolve_target(&app, &mapping, &preview.share_relative_path, &mut resolver)
        .await?
        .filter(|entry| entry.is_directory)
        .ok_or_else(|| "最终媒体目录已经不存在，无法创建分享".to_string())?;
    let share = create_fresh_organizer_share(
        app.clone(),
        &mapping.id,
        &target.id,
        if preview.share_title.trim().is_empty() {
            "整理后的媒体"
        } else {
            &preview.share_title
        },
    )
    .await?;
    let mut result = job.result.unwrap_or_default();
    result.success = true;
    result.share = Some(share.clone());
    update_job_fields(
        &path,
        &id,
        &[(
            "result_json",
            serde_json::to_value(result).map_err(|error| format!("保存整理分享失败：{error}"))?,
        )],
    )?;
    emit(
        &app,
        "job-updated",
        json!({ "job_id": id, "mapping_id": mapping.id, "status": job.status, "shared": true }),
    );
    Ok(share)
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
        return recognize_job(&app, state.inner(), &id, input, true).await;
    }
    // 源内容或整理配置在预览后发生变化时自动重新识别再执行（对齐 Node 端），
    // 而不是让 execute_job 抛"请先重新识别"逼用户手动再点一次。
    let mapping =
        get_mapping(&path, &job.mapping_id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let secrets = load_secrets(&path)?;
    let preview_stale = job.preview.as_ref().is_some_and(|preview| {
        preview.mapping_signature != mapping_signature(&mapping, &secrets)
    });
    let source_changed = if preview_stale {
        false
    } else {
        let loaded = load_candidate(&app, &mapping, &job.source_id)
            .await?
            .ok_or_else(|| "待整理云端项目已经不存在".to_string())?;
        job.preview.as_ref().is_some_and(|preview| {
            loaded.fingerprint.signature != preview.source_signature
        })
    };
    if preview_stale || source_changed {
        recognize_job(&app, state.inner(), &id, input, true).await
    } else {
        execute_job(&app, state.inner(), &id).await
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RecognitionTestInput {
    #[serde(default)]
    names: Vec<String>,
    #[serde(default)]
    recognition_words: String,
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    with_match: bool,
}

/// 识别测试工具：用当前设置（可叠加临时识别词）解析文件名，
/// 可选走完整 TMDB 匹配链做识别预览。不落库、不影响任务。
#[tauri::command]
pub async fn test_media_recognition(
    state: tauri::State<'_, OrganizerSharedState>,
    input: RecognitionTestInput,
) -> Result<Value, String> {
    let path = database_path(state.inner())?;
    let mut secrets = load_secrets(&path)?;
    if !input.recognition_words.trim().is_empty() {
        validate_auxiliary_rule_block(&input.recognition_words, "临时识别词", true)?;
        secrets.native.recognition_words = if secrets.native.recognition_words.trim().is_empty() {
            input.recognition_words.clone()
        } else {
            format!(
                "{}\n{}",
                input.recognition_words, secrets.native.recognition_words
            )
        };
    }
    let hint = input.media_type.trim().to_lowercase();
    let hint = matches!(hint.as_str(), "movie" | "tv").then_some(hint);
    let mut items = Vec::new();
    for name in input
        .names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .take(50)
    {
        let overrides = RecognitionOverrides {
            media_type: hint.clone(),
            ..Default::default()
        };
        let parsed = crate::organizer_core::parse_media_name_with_settings(
            name,
            &overrides,
            &secrets.native,
        );
        let mut row = json!({ "name": name, "parsed": parsed });
        if input.with_match {
            if secrets.api_key.trim().is_empty() {
                row["match"] = json!({ "ready": false, "message": "请先配置 TMDB API Key" });
            } else {
                let analysis = CandidateAnalysis {
                    candidate_path: name.to_string(),
                    candidate_type: "file".to_string(),
                    media_type: parsed.media_type.clone(),
                    title: parsed.title.clone(),
                    title_candidates: [parsed.cn_name.clone(), parsed.en_name.clone()]
                        .into_iter()
                        .filter(|value| !value.is_empty())
                        .collect(),
                    year: parsed.year,
                    tmdb_id: parsed.tmdb_id,
                    videos: Vec::new(),
                    sidecars: Vec::new(),
                    ignored_samples: Vec::new(),
                    query: MediaQuery {
                        title: parsed.title.clone(),
                        year: parsed.year,
                        media_type: parsed.media_type.clone(),
                        tmdb_id: parsed.tmdb_id,
                    },
                };
                let match_overrides = RecognitionOverrides {
                    media_type: hint.clone(),
                    ..Default::default()
                };
                match resolve_tmdb_match(
                    &analysis,
                    &secrets.client()?,
                    &secrets.native,
                    &match_overrides,
                )
                .await
                {
                    Ok(resolution) => {
                        row["match"] = json!({
                            "ready": resolution.ready,
                            "message": resolution.message,
                            "title": resolution.metadata.as_ref().map(|value| value.title.clone()),
                            "original_title": resolution.metadata.as_ref().map(|value| value.original_title.clone()),
                            "year": resolution.metadata.as_ref().and_then(|value| value.year),
                            "tmdb_id": resolution.selected.as_ref().map(|value| value.tmdb_id),
                            "media_type": resolution.query.media_type,
                            "candidates": resolution.candidates.iter().take(5).map(|candidate| json!({
                                "tmdb_id": candidate.tmdb_id,
                                "title": candidate.title,
                                "year": candidate.year,
                                "media_type": candidate.media_type,
                            })).collect::<Vec<_>>(),
                        });
                    }
                    Err(error) => {
                        row["match"] = json!({ "ready": false, "message": error });
                    }
                }
            }
        }
        items.push(row);
    }
    Ok(json!({ "items": items }))
}

#[tauri::command]
pub async fn retry_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: OrganizerJobInput,
) -> Result<OrganizerJob, String> {
    // 整理进行中禁止重新识别：识别会清空 preview_json 并改写状态，
    // 与正在执行的转移流程互相覆盖会导致回滚信息错乱。
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if job.status == "running" {
        return Err("该任务正在整理，请等待完成".to_string());
    }
    {
        let runtime = state.inner().lock().map_err(|error| error.to_string())?;
        if runtime.running_jobs.contains(&id) {
            return Err("该任务正在整理，请等待完成".to_string());
        }
    }
    recognize_job(&app, state.inner(), &id, input, false).await
}

/// 目录（含一层子目录）内是否还有视频文件：决定共享刮削元数据（tvshow.nfo、
/// 海报等）是否可以随重新归档一并清理。
async fn directory_still_has_video(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    relative: &str,
    resolver: &mut TargetResolver,
) -> bool {
    let Ok(Some(entry)) = resolve_target(app, mapping, relative, resolver).await else {
        return false;
    };
    if !entry.is_directory {
        return false;
    }
    let Ok(children) = resolver.list(app, &entry.id, true).await else {
        return true;
    };
    if children
        .iter()
        .any(|child| !child.is_directory && video_extension(&child.name))
    {
        return true;
    }
    for child in children.iter().filter(|child| child.is_directory) {
        if let Ok(grand) = resolver.list(app, &child.id, false).await {
            if grand
                .iter()
                .any(|item| !item.is_directory && video_extension(&item.name))
            {
                return true;
            }
        }
    }
    false
}

/// 重新归档前清理上一次执行创建的产物：
/// 1. 删除转移落位的视频/字幕等文件（按 ID，容忍已不存在）；
/// 2. 与被删视频同名前缀的单集元数据一并删除；共享元数据（tvshow.nfo、海报）
///    只在所在目录已无其他视频时删除——单集纠错不会破坏剧集的其余内容；
/// 3. 自底向上删除因此变空的目录，到整理目标根为止。
async fn cleanup_previous_outputs(
    app: &tauri::AppHandle,
    mapping: &OrganizerMapping,
    job: &OrganizerJob,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(result) = &job.result else {
        return warnings;
    };
    // 旧版本任务只记录了转移文件 ID：按 ID 清理，跳过目录收敛。
    let items: Vec<CreatedOutputItem> = if result.created_items.is_empty() {
        result
            .targets
            .iter()
            .filter(|id| !id.is_empty())
            .map(|id| CreatedOutputItem {
                id: id.clone(),
                kind: "video".to_string(),
                ..Default::default()
            })
            .collect()
    } else {
        result.created_items.clone()
    };
    if items.is_empty() {
        return warnings;
    }
    let media_kinds = ["video", "subtitle", "audio", "trailer", "extra"];
    let mut resolver = TargetResolver::new();
    let mut deleted_stems: HashSet<String> = HashSet::new();
    let mut affected_dirs: HashSet<String> = HashSet::new();
    for item in items
        .iter()
        .filter(|item| media_kinds.contains(&item.kind.as_str()) || item.kind.is_empty())
    {
        if item.id.is_empty() {
            continue;
        }
        match cloud_delete(app, &item.id).await {
            Ok(()) => {
                if !item.target_relative.is_empty() {
                    deleted_stems.insert(file_stem_lower(&path_name(&item.target_relative)));
                    affected_dirs.insert(path_parent(&item.target_relative));
                }
            }
            Err(error) => {
                // 已被手动删除/洗版替换的文件直接跳过
                if !error.contains("不存在") {
                    warnings.push(format!("清理旧文件 {} 失败：{error}", item.name));
                }
            }
        }
    }
    for item in items
        .iter()
        .filter(|item| item.kind == "nfo" || item.kind == "image")
    {
        if item.target_relative.is_empty() {
            continue;
        }
        let stem = file_stem_lower(&path_name(&item.target_relative));
        let parent_relative = path_parent(&item.target_relative);
        let tied_to_deleted_video = deleted_stems
            .iter()
            .any(|deleted| stem.starts_with(deleted.as_str()));
        let removable = if tied_to_deleted_video {
            true
        } else {
            !directory_still_has_video(app, mapping, &parent_relative, &mut resolver).await
        };
        if !removable {
            continue;
        }
        if let Ok(Some(entry)) =
            resolve_target(app, mapping, &item.target_relative, &mut resolver).await
        {
            if !entry.is_directory {
                if let Err(error) = cloud_delete(app, &entry.id).await {
                    if !error.contains("不存在") {
                        warnings.push(format!("清理旧元数据 {} 失败：{error}", item.name));
                    }
                }
                resolver.invalidate(&entry.parent_id);
                affected_dirs.insert(parent_relative);
            }
        }
    }
    // 空目录自底向上收敛：只删除确实变空的目录，有其他内容（其他版本、
    // 其他集、非本任务文件）的目录原样保留。
    let mut chains: HashSet<String> = HashSet::new();
    for dir in &affected_dirs {
        let mut current = dir.clone();
        while !current.is_empty() {
            chains.insert(current.clone());
            current = path_parent(&current);
        }
    }
    let mut ordered: Vec<String> = chains.into_iter().collect();
    ordered.sort_by_key(|value| std::cmp::Reverse(value.split('/').count()));
    for dir in ordered {
        let mut fresh = TargetResolver::new();
        let Ok(Some(entry)) = resolve_target(app, mapping, &dir, &mut fresh).await else {
            continue;
        };
        if !entry.is_directory {
            continue;
        }
        let Ok(children) = fresh.list(app, &entry.id, true).await else {
            continue;
        };
        if children.is_empty() {
            if let Err(error) = cloud_delete(app, &entry.id).await {
                warnings.push(format!("清理空目录 {dir} 失败：{error}"));
            }
        }
    }
    warnings
}

#[tauri::command]
pub async fn rearchive_organizer_job(
    app: tauri::AppHandle,
    state: tauri::State<'_, OrganizerSharedState>,
    id: String,
    input: OrganizerJobInput,
) -> Result<OrganizerJob, String> {
    let path = database_path(state.inner())?;
    let job = get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())?;
    if matches!(job.status.as_str(), "recognizing" | "running") {
        return Err("该任务正在整理，请等待完成".to_string());
    }
    let mapping =
        get_mapping(&path, &job.mapping_id)?.ok_or_else(|| "整理监控不存在".to_string())?;
    let connection = open_database(&path)?;
    let active = connection
        .query_row(
            "SELECT id FROM organizer_jobs WHERE mapping_id=?1 AND source_id=?2 AND id<>?3
             AND status IN ('recognizing','ready','running','needs_review') ORDER BY updated_at DESC LIMIT 1",
            params![job.mapping_id, job.source_id, id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("检查重复归档任务失败：{error}"))?;
    if active.is_some() {
        return Err("该源文件已有待处理的归档任务，请先处理后再重新归档".to_string());
    }
    drop(connection);
    update_job_fields(
        &path,
        &id,
        &[
            ("status", json!("recognizing")),
            ("error_code", Value::Null),
            ("message", json!("重新归档已提交，正在后台识别")),
        ],
    )?;
    let task_mapping_id = mapping.id.clone();
    emit(
        &app,
        "job-updated",
        json!({
            "job_id": id.clone(),
            "mapping_id": task_mapping_id.clone(),
            "status": "recognizing",
            "rearchive": true,
        }),
    );
    let task_app = app.clone();
    let task_state = state.inner().clone();
    let task_id = id.clone();
    let task_path = path.clone();
    let task_mapping = mapping.clone();
    let task_job = job.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            // 上次已经落位过的任务：先清理旧产物（含变空的目录），再重新识别，
            // 避免错误版本的文件夹和元数据残留在媒体库。
            if task_job
                .result
                .as_ref()
                .is_some_and(|result| !result.targets.is_empty() || !result.created_items.is_empty())
            {
                let cleanup_warnings =
                    cleanup_previous_outputs(&task_app, &task_mapping, &task_job).await;
                if !cleanup_warnings.is_empty() {
                    let _ = update_job_fields(
                        &task_path,
                        &task_id,
                        &[("message", json!(format!(
                            "旧产物清理有 {} 项提示：{}；继续重新识别",
                            cleanup_warnings.len(),
                            cleanup_warnings.join("；")
                        )))],
                    );
                }
            }
            let recognized = recognize_job(&task_app, &task_state, &task_id, input, false).await?;
            if recognized.status == "ready" {
                let _ = execute_job(&task_app, &task_state, &task_id).await?;
            }
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = result {
            let _ = update_job_fields(
                &task_path,
                &task_id,
                &[
                    ("status", json!("failed")),
                    ("error_code", json!("rearchive_failed")),
                    ("message", json!(error.clone())),
                ],
            );
            emit(
                &task_app,
                "job-updated",
                json!({
                    "job_id": task_id,
                    "mapping_id": task_mapping_id,
                    "status": "failed",
                    "message": error,
                }),
            );
        }
    });
    get_job(&path, &id)?.ok_or_else(|| "整理任务不存在".to_string())
}
