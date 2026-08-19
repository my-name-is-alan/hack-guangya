//! 文件打开：播放直链签发、云端文本读取、临时下载后系统打开、外部播放器唤起。
//!
//! 播放直链复用常驻 STRM 服务（`/strm/{fileId}?sign=…` 302 到云盘 CDN），
//! 签名是与实例绑定的稳定 HMAC，前端可长期缓存。

use crate::prelude::*;

const READ_TEXT_DEFAULT_BYTES: u64 = 512 * 1024;
const READ_TEXT_MAX_BYTES: u64 = 4 * 1024 * 1024;
const OPEN_LOCALLY_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// 批量签发播放直链：`http://127.0.0.1:{strm_port}/strm/{fileId}?sign=…`。
#[tauri::command]
pub(crate) fn get_play_urls(
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
) -> Result<Value, String> {
    let (base, secret) = {
        let guard = state.lock().map_err(|e| e.to_string())?;
        (
            format!(
                "http://127.0.0.1:{}",
                guard.virtual_library.options().strm_port
            ),
            guard.strm_sign_secret.clone(),
        )
    };
    let urls = file_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(|id| json!({ "file_id": id, "url": virtual_library::strm_url(&base, &secret, id) }))
        .collect::<Vec<_>>();
    Ok(json!({ "urls": urls }))
}

/// 读取云端小文件文本内容（歌词、文本/JSON 预览）。
/// 只 Range 拉取前 max_bytes 字节；UTF-8 优先，失败回退 GB18030。
#[tauri::command]
pub(crate) async fn read_cloud_text(
    state: tauri::State<'_, SharedState>,
    file_id: String,
    max_bytes: Option<u64>,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let file_id = file_id.trim().to_string();
    if file_id.is_empty() {
        return Err("文件 ID 为空".to_string());
    }
    let cap = max_bytes
        .unwrap_or(READ_TEXT_DEFAULT_BYTES)
        .clamp(1, READ_TEXT_MAX_BYTES);
    let (bytes, total_size) = fetch_cloud_prefix(&token, &device_id, &file_id, cap).await?;
    let truncated = total_size.is_some_and(|total| (bytes.len() as u64) < total);
    let (text, _encoding) = decode_cloud_text(&bytes);
    Ok(json!({
        "text": text,
        "truncated": truncated,
        "size": total_size.unwrap_or(bytes.len() as u64),
    }))
}

/// 下载到临时目录后用系统默认程序打开（“用 Windows/系统默认程序打开”入口）。
#[tauri::command]
pub(crate) async fn open_cloud_file_with_system(
    state: tauri::State<'_, SharedState>,
    file_id: String,
    file_name: String,
) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let file_id = file_id.trim().to_string();
    if file_id.is_empty() {
        return Err("文件 ID 为空".to_string());
    }
    let client = http_client()?;
    let mut response = send_cloud_get(&client, &token, &device_id, &file_id, false, None).await?;
    if matches!(response.status().as_u16(), 403 | 410) {
        response = send_cloud_get(&client, &token, &device_id, &file_id, true, None).await?;
    }
    if !response.status().is_success() {
        return Err(format!(
            "读取云端文件失败：HTTP {}",
            response.status().as_u16()
        ));
    }
    let size_error = format!(
        "文件超过 {} MB，请改用下载功能后再打开",
        OPEN_LOCALLY_MAX_BYTES / 1024 / 1024
    );
    if response
        .content_length()
        .is_some_and(|length| length > OPEN_LOCALLY_MAX_BYTES)
    {
        return Err(size_error);
    }
    let directory = std::env::temp_dir().join("guangya-open");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("创建临时目录失败：{error}"))?;
    let target = directory.join(format!(
        "{}-{}",
        file_id,
        sanitize_open_file_name(&file_name)
    ));
    let mut file = tokio::fs::File::create(&target)
        .await
        .map_err(|error| format!("写入临时文件失败：{error}"))?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取云端文件失败：{error}"))?;
        written += chunk.len() as u64;
        if written > OPEN_LOCALLY_MAX_BYTES {
            drop(file);
            let _ = tokio::fs::remove_file(&target).await;
            return Err(size_error);
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入临时文件失败：{error}"))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("写入临时文件失败：{error}"))?;
    drop(file);
    opener::open(&target).map_err(|error| format!("系统打开文件失败：{error}"))?;
    Ok(json!({ "path": target.to_string_lossy() }))
}

