//! 传输/网络代理等设置命令。

use crate::prelude::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct TransferSettings {
    pub(crate) upload_concurrency: usize,
    pub(crate) download_concurrency: usize,
    pub(crate) multipart_part_size: String,
}


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct NetworkPreferences {
    pub(crate) proxy_url: String,
    pub(crate) configured: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct NetworkPreferencesInput {
    #[serde(alias = "proxy", alias = "global_proxy", alias = "network_proxy")]
    pub(crate) proxy_url: Option<String>,
    #[serde(alias = "github")]
    pub(crate) github_proxy: Option<String>,
    #[serde(alias = "tmdb")]
    pub(crate) tmdb_proxy: Option<String>,
    #[serde(alias = "telegram_proxy", alias = "telegram")]
    pub(crate) tg_proxy: Option<String>,
}


#[tauri::command]
pub(crate) fn get_transfer_settings(state: tauri::State<'_, SharedState>) -> Result<TransferSettings, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(TransferSettings {
        upload_concurrency: guard.upload_concurrency,
        download_concurrency: guard.download_concurrency,
        multipart_part_size: guard.multipart_part_size.clone(),
    })
}

pub(crate) fn normalize_proxy_value(value: Option<String>, label: &str) -> Result<String, String> {
    let raw = value.unwrap_or_default().trim().to_string();
    if raw.is_empty() {
        return Ok(String::new());
    }
    if raw.len() > 512 {
        return Err(format!("{label}地址不能超过 512 个字符"));
    }
    let candidate = if raw.contains("://") {
        raw.clone()
    } else {
        format!("http://{raw}")
    };
    let mut parsed =
        reqwest::Url::parse(&candidate).map_err(|_| format!("{label}地址格式不正确"))?;
    let mut scheme = parsed.scheme().to_ascii_lowercase();
    if !matches!(
        scheme.as_str(),
        "http" | "https" | "socks" | "socks5" | "socks5h"
    ) {
        return Err(format!("{label}仅支持 HTTP、HTTPS 或 SOCKS5"));
    }
    if parsed.host_str().unwrap_or_default().is_empty()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(format!("{label}地址格式不正确"));
    }
    if scheme == "socks5h" {
        // reqwest/undici use the SOCKS5 hostname resolution path for the
        // socks5 scheme; normalize the common socks5h alias so the saved
        // value is accepted by both desktop and Web runtimes.
        parsed
            .set_scheme("socks5")
            .map_err(|_| format!("{label}协议格式不正确"))?;
        scheme = "socks5".to_string();
    }
    if matches!(scheme.as_str(), "socks" | "socks5") && parsed.port().is_none() {
        parsed
            .set_port(Some(1080))
            .map_err(|_| format!("{label}端口格式不正确"))?;
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

/// Read the unified proxy setting and migrate the first legacy per-service
/// value when opening a database created by an older release.
pub(crate) fn load_global_network_proxy(path: &Path) -> Result<String, String> {
    if let Some(value) = load_app_state(path, "network_proxy")? {
        return normalize_proxy_value(Some(value), "全局代理");
    }
    for key in [
        "network_proxy_github",
        "network_proxy_tmdb",
        "network_proxy_tg",
    ] {
        if let Some(value) = load_app_state(path, key)?.filter(|value| !value.trim().is_empty()) {
            return normalize_proxy_value(Some(value), "全局代理");
        }
    }
    Ok(String::new())
}

pub(crate) fn redact_network_error(error: &str, proxy: &str) -> String {
    let mut value = error.to_string();
    if !proxy.trim().is_empty() {
        value = value.replace(proxy, "[代理]");
        if let Ok(url) = reqwest::Url::parse(proxy) {
            if !url.username().is_empty() {
                value = value.replace(url.username(), "[用户名]");
            }
            if let Some(password) = url.password() {
                if !password.is_empty() {
                    value = value.replace(password, "[密码]");
                }
            }
        }
    }
    value.chars().take(240).collect()
}

pub(crate) fn network_public(mut value: NetworkPreferences) -> NetworkPreferences {
    value.configured = !value.proxy_url.trim().is_empty();
    value
}

#[tauri::command]
pub(crate) fn get_network_preferences(
    state: tauri::State<'_, SharedState>,
) -> Result<NetworkPreferences, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(network_public(NetworkPreferences {
        proxy_url: load_global_network_proxy(&guard.db_path)?,
        ..Default::default()
    }))
}

