//! 登录会话：持久化、短信/扫码登录、令牌刷新。

use crate::prelude::*;

#[derive(Debug, Clone)]
pub(crate) struct SmsVerificationSession {
    pub(crate) phone_number: String,
    pub(crate) is_user: bool,
    pub(crate) captcha_token: Option<String>,
}


#[derive(Debug, Clone)]
pub(crate) struct AuthSession {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) account_scope: Option<String>,
}


pub(crate) fn load_auth_session(path: &Path) -> Result<AuthSession, String> {
    let connection = open_database(path)?;
    connection
        .query_row(
            "SELECT access_token, refresh_token, account_scope FROM auth_session WHERE id = 1",
            [],
            |row| {
                Ok(AuthSession {
                    access_token: row.get(0)?,
                    refresh_token: row.get(1)?,
                    account_scope: row.get(2)?,
                })
            },
        )
        .optional()
        .map(|value| {
            value.unwrap_or(AuthSession {
                access_token: None,
                refresh_token: None,
                account_scope: None,
            })
        })
        .map_err(|e| format!("读取登录状态失败：{e}"))
}

pub(crate) fn save_auth_session(
    path: &Path,
    access_token: Option<&str>,
    refresh_token: Option<&str>,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO auth_session (id, access_token, refresh_token, updated_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               access_token = COALESCE(excluded.access_token, auth_session.access_token),
               refresh_token = COALESCE(excluded.refresh_token, auth_session.refresh_token),
               updated_at = excluded.updated_at",
            params![access_token, refresh_token, unix_timestamp()],
        )
        .map_err(|e| format!("保存登录状态失败：{e}"))?;
    Ok(())
}

pub(crate) fn replace_auth_session(
    path: &Path,
    access_token: Option<&str>,
    refresh_token: Option<&str>,
    account_scope: Option<&str>,
) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "INSERT INTO auth_session (id, access_token, refresh_token, account_scope, updated_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               access_token = excluded.access_token,
               refresh_token = excluded.refresh_token,
               account_scope = excluded.account_scope,
               updated_at = excluded.updated_at",
            params![access_token, refresh_token, account_scope, unix_timestamp()],
        )
        .map_err(|e| format!("替换登录状态失败：{e}"))?;
    Ok(())
}

pub(crate) fn clear_persisted_access_token(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "UPDATE auth_session SET access_token = NULL, updated_at = ?1 WHERE id = 1",
            params![unix_timestamp()],
        )
        .map_err(|e| format!("清理过期登录状态失败：{e}"))?;
    Ok(())
}

pub(crate) fn clear_persisted_auth_session(path: &Path) -> Result<(), String> {
    let connection = open_database(path)?;
    connection
        .execute(
            "UPDATE auth_session
             SET access_token = NULL, refresh_token = NULL, account_scope = NULL, updated_at = ?1
             WHERE id = 1",
            params![unix_timestamp()],
        )
        .map_err(|e| format!("清理过期登录状态失败：{e}"))?;
    Ok(())
}

pub(crate) fn invalidate_auth_session(app: &tauri::AppHandle, state: &SharedState) -> Result<(), String> {
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.token = None;
        guard.refresh_token = None;
        guard.auth_account_scope = None;
        reset_runtime_remote_cache(&mut guard);
        guard.db_path.clone()
    };
    let result = clear_persisted_auth_session(&db_path);
    emit_state(app, state);
    result
}


pub(crate) fn auth_hook_script() -> &'static str {
    r#"(() => {
      if (window.__guangyaAuthHook) return;
      window.__guangyaAuthHook = true;
      const send = (value) => {
        if (typeof value !== 'string' || !value.startsWith('Bearer ')) return;
        const token = value.slice(7).trim();
        if (!token) return;
        const invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
        if (invoke) invoke('capture_token', { token }).catch(() => {});
        else setTimeout(() => send(value), 500);
      };
      const fetch0 = window.fetch;
      window.fetch = function(input, init) {
        try { const headers = new Headers((init && init.headers) || (input && input.headers) || {}); send(headers.get('authorization') || headers.get('Authorization')); } catch (_) {}
        return fetch0.apply(this, arguments);
      };
      const open0 = XMLHttpRequest.prototype.open;
      const set0 = XMLHttpRequest.prototype.setRequestHeader;
      XMLHttpRequest.prototype.open = function() { this.__gyHeaders = {}; return open0.apply(this, arguments); };
      XMLHttpRequest.prototype.setRequestHeader = function(key, value) { if (key && key.toLowerCase() === 'authorization') send(value); return set0.apply(this, arguments); };
    })();"#
}