/// 探测本机常见播放器（固定安装路径 + PATH），供“用播放器打开”选择。
#[tauri::command]
pub(crate) fn list_local_players() -> Value {
    json!({ "players": detect_local_players() })
}

/// 用指定播放器可执行文件打开播放直链；只接受 HTTP(S) 地址参数。
#[tauri::command]
pub(crate) fn open_in_player(player_path: String, url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("播放地址必须是 HTTP(S) 直链".to_string());
    }
    let path = PathBuf::from(player_path.trim());
    if player_path.trim().is_empty() || !path.exists() {
        return Err("播放器路径不存在，请检查后重新填写".to_string());
    }
    spawn_player(&path, &url)
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LocalPlayer {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) path: String,
}

fn player_entry(id: &str, name: &str, path: &Path) -> LocalPlayer {
    LocalPlayer {
        id: id.to_string(),
        name: name.to_string(),
        path: path.to_string_lossy().into_owned(),
    }
}

/// 在 PATH 中查找可执行文件。
fn path_lookup(binary: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(binary))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "windows")]
fn detect_local_players() -> Vec<LocalPlayer> {
    let program_files =
        std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let program_files_x86 = std::env::var("ProgramFiles(x86)")
        .unwrap_or_else(|_| r"C:\Program Files (x86)".to_string());
    let candidates: [(&str, &str, Vec<PathBuf>); 4] = [
        (
            "potplayer",
            "PotPlayer",
            vec![
                Path::new(&program_files).join(r"DAUM\PotPlayer\PotPlayerMini64.exe"),
                Path::new(&program_files_x86).join(r"DAUM\PotPlayer\PotPlayerMini.exe"),
            ],
        ),
        (
            "vlc",
            "VLC",
            vec![
                Path::new(&program_files).join(r"VideoLAN\VLC\vlc.exe"),
                Path::new(&program_files_x86).join(r"VideoLAN\VLC\vlc.exe"),
            ],
        ),
        (
            "mpc-hc",
            "MPC-HC",
            vec![
                Path::new(&program_files).join(r"MPC-HC\mpc-hc64.exe"),
                Path::new(&program_files_x86).join(r"MPC-HC\mpc-hc.exe"),
                Path::new(&program_files_x86).join(r"K-Lite Codec Pack\MPC-HC64\mpc-hc64.exe"),
            ],
        ),
        (
            "mpv",
            "mpv",
            path_lookup("mpv.exe").into_iter().collect(),
        ),
    ];
    candidates
        .into_iter()
        .filter_map(|(id, name, paths)| {
            paths
                .into_iter()
                .find(|candidate| candidate.is_file())
                .map(|found| player_entry(id, name, &found))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn detect_local_players() -> Vec<LocalPlayer> {
    let mut players: Vec<LocalPlayer> = [
        ("iina", "IINA", "/Applications/IINA.app"),
        ("vlc", "VLC", "/Applications/VLC.app"),
    ]
    .into_iter()
    .filter(|(_, _, path)| Path::new(path).exists())
    .map(|(id, name, path)| player_entry(id, name, Path::new(path)))
    .collect();
    for (id, name, binary) in [("mpv", "mpv", "mpv")] {
        if players.iter().all(|player| player.id != id) {
            if let Some(found) = path_lookup(binary) {
                players.push(player_entry(id, name, &found));
            }
        }
    }
    players
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn detect_local_players() -> Vec<LocalPlayer> {
    [("vlc", "VLC", "vlc"), ("mpv", "mpv", "mpv")]
        .into_iter()
        .filter_map(|(id, name, binary)| {
            path_lookup(binary).map(|found| player_entry(id, name, &found))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn spawn_player(path: &Path, url: &str) -> Result<(), String> {
    // .app 包通过 open -a 启动，其余按可执行文件直接拉起。
    let is_bundle = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    let mut command = if is_bundle {
        let mut open = std::process::Command::new("open");
        open.arg("-a").arg(path).arg(url);
        open
    } else {
        let mut direct = std::process::Command::new(path);
        direct.arg(url);
        direct
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动播放器失败：{error}"))
}

#[cfg(not(target_os = "macos"))]
fn spawn_player(path: &Path, url: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err("播放器路径不是可执行文件".to_string());
    }
    std::process::Command::new(path)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动播放器失败：{error}"))
}

async fn send_cloud_get(
    client: &reqwest::Client,
    token: &str,
    device_id: &str,
    file_id: &str,
    force: bool,
    range_cap: Option<u64>,
) -> Result<reqwest::Response, String> {
    let url = cached_res_download_url(token, device_id, file_id, force).await?;
    let mut request = client.get(&url).timeout(Duration::from_secs(120));
    if let Some(cap) = range_cap {
        request = request.header(RANGE, format!("bytes=0-{}", cap.saturating_sub(1)));
    }
    request
        .send()
        .await
        .map_err(|error| format!("读取云端文件失败：{error}"))
}

/// 拉取云端文件前 cap 字节；403/410（直链过期）强制刷新重试一次。
/// 服务器可能忽略 Range 返回 200 全量，流式读取并在 cap 处截断。
async fn fetch_cloud_prefix(
    token: &str,
    device_id: &str,
    file_id: &str,
    cap: u64,
) -> Result<(Vec<u8>, Option<u64>), String> {
    let client = http_client()?;
    let mut response = send_cloud_get(&client, token, device_id, file_id, false, Some(cap)).await?;
    if matches!(response.status().as_u16(), 403 | 410) {
        response = send_cloud_get(&client, token, device_id, file_id, true, Some(cap)).await?;
    }
    if !response.status().is_success() {
        return Err(format!(
            "读取云端文件失败：HTTP {}",
            response.status().as_u16()
        ));
    }
    let total_size = response_total_size(&response);
    let mut bytes: Vec<u8> = Vec::with_capacity(cap.min(READ_TEXT_DEFAULT_BYTES) as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取云端文件失败：{error}"))?;
        let remain = cap as usize - bytes.len();
        if chunk.len() >= remain {
            bytes.extend_from_slice(&chunk[..remain]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, total_size))
}

/// 从 Content-Range（bytes 0-x/total）或 200 响应的 Content-Length 推断总大小。
fn response_total_size(response: &reqwest::Response) -> Option<u64> {
    if let Some(range) = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        if let Some((_, total)) = range.rsplit_once('/') {
            if let Ok(parsed) = total.trim().parse::<u64>() {
                return Some(parsed);
            }
        }
    }
    if response.status() == reqwest::StatusCode::OK {
        return response.content_length();
    }
    None
}

/// UTF-8（含 BOM）优先；UTF-16 按 BOM 识别；其余按 GB18030 解码
/// （中文歌词/文本的常见遗留编码）。
pub(crate) fn decode_cloud_text(bytes: &[u8]) -> (String, &'static str) {
    let stripped = bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(stripped) {
        return (text.to_string(), "utf-8");
    }
    if bytes.len() >= 2 && bytes[..2] == [0xFF, 0xFE] {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return (text.into_owned(), "utf-16le");
    }
    if bytes.len() >= 2 && bytes[..2] == [0xFE, 0xFF] {
        let (text, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return (text.into_owned(), "utf-16be");
    }
    let (text, _, _) = encoding_rs::GB18030.decode(stripped);
    (text.into_owned(), "gb18030")
}

/// 去掉路径分隔符与 Windows 保留字符，避免文件名逃出临时目录。
pub(crate) fn sanitize_open_file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|ch| match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|ch| ch == ' ' || ch == '.').to_string();
    if cleaned.is_empty() {
        "未命名文件".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_bom_text_decodes_as_utf8() {
        assert_eq!(decode_cloud_text("你好".as_bytes()).0, "你好");
        let with_bom = [&[0xEF, 0xBB, 0xBF][..], "hello".as_bytes()].concat();
        let (text, encoding) = decode_cloud_text(&with_bom);
        assert_eq!(text, "hello");
        assert_eq!(encoding, "utf-8");
    }

    #[test]
    fn gbk_text_falls_back_to_gb18030() {
        // “你好” 的 GBK 编码。
        let gbk = [0xC4, 0xE3, 0xBA, 0xC3];
        let (text, encoding) = decode_cloud_text(&gbk);
        assert_eq!(text, "你好");
        assert_eq!(encoding, "gb18030");
    }

    #[test]
    fn open_file_names_are_sanitized() {
        assert_eq!(sanitize_open_file_name("a/b\\c:d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize_open_file_name("  ..  "), "未命名文件");
        assert_eq!(sanitize_open_file_name("正常文件.json"), "正常文件.json");
    }
}