#[tauri::command]
pub(crate) fn update_network_preferences(
    state: tauri::State<'_, SharedState>,
    input: NetworkPreferencesInput,
) -> Result<NetworkPreferences, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    let current = load_global_network_proxy(&guard.db_path)?;
    let requested = input
        .proxy_url
        .or(input.github_proxy)
        .or(input.tmdb_proxy)
        .or(input.tg_proxy);
    let proxy_url = normalize_proxy_value(requested.or(Some(current)), "全局代理")?;
    save_app_state(&guard.db_path, "network_proxy", &proxy_url)?;
    // Keep the legacy keys synchronized for downgrade compatibility.
    save_app_state(&guard.db_path, "network_proxy_github", &proxy_url)?;
    save_app_state(&guard.db_path, "network_proxy_tmdb", &proxy_url)?;
    save_app_state(&guard.db_path, "network_proxy_tg", &proxy_url)?;
    // 让共享 HTTP 客户端立即切换到新代理。
    set_global_api_proxy(&proxy_url);
    Ok(network_public(NetworkPreferences {
        proxy_url,
        ..Default::default()
    }))
}

#[tauri::command]
pub(crate) async fn test_network(
    state: tauri::State<'_, SharedState>,
    target: String,
    proxy_url: Option<String>,
    tmdb_api_base: Option<String>,
    tmdb_api_key: Option<String>,
) -> Result<Value, String> {
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let normalized_target = target.trim().to_ascii_lowercase();
    if !matches!(
        normalized_target.as_str(),
        "github" | "tmdb" | "tg" | "hdhive"
    ) {
        return Err("不支持的网络测试目标".to_string());
    }
    let proxy = normalize_proxy_value(
        proxy_url.or(Some(load_global_network_proxy(&db_path)?)),
        "全局代理",
    )?;
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(15));
    if !proxy.is_empty() {
        builder = builder.proxy(
            reqwest::Proxy::all(&proxy).map_err(|error| format!("初始化网络代理失败：{error}"))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| format!("初始化网络测试客户端失败：{error}"))?;
    let mut endpoint = match normalized_target.as_str() {
        "github" => "https://api.github.com/zen".to_string(),
        "tg" => "https://api.telegram.org".to_string(),
        "hdhive" => load_app_state(&db_path, "hdhive_base_url")?.unwrap_or_default(),
        _ => format!(
            "{}/configuration",
            tmdb_api_base
                .unwrap_or_else(|| "https://api.themoviedb.org/3".to_string())
                .trim_end_matches('/')
        ),
    };
    if normalized_target == "hdhive" && endpoint.trim().is_empty() {
        return Ok(
            json!({ "target": normalized_target, "success": false, "reachable": false, "configured": false, "status": 0, "latency_ms": 0, "proxy": if proxy.is_empty() { "直连" } else { "已配置代理" }, "message": "尚未配置 HDHive 地址" }),
        );
    }
    let mut request = client.get(&endpoint).header("accept", "application/json");
    if normalized_target == "tmdb" {
        if let Some(key) = tmdb_api_key.filter(|value| !value.trim().is_empty()) {
            if key.starts_with("eyJ") || key.len() > 80 {
                request = request.bearer_auth(key);
            } else {
                endpoint.push_str(&format!(
                    "?api_key={}",
                    utf8_percent_encode(&key, NON_ALPHANUMERIC)
                ));
                request = client.get(&endpoint).header("accept", "application/json");
            }
        }
    }
    let started = Instant::now();
    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let reachable = true;
            let success = status < 500 && (normalized_target != "tmdb" || status < 400);
            let message = if success {
                "网络可达".to_string()
            } else if reachable {
                format!("网络可达，上游返回 HTTP {status}")
            } else {
                format!("上游返回 HTTP {status}")
            };
            Ok(
                json!({ "target": normalized_target, "success": success, "reachable": reachable, "status": status, "latency_ms": started.elapsed().as_millis(), "proxy": if proxy.is_empty() { "直连" } else { "已配置代理" }, "message": message }),
            )
        }
        Err(error) => Ok(
            json!({ "target": normalized_target, "success": false, "reachable": false, "status": 0, "latency_ms": started.elapsed().as_millis(), "proxy": if proxy.is_empty() { "直连" } else { "已配置代理" }, "message": format!("连接失败：{}", redact_network_error(&error.to_string(), &proxy)) }),
        ),
    }
}
#[tauri::command]
pub(crate) fn update_transfer_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    upload_concurrency: usize,
    download_concurrency: usize,
    multipart_part_size: Option<String>,
) -> Result<Snapshot, String> {
    if !(1..=MAX_TRANSFER_CONCURRENCY).contains(&upload_concurrency)
        || !(1..=MAX_TRANSFER_CONCURRENCY).contains(&download_concurrency)
    {
        return Err(format!(
            "上传和下载并发数必须在 1–{MAX_TRANSFER_CONCURRENCY} 之间"
        ));
    }
    let multipart_part_size = multipart_part_size
        .map(|value| validate_multipart_part_size(&value))
        .transpose()?;
    let next = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard.upload_concurrency = upload_concurrency;
        guard.download_concurrency = download_concurrency;
        if let Some(multipart_part_size) = multipart_part_size {
            guard.multipart_part_size = multipart_part_size;
        }
        save_config(&guard);
        snapshot(&guard)
    };
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(next)
}