pub(crate) fn normalize_china_phone(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("请输入有效的中国大陆手机号".to_string());
    }

    let mut plus_count = 0_u8;
    let mut parentheses_depth = 0_u8;
    for (index, character) in trimmed.char_indices() {
        match character {
            '0'..='9' | ' ' | '-' => {}
            '+' if index == 0 && plus_count == 0 => plus_count += 1,
            '(' if parentheses_depth == 0 => parentheses_depth = 1,
            ')' if parentheses_depth == 1 => parentheses_depth = 0,
            _ => return Err("请输入有效的中国大陆手机号".to_string()),
        }
    }
    if parentheses_depth != 0 {
        return Err("请输入有效的中国大陆手机号".to_string());
    }

    let compact = trimmed
        .chars()
        .filter(|character| character.is_ascii_digit() || *character == '+')
        .collect::<String>();
    let local = compact
        .strip_prefix("+86")
        .or_else(|| compact.strip_prefix("0086"))
        .or_else(|| (compact.len() == 13 && compact.starts_with("86")).then_some(&compact[2..]))
        .unwrap_or(compact.as_str());
    if local.len() != 11 || !local.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("请输入 11 位中国大陆手机号".to_string());
    }
    let digits = local.as_bytes();
    if digits[0] != b'1' || !(b'3'..=b'9').contains(&digits[1]) {
        return Err("请输入有效的中国大陆手机号".to_string());
    }
    Ok(format!("+86 {local}"))
}

pub(crate) fn masked_phone_name(phone_number: &str) -> String {
    let local = phone_number
        .trim()
        .strip_prefix("+86 ")
        .unwrap_or(phone_number);
    if local.len() == 11 && local.bytes().all(|byte| byte.is_ascii_digit()) {
        format!("用户{}****{}", &local[..3], &local[7..])
    } else {
        "光鸭用户".to_string()
    }
}


pub(crate) fn auth_context(state: &tauri::State<'_, SharedState>) -> Result<(String, String), String> {
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok((
        guard
            .token
            .clone()
            .ok_or_else(|| "请先登录光鸭云盘".to_string())?,
        guard.device_id.clone(),
    ))
}


#[tauri::command]
pub(crate) async fn get_overview(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let assets = api_post(&token, &device_id, "/assets/v1/get_assets", json!({}), &[]).await?;
    let profile = account_get(&token, &device_id, "/v1/user/me")
        .await
        .unwrap_or_else(|_| json!({}));
    Ok(json!({ "assets": assets.data.unwrap_or_else(|| json!({})), "profile": profile }))
}

#[tauri::command]
pub(crate) async fn get_assets(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(&token, &device_id, "/assets/v1/get_assets", json!({}), &[]).await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}

#[tauri::command]
pub(crate) async fn get_global_config(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let (token, device_id) = auth_context(&state)?;
    let response = api_post(
        &token,
        &device_id,
        "/misc/v1/get_global_config",
        json!({}),
        &[],
    )
    .await?;
    Ok(response.data.unwrap_or_else(|| json!({})))
}


pub(crate) async fn refresh_saved_session(app: tauri::AppHandle, state: SharedState) -> Result<bool, String> {
    let (refresh_token, device_id) = {
        let guard = state.lock().map_err(|e| e.to_string())?;
        (guard.refresh_token.clone(), guard.device_id.clone())
    };
    let Some(refresh_token) = refresh_token else {
        return Ok(false);
    };
    let (status_code, payload) = account_post(
        &device_id,
        "/v1/auth/token",
        json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": OAUTH_CLIENT_ID,
            "client_secret": OAUTH_CLIENT_SECRET,
        }),
    )
    .await?;
    if status_code >= 400 {
        let message = payload
            .get("error_description")
            .or_else(|| payload.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("刷新登录状态失败")
            .to_string();
        if matches!(status_code, 400 | 401 | 403) {
            invalidate_auth_session(&app, &state)?;
            return Err(format!("登录态已失效，请重新扫码登录：{message}"));
        }
        return Err(message);
    }
    let access_token = payload
        .get("access_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("access_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "刷新登录状态时没有返回 access_token".to_string())?;
    let next_refresh = payload
        .get("refresh_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("refresh_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.token = Some(access_token.clone());
        if next_refresh.is_some() {
            guard.refresh_token = next_refresh.clone();
        }
        reset_runtime_remote_cache(&mut guard);
        guard.db_path.clone()
    };
    save_auth_session(&db_path, Some(&access_token), next_refresh.as_deref())?;
    emit_state(&app, &state);
    drain_queue(app, state);
    Ok(true)
}

