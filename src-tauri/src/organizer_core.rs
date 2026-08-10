use regex::{Captures, Regex, RegexBuilder};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::fs as async_fs;
use uuid::Uuid;

pub const NATIVE_ENGINE_VERSION: &str = "guangya-cloud-native-v3";
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "3gp", "asf", "avi", "f4v", "flv", "iso", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg",
    "mts", "rm", "rmvb", "strm", "tp", "ts", "vob", "webm", "wmv",
];
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "idx", "smi", "srt", "ssa", "sub", "sup", "vtt"];
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "ac3", "dts", "eac3", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeSettings {
    pub language: String,
    pub image_language: String,
    pub include_adult: bool,
    pub minimum_match_score: f64,
    #[serde(default = "default_true")]
    pub word_segment_search: bool,
    #[serde(default = "default_true")]
    pub similarity_match: bool,
    #[serde(default)]
    pub recognition_words: String,
    #[serde(default)]
    pub release_groups: String,
    #[serde(default)]
    pub render_words: String,
    #[serde(default)]
    pub capture_groups: String,
    pub movie_folder_format: String,
    pub movie_file_format: String,
    pub tv_folder_format: String,
    pub season_folder_format: String,
    pub episode_file_format: String,
}

fn default_true() -> bool {
    true
}

