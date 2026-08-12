//! 全局常量：API 端点、超时、并发、分页等配置。

use crate::prelude::*;

pub(crate) const API_BASE: &str = "https://api.guangyapan.com";
pub(crate) const ACCOUNT_BASE: &str = "https://account.guangyapan.com";
pub(crate) const DEVELOPER_API_BASE: &str = "https://dapi.guangyapan.com";
// Kept in sync with api_map's live Windows PC profile.
pub(crate) const OAUTH_CLIENT_ID: &str = "aMe_SVSlkrbQXpUT";
pub(crate) const OAUTH_CLIENT_SECRET: &str = "FNAfp5IFEfCn5MYsIUTewg";
pub(crate) const API_DEVICE_TYPE: &str = "5";
pub(crate) const API_APP_VERSION: &str = "1.0.2";
pub(crate) const API_VERSION_CODE: &str = "1002";
pub(crate) const API_USER_AGENT: &str = "GuangyapanPC/1.0.2";
pub(crate) const AUTH_URL: &str = "https://www.guangyapan.com/#/";
pub(crate) const DEFAULT_UPLOAD_CONCURRENCY: usize = 2;
pub(crate) const DEFAULT_DOWNLOAD_CONCURRENCY: usize = 2;
pub(crate) const MAX_TRANSFER_CONCURRENCY: usize = 8;
pub(crate) const FLASH_PREFLIGHT_CONCURRENCY: usize = 1;
pub(crate) const FLASH_PREFLIGHT_TOKEN_MAX_AGE_SECS: u64 = 10 * 60;
pub(crate) const DEFAULT_MULTIPART_PART_SIZE: &str = "auto";
pub(crate) const MULTIPART_PART_SIZE_OPTIONS: &[&str] = &["auto", "4m", "8m", "16m"];
pub(crate) const DEFAULT_CACHE_MAX_ENTRIES: usize = 10_000;
pub(crate) const MIN_CACHE_MAX_ENTRIES: usize = 100;
pub(crate) const MAX_CACHE_MAX_ENTRIES: usize = 100_000;
pub(crate) const REMOTE_DIRECTORY_CACHE_KEY_SEPARATOR: char = '\0';
/// 远端"路径→ID"映射的新鲜窗口：命中缓存但超过该时长未经远端确认时，
/// 必须先用 `find_remote_folder` 复核，防止把文件上传到已被其它客户端
/// 删除或改名的目录（与 Node 端 `remoteDirectoryFreshMs` 对齐）。
pub(crate) const REMOTE_DIRECTORY_CACHE_FRESH_SECS: u64 = 15;
pub(crate) const OSS_WRITE_RETRY_TIMES: usize = 5;
pub(crate) const FILE_STABILITY_WAIT_MS: u64 = 1_200;
pub(crate) const FILE_BUSY_RETRY_SECS: u64 = 3;
pub(crate) const POLL_INTERVAL_SECS: u64 = 5;
pub(crate) const API_CONNECT_TIMEOUT_SECS: u64 = 15;
pub(crate) const API_REQUEST_TIMEOUT_SECS: u64 = 30;
pub(crate) const FILE_LIST_REQUEST_TIMEOUT_SECS: u64 = 12;
pub(crate) const OSS_REQUEST_TIMEOUT_SECS: u64 = 600;
pub(crate) const OSS_MULTIPART_TARGET_PARTS: u64 = 9_000;
pub(crate) const OSS_MIB: u64 = 1024 * 1024;
pub(crate) const OSS_LARGE_FILE_PART_SIZE: u64 = 16 * OSS_MIB;
pub(crate) const CLOUD_CONFIRM_TIMEOUT_SECS: u64 = 600;
pub(crate) const PENDING_UPLOAD_RETRY_SECS: u64 = 15;
pub(crate) const UPLOAD_STATE_OSS_COMPLETE: &str = "oss_complete";
pub(crate) const UPLOAD_STATE_CLOUD_CONFIRMED: &str = "cloud_confirmed";
pub(crate) const UPLOAD_CANCELLED_MESSAGE: &str = "上传已取消";
pub(crate) const UPLOAD_PAUSED_MESSAGE: &str = "上传已暂停";
pub(crate) const AUTO_SHARE_QUIET_SECS: i64 = 30;
pub(crate) const TOKEN_REFRESH_INTERVAL_SECS: u64 = 20 * 60;
pub(crate) const DEFAULT_WEBDAV_PORT: u16 = 19_090;
pub(crate) const DEFAULT_WEBDAV_USERNAME: &str = "guangya";
pub(crate) const MAX_GCID_IMPORT_CONCURRENCY: usize = 32;
pub(crate) const MAX_GCID_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_GCID_IMPORT_ATTEMPTS: i64 = 5;
pub(crate) const GCID_EXPORT_SCAN_CONCURRENCY: usize = 24;
pub(crate) const GCID_EXPORT_INVENTORY_PAGE_SIZE: u64 = 1_000;
pub(crate) const GCID_EXPORT_INVENTORY_THRESHOLD: u64 = 500;
pub(crate) const GCID_EXPORT_FILE_CONCURRENCY: usize = 20;
pub(crate) const GCID_EXPORT_RANGE_CONCURRENCY: usize = 3;
pub(crate) const GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY: usize = 24;
pub(crate) const GCID_EXPORT_SCAN_ATTEMPTS: usize = 5;
pub(crate) const GCID_EXPORT_RANGE_ATTEMPTS: usize = 3;
pub(crate) const GCID_EXPORT_REQUEST_TIMEOUT_SECS: u64 = 30;
pub(crate) const GCID_EXPORT_READ_IDLE_TIMEOUT_SECS: u64 = 45;
pub(crate) const GCID_EXPORT_SNAPSHOT_FRESH_SECS: i64 = 10 * 60;
pub(crate) const GCID_EXPORT_DIAGNOSTIC_INFO_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const GCID_EXPORT_DIAGNOSTIC_DETAIL_LIMIT_BYTES: u64 = 24 * 1024 * 1024;
pub(crate) const CLEAR_RECYCLE_BIN_DEADLINE_SECS: u64 = 120;
pub(crate) const CLEAR_RECYCLE_BIN_POLL_INTERVAL_SECS: u64 = 1;
pub(crate) const DEVELOPER_NAME_RENAME_CONCURRENCY: usize = 2;
pub(crate) const DEVELOPER_NAME_RENAME_ATTEMPTS: usize = 5;
pub(crate) const DEVELOPER_PRE_AUDIT_BATCH_SIZE: usize = 20;
pub(crate) const DOWNLOAD_PARALLEL_MIN_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const DOWNLOAD_RANGE_MIN_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const DOWNLOAD_RANGE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const DOWNLOAD_MAX_HTTP_CONNECTIONS: usize = 8;
pub(crate) const DOWNLOAD_MAX_CONNECTIONS_PER_FILE: usize = 4;
pub(crate) const DOWNLOAD_PROBE_TIMEOUT_SECS: u64 = 12;
pub(crate) const DOWNLOAD_READ_IDLE_TIMEOUT_SECS: u64 = 45;
pub(crate) const DOWNLOAD_RANGE_ATTEMPTS: usize = 3;
pub(crate) const DEFAULT_API_PAGE_SIZE: u64 = 100;
pub(crate) const DEFAULT_RECENT_PAGE_SIZE: u64 = 20;
pub(crate) const MAX_API_PAGE_SIZE: u64 = 100;
pub(crate) const MAX_API_ID_LENGTH: usize = 256;
pub(crate) const MAX_API_ID_BATCH: usize = 1_000;
pub(crate) const MAX_API_CURSOR_LENGTH: usize = 256;
pub(crate) const MAX_REMOTE_NAME_LENGTH: usize = 255;
pub(crate) const MAX_OFFLINE_URL_LENGTH: usize = 8_192;
pub(crate) const MAX_OFFLINE_FILE_INDEXES: usize = 1_000;
pub(crate) const OFFLINE_RESTORE_POLL_SECS: u64 = 5;
pub(crate) const OFFLINE_RESTORE_RETRY_SECS: i64 = 15;
pub(crate) static OFFLINE_RESTORE_RECONCILING: AtomicUsize = AtomicUsize::new(0);
pub(crate) const MAX_SHARE_TRAFFIC_BYTES: u64 = 1_125_899_906_842_624;
pub(crate) const DEFAULT_MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "heif", "avif", "tif", "tiff",
    "raw", "cr2", "nef", "arw", "dng", "mp4", "mov", "mkv", "avi", "wmv", "flv", "webm", "m4v",
    "ts", "mts", "m2ts", "3gp", "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff",
];
pub(crate) const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "heif", "avif", "tif", "tiff",
    "raw", "cr2", "nef", "arw", "dng",
];
pub(crate) const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "avi", "wmv", "flv", "webm", "m4v", "ts", "mts", "m2ts", "3gp",
];
pub(crate) const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "lrc"];
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff",
];
pub(crate) const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "rtf", "odt", "ods",
    "odp", "epub",
];
pub(crate) const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "zipx", "iso",
];
pub(crate) const CLOUD_FILE_TYPE_IMAGE: u8 = 1;
pub(crate) const CLOUD_FILE_TYPE_VIDEO: u8 = 2;
pub(crate) const CLOUD_FILE_TYPE_AUDIO: u8 = 3;
pub(crate) const CLOUD_FILE_TYPE_DOCUMENT: u8 = 4;
pub(crate) const CLOUD_FILE_TYPE_ARCHIVE: u8 = 5;