#[tauri::command]
pub(crate) async fn refresh_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<Value, String> {
    if refresh_saved_session(app, state.inner().clone()).await? {
        Ok(json!({ "authenticated": true }))
    } else {
        Err("登录态已失效，且没有可用的刷新令牌，请重新登录".to_string())
    }
}

pub(crate) async fn token_refresh_loop(app: tauri::AppHandle, state: SharedState) {
    loop {
        sleep(Duration::from_secs(TOKEN_REFRESH_INTERVAL_SECS)).await;
        let can_refresh = state
            .lock()
            .ok()
            .and_then(|guard| guard.refresh_token.clone())
            .is_some();
        if !can_refresh {
            continue;
        }
        if let Err(error) = refresh_saved_session(app.clone(), state.clone()).await {
            status(
                &app,
                "warning",
                format!("自动续期失败，将稍后重试：{error}"),
            );
        }
    }
}

#[tauri::command]
pub(crate) async fn request_sms_code(
    state: tauri::State<'_, SharedState>,
    phone: String,
    captcha_token: Option<String>,
) -> Result<Value, String> {
    let phone_number = normalize_china_phone(&phone)?;
    let device_id = state
        .lock()
        .map_err(|error| error.to_string())?
        .device_id
        .clone();
    let supplied_captcha_token = captcha_token.filter(|value| !value.trim().is_empty());
    let resolved_captcha_token = if let Some(token) = supplied_captcha_token {
        Some(token.trim().to_string())
    } else {
        let (status_code, payload) = account_post_with_captcha(
            &device_id,
            "/v1/shield/captcha/init",
            json!({
                "client_id": OAUTH_CLIENT_ID,
                "action": "POST:/v1/auth/verification",
                "device_id": device_id,
                "captcha_token": Value::Null,
                "meta": { "phone_number": phone_number }
            }),
            None,
        )
        .await?;
        if !(200..300).contains(&status_code) {
            if let Some(challenge) =
                captcha_challenge_response(&payload, payload_mentions_captcha(&payload))
            {
                return Ok(challenge);
            }
            return Err(account_error_message(&payload, "初始化短信安全验证失败"));
        }
        if account_payload_string(&payload, &["captcha_url", "captchaUrl", "url"]).is_some() {
            return captcha_challenge_response(&payload, true)
                .ok_or_else(|| "短信安全验证响应无效".to_string());
        }
        Some(
            account_payload_string(&payload, &["captcha_token", "captchaToken"])
                .ok_or_else(|| "短信安全验证没有返回 token 或验证页面".to_string())?,
        )
    };

    let (status_code, payload) = account_post_with_captcha(
        &device_id,
        "/v1/auth/verification",
        json!({
            "phone_number": phone_number,
            "target": "ANY",
            "client_id": OAUTH_CLIENT_ID,
            "usage": "SIGN_IN",
            "selected_channel": "VERIFICATION_PHONE",
        }),
        resolved_captcha_token.as_deref(),
    )
    .await?;
    if !(200..300).contains(&status_code) {
        if let Some(challenge) =
            captcha_challenge_response(&payload, payload_mentions_captcha(&payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&payload, "发送短信验证码失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&payload, false) {
        return Ok(challenge);
    }
    let verification_id = account_payload_string(&payload, &["verification_id"])
        .ok_or_else(|| "短信接口没有返回 verification_id".to_string())?;
    let is_user = account_payload_bool(&payload, "is_user")
        .ok_or_else(|| "短信接口没有返回 is_user".to_string())?;
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard
            .sms_verifications
            .retain(|_, verification| verification.phone_number != phone_number);
        guard.sms_verifications.insert(
            verification_id.clone(),
            SmsVerificationSession {
                phone_number: phone_number.clone(),
                is_user,
                captcha_token: resolved_captcha_token,
            },
        );
    }
    let mut result = flatten_account_payload(&payload);
    result.insert("request_id".to_string(), json!(verification_id));
    result.insert("phone_number".to_string(), json!(phone_number));
    result.insert("is_user".to_string(), json!(is_user));
    result.insert("captcha_required".to_string(), json!(false));
    Ok(Value::Object(result))
}

#[tauri::command]
pub(crate) async fn login_with_sms(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    phone: String,
    code: String,
    request_id: String,
    captcha_token: Option<String>,
) -> Result<Value, String> {
    let phone_number = normalize_china_phone(&phone)?;
    let verification_code = code.trim();
    if !(4..=8).contains(&verification_code.len())
        || !verification_code.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("请输入有效的短信验证码".to_string());
    }
    let verification_id = request_id.trim();
    if verification_id.is_empty() {
        return Err("请先获取短信验证码".to_string());
    }
    let (verification, device_id) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        let verification = guard
            .sms_verifications
            .get(verification_id)
            .cloned()
            .ok_or_else(|| "短信验证码请求已失效，请重新获取".to_string())?;
        (verification, guard.device_id.clone())
    };
    if verification.phone_number != phone_number {
        return Err("手机号与验证码请求不一致，请重新获取".to_string());
    }
    let resolved_captcha_token = captcha_token
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or(verification.captcha_token.clone());
    let (verify_status, verify_payload) = account_post_with_captcha(
        &device_id,
        "/v1/auth/verification/verify",
        json!({
            "verification_id": verification_id,
            "verification_code": verification_code,
            "client_id": OAUTH_CLIENT_ID
        }),
        resolved_captcha_token.as_deref(),
    )
    .await?;
    if !(200..300).contains(&verify_status) {
        if let Some(challenge) =
            captcha_challenge_response(&verify_payload, payload_mentions_captcha(&verify_payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&verify_payload, "短信验证码校验失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&verify_payload, false) {
        return Ok(challenge);
    }
    let verification_token = account_payload_string(&verify_payload, &["verification_token"])
        .ok_or_else(|| "短信校验接口没有返回 verification_token".to_string())?;
    let (endpoint, body) = if verification.is_user {
        (
            "/v1/auth/signin",
            json!({
                "username": phone_number,
                "verification_code": verification_code,
                "verification_token": verification_token,
                "client_id": OAUTH_CLIENT_ID
            }),
        )
    } else {
        (
            "/v1/auth/signup",
            json!({
                "phone_number": phone_number,
                "verification_code": verification_code,
                "verification_token": verification_token,
                "client_id": OAUTH_CLIENT_ID,
                "name": masked_phone_name(&phone_number)
            }),
        )
    };
    let (login_status, login_payload) = account_post_with_captcha(
        &device_id,
        endpoint,
        body,
        resolved_captcha_token.as_deref(),
    )
    .await?;
    if !(200..300).contains(&login_status) {
        if let Some(challenge) =
            captcha_challenge_response(&login_payload, payload_mentions_captcha(&login_payload))
        {
            return Ok(challenge);
        }
        return Err(account_error_message(&login_payload, "手机号登录失败"));
    }
    if let Some(challenge) = captcha_challenge_response(&login_payload, false) {
        return Ok(challenge);
    }
    let access_token = account_payload_string(&login_payload, &["access_token"])
        .ok_or_else(|| "登录接口没有返回 access_token".to_string())?;
    let refresh_token = account_payload_string(&login_payload, &["refresh_token"]);
    let account_scope = new_auth_account_scope(&access_token);
    let db_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    replace_auth_session(
        &db_path,
        Some(&access_token),
        refresh_token.as_deref(),
        Some(&account_scope),
    )?;
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        guard.token = Some(access_token);
        guard.refresh_token = refresh_token;
        guard.auth_account_scope = Some(account_scope);
        guard.sms_verifications.remove(verification_id);
        reset_runtime_remote_cache(&mut guard);
    }
    status(
        &app,
        "success",
        "手机号登录成功，可以开始使用云盘和备份任务",
    );
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(json!({ "authenticated": true, "is_user": verification.is_user }))
}