impl Default for NativeSettings {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            image_language: "zh,null,en".to_string(),
            include_adult: false,
            minimum_match_score: 0.72,
            word_segment_search: true,
            similarity_match: true,
            recognition_words: String::new(),
            release_groups: String::new(),
            render_words: String::new(),
            capture_groups: String::new(),
            movie_folder_format: "{title} ({year})".to_string(),
            movie_file_format: "{title} ({year}){edition}{quality}{part}".to_string(),
            tv_folder_format: "{title} ({year})".to_string(),
            season_folder_format: "Season {season:02}".to_string(),
            episode_file_format:
                "{title} - S{season:02}E{episode:02}{episode_end} - {episode_title}".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecognitionOverrides {
    pub media_type: Option<String>,
    pub tmdb_id: Option<i64>,
    pub title: Option<String>,
    pub year: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_end: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedMediaName {
    pub original: String,
    pub title: String,
    pub year: Option<i64>,
    pub media_type: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_end: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub edition: String,
    pub quality: String,
    pub part: Option<String>,
    pub video_format: String,
    pub resource_type: String,
    pub source: String,
    pub effect: String,
    pub audio_info: String,
    pub video_codec: String,
    pub audio_codec: String,
    pub release_group: String,
    pub release_type: String,
    pub high_quality: String,
    pub dolby_vision: String,
    pub dynamic_range: String,
    pub frame_rate: String,
    pub color_depth: String,
    #[serde(default)]
    pub media_probed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedVideo {
    pub source: String,
    pub parsed: ParsedMediaName,
    pub extra_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedSidecar {
    pub source: String,
    pub kind: String,
    pub video_source: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaQuery {
    pub title: String,
    pub year: Option<i64>,
    pub media_type: String,
    #[serde(default)]
    pub tmdb_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateAnalysis {
    pub candidate_path: String,
    pub candidate_type: String,
    pub media_type: String,
    pub title: String,
    pub year: Option<i64>,
    #[serde(default)]
    pub tmdb_id: Option<i64>,
    pub videos: Vec<AnalyzedVideo>,
    pub sidecars: Vec<AnalyzedSidecar>,
    pub ignored_samples: Vec<String>,
    pub query: MediaQuery,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TmdbCandidate {
    pub tmdb_id: i64,
    pub media_type: String,
    pub title: String,
    pub original_title: String,
    pub year: Option<i64>,
    pub release_date: String,
    pub overview: String,
    pub vote_average: f64,
    pub popularity: f64,
    pub poster_path: String,
    pub poster_url: String,
    pub score: f64,
    pub forced: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaActor {
    pub name: String,
    pub role: String,
    pub order: i64,
    pub thumb: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EpisodeMetadata {
    pub episode_number: i64,
    pub season_number: i64,
    pub name: String,
    pub overview: String,
    pub air_date: String,
    pub runtime: i64,
    pub vote_average: f64,
    pub still_path: String,
    pub still_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeasonMetadata {
    pub season_number: i64,
    pub name: String,
    pub overview: String,
    pub air_date: String,
    pub poster_path: String,
    pub poster_url: String,
    pub episodes: Vec<EpisodeMetadata>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaMetadata {
    pub tmdb_id: i64,
    pub imdb_id: String,
    pub media_type: String,
    pub title: String,
    pub original_title: String,
    pub year: Option<i64>,
    pub release_date: String,
    pub overview: String,
    pub tagline: String,
    pub status: String,
    pub runtime: i64,
    pub vote_average: f64,
    pub vote_count: i64,
    pub genres: Vec<String>,
    #[serde(default)]
    pub genre_ids: Vec<i64>,
    pub studios: Vec<String>,
    pub countries: Vec<String>,
    #[serde(default)]
    pub original_language: String,
    #[serde(default)]
    pub origin_countries: Vec<String>,
    pub directors: Vec<String>,
    pub actors: Vec<MediaActor>,
    pub poster_path: String,
    pub backdrop_path: String,
    pub poster_url: String,
    pub backdrop_url: String,
    pub seasons: HashMap<String, SeasonMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResolution {
    pub ready: bool,
    pub error_code: Option<String>,
    pub message: String,
    pub query: MediaQuery,
    pub candidates: Vec<TmdbCandidate>,
    pub selected: Option<TmdbCandidate>,
    pub metadata: Option<MediaMetadata>,
}

#[derive(Debug, Clone)]
pub struct PreviewMapping {
    pub target_path: String,
    pub transfer_type: String,
    pub conflict_policy: String,
    pub scrape: bool,
    pub sync_extras: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeneratorSpec {
    #[serde(rename = "type")]
    pub generator_type: String,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreviewItem {
    pub success: bool,
    pub kind: String,
    pub source: Option<String>,
    pub target: String,
    pub operation: String,
    pub action: String,
    pub exists: bool,
    pub renamed_for_conflict: bool,
    pub message: String,
    pub error_code: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub episode_end: Option<i64>,
    pub generator: Option<GeneratorSpec>,
    pub image_role: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreviewSummary {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub warnings: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreviewData {
    pub summary: PreviewSummary,
    pub items: Vec<PreviewItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NativePreview {
    pub success: bool,
    pub engine: String,
    pub mapping_signature: String,
    pub source_signature: String,
    pub query: MediaQuery,
    pub candidates: Vec<TmdbCandidate>,
    pub selected: Option<TmdbCandidate>,
    pub metadata: Option<MediaMetadata>,
    pub target_root: String,
    pub media_root: String,
    pub error_code: Option<String>,
    pub message: String,
    pub ignored_samples: Vec<String>,
    pub data: PreviewData,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub transferred: usize,
    pub skipped: usize,
    pub scraped: usize,
    pub warnings: Vec<String>,
    pub targets: Vec<String>,
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid organizer regex")
}

fn file_name_text(value: &str) -> String {
    value
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .next_back()
        .unwrap_or_default()
        .to_string()
}

fn stem_text(value: &str) -> String {
    let name = file_name_text(value);
    Path::new(&name)
        .file_stem()
        .and_then(|part| part.to_str())
        .unwrap_or(&name)
        .to_string()
}

fn normalize_spaces(value: &str) -> String {
    regex(r"[._]+")
        .replace_all(value, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|character: char| character.is_whitespace() || "-–—".contains(character))
        .trim()
        .to_string()
}

fn strip_technical_brackets(value: &str) -> String {
    let first = regex(r"^\s*\[[^\]]{1,80}\]\s*").replace_all(value, " ");
    regex(r"(?i)[\[\{\(](?:2160p|1080p|720p|480p|4k|uhd|hdr10?\+?|dv|dolby[ ._-]*vision|x26[45]|h26[45]|hevc|av1|web[ ._-]*dl|webrip|bluray|remux|aac|dts|truehd|flac|中字|中英字幕|简繁)[^\]\}\)]*[\]\}\)]")
        .replace_all(&first, " ")
        .to_string()
}

fn release_quality(value: &str) -> String {
    let patterns = [
        r"(?i)\b(2160p|1080p|720p|480p|4k|uhd)\b",
        r"(?i)\b(BluRay|Blu-Ray|REMUX|WEB[ ._-]?DL|WEBRip|HDTV|BDRip|BRRip|DVDRip)\b",
        r"(?i)\b(x265|x264|H\.?(?:265|264)|HEVC|AV1)\b",
    ];
    let mut values = Vec::new();
    for pattern in patterns {
        if let Some(capture) = regex(pattern).captures(value).and_then(|item| item.get(1)) {
            let normalized = regex(r"[ ._-]+")
                .replace_all(capture.as_str(), "-")
                .to_string();
            if !values
                .iter()
                .any(|current: &String| current.eq_ignore_ascii_case(&normalized))
            {
                values.push(normalized);
            }
        }
    }
    values.join(" ")
}

fn release_edition(value: &str) -> String {
    let patterns = [
        ("Director’s Cut", r"(?i)director(?:'|’)?s[ ._-]*cut"),
        (
            "Extended Cut",
            r"(?i)extended(?:[ ._-]*(?:cut|edition|version))?",
        ),
        ("IMAX", r"(?i)\bimax\b"),
        ("Unrated", r"(?i)\bunrated\b|\bunrate\b"),
        ("Uncut", r"(?i)\buncut\b"),
        ("Remastered", r"(?i)\bremaster(?:ed)?\b"),
        ("Theatrical Cut", r"(?i)theatrical(?:[ ._-]*cut)?"),
        ("Special Edition", r"(?i)special[ ._-]*edition"),
    ];
    patterns
        .iter()
        .find(|(_, pattern)| regex(pattern).is_match(value))
        .map(|(name, _)| (*name).to_string())
        .unwrap_or_default()
}

fn release_part(value: &str) -> Option<String> {
    let captures = regex(r"(?i)(?:^|[ ._\-])(CD|DISC|DISK|PART)[ ._\-]?(\d{1,2})(?:$|[ ._\-])")
        .captures(value)?;
    let kind = if captures.get(1)?.as_str().eq_ignore_ascii_case("part") {
        "Part"
    } else {
        "CD"
    };
    Some(format!(
        "{kind}{}",
        captures.get(2)?.as_str().parse::<i64>().ok()?
    ))
}

#[derive(Default)]
struct TvNumbers {
    season: Option<i64>,
    episode: Option<i64>,
    episode_end: Option<i64>,
    marker_index: Option<usize>,
}

fn tv_numbers(value: &str, media_type_hint: &str) -> TvNumbers {
    let patterns = [
        r"(?i)(?:^|[^A-Za-z0-9])S(\d{1,3})[ ._\-]*E(\d{1,4})(?:[ ._\-]*(?:E|\-E?)(\d{1,4}))?(?:v\d+)?(?:$|[^A-Za-z0-9])",
        r"(?i)(?:^|[^0-9])(\d{1,3})x(\d{1,4})(?:[ ._\-]*(?:x|\-)(\d{1,4}))?(?:$|[^0-9])",
        r"第\s*(\d{1,3})\s*季[^0-9]{0,12}第?\s*(\d{1,4})(?:\s*[\-~至]\s*(\d{1,4}))?\s*[集話话]",
        r"(?i)Season[ ._\-]*(\d{1,3})[^0-9]{0,12}(?:Episode|EP?)[ ._\-]*(\d{1,4})(?:[ ._\-]*(?:\-|EP?)(\d{1,4}))?",
    ];
    for pattern in patterns {
        let matcher = regex(pattern);
        if let Some(captures) = matcher.captures(value) {
            let matched = captures.get(0);
            return TvNumbers {
                season: captures.get(1).and_then(|item| item.as_str().parse().ok()),
                episode: captures.get(2).and_then(|item| item.as_str().parse().ok()),
                episode_end: captures.get(3).and_then(|item| item.as_str().parse().ok()),
                marker_index: matched.map(|item| item.start()),
            };
        }
    }
    let season_regex = regex(
        r"(?i)(?:^|[^A-Za-z0-9])S(?:eason)?[ ._\-]?(\d{1,3})(?:$|[^A-Za-z0-9])|第\s*(\d{1,3})\s*季",
    );
    let season_capture = season_regex.captures(value);
    let season = season_capture.as_ref().and_then(|captures| {
        captures
            .get(1)
            .or_else(|| captures.get(2))
            .and_then(|item| item.as_str().parse().ok())
    });
    let season_index = season_capture
        .as_ref()
        .and_then(|captures| captures.get(0))
        .map(|item| item.start());
    let episode_regex = regex(
        r"(?i)(?:^|[^A-Za-z0-9])EP?[ ._\-]?(\d{1,4})(?:[ ._\-]*(?:\-|EP?)(\d{1,4}))?(?:v\d+)?(?:$|[^A-Za-z0-9])|第?\s*(\d{1,4})(?:\s*[\-~至]\s*(\d{1,4}))?\s*[集話话]",
    );
    let episode_capture = episode_regex.captures(value);
    if let Some(captures) = episode_capture {
        if media_type_hint == "tv" || season.is_some() {
            let episode = captures
                .get(1)
                .or_else(|| captures.get(3))
                .and_then(|item| item.as_str().parse().ok());
            let episode_end = captures
                .get(2)
                .or_else(|| captures.get(4))
                .and_then(|item| item.as_str().parse().ok());
            let episode_index = captures.get(0).map(|item| item.start());
            return TvNumbers {
                season,
                episode,
                episode_end,
                marker_index: match (season_index, episode_index) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (left, right) => left.or(right),
                },
            };
        }
    }
    if media_type_hint == "tv" {
        if let Some(captures) =
            regex(r"(?i)(?:^|\s)[\-–—]\s*(\d{1,4})(?:v\d+)?(?:\s|$)").captures(value)
        {
            return TvNumbers {
                season: season.or(Some(1)),
                episode: captures.get(1).and_then(|item| item.as_str().parse().ok()),
                episode_end: None,
                marker_index: captures.get(0).map(|item| item.start()),
            };
        }
    }
    TvNumbers {
        season,
        episode: None,
        episode_end: None,
        marker_index: season_index,
    }
}

fn clean_title(value: &str) -> String {
    let stripped = strip_technical_brackets(value);
    let release_words: HashSet<&str> = [
        "bluray",
        "blu-ray",
        "bdrip",
        "brrip",
        "web",
        "webdl",
        "web-dl",
        "webrip",
        "hdtv",
        "dvdrip",
        "hdrip",
        "remux",
        "x264",
        "x265",
        "h264",
        "h265",
        "hevc",
        "avc",
        "av1",
        "10bit",
        "8bit",
        "hdr",
        "hdr10",
        "hdr10plus",
        "dv",
        "dolbyvision",
        "aac",
        "ac3",
        "eac3",
        "ddp",
        "dts",
        "dtshd",
        "truehd",
        "atmos",
        "flac",
        "mp3",
        "proper",
        "repack",
        "rerip",
        "complete",
        "internal",
        "subbed",
        "dubbed",
        "multi",
        "dual",
        "国语",
        "国英双语",
        "中英字幕",
        "中字",
        "简繁",
    ]
    .into_iter()
    .collect();
    let mut retained = Vec::new();
    let normalized_title = normalize_spaces(&stripped);
    for token in normalized_title.split_whitespace() {
        let normalized = token
            .to_lowercase()
            .chars()
            .filter(|character| character.is_alphanumeric() || *character == '-')
            .collect::<String>();
        if normalized.is_empty()
            || release_words.contains(normalized.as_str())
            || regex(r"(?i)^(?:2160|1080|720|480)p$").is_match(&normalized)
            || regex(r"(?i)^(?:x|h)?26[45]$").is_match(&normalized)
            || regex(r"(?i)^(?:aac|ddp|dts|flac)\d*(?:\.\d+)?$").is_match(&normalized)
        {
            continue;
        }
        retained.push(token);
    }
    normalize_spaces(&retained.join(" "))
}

#[derive(Default)]
struct RecognitionDirectives {
    tmdb_id: Option<i64>,
    media_type: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
}

fn rule_lines(value: &str) -> Vec<&str> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(2_000)
        .collect()
}

fn user_pattern_source(value: &str) -> (&str, bool) {
    let mut source = value.trim();
    let insensitive = source.starts_with("(?i)");
    if insensitive {
        source = &source[4..];
    }
    (source, insensitive)
}

fn compile_user_pattern(value: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let (source, insensitive) = user_pattern_source(value);
    let cache_key = format!("{}\0{source}", if insensitive { "i" } else { "" });
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some(pattern) = cache.get(&cache_key) {
            return pattern.clone();
        }
    }
    let pattern = RegexBuilder::new(source)
        .case_insensitive(insensitive)
        .unicode(true)
        .build()
        .unwrap_or_else(|_| {
            RegexBuilder::new(&regex::escape(source))
                .case_insensitive(true)
                .unicode(true)
                .build()
                .expect("escaped organizer user regex")
        });
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= 4_096 {
            cache.clear();
        }
        cache.insert(cache_key, pattern.clone());
    }
    pattern
}

pub fn validate_auxiliary_rule_block(
    value: &str,
    label: &str,
    replacement: bool,
) -> Result<(), String> {
    for (index, raw_line) in value.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let pattern_text = if replacement {
            line.split_once("=>")
                .map(|(pattern, _)| pattern.trim())
                .unwrap_or(line)
        } else {
            line
        };
        if pattern_text.is_empty() {
            return Err(format!("{label}第 {} 行缺少正则表达式", index + 1));
        }
        if pattern_text.starts_with("@?{") {
            return Err(format!(
                "{label}第 {} 行使用了尚未支持的 @? 条件规则",
                index + 1
            ));
        }
        let (source, insensitive) = user_pattern_source(pattern_text);
        if source.contains("(?P<") || source.contains("(?<") {
            return Err(format!(
                "{label}第 {} 行使用了统一规则语法不支持的命名捕获",
                index + 1
            ));
        }
        RegexBuilder::new(source)
            .case_insensitive(insensitive)
            .unicode(true)
            .build()
            .map_err(|error| format!("{label}第 {} 行正则无效：{error}", index + 1))?;
    }
    Ok(())
}

fn calculate_captured_number(value: &str, expression: &str) -> String {
    let Ok(mut result) = value.parse::<f64>() else {
        return value.to_string();
    };
    for captures in regex(r"([+\-*/])\s*(\d+(?:\.\d+)?)").captures_iter(expression) {
        let number = captures
            .get(2)
            .and_then(|item| item.as_str().parse::<f64>().ok())
            .unwrap_or(0.0);
        match captures.get(1).map(|item| item.as_str()) {
            Some("+") => result += number,
            Some("-") => result -= number,
            Some("*") => result *= number,
            Some("/") if number != 0.0 => result /= number,
            _ => {}
        }
    }
    if result.fract().abs() < f64::EPSILON {
        format!("{}", result as i64)
    } else {
        format!("{result:.4}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn expand_rule_replacement(template: &str, captures: &Captures<'_>) -> String {
    regex(r"\\(\d+)(?:@([+\-*/\d.\s]+))?")
        .replace_all(template, |reference: &Captures<'_>| {
            let index = reference
                .get(1)
                .and_then(|item| item.as_str().parse::<usize>().ok())
                .unwrap_or(0);
            let value = captures
                .get(index)
                .map(|item| item.as_str())
                .unwrap_or_default();
            reference
                .get(2)
                .map(|expression| calculate_captured_number(value, expression.as_str()))
                .unwrap_or_else(|| value.to_string())
        })
        .to_string()
}

fn extract_recognition_directives(value: &str, directives: &mut RecognitionDirectives) -> String {
    let directive_regex = regex(r"\{\[([^\]]+)\]\}");
    for captures in directive_regex.captures_iter(value) {
        let body = captures
            .get(1)
            .map(|item| item.as_str())
            .unwrap_or_default();
        for entry in body.split(';') {
            let mut parts = entry.splitn(2, '=');
            let key = parts.next().unwrap_or_default().trim().to_lowercase();
            let raw = parts.next().unwrap_or_default().trim();
            match key.as_str() {
                "tmdbid" | "tmdb_id" => {
                    directives.tmdb_id = raw.parse::<i64>().ok().filter(|id| *id > 0)
                }
                "type" if matches!(raw.to_lowercase().as_str(), "movie" | "tv") => {
                    directives.media_type = Some(raw.to_lowercase())
                }
                "s" | "season" => {
                    directives.season = raw.parse::<i64>().ok().filter(|number| *number >= 0)
                }
                "e" | "episode" => {
                    directives.episode = raw.parse::<i64>().ok().filter(|number| *number >= 0)
                }
                _ => {}
            }
        }
    }
    directive_regex.replace_all(value, "").to_string()
}

fn apply_rule_block(value: &str, block: &str, directives: &mut RecognitionDirectives) -> String {
    let mut current = value.to_string();
    for line in rule_lines(block) {
        let (pattern_text, replacement) = line
            .split_once("=>")
            .map(|(left, right)| (left.trim(), right.trim()))
            .unwrap_or((line, ""));
        if pattern_text.is_empty() || pattern_text.starts_with("@?{") {
            continue;
        }
        let pattern = compile_user_pattern(pattern_text);
        current = pattern
            .replace_all(&current, |captures: &Captures<'_>| {
                let expanded = expand_rule_replacement(replacement, captures);
                extract_recognition_directives(&expanded, directives)
            })
            .to_string();
    }
    current
}

fn apply_auxiliary_recognition(
    value: &str,
    settings: &NativeSettings,
) -> (String, RecognitionDirectives) {
    let mut directives = RecognitionDirectives::default();
    let recognized = apply_rule_block(value, &settings.recognition_words, &mut directives);
    let rendered = apply_rule_block(&recognized, &settings.render_words, &mut directives);
    (
        regex(r"\s{2,}")
            .replace_all(rendered.trim(), " ")
            .to_string(),
        directives,
    )
}

fn technical_capture(value: &str, pattern: &str) -> String {
    regex(pattern)
        .captures(value)
        .and_then(|captures| captures.get(1).or_else(|| captures.get(0)))
        .map(|item| item.as_str().to_string())
        .unwrap_or_default()
}

fn known_release_group(value: &str, settings: &NativeSettings) -> String {
    let lower = value.to_lowercase();
    let mut groups = rule_lines(&settings.release_groups);
    groups.sort_by_key(|group| std::cmp::Reverse(group.len()));
    if let Some(group) = groups
        .into_iter()
        .find(|group| lower.contains(&group.to_lowercase()))
    {
        return group.to_string();
    }
    for rule in rule_lines(&settings.capture_groups) {
        if let Some(captures) = compile_user_pattern(rule).captures(value) {
            return captures
                .iter()
                .skip(1)
                .flatten()
                .next()
                .or_else(|| captures.get(0))
                .map(|item| item.as_str().trim().to_string())
                .unwrap_or_default();
        }
    }
    regex(r"-([A-Za-z0-9][A-Za-z0-9@._-]{1,48})$")
        .captures(value)
        .and_then(|captures| captures.get(1))
        .or_else(|| {
            regex(r"^\[([^\]]{2,48})\]")
                .captures(value)
                .and_then(|captures| captures.get(1))
        })
        .map(|item| item.as_str().trim().to_string())
        .unwrap_or_default()
}

pub fn parse_media_name_with_settings(
    value: &str,
    options: &RecognitionOverrides,
    settings: &NativeSettings,
) -> ParsedMediaName {
    let raw_original = stem_text(value);
    let (recognized, directives) = apply_auxiliary_recognition(&raw_original, settings);
    let original = if recognized.is_empty() {
        raw_original
    } else {
        recognized
    };
    let hint = options
        .media_type
        .clone()
        .unwrap_or_default()
        .to_lowercase();
    let technical = strip_technical_brackets(&original);
    let numbers = tv_numbers(&technical, &hint);
    let year_capture =
        regex(r"(?:^|[^0-9])(19\d{2}|20\d{2}|21\d{2})(?:$|[^0-9pP])").captures(&technical);
    let parsed_year = year_capture
        .as_ref()
        .and_then(|capture| capture.get(1))
        .and_then(|item| item.as_str().parse::<i64>().ok());
    let year_index = year_capture
        .as_ref()
        .and_then(|capture| capture.get(0))
        .map(|item| item.start());
    let cut_index = [numbers.marker_index, year_index]
        .into_iter()
        .flatten()
        .min();
    let mut title = clean_title(
        cut_index
            .map(|index| &technical[..index])
            .unwrap_or(&technical),
    );
    if title.is_empty()
        || regex(r"(?i)^(?:season|episode|ep|complete|disc|disk|part)\s*\d*$").is_match(&title)
    {
        title = options.title.clone().unwrap_or_default();
    }
    let season = options.season.or(directives.season).or(numbers.season);
    let episode = options.episode.or(directives.episode).or(numbers.episode);
    let episode_end = options.episode_end.or(numbers.episode_end);
    let media_type = if !hint.is_empty() {
        hint
    } else if let Some(media_type) = directives.media_type {
        media_type
    } else if episode.is_some() || season.is_some() {
        "tv".to_string()
    } else {
        "movie".to_string()
    };
    let video_format = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(2160p|1080p|720p|480p|4k|uhd)(?:$|[ ._\-])",
    )
    .to_lowercase()
    .replace("4k", "2160p")
    .replace("uhd", "2160p");
    let resource_type = technical_capture(&original, r"(?i)(?:^|[ ._\-])(REMUX|WEB[ ._\-]?DL|WEBRip|Blu[ ._\-]?Ray|BDRip|HDTV|DVDRip|UHDRip)(?:$|[ ._\-])")
        .replace([' ', '_'], "-");
    let raw_video_codec = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(AV1|HEVC|H[ .]?265|x265|AVC|H[ .]?264|x264)(?:$|[ ._\-])",
    );
    let video_codec = if regex(r"(?i)(?:265|hevc)").is_match(&raw_video_codec) {
        "HEVC"
    } else if regex(r"(?i)(?:264|avc)").is_match(&raw_video_codec) {
        "AVC"
    } else if raw_video_codec.is_empty() {
        ""
    } else {
        "AV1"
    }
    .to_string();
    let audio_codec = technical_capture(&original, r"(?i)(?:^|[ ._\-])(Atmos[ ._\-]*TrueHD|TrueHD|DTS[ ._\-]*HD(?:[ ._\-]*MA)?|DTS|DDP|EAC3|AC3|AAC|FLAC|LPCM|OPUS)(?:[ ._\-]?(?:7\.1|5\.1|2\.0|1\.0))?(?:$|[ ._\-])").replace([' ', '_'], "-");
    let audio_info = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(?:(?:Atmos[ ._\-]*TrueHD|TrueHD|DTS[ ._\-]*HD(?:[ ._\-]*MA)?|DTS|DDP|EAC3|AC3|AAC|FLAC|LPCM|OPUS)[ ._\-]*)?((?:Atmos[ ._\-]*)?(?:7\.1|5\.1|2\.0|1\.0))(?:$|[ ._\-])",
    )
    .replace('_', " ");
    let dolby_vision = regex(r"(?i)(?:^|[ ._\-])(?:DV|DoVi|Dolby[ ._\-]*Vision)(?:$|[ ._\-])")
        .is_match(&original)
        .then(|| "DV".to_string())
        .unwrap_or_default();
    let dynamic_range = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(HDR10\+|HDR10|HDR|HLG|SDR)(?:$|[ ._\-])",
    )
    .to_uppercase();
    let effect = [dolby_vision.clone(), dynamic_range.clone()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let source = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(AMZN|Amazon|NF|Netflix|ATVP|AppleTV|DSNP|Disney\+|HMAX|HBO|Hulu|Bilibili|CR|TVING|Viu)(?:$|[ ._\-])",
    );
    let frame_rate = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])((?:23\.976|24|25|29\.97|30|50|59\.94|60|120)(?:fps|p))(?:$|[ ._\-])",
    )
    .to_lowercase();
    let color_depth = technical_capture(
        &original,
        r"(?i)(?:^|[ ._\-])(8bit|10bit|12bit)(?:$|[ ._\-])",
    )
    .to_lowercase();
    ParsedMediaName {
        original: original.clone(),
        title,
        year: options.year.or(parsed_year),
        media_type,
        season,
        episode,
        episode_end,
        tmdb_id: directives.tmdb_id,
        edition: release_edition(&original),
        quality: release_quality(&original),
        part: release_part(&original),
        video_format,
        resource_type: resource_type.clone(),
        source,
        effect,
        audio_info,
        video_codec,
        audio_codec,
        release_group: known_release_group(&original, settings),
        release_type: resource_type.clone(),
        high_quality: regex(r"(?i)(?:^|[ ._\-])HQ(?:$|[ ._\-])")
            .is_match(&original)
            .then(|| "HQ".to_string())
            .unwrap_or_default(),
        dolby_vision,
        dynamic_range,
        frame_rate,
        color_depth,
        media_probed: false,
    }
}

pub fn parse_media_name(value: &str, options: &RecognitionOverrides) -> ParsedMediaName {
    parse_media_name_with_settings(value, options, &NativeSettings::default())
}

pub fn normalize_search_title(value: &str) -> String {
    value
        .to_lowercase()
        .replace('&', " and ")
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn ngrams(value: &str) -> HashSet<String> {
    let chars: Vec<char> = normalize_search_title(value).chars().collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    if chars.len() <= 2 {
        return [chars.iter().collect()].into_iter().collect();
    }
    chars
        .windows(2)
        .map(|window| window.iter().collect())
        .collect()
}

pub fn title_similarity(left: &str, right: &str) -> f64 {
    let a = normalize_search_title(left);
    let b = normalize_search_title(right);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.contains(&b) || b.contains(&a) {
        return (a.chars().count().min(b.chars().count()) as f64
            / a.chars().count().max(b.chars().count()) as f64)
            * 0.9;
    }
    let left_grams = ngrams(&a);
    let right_grams = ngrams(&b);
    let intersection = left_grams.intersection(&right_grams).count();
    (2 * intersection) as f64 / (left_grams.len() + right_grams.len()).max(1) as f64
}

pub fn score_tmdb_candidate(
    query: &MediaQuery,
    item: &Value,
    title: &str,
    original_title: &str,
) -> f64 {
    let title_score =
        title_similarity(&query.title, title).max(title_similarity(&query.title, original_title));
    let date = item
        .get("release_date")
        .or_else(|| item.get("first_air_date"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let candidate_year = date.get(..4).and_then(|year| year.parse::<i64>().ok());
    let year_score = match (query.year, candidate_year) {
        (Some(left), Some(right)) => match (left - right).abs() {
            0 => 1.0,
            1 => 0.72,
            2 => 0.35,
            _ => 0.0,
        },
        _ => 0.55,
    };
    let popularity = item
        .get("popularity")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let popularity_score = ((popularity + 1.0).max(1.0).log10() / 3.0).clamp(0.0, 1.0);
    (title_score * 0.79 + year_score * 0.16 + popularity_score * 0.05).clamp(0.0, 1.0)
}

fn useful_name(name: &str) -> bool {
    let normalized = name.trim().to_lowercase();
    !normalized.is_empty()
        && !normalized.starts_with('.')
        && !normalized.starts_with("~$")
        && ![
            "@eadir",
            "#recycle",
            "$recycle.bin",
            "system volume information",
        ]
        .contains(&normalized.as_str())
}

fn extension_in(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extensions
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn is_sample_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    regex(r"(?:^|/)(?:sample|samples)(?:/|$)").is_match(&normalized)
        || regex(r"(?i)(?:^|[ ._\-])sample(?:[ ._\-]|$)").is_match(&stem_text(&normalized))
}

fn extra_kind(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let stem = stem_text(&normalized);
    if regex(r"(?i)(?:^|[ ._\-])trailer(?:[ ._\-]|$)").is_match(&stem)
        || normalized.contains("/trailer/")
        || normalized.contains("/trailers/")
    {
        "trailer".to_string()
    } else if regex(r"/(?:extras?|featurettes?|behind the scenes|deleted scenes|interviews?)/")
        .is_match(&normalized)
    {
        "extra".to_string()
    } else {
        String::new()
    }
}

fn walk_candidate(
    candidate: &Path,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>), String> {
    let metadata =
        fs::symlink_metadata(candidate).map_err(|error| format!("读取待整理路径失败：{error}"))?;
    let mut videos = Vec::new();
    let mut subtitles = Vec::new();
    let mut audio = Vec::new();
    let mut samples = Vec::new();
    let consume = |file: PathBuf,
                   videos: &mut Vec<PathBuf>,
                   subtitles: &mut Vec<PathBuf>,
                   audio: &mut Vec<PathBuf>,
                   samples: &mut Vec<PathBuf>| {
        if extension_in(&file, VIDEO_EXTENSIONS) {
            if is_sample_path(&file) {
                samples.push(file);
            } else {
                videos.push(file);
            }
        } else if extension_in(&file, SUBTITLE_EXTENSIONS) {
            subtitles.push(file);
        } else if extension_in(&file, AUDIO_EXTENSIONS) {
            audio.push(file);
        }
    };
    if metadata.is_file() {
        consume(
            candidate.to_path_buf(),
            &mut videos,
            &mut subtitles,
            &mut audio,
            &mut samples,
        );
    } else if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let mut pending = vec![candidate.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in
                fs::read_dir(&directory).map_err(|error| format!("扫描媒体目录失败：{error}"))?
            {
                let entry = entry.map_err(|error| format!("读取媒体目录项失败：{error}"))?;
                let name = entry.file_name().to_string_lossy().to_string();
                if !useful_name(&name) {
                    continue;
                }
                let file_type = entry
                    .file_type()
                    .map_err(|error| format!("读取文件类型失败：{error}"))?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    consume(
                        entry.path(),
                        &mut videos,
                        &mut subtitles,
                        &mut audio,
                        &mut samples,
                    );
                }
            }
        }
    }
    videos.sort();
    subtitles.sort();
    audio.sort();
    Ok((videos, subtitles, audio, samples))
}

fn parent_season(file: &Path, candidate: &Path) -> Option<i64> {
    let relative = file
        .parent()?
        .strip_prefix(candidate)
        .ok()?
        .to_string_lossy();
    let captures = regex(r"(?i)(?:^|[\\/])(?:Season|S)[ ._\-]?(\d{1,3})(?:$|[\\/])|(?:^|[\\/])第\s*(\d{1,3})\s*季(?:$|[\\/])")
        .captures(&relative)?;
    captures
        .get(1)
        .or_else(|| captures.get(2))
        .and_then(|item| item.as_str().parse().ok())
}

fn most_useful_title(parsed: &[ParsedMediaName], fallback: &str) -> String {
    let mut counts: HashMap<String, (String, usize)> = HashMap::new();
    for item in parsed {
        if item.title.trim().is_empty() {
            continue;
        }
        let key = normalize_search_title(&item.title);
        let entry = counts.entry(key).or_insert((item.title.clone(), 0));
        entry.1 += 1;
        if item.title.len() > entry.0.len() {
            entry.0 = item.title.clone();
        }
    }
    counts
        .into_values()
        .max_by(|left, right| left.1.cmp(&right.1).then(left.0.len().cmp(&right.0.len())))
        .map(|item| item.0)
        .unwrap_or_else(|| fallback.to_string())
}

fn best_sidecar_video(sidecar: &Path, videos: &[AnalyzedVideo]) -> Option<String> {
    if videos.is_empty() {
        return None;
    }
    let sidecar_name = stem_text(&sidecar.to_string_lossy());
    let stripped = regex(
        r"(?i)(?:chs|cht|chi|eng|jpn|kor|zh[-_.]?(?:cn|tw)|简体|繁体|繁體|字幕|forced|sdh|default)",
    )
    .replace_all(&sidecar_name, "");
    let sidecar_stem = normalize_search_title(&stripped);
    if let Some(video) = videos
        .iter()
        .find(|video| sidecar_stem == normalize_search_title(&stem_text(&video.source)))
    {
        return Some(video.source.clone());
    }
    let parsed = parse_media_name(
        &sidecar.to_string_lossy(),
        &RecognitionOverrides {
            media_type: Some("tv".to_string()),
            ..Default::default()
        },
    );
    if let Some(episode) = parsed.episode {
        if let Some(video) = videos.iter().find(|video| {
            video.parsed.episode == Some(episode)
                && (parsed.season.is_none() || video.parsed.season == parsed.season)
        }) {
            return Some(video.source.clone());
        }
    }
    let same_directory: Vec<&AnalyzedVideo> = videos
        .iter()
        .filter(|video| Path::new(&video.source).parent() == sidecar.parent())
        .collect();
    if same_directory.len() == 1 {
        return Some(same_directory[0].source.clone());
    }
    (videos.len() == 1).then(|| videos[0].source.clone())
}

pub fn analyze_candidate(
    candidate: &Path,
    overrides: &RecognitionOverrides,
) -> Result<CandidateAnalysis, String> {
    let absolute = candidate
        .canonicalize()
        .map_err(|error| format!("待整理文件已经不存在：{error}"))?;
    let metadata =
        fs::symlink_metadata(&absolute).map_err(|error| format!("读取待整理路径失败：{error}"))?;
    let (video_paths, subtitles, audio, samples) = walk_candidate(&absolute)?;
    if video_paths.is_empty() {
        return Err("没有找到可整理的视频文件".to_string());
    }
    let candidate_name = if metadata.is_file() {
        stem_text(&absolute.to_string_lossy())
    } else {
        absolute
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let group = parse_media_name(&candidate_name, overrides);
    let mut preliminary = Vec::new();
    for source in &video_paths {
        let mut options = overrides.clone();
        options.media_type = overrides
            .media_type
            .clone()
            .or(Some(group.media_type.clone()));
        options.title = Some(group.title.clone());
        options.season = overrides
            .season
            .or_else(|| parent_season(source, &absolute));
        if video_paths.len() != 1 {
            options.episode = None;
            options.episode_end = None;
        }
        preliminary.push(parse_media_name(&source.to_string_lossy(), &options));
    }
    let inferred_tv = preliminary
        .iter()
        .filter(|item| item.episode.is_some() || item.season.is_some())
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
        .unwrap_or_else(|| most_useful_title(&preliminary, &group.title));
    let year = overrides
        .year
        .or(group.year)
        .or_else(|| preliminary.iter().find_map(|item| item.year));
    let mut videos = Vec::new();
    for source in video_paths {
        let mut options = overrides.clone();
        options.media_type = Some(media_type.clone());
        options.title = Some(title.clone());
        options.year = year;
        options.season = overrides
            .season
            .or_else(|| parent_season(&source, &absolute));
        if preliminary.len() != 1 {
            options.episode = None;
            options.episode_end = None;
        }
        videos.push(AnalyzedVideo {
            source: source.to_string_lossy().to_string(),
            parsed: parse_media_name(&source.to_string_lossy(), &options),
            extra_kind: extra_kind(&source),
        });
    }
    let mut sidecars = Vec::new();
    for (source, kind) in subtitles
        .into_iter()
        .map(|path| (path, "subtitle"))
        .chain(audio.into_iter().map(|path| (path, "audio")))
    {
        sidecars.push(AnalyzedSidecar {
            video_source: best_sidecar_video(&source, &videos),
            source: source.to_string_lossy().to_string(),
            kind: kind.to_string(),
        });
    }
    Ok(CandidateAnalysis {
        candidate_path: absolute.to_string_lossy().to_string(),
        candidate_type: if metadata.is_file() { "file" } else { "dir" }.to_string(),
        media_type: media_type.clone(),
        title: title.clone(),
        year,
        tmdb_id: group
            .tmdb_id
            .or_else(|| preliminary.iter().find_map(|item| item.tmdb_id)),
        videos,
        sidecars,
        ignored_samples: samples
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        query: MediaQuery {
            title,
            year,
            media_type,
            tmdb_id: group.tmdb_id,
        },
    })
}

#[derive(Clone)]
pub struct TmdbClient {
    client: Client,
    api_key: String,
    language: String,
    image_language: String,
    include_adult: bool,
    api_base: String,
    image_base: String,
}

impl TmdbClient {
    pub fn new(
        api_key: String,
        language: String,
        image_language: String,
        include_adult: bool,
        api_base: String,
        image_base: String,
        proxy: Option<String>,
    ) -> Result<Self, String> {
        let mut builder = Client::builder().timeout(Duration::from_secs(20));
        if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy.trim())
                    .map_err(|error| format!("初始化 TMDB 代理失败：{error}"))?,
            );
        }
        let client = builder
            .build()
            .map_err(|error| format!("初始化 TMDB 客户端失败：{error}"))?;
        Ok(Self {
            client,
            api_key,
            language,
            image_language,
            include_adult,
            api_base: api_base.trim_end_matches('/').to_string(),
            image_base: image_base.trim_end_matches('/').to_string(),
        })
    }

    fn image_url(&self, image_path: &str, size: &str) -> String {
        if image_path.is_empty() {
            String::new()
        } else if image_path.starts_with("http://") || image_path.starts_with("https://") {
            image_path.to_string()
        } else if self.image_base.contains("{size}") {
            format!(
                "{}/{}",
                self.image_base.replace("{size}", size),
                image_path.trim_start_matches('/')
            )
        } else {
            format!(
                "{}/{}/{}",
                self.image_base,
                size,
                image_path.trim_start_matches('/')
            )
        }
    }

    async fn request(
        &self,
        endpoint: &str,
        parameters: &[(&str, String)],
    ) -> Result<Value, String> {
        if self.api_key.trim().is_empty() {
            return Err("请先配置 TMDB API Key 或 Read Access Token".to_string());
        }
        let mut url = Url::parse(&format!(
            "{}/{}",
            self.api_base,
            endpoint.trim_start_matches('/')
        ))
        .map_err(|error| format!("TMDB 地址无效：{error}"))?;
        let bearer = self.api_key.starts_with("eyJ") || self.api_key.len() > 80;
        {
            let mut query = url.query_pairs_mut();
            if !bearer {
                query.append_pair("api_key", &self.api_key);
            }
            query.append_pair("language", &self.language);
            for (name, value) in parameters {
                if !value.is_empty() {
                    query.append_pair(name, value);
                }
            }
        }
        let mut request = self.client.get(url).header("accept", "application/json");
        if bearer {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("TMDB 请求失败：{error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("读取 TMDB 响应失败：{error}"))?;
        let payload: Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| serde_json::json!({ "status_message": text }));
        if !status.is_success() || payload.get("success").and_then(Value::as_bool) == Some(false) {
            let message = payload
                .get("status_message")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("TMDB 请求失败（HTTP {}）", status.as_u16()));
            return Err(message);
        }
        Ok(payload)
    }

    pub async fn test(&self) -> Result<(), String> {
        self.request("configuration", &[]).await.map(|_| ())
    }

    pub async fn search(&self, query: &MediaQuery) -> Result<Vec<TmdbCandidate>, String> {
        let media_type = if query.media_type == "tv" {
            "tv"
        } else {
            "movie"
        };
        let mut parameters = vec![
            ("query", query.title.clone()),
            ("include_adult", self.include_adult.to_string()),
            ("page", "1".to_string()),
        ];
        if let Some(year) = query.year {
            parameters.push((
                if media_type == "tv" {
                    "first_air_date_year"
                } else {
                    "year"
                },
                year.to_string(),
            ));
        }
        let mut payload = self
            .request(&format!("search/{media_type}"), &parameters)
            .await?;
        if payload
            .get("results")
            .and_then(Value::as_array)
            .map(|items| items.is_empty())
            .unwrap_or(true)
            && query.year.is_some()
        {
            payload = self
                .request(
                    &format!("search/{media_type}"),
                    &[
                        ("query", query.title.clone()),
                        ("include_adult", self.include_adult.to_string()),
                        ("page", "1".to_string()),
                    ],
                )
                .await?;
        }
        let mut results = payload
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(20)
            .filter_map(|item| self.normalize_candidate(&item, media_type, query))
            .collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    right
                        .popularity
                        .partial_cmp(&left.popularity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        Ok(results)
    }

    fn normalize_candidate(
        &self,
        item: &Value,
        media_type: &str,
        query: &MediaQuery,
    ) -> Option<TmdbCandidate> {
        let title = item
            .get(if media_type == "tv" { "name" } else { "title" })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let original_title = item
            .get(if media_type == "tv" {
                "original_name"
            } else {
                "original_title"
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let release_date = item
            .get(if media_type == "tv" {
                "first_air_date"
            } else {
                "release_date"
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let poster_path = item
            .get("poster_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Some(TmdbCandidate {
            tmdb_id: item.get("id")?.as_i64()?,
            media_type: media_type.to_string(),
            title: title.clone(),
            original_title: original_title.clone(),
            year: release_date.get(..4).and_then(|value| value.parse().ok()),
            release_date,
            overview: item
                .get("overview")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            vote_average: item
                .get("vote_average")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            popularity: item
                .get("popularity")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            poster_url: self.image_url(&poster_path, "w342"),
            poster_path,
            score: (score_tmdb_candidate(query, item, &title, &original_title) * 10_000.0).round()
                / 10_000.0,
            forced: false,
        })
    }

    pub async fn details(&self, media_type: &str, tmdb_id: i64) -> Result<MediaMetadata, String> {
        let media_type = if media_type == "tv" { "tv" } else { "movie" };
        let payload = self
            .request(
                &format!("{media_type}/{tmdb_id}"),
                &[
                    (
                        "append_to_response",
                        "credits,external_ids,images".to_string(),
                    ),
                    ("include_image_language", self.image_language.clone()),
                ],
            )
            .await?;
        Ok(self.normalize_metadata(&payload, media_type))
    }

    fn normalize_metadata(&self, item: &Value, media_type: &str) -> MediaMetadata {
        let title = item
            .get(if media_type == "tv" { "name" } else { "title" })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let original_title = item
            .get(if media_type == "tv" {
                "original_name"
            } else {
                "original_title"
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let release_date = item
            .get(if media_type == "tv" {
                "first_air_date"
            } else {
                "release_date"
            })
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let poster_path = item
            .get("poster_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let backdrop_path = item
            .get("backdrop_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let crew = item
            .pointer("/credits/crew")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let directors = crew
            .into_iter()
            .filter(|person| {
                matches!(
                    person.get("job").and_then(Value::as_str),
                    Some("Director") | Some("Series Director")
                )
            })
            .filter_map(|person| {
                person
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .take(12)
            .collect();
        let actors = item
            .pointer("/credits/cast")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(30)
            .map(|person| {
                let profile = person
                    .get("profile_path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                MediaActor {
                    name: person
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    role: person
                        .get("character")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    order: person.get("order").and_then(Value::as_i64).unwrap_or(0),
                    thumb: self.image_url(profile, "w185"),
                }
            })
            .collect();
        let string_list = |name: &str, child: &str| {
            item.get(name)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|entry| entry.get(child).and_then(Value::as_str).map(str::to_string))
                .collect::<Vec<_>>()
        };
        MediaMetadata {
            tmdb_id: item.get("id").and_then(Value::as_i64).unwrap_or(0),
            imdb_id: item
                .get("imdb_id")
                .or_else(|| item.pointer("/external_ids/imdb_id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            media_type: media_type.to_string(),
            title,
            original_title,
            year: release_date.get(..4).and_then(|value| value.parse().ok()),
            release_date,
            overview: item
                .get("overview")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tagline: item
                .get("tagline")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            status: item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            runtime: item
                .get("runtime")
                .and_then(Value::as_i64)
                .or_else(|| {
                    item.get("episode_run_time")
                        .and_then(Value::as_array)
                        .and_then(|values| values.first())
                        .and_then(Value::as_i64)
                })
                .unwrap_or(0),
            vote_average: item
                .get("vote_average")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            vote_count: item.get("vote_count").and_then(Value::as_i64).unwrap_or(0),
            genres: string_list("genres", "name"),
            genre_ids: item
                .get("genres")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("id").and_then(Value::as_i64))
                        .collect()
                })
                .unwrap_or_default(),
            studios: string_list("production_companies", "name"),
            countries: string_list("production_countries", "iso_3166_1"),
            original_language: item
                .get("original_language")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase(),
            origin_countries: item
                .get("origin_country")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|value| value.to_uppercase())
                        .collect()
                })
                .unwrap_or_else(|| string_list("production_countries", "iso_3166_1")),
            directors,
            actors,
            poster_url: self.image_url(&poster_path, "original"),
            backdrop_url: self.image_url(&backdrop_path, "original"),
            poster_path,
            backdrop_path,
            seasons: HashMap::new(),
        }
    }

    pub async fn season(&self, tmdb_id: i64, season_number: i64) -> Result<SeasonMetadata, String> {
        let payload = self
            .request(
                &format!("tv/{tmdb_id}/season/{season_number}"),
                &[
                    ("append_to_response", "images,external_ids".to_string()),
                    ("include_image_language", self.image_language.clone()),
                ],
            )
            .await?;
        let poster_path = payload
            .get("poster_path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let episodes = payload
            .get("episodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|episode| {
                let still_path = episode
                    .get("still_path")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                EpisodeMetadata {
                    episode_number: episode
                        .get("episode_number")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    season_number: episode
                        .get("season_number")
                        .and_then(Value::as_i64)
                        .unwrap_or(season_number),
                    name: episode
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    overview: episode
                        .get("overview")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    air_date: episode
                        .get("air_date")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    runtime: episode.get("runtime").and_then(Value::as_i64).unwrap_or(0),
                    vote_average: episode
                        .get("vote_average")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    still_url: self.image_url(&still_path, "original"),
                    still_path,
                }
            })
            .collect();
        Ok(SeasonMetadata {
            season_number: payload
                .get("season_number")
                .and_then(Value::as_i64)
                .unwrap_or(season_number),
            name: payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            overview: payload
                .get("overview")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            air_date: payload
                .get("air_date")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            poster_url: self.image_url(&poster_path, "original"),
            poster_path,
            episodes,
            error: None,
        })
    }
}

fn segmented_search_titles(value: &str) -> Vec<String> {
    let original = value.trim();
    let original_key = normalize_search_title(original);
    let mut values = Vec::new();
    let mut add = |candidate: &str| {
        let candidate = candidate.trim_matches(|character: char| {
            character.is_whitespace() || ":：·-".contains(character)
        });
        let key = normalize_search_title(candidate);
        if candidate.chars().count() >= 2
            && key != original_key
            && !values
                .iter()
                .any(|item: &String| normalize_search_title(item) == key)
        {
            values.push(candidate.to_string());
        }
    };
    for part in original.split(['/', '|', '｜']) {
        add(part);
    }
    let without_brackets = regex(r"[（(【\[].*?[）)】\]]").replace_all(original, " ");
    add(&without_brackets);
    let cjk = regex(r"[\p{Han}\p{Hiragana}\p{Katakana}\p{Hangul}]{2,}")
        .find_iter(original)
        .map(|item| item.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    add(&cjk);
    let latin = regex(r"[A-Za-z][A-Za-z0-9'’:&+\-]*(?:\s+[A-Za-z0-9][A-Za-z0-9'’:&+\-]*)*")
        .find_iter(original)
        .map(|item| item.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    add(&latin);
    values.truncate(3);
    values
}

fn exact_tmdb_candidate(candidate: &TmdbCandidate, query: &MediaQuery) -> bool {
    [candidate.title.as_str(), candidate.original_title.as_str()]
        .into_iter()
        .any(|title| normalize_search_title(title) == normalize_search_title(&query.title))
        && (query.year.is_none() || candidate.year.is_none() || query.year == candidate.year)
}

pub async fn resolve_tmdb_match(
    analysis: &CandidateAnalysis,
    client: &TmdbClient,
    settings: &NativeSettings,
    overrides: &RecognitionOverrides,
) -> Result<MatchResolution, String> {
    let media_type = if overrides.media_type.as_deref() == Some("tv") || analysis.media_type == "tv"
    {
        "tv"
    } else {
        "movie"
    };
    let query = MediaQuery {
        title: overrides
            .title
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| analysis.title.clone()),
        year: overrides.year.or(analysis.year),
        media_type: media_type.to_string(),
        tmdb_id: overrides.tmdb_id.or(analysis.tmdb_id),
    };
    if query.title.is_empty() && query.tmdb_id.is_none() {
        return Ok(MatchResolution {
            ready: false,
            error_code: Some("title_required".to_string()),
            message: "无法从文件名提取媒体名称，请输入名称或 TMDB ID".to_string(),
            query,
            candidates: Vec::new(),
            selected: None,
            metadata: None,
        });
    }
    let mut candidates;
    let selected;
    let metadata;
    if let Some(tmdb_id) = query.tmdb_id {
        metadata = client.details(media_type, tmdb_id).await?;
        selected = TmdbCandidate {
            tmdb_id: metadata.tmdb_id,
            media_type: media_type.to_string(),
            title: metadata.title.clone(),
            original_title: metadata.original_title.clone(),
            year: metadata.year,
            release_date: metadata.release_date.clone(),
            overview: metadata.overview.clone(),
            vote_average: metadata.vote_average,
            popularity: 0.0,
            poster_path: metadata.poster_path.clone(),
            poster_url: metadata.poster_url.clone(),
            score: 1.0,
            forced: true,
        };
        candidates = vec![selected.clone()];
    } else {
        candidates = client.search(&query).await?;
        if candidates.is_empty() && settings.word_segment_search {
            let mut seen = HashSet::new();
            for title in segmented_search_titles(&query.title) {
                let mut segmented_query = query.clone();
                segmented_query.title = title;
                for candidate in client.search(&segmented_query).await? {
                    if seen.insert((candidate.media_type.clone(), candidate.tmdb_id)) {
                        candidates.push(candidate);
                    }
                }
                if candidates.len() >= 20 {
                    break;
                }
            }
            candidates.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        right
                            .popularity
                            .partial_cmp(&left.popularity)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            candidates.truncate(20);
        }
        let first = candidates.first().cloned();
        let second = candidates.get(1);
        let exact = first
            .as_ref()
            .is_some_and(|item| exact_tmdb_candidate(item, &query));
        let automatic = if settings.similarity_match {
            first
                .as_ref()
                .filter(|item| {
                    item.score >= settings.minimum_match_score
                        && (exact
                            || second.is_none()
                            || item.score - second.map(|value| value.score).unwrap_or(0.0) >= 0.06)
                })
                .cloned()
        } else {
            candidates
                .iter()
                .find(|item| exact_tmdb_candidate(item, &query))
                .cloned()
        };
        let Some(automatic) = automatic else {
            return Ok(MatchResolution {
                ready: false,
                error_code: Some(
                    if candidates.is_empty() {
                        "tmdb_not_found"
                    } else {
                        "ambiguous_match"
                    }
                    .to_string(),
                ),
                message: if candidates.is_empty() {
                    "TMDB 未找到匹配结果，请修改名称、年份或直接填写 TMDB ID".to_string()
                } else {
                    "找到多个可能结果，请选择正确的 TMDB 条目".to_string()
                },
                query,
                candidates,
                selected: None,
                metadata: None,
            });
        };
        selected = automatic;
        metadata = client.details(media_type, selected.tmdb_id).await?;
    }
    let mut metadata = metadata;
    if media_type == "tv" {
        let mut seasons: Vec<i64> = analysis
            .videos
            .iter()
            .filter_map(|video| video.parsed.season)
            .collect();
        if let Some(season) = overrides.season {
            seasons.push(season);
        }
        seasons.sort_unstable();
        seasons.dedup();
        for season_number in seasons {
            let season = match client.season(selected.tmdb_id, season_number).await {
                Ok(value) => value,
                Err(error) => SeasonMetadata {
                    season_number,
                    name: format!("Season {season_number}"),
                    error: Some(error),
                    ..Default::default()
                },
            };
            metadata.seasons.insert(season_number.to_string(), season);
        }
    }
    Ok(MatchResolution {
        ready: true,
        error_code: None,
        message: format!(
            "已匹配 {}{}",
            metadata.title,
            metadata
                .year
                .map(|year| format!(" ({year})"))
                .unwrap_or_default()
        ),
        query,
        candidates,
        selected: Some(selected),
        metadata: Some(metadata),
    })
}

pub fn sanitize_component(value: &str, fallback: &str) -> String {
    let mut result = value
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', ' '])
        .trim()
        .to_string();
    if result.is_empty() {
        result = fallback.to_string();
    }
    if regex(r"(?i)^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$").is_match(&result) {
        result.insert(0, '_');
    }
    result.chars().take(180).collect()
}

pub fn render_template(template: &str, context: &HashMap<&str, String>) -> String {
    let token = regex(r"(?i)\{([a-z_]+)(?::(\d+))?\}");
    let rendered = token
        .replace_all(template.trim(), |captures: &regex::Captures| {
            let key = captures
                .get(1)
                .map(|item| item.as_str())
                .unwrap_or_default();
            let value = context.get(key).cloned().unwrap_or_default();
            captures
                .get(2)
                .and_then(|item| item.as_str().parse::<usize>().ok())
                .map(|width| format!("{value:0>width$}"))
                .unwrap_or(value)
        })
        .to_string();
    let cleaned = regex(r"\(\s*\)|\[\s*\]").replace_all(&rendered, "");
    let cleaned = regex(r"\s+-\s+-\s+").replace_all(&cleaned, " - ");
    let cleaned = regex(r"(?:\s+-\s*)+$").replace_all(&cleaned, "");
    sanitize_component(&cleaned, "Unknown")
}

fn episode_details(
    metadata: &MediaMetadata,
    season: i64,
    episode: i64,
) -> Option<&EpisodeMetadata> {
    metadata
        .seasons
        .get(&season.to_string())?
        .episodes
        .iter()
        .find(|item| item.episode_number == episode)
}

fn template_context(
    metadata: &MediaMetadata,
    parsed: Option<&ParsedMediaName>,
    episode: Option<&EpisodeMetadata>,
) -> HashMap<&'static str, String> {
    let parsed = parsed.cloned().unwrap_or_default();
    HashMap::from([
        ("title", metadata.title.clone()),
        ("original_title", metadata.original_title.clone()),
        (
            "year",
            metadata
                .year
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        ("tmdb_id", metadata.tmdb_id.to_string()),
        (
            "season",
            parsed
                .season
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        (
            "episode",
            parsed
                .episode
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        (
            "episode_end",
            parsed
                .episode_end
                .filter(|value| Some(*value) != parsed.episode)
                .map(|value| format!("-E{value:02}"))
                .unwrap_or_default(),
        ),
        (
            "episode_title",
            episode.map(|value| value.name.clone()).unwrap_or_default(),
        ),
        (
            "edition",
            (!parsed.edition.is_empty())
                .then(|| format!(" - {}", parsed.edition))
                .unwrap_or_default(),
        ),
        (
            "quality",
            (!parsed.quality.is_empty())
                .then(|| format!(" - {}", parsed.quality))
                .unwrap_or_default(),
        ),
        (
            "part",
            parsed
                .part
                .map(|value| format!(" - {value}"))
                .unwrap_or_default(),
        ),
    ])
}

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(digest)[..8].to_string()
}

fn normalized_claim(path: &Path) -> String {
    let value = path.to_string_lossy().to_string();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

fn ensure_within(root: &Path, candidate: PathBuf) -> Result<PathBuf, String> {
    let absolute_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(root)
    };
    let absolute_candidate = if candidate.is_absolute() {
        candidate
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(candidate)
    };
    absolute_candidate
        .strip_prefix(&absolute_root)
        .map_err(|_| "整理目标路径超出媒体库根目录".to_string())?;
    Ok(absolute_candidate)
}

fn resolve_target(
    target: PathBuf,
    source: &str,
    policy: &str,
    claimed: &mut HashSet<String>,
) -> (PathBuf, String, bool, bool) {
    let key = normalized_claim(&target);
    let already_claimed = claimed.contains(&key);
    let target_exists = target.exists();
    if !already_claimed && !target_exists {
        claimed.insert(key);
        return (target, "create".to_string(), false, false);
    }
    if !already_claimed && policy == "skip" {
        claimed.insert(key);
        return (target, "skip".to_string(), true, false);
    }
    if !already_claimed && policy == "overwrite" {
        claimed.insert(key);
        return (target, "overwrite".to_string(), true, false);
    }
    let extension = target
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let stem = target
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("media");
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let target_text = target.to_string_lossy().to_string();
    let hash = short_hash(if source.is_empty() {
        &target_text
    } else {
        source
    });
    let mut index = 0usize;
    loop {
        let suffix = if index == 0 {
            hash.clone()
        } else {
            format!("{hash}-{}", index + 1)
        };
        let filename = if extension.is_empty() {
            format!("{stem} [{suffix}]")
        } else {
            format!("{stem} [{suffix}].{extension}")
        };
        let candidate = parent.join(filename);
        let candidate_key = normalized_claim(&candidate);
        if !claimed.contains(&candidate_key) && !candidate.exists() {
            claimed.insert(candidate_key);
            return (candidate, "create".to_string(), false, true);
        }
        index += 1;
    }
}

fn language_suffix(path: &str) -> String {
    let name = stem_text(path).to_lowercase();
    let language = if regex(r"(?i)(?:zh[-_. ]?(?:cn|hans)|chs|sc|简体|簡體|简中)").is_match(&name)
    {
        ".zh-CN"
    } else if regex(r"(?i)(?:zh[-_. ]?(?:tw|hant)|cht|tc|繁体|繁體|繁中)").is_match(&name) {
        ".zh-TW"
    } else if regex(r"(?i)(?:^|[._ -])(?:eng|en)(?:[._ -]|$)|英文").is_match(&name) {
        ".en"
    } else if regex(r"(?i)(?:^|[._ -])(?:jpn|ja|jp)(?:[._ -]|$)|日文|日语|日語").is_match(&name)
    {
        ".ja"
    } else if regex(r"(?i)(?:^|[._ -])(?:kor|ko|kr)(?:[._ -]|$)|韩文|韓文|韩语|韓語")
        .is_match(&name)
    {
        ".ko"
    } else {
        ""
    };
    let forced = if regex(r"(?i)(?:^|[._ -])forced(?:[._ -]|$)").is_match(&name) {
        ".forced"
    } else {
        ""
    };
    let sdh = if regex(r"(?i)(?:^|[._ -])(?:sdh|hi)(?:[._ -]|$)").is_match(&name) {
        ".sdh"
    } else {
        ""
    };
    format!("{language}{forced}{sdh}")
}

fn make_item(
    kind: &str,
    source: Option<String>,
    target: PathBuf,
    operation: &str,
    action: String,
    exists: bool,
    renamed: bool,
    message: String,
) -> PreviewItem {
    PreviewItem {
        success: true,
        kind: kind.to_string(),
        source,
        target: target.to_string_lossy().to_string(),
        operation: operation.to_string(),
        action,
        exists,
        renamed_for_conflict: renamed,
        message,
        ..Default::default()
    }
}

pub fn build_native_preview(
    analysis: &CandidateAnalysis,
    matched: MatchResolution,
    mapping: &PreviewMapping,
    settings: &NativeSettings,
    mapping_signature: String,
    source_signature: String,
) -> Result<NativePreview, String> {
    if !matched.ready || matched.metadata.is_none() {
        return Ok(NativePreview {
            success: false,
            engine: NATIVE_ENGINE_VERSION.to_string(),
            mapping_signature,
            source_signature,
            query: matched.query,
            candidates: matched.candidates,
            selected: None,
            metadata: None,
            error_code: matched.error_code,
            message: matched.message,
            data: PreviewData::default(),
            ..Default::default()
        });
    }
    let metadata = matched.metadata.clone().expect("matched metadata");
    let target_root = PathBuf::from(&mapping.target_path);
    let media_folder = render_template(
        if metadata.media_type == "tv" {
            &settings.tv_folder_format
        } else {
            &settings.movie_folder_format
        },
        &template_context(&metadata, None, None),
    );
    let media_root = ensure_within(&target_root, target_root.join(media_folder))?;
    let mut claimed = HashSet::new();
    let mut items = Vec::new();
    let mut video_targets: HashMap<String, String> = HashMap::new();
    for video in &analysis.videos {
        let extension = Path::new(&video.source)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if !video.extra_kind.is_empty() {
            let folder = if video.extra_kind == "trailer" {
                "trailers"
            } else {
                "extras"
            };
            let name = sanitize_component(&stem_text(&video.source), "extra");
            let target = ensure_within(
                &target_root,
                media_root.join(folder).join(format!("{name}.{extension}")),
            )?;
            let (target, action, exists, renamed) = resolve_target(
                target,
                &video.source,
                &mapping.conflict_policy,
                &mut claimed,
            );
            let item = make_item(
                &video.extra_kind,
                Some(video.source.clone()),
                target,
                &mapping.transfer_type,
                action.clone(),
                exists,
                renamed,
                if action == "skip" {
                    "目标已存在，将跳过"
                } else {
                    "附加视频"
                }
                .to_string(),
            );
            video_targets.insert(video.source.clone(), item.target.clone());
            items.push(item);
            continue;
        }
        if metadata.media_type == "tv"
            && (video.parsed.season.is_none() || video.parsed.episode.is_none())
        {
            items.push(PreviewItem {
                success: false,
                kind: "video".to_string(),
                source: Some(video.source.clone()),
                operation: mapping.transfer_type.clone(),
                action: "error".to_string(),
                error_code: Some("episode_required".to_string()),
                message: "未识别到季集号，请人工填写季号/集号或调整文件名".to_string(),
                ..Default::default()
            });
            continue;
        }
        let episode = match (video.parsed.season, video.parsed.episode) {
            (Some(season), Some(episode)) => episode_details(&metadata, season, episode),
            _ => None,
        };
        let context = template_context(&metadata, Some(&video.parsed), episode);
        let (directory, filename) = if metadata.media_type == "tv" {
            (
                media_root.join(render_template(&settings.season_folder_format, &context)),
                render_template(&settings.episode_file_format, &context),
            )
        } else {
            (
                media_root.clone(),
                render_template(&settings.movie_file_format, &context),
            )
        };
        let target = ensure_within(
            &target_root,
            directory.join(format!("{filename}.{extension}")),
        )?;
        let (target, action, exists, renamed) = resolve_target(
            target,
            &video.source,
            &mapping.conflict_policy,
            &mut claimed,
        );
        let mut item = make_item(
            "video",
            Some(video.source.clone()),
            target,
            &mapping.transfer_type,
            action.clone(),
            exists,
            renamed,
            if action == "skip" {
                "目标已存在，将跳过"
            } else if renamed {
                "目标冲突，已追加短标识"
            } else {
                "可执行"
            }
            .to_string(),
        );
        item.season = video.parsed.season;
        item.episode = video.parsed.episode;
        item.episode_end = video.parsed.episode_end;
        video_targets.insert(video.source.clone(), item.target.clone());
        items.push(item);
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
            let extension = Path::new(&sidecar.source)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let video_path = Path::new(video_target);
            let stem = video_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("media");
            let target = video_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!(
                    "{}{}.{}",
                    stem,
                    language_suffix(&sidecar.source),
                    extension
                ));
            let target = ensure_within(&target_root, target)?;
            let (target, action, exists, renamed) = resolve_target(
                target,
                &sidecar.source,
                &mapping.conflict_policy,
                &mut claimed,
            );
            items.push(make_item(
                &sidecar.kind,
                Some(sidecar.source.clone()),
                target,
                &mapping.transfer_type,
                action.clone(),
                exists,
                renamed,
                if action == "skip" {
                    "目标已存在，将跳过"
                } else {
                    "跟随主视频整理"
                }
                .to_string(),
            ));
        }
    }
    if mapping.scrape {
        let main_videos: Vec<PreviewItem> = items
            .iter()
            .filter(|item| item.success && item.kind == "video")
            .cloned()
            .collect();
        if metadata.media_type == "movie" {
            for video in &main_videos {
                let target_path = Path::new(&video.target);
                let target = target_path.with_extension("nfo");
                let (target, action, exists, renamed) = resolve_target(
                    target,
                    &format!("movie-nfo:{}", metadata.tmdb_id),
                    &mapping.conflict_policy,
                    &mut claimed,
                );
                let mut item = make_item(
                    "nfo",
                    None,
                    target,
                    "generate",
                    action,
                    exists,
                    renamed,
                    "生成电影 NFO".to_string(),
                );
                item.generator = Some(GeneratorSpec {
                    generator_type: "movie".to_string(),
                    ..Default::default()
                });
                items.push(item);
            }
        } else {
            let (target, action, exists, renamed) = resolve_target(
                media_root.join("tvshow.nfo"),
                &format!("tv-nfo:{}", metadata.tmdb_id),
                &mapping.conflict_policy,
                &mut claimed,
            );
            let mut item = make_item(
                "nfo",
                None,
                target,
                "generate",
                action,
                exists,
                renamed,
                "生成剧集 NFO".to_string(),
            );
            item.generator = Some(GeneratorSpec {
                generator_type: "tvshow".to_string(),
                ..Default::default()
            });
            items.push(item);
            let mut seasons: Vec<i64> = main_videos.iter().filter_map(|item| item.season).collect();
            seasons.sort_unstable();
            seasons.dedup();
            for season in seasons {
                let mut parsed = ParsedMediaName::default();
                parsed.season = Some(season);
                let folder = render_template(
                    &settings.season_folder_format,
                    &template_context(&metadata, Some(&parsed), None),
                );
                let (target, action, exists, renamed) = resolve_target(
                    media_root.join(folder).join("season.nfo"),
                    &format!("season-nfo:{}:{season}", metadata.tmdb_id),
                    &mapping.conflict_policy,
                    &mut claimed,
                );
                let mut item = make_item(
                    "nfo",
                    None,
                    target,
                    "generate",
                    action,
                    exists,
                    renamed,
                    format!("生成第 {season} 季 NFO"),
                );
                item.generator = Some(GeneratorSpec {
                    generator_type: "season".to_string(),
                    season: Some(season),
                    episode: None,
                });
                items.push(item);
            }
            for video in &main_videos {
                let (target, action, exists, renamed) = resolve_target(
                    Path::new(&video.target).with_extension("nfo"),
                    &format!(
                        "episode-nfo:{}:{:?}:{:?}",
                        metadata.tmdb_id, video.season, video.episode
                    ),
                    &mapping.conflict_policy,
                    &mut claimed,
                );
                let mut item = make_item(
                    "nfo",
                    None,
                    target,
                    "generate",
                    action,
                    exists,
                    renamed,
                    "生成单集 NFO".to_string(),
                );
                item.generator = Some(GeneratorSpec {
                    generator_type: "episode".to_string(),
                    season: video.season,
                    episode: video.episode,
                });
                items.push(item);
            }
        }
        if !metadata.poster_url.is_empty() {
            let (target, action, exists, renamed) = resolve_target(
                media_root.join("poster.jpg"),
                &metadata.poster_url,
                &mapping.conflict_policy,
                &mut claimed,
            );
            let mut item = make_item(
                "image",
                Some(metadata.poster_url.clone()),
                target,
                "download",
                action,
                exists,
                renamed,
                "下载海报".to_string(),
            );
            item.image_role = Some("poster".to_string());
            items.push(item);
        }
        if !metadata.backdrop_url.is_empty() {
            let (target, action, exists, renamed) = resolve_target(
                media_root.join("fanart.jpg"),
                &metadata.backdrop_url,
                &mapping.conflict_policy,
                &mut claimed,
            );
            let mut item = make_item(
                "image",
                Some(metadata.backdrop_url.clone()),
                target,
                "download",
                action,
                exists,
                renamed,
                "下载背景图".to_string(),
            );
            item.image_role = Some("fanart".to_string());
            items.push(item);
        }
        if metadata.media_type == "tv" {
            for season in metadata.seasons.values() {
                if season.poster_url.is_empty() {
                    continue;
                }
                let (target, action, exists, renamed) = resolve_target(
                    media_root.join(format!("season{:02}-poster.jpg", season.season_number)),
                    &season.poster_url,
                    &mapping.conflict_policy,
                    &mut claimed,
                );
                let mut item = make_item(
                    "image",
                    Some(season.poster_url.clone()),
                    target,
                    "download",
                    action,
                    exists,
                    renamed,
                    format!("下载第 {} 季海报", season.season_number),
                );
                item.image_role = Some("season-poster".to_string());
                item.season = Some(season.season_number);
                items.push(item);
            }
        }
    }
    let failed = items.iter().filter(|item| !item.success).count();
    let skipped = items.iter().filter(|item| item.action == "skip").count();
    let warnings = skipped
        + analysis.ignored_samples.len()
        + metadata
            .seasons
            .values()
            .filter(|season| season.error.is_some())
            .count();
    let success = failed == 0
        && items
            .iter()
            .any(|item| item.success && item.kind == "video");
    let message = if failed > 0 {
        format!("有 {failed} 项无法生成目标，请人工修正")
    } else {
        format!(
            "已生成 {} 项原生整理预览{}",
            items.len(),
            if warnings > 0 {
                format!("，{warnings} 项提示")
            } else {
                String::new()
            }
        )
    };
    let error_code = items
        .iter()
        .find(|item| !item.success)
        .and_then(|item| item.error_code.clone());
    Ok(NativePreview {
        success,
        engine: NATIVE_ENGINE_VERSION.to_string(),
        mapping_signature,
        source_signature,
        query: matched.query,
        candidates: matched.candidates,
        selected: matched.selected,
        metadata: Some(metadata),
        target_root: target_root.to_string_lossy().to_string(),
        media_root: media_root.to_string_lossy().to_string(),
        error_code,
        message,
        ignored_samples: analysis.ignored_samples.clone(),
        data: PreviewData {
            summary: PreviewSummary {
                total: items.len(),
                success: items.len() - failed,
                failed,
                warnings,
                skipped,
            },
            items,
        },
    })
}

pub fn classify_preview(preview: Option<&NativePreview>) -> (bool, Option<String>, String) {
    let Some(preview) = preview else {
        return (
            false,
            Some("preview_required".to_string()),
            "当前任务没有可执行的原生整理预览".to_string(),
        );
    };
    if preview.engine != NATIVE_ENGINE_VERSION || !preview.success {
        return (
            false,
            preview
                .error_code
                .clone()
                .or(Some("preview_required".to_string())),
            if preview.message.is_empty() {
                "当前任务没有可执行的原生整理预览".to_string()
            } else {
                preview.message.clone()
            },
        );
    }
    if preview.metadata.is_none() {
        return (
            false,
            Some("metadata_required".to_string()),
            "原生整理预览缺少媒体元数据，请重新识别".to_string(),
        );
    }
    let failed = preview.data.items.iter().find(|item| !item.success);
    if !preview.data.items.iter().any(|item| item.kind == "video") || failed.is_some() {
        return (
            false,
            failed
                .and_then(|item| item.error_code.clone())
                .or(Some("preview_failed".to_string())),
            failed
                .map(|item| item.message.clone())
                .unwrap_or_else(|| "预览中存在不可执行项".to_string()),
        );
    }
    (
        true,
        None,
        if preview.message.is_empty() {
            format!("已生成 {} 项整理目标", preview.data.items.len())
        } else {
            preview.message.clone()
        },
    )
}

fn xml_escape(value: impl ToString) -> String {
    value
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_node(name: &str, value: impl ToString, indent: &str) -> String {
    let value = value.to_string();
    if value.is_empty() {
        String::new()
    } else {
        format!("{indent}<{name}>{}</{name}>\n", xml_escape(value))
    }
}

fn common_nfo(metadata: &MediaMetadata, indent: &str) -> String {
    let mut body = String::new();
    body.push_str(&xml_node("title", &metadata.title, indent));
    body.push_str(&xml_node("originaltitle", &metadata.original_title, indent));
    body.push_str(&xml_node(
        "year",
        metadata
            .year
            .map(|value| value.to_string())
            .unwrap_or_default(),
        indent,
    ));
    body.push_str(&xml_node("premiered", &metadata.release_date, indent));
    body.push_str(&xml_node("plot", &metadata.overview, indent));
    body.push_str(&xml_node("outline", &metadata.overview, indent));
    body.push_str(&xml_node("tagline", &metadata.tagline, indent));
    body.push_str(&xml_node("runtime", metadata.runtime, indent));
    body.push_str(&xml_node("rating", metadata.vote_average, indent));
    body.push_str(&xml_node("votes", metadata.vote_count, indent));
    body.push_str(&format!(
        "{indent}<uniqueid type=\"tmdb\" default=\"true\">{}</uniqueid>\n",
        metadata.tmdb_id
    ));
    if !metadata.imdb_id.is_empty() {
        body.push_str(&format!(
            "{indent}<uniqueid type=\"imdb\">{}</uniqueid>\n",
            xml_escape(&metadata.imdb_id)
        ));
    }
    for genre in &metadata.genres {
        body.push_str(&xml_node("genre", genre, indent));
    }
    for studio in &metadata.studios {
        body.push_str(&xml_node("studio", studio, indent));
    }
    for director in &metadata.directors {
        body.push_str(&xml_node("director", director, indent));
    }
    for actor in &metadata.actors {
        body.push_str(&format!("{indent}<actor>\n"));
        body.push_str(&xml_node("name", &actor.name, &format!("{indent}  ")));
        body.push_str(&xml_node("role", &actor.role, &format!("{indent}  ")));
        body.push_str(&xml_node("thumb", &actor.thumb, &format!("{indent}  ")));
        body.push_str(&format!("{indent}</actor>\n"));
    }
    body
}

pub fn render_nfo(generator: &GeneratorSpec, metadata: &MediaMetadata) -> String {
    let declaration = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    match generator.generator_type.as_str() {
        "movie" => format!(
            "{declaration}<movie>\n{}</movie>\n",
            common_nfo(metadata, "  ")
        ),
        "tvshow" => format!(
            "{declaration}<tvshow>\n{}</tvshow>\n",
            common_nfo(metadata, "  ")
        ),
        "season" => {
            let season_number = generator.season.unwrap_or(0);
            let season = metadata
                .seasons
                .get(&season_number.to_string())
                .cloned()
                .unwrap_or_default();
            format!(
                "{declaration}<season>\n{}{}</season>\n",
                xml_node(
                    "title",
                    if season.name.is_empty() {
                        format!("Season {season_number}")
                    } else {
                        season.name
                    },
                    "  "
                ),
                format!(
                    "{}{}{}",
                    xml_node("seasonnumber", season_number, "  "),
                    xml_node("plot", season.overview, "  "),
                    xml_node("premiered", season.air_date, "  ")
                )
            )
        }
        _ => {
            let season = generator.season.unwrap_or(0);
            let episode_number = generator.episode.unwrap_or(0);
            let episode = episode_details(metadata, season, episode_number)
                .cloned()
                .unwrap_or_default();
            let mut body = xml_node(
                "title",
                if episode.name.is_empty() {
                    format!("Episode {episode_number}")
                } else {
                    episode.name
                },
                "  ",
            );
            body.push_str(&xml_node("showtitle", &metadata.title, "  "));
            body.push_str(&xml_node("season", season, "  "));
            body.push_str(&xml_node("episode", episode_number, "  "));
            body.push_str(&xml_node("aired", episode.air_date, "  "));
            body.push_str(&xml_node("plot", episode.overview, "  "));
            body.push_str(&xml_node("runtime", episode.runtime, "  "));
            body.push_str(&xml_node("rating", episode.vote_average, "  "));
            body.push_str(&format!(
                "  <uniqueid type=\"tmdb\" default=\"true\">{}</uniqueid>\n",
                metadata.tmdb_id
            ));
            format!("{declaration}<episodedetails>\n{body}</episodedetails>\n")
        }
    }
}

async fn remove_if_exists(path: &Path) {
    let _ = async_fs::remove_file(path).await;
}

async fn backup_existing_file(target: &Path) -> Result<Option<PathBuf>, String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("检查已有目标失败：{error}")),
    };
    if metadata.is_dir() {
        return Err("目标路径已被目录占用，不能覆盖".to_string());
    }
    let backup = PathBuf::from(format!(
        "{}.guangya-backup-{}",
        target.to_string_lossy(),
        Uuid::new_v4()
    ));
    async_fs::rename(target, &backup)
        .await
        .map_err(|error| format!("备份已有目标失败：{error}"))?;
    Ok(Some(backup))
}

async fn commit_temporary_file(
    temporary: &Path,
    target: &Path,
    overwrite: bool,
) -> Result<(), String> {
    if overwrite {
        return async_fs::rename(temporary, target)
            .await
            .map_err(|error| format!("提交目标文件失败：{error}"));
    }
    async_fs::hard_link(temporary, target)
        .await
        .map_err(|error| format!("提交目标文件失败：{error}"))?;
    if let Err(error) = async_fs::remove_file(temporary).await {
        remove_if_exists(target).await;
        return Err(format!("清理临时文件失败：{error}"));
    }
    Ok(())
}

async fn atomic_write(target: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        async_fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("创建目标目录失败：{error}"))?;
    }
    let temporary = target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.guangya-{}.part",
            target
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("metadata"),
            Uuid::new_v4()
        ));
    if let Err(error) = async_fs::write(&temporary, bytes).await {
        remove_if_exists(&temporary).await;
        return Err(format!("写入临时文件失败：{error}"));
    }
    let backup = if overwrite {
        match backup_existing_file(target).await {
            Ok(backup) => backup,
            Err(error) => {
                remove_if_exists(&temporary).await;
                return Err(error);
            }
        }
    } else {
        match fs::symlink_metadata(target) {
            Ok(_) => {
                remove_if_exists(&temporary).await;
                return Err("目标在执行期间已存在，请重新生成预览".to_string());
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                remove_if_exists(&temporary).await;
                return Err(format!("检查目标文件失败：{error}"));
            }
        }
    };
    if let Err(error) = commit_temporary_file(&temporary, target, overwrite).await {
        remove_if_exists(&temporary).await;
        if let Some(backup) = backup.as_ref() {
            remove_if_exists(target).await;
            let _ = async_fs::rename(backup, target).await;
        }
        return Err(error);
    }
    if let Some(backup) = backup.as_ref() {
        remove_if_exists(backup).await;
    }
    Ok(())
}

#[derive(Default)]
struct FileTransaction {
    created: Vec<PathBuf>,
    moved: Vec<(PathBuf, PathBuf)>,
    backups: Vec<(PathBuf, PathBuf)>,
    delete_after_commit: Vec<PathBuf>,
    transferred: Vec<String>,
    skipped: Vec<String>,
}

async fn transfer_file(
    item: &PreviewItem,
    transaction: &mut FileTransaction,
) -> Result<(), String> {
    let source = PathBuf::from(item.source.as_deref().unwrap_or_default());
    let target = PathBuf::from(&item.target);
    if item.action == "skip" {
        transaction.skipped.push(item.target.clone());
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        async_fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("创建目标目录失败：{error}"))?;
    }
    if item.action == "overwrite" {
        if let Some(backup) = backup_existing_file(&target).await? {
            transaction.backups.push((target.clone(), backup));
        }
    }
    let temporary = target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.guangya-{}.part",
            target
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("media"),
            Uuid::new_v4()
        ));
    let operation_result: Result<(), String> = match item.operation.as_str() {
        "move" => match async_fs::rename(&source, &temporary).await {
            Ok(()) => {
                match commit_temporary_file(&temporary, &target, item.action == "overwrite").await {
                    Ok(()) => {
                        transaction.moved.push((source.clone(), target.clone()));
                        Ok(())
                    }
                    Err(error) => {
                        if let Err(restore_error) = async_fs::rename(&temporary, &source).await {
                            Err(format!(
                            "提交移动文件失败：{error}；源文件保留在 {}，恢复失败：{restore_error}",
                            temporary.to_string_lossy()
                        ))
                        } else {
                            Err(format!("提交移动文件失败：{error}"))
                        }
                    }
                }
            }
            Err(error) if error.kind() == ErrorKind::CrossesDevices => {
                match async_fs::copy(&source, &temporary).await {
                    Err(copy_error) => Err(format!("跨盘复制失败：{copy_error}")),
                    Ok(_) => {
                        match commit_temporary_file(&temporary, &target, item.action == "overwrite")
                            .await
                        {
                            Err(rename_error) => {
                                Err(format!("提交跨盘移动文件失败：{rename_error}"))
                            }
                            Ok(()) => {
                                transaction.created.push(target.clone());
                                transaction.delete_after_commit.push(source.clone());
                                Ok(())
                            }
                        }
                    }
                }
            }
            Err(error) => Err(format!("移动文件失败：{error}")),
        },
        "link" => match async_fs::hard_link(&source, &temporary).await {
            Err(error) => Err(format!("创建硬链接失败：{error}")),
            Ok(()) => {
                match commit_temporary_file(&temporary, &target, item.action == "overwrite").await {
                    Err(error) => Err(format!("提交硬链接失败：{error}")),
                    Ok(()) => {
                        transaction.created.push(target.clone());
                        Ok(())
                    }
                }
            }
        },
        "softlink" => {
            #[cfg(windows)]
            let result = std::os::windows::fs::symlink_file(&source, &target);
            #[cfg(unix)]
            let result = std::os::unix::fs::symlink(&source, &target);
            #[cfg(not(any(windows, unix)))]
            let result: std::io::Result<()> = Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "当前平台不支持软链接",
            ));
            match result {
                Err(error) => Err(format!("创建软链接失败：{error}")),
                Ok(()) => {
                    transaction.created.push(target.clone());
                    Ok(())
                }
            }
        }
        _ => match async_fs::copy(&source, &temporary).await {
            Err(error) => Err(format!("复制文件失败：{error}")),
            Ok(_) => {
                match commit_temporary_file(&temporary, &target, item.action == "overwrite").await {
                    Err(error) => Err(format!("提交复制文件失败：{error}")),
                    Ok(()) => {
                        transaction.created.push(target.clone());
                        Ok(())
                    }
                }
            }
        },
    };
    if let Err(error) = operation_result {
        let source_at_risk = item.operation == "move" && temporary.exists() && !source.exists();
        if !source_at_risk {
            remove_if_exists(&temporary).await;
        }
        return Err(format!(
            "{} 整理失败：{}{}",
            source
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("媒体文件"),
            error,
            if source_at_risk {
                format!("；源文件保留在临时路径 {}", temporary.to_string_lossy())
            } else {
                String::new()
            }
        ));
    }
    transaction.transferred.push(item.target.clone());
    Ok(())
}

