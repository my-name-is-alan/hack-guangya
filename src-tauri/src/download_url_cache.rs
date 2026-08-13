//! 云盘签名直链缓存：fileId → 短期有效的 CDN 直链。
//!
//! WebDAV 读取、STRM 直链端点与虚拟库元数据下载共用同一份缓存，
//! 避免每个 Range/播放请求都调用一次 `get_res_download_url`。
//! 直链实测有效期约 6 小时；缓存有效期从 URL 签名参数解析并预留
//! 安全余量，解析失败时回退 30 分钟，上限 60 分钟。

use crate::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 2_048;
const FALLBACK_TTL_SECS: i64 = 30 * 60;
const MAX_TTL_SECS: i64 = 60 * 60;
const SAFETY_MARGIN_SECS: i64 = 5 * 60;
const MIN_TTL_SECS: i64 = 30;

struct CachedUrl {
    url: String,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct DownloadUrlCache {
    entries: Mutex<HashMap<String, CachedUrl>>,
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

pub(crate) fn download_url_cache() -> &'static DownloadUrlCache {
    static CACHE: OnceLock<DownloadUrlCache> = OnceLock::new();
    CACHE.get_or_init(DownloadUrlCache::default)
}

fn compact_date_to_unix(value: &str) -> Option<i64> {
    // 20260813T023000Z（OSS V4 / AWS SigV4 签名日期）
    let bytes = value.as_bytes();
    if bytes.len() != 16 || bytes[8] != b'T' || bytes[15] != b'Z' {
        return None;
    }
    let year: i32 = value.get(0..4)?.parse().ok()?;
    let month: u8 = value.get(4..6)?.parse().ok()?;
    let day: u8 = value.get(6..8)?.parse().ok()?;
    let hour: u8 = value.get(9..11)?.parse().ok()?;
    let minute: u8 = value.get(11..13)?.parse().ok()?;
    let second: u8 = value.get(13..15)?.parse().ok()?;
    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    let datetime = date.with_hms(hour, minute, second).ok()?;
    Some(datetime.assume_utc().unix_timestamp())
}

pub(crate) fn parse_signed_url_expiry_unix(url: &str) -> Option<i64> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let mut params: HashMap<String, String> = HashMap::new();
    for (name, value) in parsed.query_pairs() {
        params.insert(name.to_ascii_lowercase(), value.to_string());
    }
    let mut candidates: Vec<i64> = Vec::new();
    for name in ["expires", "x-oss-expires"] {
        let Some(value) = params.get(name) else {
            continue;
        };
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(number) = value.parse::<i64>() else {
            continue;
        };
        // x-oss-expires 也可能是相对时长（V4），只把明显的时间戳当作绝对过期。
        if name == "expires" || number > 10_000_000 {
            let seconds = if number > 10_000_000_000 {
                number / 1000
            } else {
                number
            };
            if seconds > 0 {
                candidates.push(seconds);
            }
        }
    }
    for (date_name, duration_name) in [("x-oss-date", "x-oss-expires"), ("x-amz-date", "x-amz-expires")]
    {
        let (Some(date), Some(duration)) = (params.get(date_name), params.get(duration_name))
        else {
            continue;
        };
        let Some(base) = compact_date_to_unix(date) else {
            continue;
        };
        let Ok(duration) = duration.parse::<i64>() else {
            continue;
        };
        if duration > 0 && duration <= 30 * 86_400 {
            candidates.push(base + duration);
        }
    }
    candidates.into_iter().min()
}

fn ttl_for(url: &str) -> Duration {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0);
    let candidate = parse_signed_url_expiry_unix(url)
        .map(|expiry| expiry - SAFETY_MARGIN_SECS - now_unix)
        .unwrap_or(FALLBACK_TTL_SECS);
    Duration::from_secs(candidate.clamp(MIN_TTL_SECS, MAX_TTL_SECS) as u64)
}

impl DownloadUrlCache {
    pub(crate) fn peek(&self, file_id: &str) -> Option<String> {
        let mut entries = self.entries.lock().ok()?;
        match entries.get(file_id) {
            Some(cached) if cached.expires_at > Instant::now() => Some(cached.url.clone()),
            Some(_) => {
                entries.remove(file_id);
                None
            }
            None => None,
        }
    }

    pub(crate) fn store(&self, file_id: &str, url: &str) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() >= MAX_ENTRIES && !entries.contains_key(file_id) {
            let now = Instant::now();
            entries.retain(|_, cached| cached.expires_at > now);
            if entries.len() >= MAX_ENTRIES {
                if let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, cached)| cached.expires_at)
                    .map(|(key, _)| key.clone())
                {
                    entries.remove(&oldest);
                }
            }
        }
        entries.insert(
            file_id.to_string(),
            CachedUrl {
                url: url.to_string(),
                expires_at: Instant::now() + ttl_for(url),
            },
        );
    }

    pub(crate) fn invalidate(&self, file_id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(file_id);
        }
    }

    fn gate(&self, file_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let Ok(mut gates) = self.gates.lock() else {
            return Arc::new(tokio::sync::Mutex::new(()));
        };
        if gates.len() > MAX_ENTRIES {
            gates.clear();
        }
        gates
            .entry(file_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

/// 获取（必要时申请并缓存）单文件下载直链；并发请求同一文件只发起一次上游调用。
pub(crate) async fn cached_res_download_url(
    token: &str,
    device_id: &str,
    file_id: &str,
    force: bool,
) -> Result<String, String> {
    let cache = download_url_cache();
    if force {
        cache.invalidate(file_id);
    } else if let Some(url) = cache.peek(file_id) {
        return Ok(url);
    }
    let gate = cache.gate(file_id);
    let _guard = gate.lock().await;
    if !force {
        if let Some(url) = cache.peek(file_id) {
            return Ok(url);
        }
    }
    let response = api_post(
        token,
        device_id,
        "/userres/v1/get_res_download_url",
        json!({ "fileId": file_id }),
        &[],
    )
    .await?;
    let data = response.data.unwrap_or_default();
    let url = ["signedURL", "signedUrl", "downloadUrl", "downloadURL", "url"]
        .iter()
        .find_map(|key| data.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "光鸭没有返回文件下载地址".to_string())?;
    cache.store(file_id, &url);
    Ok(url)
}