#[tauri::command]
pub(crate) fn clear_expired_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<(), String> {
    invalidate_auth_session(&app, state.inner())
}

#[tauri::command]
pub(crate) async fn start_device_login(state: tauri::State<'_, SharedState>) -> Result<Value, String> {
    let device_id = state
        .lock()
        .map_err(|error| error.to_string())?
        .device_id
        .clone();
    let (status, payload) = account_post(
        &device_id,
        "/v1/auth/device/code",
        json!({
            "scope": "user",
            "client_id": OAUTH_CLIENT_ID,
            "meta": { "scene": "pc_login" },
        }),
    )
    .await?;
    if status >= 400 {
        return Err(payload
            .get("error_description")
            .or_else(|| payload.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or("无法创建扫码登录任务")
            .to_string());
    }
    Ok(payload.get("data").cloned().unwrap_or(payload))
}

pub(crate) fn device_login_wait_response(status_code: u16, payload: &Value) -> Result<Option<Value>, String> {
    if let Some(error) = account_payload_string(payload, &["error"]) {
        return match error.trim().to_ascii_lowercase().as_str() {
            "authorization_pending" => Ok(Some(json!({
                "pending": true,
                "message": "等待扫码确认",
            }))),
            "slow_down" => Ok(Some(json!({
                "pending": true,
                "slow_down": true,
                "interval_increment": 5,
                "message": "请求过于频繁，已延长扫码查询间隔",
            }))),
            _ => Err(account_error_message(payload, "扫码登录失败")),
        };
    }
    if matches!(status_code, 202 | 428) {
        return Ok(Some(json!({
            "pending": true,
            "message": "等待扫码确认",
        })));
    }
    if status_code >= 400 {
        return Err(account_error_message(payload, "扫码登录失败"));
    }
    Ok(None)
}

#[tauri::command]
pub(crate) async fn poll_device_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    device_code: String,
) -> Result<Value, String> {
    let device_id = state
        .lock()
        .map_err(|error| error.to_string())?
        .device_id
        .clone();
    let (status_code, payload) = account_post(
        &device_id,
        "/v1/auth/token",
        json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "client_id": OAUTH_CLIENT_ID,
            "client_secret": OAUTH_CLIENT_SECRET,
        }),
    )
    .await?;
    let token = payload
        .get("access_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("access_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let refresh_token = payload
        .get("refresh_token")
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("refresh_token"))
        })
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(token) = token {
        let account_scope = new_auth_account_scope(&token);
        let db_path = {
            let mut guard = state.lock().map_err(|e| e.to_string())?;
            guard.token = Some(token.clone());
            guard.refresh_token = refresh_token.clone();
            guard.auth_account_scope = Some(account_scope.clone());
            reset_runtime_remote_cache(&mut guard);
            guard.db_path.clone()
        };
        if let Err(message) = replace_auth_session(
            &db_path,
            Some(&token),
            refresh_token.as_deref(),
            Some(&account_scope),
        ) {
            status(&app, "error", message);
        }
        status(&app, "success", "扫码登录成功，可以开始使用云盘和备份任务");
        emit_state(&app, state.inner());
        drain_queue(app, state.inner().clone());
        return Ok(json!({ "authenticated": true }));
    }
    if let Some(waiting) = device_login_wait_response(status_code, &payload)? {
        return Ok(waiting);
    }
    Err(account_error_message(&payload, "扫码登录失败"))
}

#[tauri::command]
pub(crate) fn open_login(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("auth") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(
        &app,
        "auth",
        WebviewUrl::External(
            AUTH_URL
                .parse()
                .map_err(|e| format!("登录页地址错误：{e}"))?,
        ),
    )
    .title("登录光鸭云盘")
    .inner_size(1120.0, 820.0)
    .initialization_script(auth_hook_script())
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub(crate) async fn capture_token(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    token: String,
) -> Result<(), String> {
    if token.len() < 20 {
        return Ok(());
    }
    let account_scope = new_auth_account_scope(&token);
    let db_path = {
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        if guard.token.as_deref() == Some(token.as_str()) && guard.refresh_token.is_none() {
            return Ok(());
        }
        guard.token = Some(token.clone());
        guard.refresh_token = None;
        guard.auth_account_scope = Some(account_scope.clone());
        reset_runtime_remote_cache(&mut guard);
        guard.db_path.clone()
    };
    if let Err(message) = replace_auth_session(&db_path, Some(&token), None, Some(&account_scope)) {
        status(&app, "error", message);
    }
    status(&app, "success", "已捕获官方登录态，可以开始监控上传");
    emit_state(&app, state.inner());
    drain_queue(app, state.inner().clone());
    Ok(())
}