async fn rollback(transaction: &mut FileTransaction) {
    for (source, target) in transaction.moved.iter().rev() {
        if target.exists() && !source.exists() {
            if let Some(parent) = source.parent() {
                let _ = async_fs::create_dir_all(parent).await;
            }
            let _ = async_fs::rename(target, source).await;
        }
    }
    for target in transaction.created.iter().rev() {
        remove_if_exists(target).await;
    }
    for (target, backup) in transaction.backups.iter().rev() {
        remove_if_exists(target).await;
        if backup.exists() {
            let _ = async_fs::rename(backup, target).await;
        }
    }
}

async fn cleanup_empty_parents(source: &Path, boundary: &Path) {
    let mut current = source.parent().map(Path::to_path_buf);
    while let Some(directory) = current {
        if directory != boundary && !directory.starts_with(boundary) {
            break;
        }
        let empty = fs::read_dir(&directory)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty {
            break;
        }
        let _ = async_fs::remove_dir(&directory).await;
        if directory == boundary {
            break;
        }
        current = directory.parent().map(Path::to_path_buf);
    }
}

pub async fn execute_preview(
    preview: &NativePreview,
    source_boundary: Option<&Path>,
) -> Result<ExecutionResult, String> {
    let (ready, _, message) = classify_preview(Some(preview));
    if !ready {
        return Err(message);
    }
    let mut transaction = FileTransaction::default();
    for item in preview.data.items.iter().filter(|item| {
        ["video", "subtitle", "audio", "trailer", "extra"].contains(&item.kind.as_str())
    }) {
        if let Err(error) = transfer_file(item, &mut transaction).await {
            rollback(&mut transaction).await;
            return Err(error);
        }
    }
    let mut warnings = Vec::new();
    let mut deleted_after_commit = Vec::new();
    for source in &transaction.delete_after_commit {
        match async_fs::remove_file(source).await {
            Ok(()) => deleted_after_commit.push(source.clone()),
            Err(error) => warnings.push(format!(
                "{}：目标已写入，但跨盘移动的源文件删除失败：{error}",
                source
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("媒体文件")
            )),
        }
    }
    for (_, backup) in &transaction.backups {
        remove_if_exists(backup).await;
    }
    if let Some(boundary) = source_boundary {
        for (source, _) in &transaction.moved {
            cleanup_empty_parents(source, boundary).await;
        }
        for source in &deleted_after_commit {
            cleanup_empty_parents(source, boundary).await;
        }
    }
    let metadata = preview
        .metadata
        .as_ref()
        .ok_or_else(|| "预览缺少媒体元数据".to_string())?;
    let image_client = Client::builder().timeout(Duration::from_secs(30)).build();
    let mut scraped = 0usize;
    for item in preview
        .data
        .items
        .iter()
        .filter(|item| item.kind == "nfo" || item.kind == "image")
    {
        if item.action == "skip" {
            transaction.skipped.push(item.target.clone());
            continue;
        }
        let target = PathBuf::from(&item.target);
        let result = if item.kind == "nfo" {
            match item.generator.as_ref() {
                Some(generator) => {
                    atomic_write(
                        &target,
                        render_nfo(generator, metadata).as_bytes(),
                        item.action == "overwrite",
                    )
                    .await
                }
                None => Err("NFO 生成描述缺失".to_string()),
            }
        } else {
            let url = item.source.as_deref().unwrap_or_default();
            match image_client.as_ref() {
                Err(error) => Err(format!("初始化图片下载客户端失败：{error}")),
                Ok(image_client) => match image_client
                    .get(url)
                    .header("accept", "image/*")
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        match response.bytes().await {
                            Ok(bytes) if !bytes.is_empty() && bytes.len() <= 25 * 1024 * 1024 => {
                                atomic_write(&target, &bytes, item.action == "overwrite").await
                            }
                            Ok(bytes) if bytes.is_empty() => Err("图片响应为空".to_string()),
                            Ok(_) => Err("图片超过 25 MB 安全限制".to_string()),
                            Err(error) => Err(format!("读取图片失败：{error}")),
                        }
                    }
                    Ok(response) => Err(format!(
                        "图片下载失败（HTTP {}）",
                        response.status().as_u16()
                    )),
                    Err(error) => Err(format!("图片下载失败：{error}")),
                },
            }
        };
        match result {
            Ok(()) => scraped += 1,
            Err(error) => warnings.push(format!(
                "{}：{}",
                target
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("刮削文件"),
                error
            )),
        }
    }
    Ok(ExecutionResult {
        success: true,
        transferred: transaction.transferred.len(),
        skipped: transaction.skipped.len(),
        scraped,
        warnings,
        targets: transaction.transferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn parser_extracts_episode_range_and_movie_edition() {
        let tv = parse_media_name(
            "Example.Show.S02E03-E04.2160p.WEB-DL.x265.mkv",
            &RecognitionOverrides {
                media_type: Some("tv".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(tv.title, "Example Show");
        assert_eq!(tv.season, Some(2));
        assert_eq!(tv.episode, Some(3));
        assert_eq!(tv.episode_end, Some(4));
        assert!(tv.quality.to_lowercase().contains("2160p"));
        let movie = parse_media_name(
            "Blade.Runner.1982.Directors.Cut.1080p.BluRay.mkv",
            &RecognitionOverrides::default(),
        );
        assert_eq!(movie.title, "Blade Runner");
        assert_eq!(movie.year, Some(1982));
        assert_eq!(movie.edition, "Director’s Cut");
    }

    #[test]
    fn auxiliary_rules_apply_capture_math_forced_tmdb_and_rich_metadata() {
        let mut settings = NativeSettings::default();
        settings.recognition_words =
            r"(?i)^Alias\.(\d+) => Example.Show.S01E\1@-12{[tmdbid=93740;type=tv]}".to_string();
        settings.release_groups = "WiKi".to_string();
        settings.render_words = r"(?i)H[ .]?265 => HEVC".to_string();
        let parsed = parse_media_name_with_settings(
            "Alias.24.2160p.WEB-DL.H.265.DDP5.1-WiKi.mkv",
            &RecognitionOverrides {
                media_type: Some("tv".to_string()),
                ..Default::default()
            },
            &settings,
        );
        assert_eq!(parsed.title, "Example Show");
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(12));
        assert_eq!(parsed.tmdb_id, Some(93740));
        assert_eq!(parsed.year, None);
        assert_eq!(parsed.video_format, "2160p");
        assert_eq!(parsed.resource_type, "WEB-DL");
        assert_eq!(parsed.release_type, "WEB-DL");
        assert_eq!(parsed.video_codec, "HEVC");
        assert_eq!(parsed.audio_codec, "DDP");
        assert_eq!(parsed.audio_info, "5.1");
        assert_eq!(parsed.release_group, "WiKi");
    }

    #[test]
    fn auxiliary_rule_validation_rejects_unsupported_regex_instead_of_literal_fallback() {
        let error =
            validate_auxiliary_rule_block(r"Show(?=\.S\d+) => Series", "自定义识别词", true)
                .expect_err("look-around is unsupported by the native regex engine");
        assert!(error.contains("第 1 行正则无效"));
        assert!(validate_auxiliary_rule_block(
            r"(?i)^Alias\.(\d+) => Show.S01E\1@-12",
            "自定义识别词",
            true,
        )
        .is_ok());
    }

    #[test]
    fn segmented_search_keeps_cjk_and_latin_title_variants_in_parity_with_web() {
        let values = segmented_search_titles("流浪地球 The Wandering Earth (2019)");
        assert!(values.iter().any(|value| value == "流浪地球"));
        assert!(values.iter().any(|value| value == "The Wandering Earth"));
    }

    #[test]
    fn exact_title_similarity_is_one() {
        assert_eq!(title_similarity("The Matrix", "The.Matrix"), 1.0);
    }

    #[test]
    fn nfo_escapes_xml_and_keeps_tmdb_id() {
        let metadata = MediaMetadata {
            tmdb_id: 7,
            title: "A & B".to_string(),
            original_title: "A < B".to_string(),
            actors: Vec::new(),
            ..Default::default()
        };
        let output = render_nfo(
            &GeneratorSpec {
                generator_type: "movie".to_string(),
                ..Default::default()
            },
            &metadata,
        );
        assert!(output.contains("A &amp; B"));
        assert!(output.contains("A &lt; B"));
        assert!(output.contains("default=\"true\">7</uniqueid>"));
    }

    #[tokio::test]
    async fn move_transaction_restores_earlier_files_when_a_later_item_fails() {
        let root = std::env::temp_dir().join(format!("guangya-organizer-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create organizer test directory");
        let source = root.join("source.mkv");
        let missing = root.join("missing.mkv");
        let target = root.join("library/source.mkv");
        fs::write(&source, b"video").expect("write organizer test source");
        let preview = NativePreview {
            success: true,
            engine: NATIVE_ENGINE_VERSION.to_string(),
            message: "ready".to_string(),
            metadata: Some(MediaMetadata::default()),
            data: PreviewData {
                items: vec![
                    PreviewItem {
                        success: true,
                        kind: "video".to_string(),
                        source: Some(source.to_string_lossy().to_string()),
                        target: target.to_string_lossy().to_string(),
                        operation: "move".to_string(),
                        action: "create".to_string(),
                        ..Default::default()
                    },
                    PreviewItem {
                        success: true,
                        kind: "video".to_string(),
                        source: Some(missing.to_string_lossy().to_string()),
                        target: root
                            .join("library/missing.mkv")
                            .to_string_lossy()
                            .to_string(),
                        operation: "move".to_string(),
                        action: "create".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let error = execute_preview(&preview, None)
            .await
            .expect_err("missing second source must fail the transaction");
        assert!(error.contains("missing.mkv"));
        assert_eq!(fs::read(&source).expect("source restored"), b"video");
        assert!(!target.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_metadata_overwrite_preserves_existing_file() {
        let root = std::env::temp_dir().join(format!("guangya-organizer-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create organizer test directory");
        let poster = root.join("poster.jpg");
        fs::write(&poster, b"existing-poster").expect("write existing poster");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind metadata failure server");
        let address = listener
            .local_addr()
            .expect("read metadata failure address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept metadata request");
            stream
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .expect("write metadata failure response");
        });
        let preview = NativePreview {
            success: true,
            engine: NATIVE_ENGINE_VERSION.to_string(),
            message: "ready".to_string(),
            metadata: Some(MediaMetadata::default()),
            data: PreviewData {
                items: vec![
                    PreviewItem {
                        success: true,
                        kind: "video".to_string(),
                        source: Some(root.join("source.mkv").to_string_lossy().to_string()),
                        target: root.join("movie.mkv").to_string_lossy().to_string(),
                        operation: "copy".to_string(),
                        action: "skip".to_string(),
                        ..Default::default()
                    },
                    PreviewItem {
                        success: true,
                        kind: "image".to_string(),
                        source: Some(format!("http://{address}/poster.jpg")),
                        target: poster.to_string_lossy().to_string(),
                        operation: "download".to_string(),
                        action: "overwrite".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let result = execute_preview(&preview, None)
            .await
            .expect("metadata failure is a completed warning");
        server.await.expect("metadata failure server completed");
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            fs::read(&poster).expect("poster preserved"),
            b"existing-poster"
        );
        let _ = fs::remove_dir_all(root);
    }
}
