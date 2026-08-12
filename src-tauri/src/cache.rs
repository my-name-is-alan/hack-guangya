//! 远端目录缓存：失效、修剪、统计与远端路径解析。

use crate::prelude::*;

#[derive(Default)]
pub(crate) struct RemoteCacheGates {
    pub(crate) gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl RemoteCacheGates {
    pub(crate) fn gate(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let Ok(mut gates) = self.gates.lock() else {
            return Arc::new(tokio::sync::Mutex::new(()));
        };
        if gates.len() > MAX_CACHE_MAX_ENTRIES.saturating_mul(2) {
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        }
        gates
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub(crate) struct CacheSettings {
    pub(crate) enabled: bool,
    pub(crate) max_entries: usize,
}


#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct MetadataCacheStats {
    pub(crate) bytes: u64,
    pub(crate) entries: u64,
    pub(crate) file_fingerprints_bytes: u64,
    pub(crate) file_fingerprints_entries: u64,
    pub(crate) remote_cache_bytes: u64,
    pub(crate) remote_cache_entries: u64,
    pub(crate) policy: CacheSettings,
}


pub(crate) fn default_cache_enabled() -> bool {
    true
}
pub(crate) fn default_cache_max_entries() -> usize {
    DEFAULT_CACHE_MAX_ENTRIES
}
pub(crate) fn parse_cache_enabled(value: Option<&str>) -> bool {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("false" | "0" | "off" | "disabled") => false,
        Some("true" | "1" | "on" | "enabled") => true,
        _ => default_cache_enabled(),
    }
}
pub(crate) fn validate_cache_max_entries(value: usize) -> Result<usize, String> {
    if (MIN_CACHE_MAX_ENTRIES..=MAX_CACHE_MAX_ENTRIES).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "缓存条目上限必须在 {MIN_CACHE_MAX_ENTRIES}–{MAX_CACHE_MAX_ENTRIES} 之间"
        ))
    }
}
pub(crate) fn parse_cache_max_entries(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .and_then(|value| validate_cache_max_entries(value).ok())
        .unwrap_or_else(default_cache_max_entries)
}
pub(crate) fn cache_settings(state: &RuntimeState) -> CacheSettings {
    CacheSettings {
        enabled: state.cache_enabled,
        max_entries: state.cache_max_entries,
    }
}

pub(crate) fn reset_remote_cache(remote_cache: &mut HashMap<String, String>, generation: &mut u64) {
    remote_cache.clear();
    remote_cache.insert(String::new(), String::new());
    *generation = generation.wrapping_add(1);
}

pub(crate) fn reset_runtime_remote_cache(state: &mut RuntimeState) {
    reset_remote_cache(&mut state.remote_cache, &mut state.remote_cache_generation);
    state.remote_cache_validated_at.clear();
}

pub(crate) fn invalidate_remote_directory_cache(state: &SharedState) {
    if let Ok(mut guard) = state.lock() {
        reset_runtime_remote_cache(&mut guard);
    }
}

/// 按父目录/条目精确失效"路径→ID"映射，避免任何一次写操作都全表清空
/// （全表清空会让并发上传的 `ensure_remote_path` 反复重试直至报错）。
///
/// 移除规则：key 形如 `parent\0name`——
/// - key 的 parent 命中 `parent_ids`（该目录的直接子项可能已变化）；
/// - key 的 parent 命中 `entry_ids`（被删除/移动条目自身的子目录映射）；
/// - key 的 value 命中 `entry_ids`（指向被删除/移动条目的映射本身）。
pub(crate) fn invalidate_remote_directory_children(
    remote_cache: &mut HashMap<String, String>,
    validated_at: &mut HashMap<String, Instant>,
    parent_ids: &HashSet<String>,
    entry_ids: &[String],
) -> bool {
    let entry_ids = entry_ids.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut removed = false;
    remote_cache.retain(|key, value| {
        let parent = key
            .split(REMOTE_DIRECTORY_CACHE_KEY_SEPARATOR)
            .next()
            .unwrap_or("");
        let keep = !parent_ids.contains(parent)
            && !entry_ids.contains(parent)
            && !entry_ids.contains(value.as_str());
        if !keep {
            validated_at.remove(key);
            removed = true;
        }
        keep
    });
    removed
}

pub(crate) fn publish_cloud_mutation(
    app: &tauri::AppHandle,
    state: &SharedState,
    parent_ids: impl IntoIterator<Item = String>,
    entry_ids: &[String],
    unknown_parent_requires_full_refresh: bool,
    source: &str,
) {
    let mut parent_ids = parent_ids.into_iter().collect::<HashSet<_>>();
    let all_entries_located = if entry_ids.is_empty() {
        true
    } else {
        let (located_parents, all_entries_located) =
            webdav::invalidate_directory_cache_entries(entry_ids);
        parent_ids.extend(located_parents);
        all_entries_located
    };
    let all = unknown_parent_requires_full_refresh && !all_entries_located;
    if let Ok(mut guard) = state.lock() {
        if all || (parent_ids.is_empty() && entry_ids.is_empty()) {
            reset_runtime_remote_cache(&mut guard);
        } else {
            let RuntimeState {
                remote_cache,
                remote_cache_validated_at,
                remote_cache_generation,
                ..
            } = &mut *guard;
            if invalidate_remote_directory_children(
                remote_cache,
                remote_cache_validated_at,
                &parent_ids,
                entry_ids,
            ) {
                *remote_cache_generation = remote_cache_generation.wrapping_add(1);
            }
        }
    }
    if all {
        webdav::invalidate_all_directory_cache();
    } else {
        for parent_id in &parent_ids {
            webdav::invalidate_directory_cache(parent_id);
        }
    }
    webdav::publish_directory_invalidation(app, parent_ids, all, source);
}

pub(crate) fn publish_all_cloud_directories_changed(
    app: &tauri::AppHandle,
    state: &SharedState,
    source: &str,
) {
    invalidate_remote_directory_cache(state);
    webdav::invalidate_all_directory_cache();
    webdav::publish_directory_invalidation(app, Vec::new(), true, source);
}

pub(crate) fn publish_directory_contents_changed(
    app: &tauri::AppHandle,
    parent_ids: impl IntoIterator<Item = String>,
    source: &str,
) {
    let parent_ids = parent_ids.into_iter().collect::<HashSet<_>>();
    for parent_id in &parent_ids {
        webdav::invalidate_directory_cache(parent_id);
    }
    webdav::publish_directory_invalidation(app, parent_ids, false, source);
}

pub(crate) fn cached_remote_path_id(
    state: &SharedState,
    base_parent_id: &str,
    remote_path: &str,
) -> Option<String> {
    let normalized = normalize_remote_path(remote_path);
    if normalized.is_empty() {
        return Some(base_parent_id.to_string());
    }
    let guard = state.lock().ok()?;
    let mut parent_id = base_parent_id.to_string();
    for part in normalized.split('/') {
        parent_id = guard
            .remote_cache
            .get(&remote_directory_cache_key(&parent_id, part))?
            .clone();
    }
    Some(parent_id)
}

pub(crate) fn remote_directory_cache_key(parent_id: &str, name: &str) -> String {
    format!("{parent_id}{REMOTE_DIRECTORY_CACHE_KEY_SEPARATOR}{name}")
}

pub(crate) fn reconcile_remote_directory_cache_entries(
    remote_cache: &mut HashMap<String, String>,
    parent_id: &str,
    page: u64,
    data: &Value,
) -> bool {
    let list = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let directories = list
        .iter()
        .filter(|item| value_as_u64(item.get("resType")) == Some(2))
        .filter_map(|item| {
            let name = item.get("fileName").and_then(Value::as_str)?;
            let file_id = item.get("fileId").and_then(Value::as_str)?;
            Some((name.to_string(), file_id.to_string()))
        })
        .collect::<HashMap<_, _>>();
    let total = value_as_u64(data.get("total")).unwrap_or(list.len() as u64);
    let complete_snapshot = page == 0 && total <= list.len() as u64;
    let prefix = format!("{parent_id}{REMOTE_DIRECTORY_CACHE_KEY_SEPARATOR}");
    let mut invalidated = false;
    if complete_snapshot {
        remote_cache.retain(|key, _| {
            if !key.starts_with(&prefix) {
                return true;
            }
            let retained = directories.contains_key(&key[prefix.len()..]);
            invalidated |= !retained;
            retained
        });
    }
    for (name, file_id) in directories {
        let key = remote_directory_cache_key(parent_id, &name);
        invalidated |= remote_cache
            .get(&key)
            .is_some_and(|cached_id| cached_id != &file_id);
        remote_cache.insert(key, file_id);
    }
    invalidated
}

pub(crate) fn reconcile_remote_directory_cache_page(
    state: &SharedState,
    parent_id: &str,
    page: u64,
    data: &Value,
) {
    let Ok(mut guard) = state.lock() else {
        return;
    };
    if !guard.cache_enabled {
        return;
    }
    if reconcile_remote_directory_cache_entries(&mut guard.remote_cache, parent_id, page, data) {
        guard.remote_cache_generation = guard.remote_cache_generation.wrapping_add(1);
    }
    let max_entries = guard.cache_max_entries;
    trim_remote_cache(&mut guard.remote_cache, max_entries);
    // 刚从上游读到的目录页等同于一次成功复核。
    let now = Instant::now();
    let prefix = format!("{parent_id}{REMOTE_DIRECTORY_CACHE_KEY_SEPARATOR}");
    let RuntimeState {
        remote_cache,
        remote_cache_validated_at,
        ..
    } = &mut *guard;
    for key in remote_cache.keys() {
        if key.starts_with(&prefix) {
            remote_cache_validated_at.insert(key.clone(), now);
        }
    }
    remote_cache_validated_at.retain(|key, _| remote_cache.contains_key(key));
}

pub(crate) fn trim_file_fingerprint_cache(database: &Path, max_entries: usize) -> Result<(), String> {
    let max_entries = i64::try_from(max_entries).map_err(|_| "缓存条目上限无效".to_string())?;
    open_database(database)?
        .execute(
            "DELETE FROM file_fingerprints
             WHERE rowid IN (
               SELECT rowid FROM file_fingerprints
               ORDER BY computed_at DESC, rowid DESC
               LIMIT -1 OFFSET ?1
             )",
            params![max_entries],
        )
        .map_err(|error| format!("裁剪秒传指纹缓存失败：{error}"))?;
    Ok(())
}

pub(crate) fn trim_gcid_export_snapshot_cache(database: &Path, max_entries: usize) -> Result<(), String> {
    let max_entries = i64::try_from(max_entries).map_err(|_| "缓存条目上限无效".to_string())?;
    open_database(database)?
        .execute(
            "DELETE FROM gcid_export_snapshots
             WHERE rowid IN (
               SELECT rowid FROM gcid_export_snapshots
               ORDER BY last_used_at DESC, rowid DESC
               LIMIT -1 OFFSET ?1
             )",
            params![max_entries],
        )
        .map_err(|error| format!("裁剪秒传 JSON 快照缓存失败：{error}"))?;
    Ok(())
}

pub(crate) fn trim_gcid_export_file_hash_cache(database: &Path, max_entries: usize) -> Result<(), String> {
    let max_entries = i64::try_from(max_entries).map_err(|_| "缓存条目上限无效".to_string())?;
    open_database(database)?
        .execute(
            "DELETE FROM gcid_export_file_hashes
             WHERE rowid IN (
               SELECT rowid FROM gcid_export_file_hashes
               ORDER BY last_used_at DESC, rowid DESC
               LIMIT -1 OFFSET ?1
             )",
            params![max_entries],
        )
        .map_err(|error| format!("裁剪云端秒传指纹缓存失败：{error}"))?;
    Ok(())
}

pub(crate) fn trim_remote_cache(remote_cache: &mut HashMap<String, String>, max_entries: usize) {
    let excess = remote_cache
        .len()
        .saturating_sub(usize::from(remote_cache.contains_key("")))
        .saturating_sub(max_entries);
    let keys = remote_cache
        .keys()
        .filter(|key| !key.is_empty())
        .take(excess)
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        remote_cache.remove(&key);
    }
}

pub(crate) fn file_fingerprint_cache_usage(database: &Path) -> Result<(u64, u64), String> {
    let connection = open_database(database)?;
    let mut statement = connection
        .prepare("SELECT file_path, modified_ms, gcid, cid FROM file_fingerprints")
        .map_err(|error| format!("读取秒传指纹缓存统计失败：{error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("读取秒传指纹缓存统计失败：{error}"))?;
    let mut entries = 0_u64;
    let mut bytes = 0_u64;
    for row in rows {
        let (file_path, modified_ms, gcid, cid) =
            row.map_err(|error| format!("解析秒传指纹缓存统计失败：{error}"))?;
        entries = entries.saturating_add(1);
        bytes = bytes.saturating_add(
            u64::try_from(file_path.len() + modified_ms.len() + gcid.len() + cid.len())
                .unwrap_or(u64::MAX)
                .saturating_add(16),
        );
    }
    Ok((entries, bytes))
}

pub(crate) fn gcid_export_snapshot_cache_usage(database: &Path) -> Result<(u64, u64), String> {
    let (entries, bytes) = open_database(database)?
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(root_signatures_json) + LENGTH(export_json) + 64), 0)
             FROM gcid_export_snapshots",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| format!("读取秒传 JSON 快照缓存统计失败：{error}"))?;
    Ok((
        u64::try_from(entries).unwrap_or(0),
        u64::try_from(bytes).unwrap_or(0),
    ))
}

pub(crate) fn gcid_export_file_hash_cache_usage(database: &Path) -> Result<(u64, u64), String> {
    let (entries, bytes) = open_database(database)?
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(file_id) + LENGTH(file_size) + LENGTH(gcid) + LENGTH(cid) + 64), 0)
             FROM gcid_export_file_hashes",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| format!("读取云端秒传指纹缓存统计失败：{error}"))?;
    Ok((
        u64::try_from(entries).unwrap_or(0),
        u64::try_from(bytes).unwrap_or(0),
    ))
}

pub(crate) fn remote_cache_usage(remote_cache: &HashMap<String, String>) -> (u64, u64) {
    remote_cache
        .iter()
        .filter(|(key, value)| !(key.is_empty() && value.is_empty()))
        .fold((0_u64, 0_u64), |(entries, bytes), (key, value)| {
            (
                entries.saturating_add(1),
                bytes.saturating_add(u64::try_from(key.len() + value.len()).unwrap_or(u64::MAX)),
            )
        })
}

pub(crate) fn metadata_cache_stats(
    database: &Path,
    remote_cache: &HashMap<String, String>,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    let (file_fingerprints_entries, file_fingerprints_bytes) =
        file_fingerprint_cache_usage(database)?;
    let (remote_edge_entries, remote_edge_bytes) = remote_cache_usage(remote_cache);
    let (snapshot_entries, snapshot_bytes) = gcid_export_snapshot_cache_usage(database)?;
    let (cloud_hash_entries, cloud_hash_bytes) = gcid_export_file_hash_cache_usage(database)?;
    let remote_cache_entries = remote_edge_entries
        .saturating_add(snapshot_entries)
        .saturating_add(cloud_hash_entries);
    let remote_cache_bytes = remote_edge_bytes
        .saturating_add(snapshot_bytes)
        .saturating_add(cloud_hash_bytes);
    Ok(MetadataCacheStats {
        bytes: file_fingerprints_bytes.saturating_add(remote_cache_bytes),
        entries: file_fingerprints_entries.saturating_add(remote_cache_entries),
        file_fingerprints_bytes,
        file_fingerprints_entries,
        remote_cache_bytes,
        remote_cache_entries,
        policy,
    })
}

pub(crate) fn clear_metadata_cache_storage(
    database: &Path,
    remote_cache: &mut HashMap<String, String>,
    remote_cache_generation: &mut u64,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    open_database(database)?
        .execute_batch(
            "DELETE FROM file_fingerprints;
             DELETE FROM gcid_export_snapshots;
             DELETE FROM gcid_export_file_hashes;",
        )
        .map_err(|error| format!("清理秒传指纹缓存失败：{error}"))?;
    reset_remote_cache(remote_cache, remote_cache_generation);
    metadata_cache_stats(database, remote_cache, policy)
}

pub(crate) fn apply_cache_policy(
    database: &Path,
    remote_cache: &mut HashMap<String, String>,
    remote_cache_generation: &mut u64,
    policy: CacheSettings,
) -> Result<MetadataCacheStats, String> {
    if !policy.enabled {
        return clear_metadata_cache_storage(
            database,
            remote_cache,
            remote_cache_generation,
            policy,
        );
    }
    trim_file_fingerprint_cache(database, policy.max_entries)?;
    trim_gcid_export_snapshot_cache(database, policy.max_entries)?;
    trim_gcid_export_file_hash_cache(database, policy.max_entries)?;
    trim_remote_cache(remote_cache, policy.max_entries);
    metadata_cache_stats(database, remote_cache, policy)
}


pub(crate) async fn find_remote_folder(
    token: &str,
    device_id: &str,
    parent_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let mut seen = 0_u64;
    for page in 0..100 {
        let result = api_post(token, device_id, "/userres/v1/file/get_file_list", json!({ "page": page, "pageSize": 100, "parentId": parent_id, "resType": 2, "needSubFolderStat": true }), &[]).await?;
        let data = result.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(found) = list.iter().find(|item| {
            item.get("resType").and_then(Value::as_i64) == Some(2)
                && item.get("fileName").and_then(Value::as_str) == Some(name)
        }) {
            return Ok(found
                .get("fileId")
                .and_then(Value::as_str)
                .map(str::to_owned));
        }
        seen = seen.saturating_add(list.len() as u64);
        let total = data.get("total").and_then(Value::as_u64);
        if remote_folder_page_complete(seen, list.len(), total) {
            break;
        }
    }
    Ok(None)
}

pub(crate) fn remote_folder_page_complete(seen: u64, page_len: usize, total: Option<u64>) -> bool {
    page_len == 0 || total.is_some_and(|total| seen >= total) || (total.is_none() && page_len < 100)
}

pub(crate) async fn ensure_remote_path(
    app: &tauri::AppHandle,
    state: &SharedState,
    token: &str,
    device_id: &str,
    base_parent_id: &str,
    remote_path: &str,
) -> Result<String, String> {
    let normalized = normalize_remote_path(remote_path);
    if normalized.is_empty() {
        return Ok(base_parent_id.to_string());
    }
    'resolve: for _ in 0..8 {
        let mut parent = base_parent_id.to_string();
        let mut prefix = String::new();
        for part in normalized.split('/') {
            prefix = if prefix.is_empty() {
                part.to_owned()
            } else {
                format!("{prefix}/{part}")
            };
            let cache_key = remote_directory_cache_key(&parent, part);
            let (captured_generation, gate_pool) = {
                let guard = state.lock().map_err(|error| error.to_string())?;
                (
                    guard.remote_cache_generation,
                    Arc::clone(&guard.remote_cache_gates),
                )
            };
            let gate = gate_pool.gate(&format!("{captured_generation}\0{cache_key}"));
            let _gate = gate.lock().await;
            let (generation, cached, cached_fresh) = {
                let guard = state.lock().map_err(|error| error.to_string())?;
                let cached = guard
                    .cache_enabled
                    .then(|| guard.remote_cache.get(&cache_key).cloned())
                    .flatten();
                let fresh = guard
                    .remote_cache_validated_at
                    .get(&cache_key)
                    .is_some_and(|at| {
                        at.elapsed() <= Duration::from_secs(REMOTE_DIRECTORY_CACHE_FRESH_SECS)
                    });
                (guard.remote_cache_generation, cached, fresh)
            };
            if generation != captured_generation {
                continue 'resolve;
            }
            if let Some(cached) = cached {
                if cached_fresh {
                    parent = cached;
                    continue;
                }
                // 超过新鲜窗口：先向远端复核该映射是否仍然成立，
                // 防止把文件上传到已被其它客户端删除/改名的目录。
                let verified = find_remote_folder(token, device_id, &parent, part).await?;
                let mut guard = state.lock().map_err(|error| error.to_string())?;
                if guard.remote_cache_generation != generation {
                    continue 'resolve;
                }
                match verified {
                    Some(remote_id) if remote_id == cached => {
                        guard
                            .remote_cache_validated_at
                            .insert(cache_key.clone(), Instant::now());
                        drop(guard);
                        parent = cached;
                        continue;
                    }
                    _ => {
                        // 映射已失效：丢弃这一条并重新解析整条路径。
                        guard.remote_cache.remove(&cache_key);
                        guard.remote_cache_validated_at.remove(&cache_key);
                        guard.remote_cache_generation =
                            guard.remote_cache_generation.wrapping_add(1);
                        continue 'resolve;
                    }
                }
            }
            let result = api_post(
                token,
                device_id,
                "/userres/v1/file/create_dir",
                json!({ "parentId": parent, "dirName": part, "failIfNameExist": true }),
                &[159],
            )
            .await?;
            let created = result.code != 159;
            let mut file_id = result
                .data
                .as_ref()
                .and_then(|data| data.get("fileId"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if file_id.is_none() && result.code == 159 {
                file_id = find_remote_folder(token, device_id, &parent, part).await?;
            }
            let file_id = file_id.ok_or_else(|| format!("无法创建或定位远程目录：{prefix}"))?;
            if created {
                if let Some(task_id) = result.data.as_ref().and_then(operation_task_id) {
                    wait_operation_task(token, device_id, &task_id).await?;
                }
                webdav::invalidate_directory_cache(&parent);
                webdav::publish_directory_invalidation(
                    app,
                    [parent.clone()],
                    false,
                    "upload-create-directory",
                );
            }
            {
                let mut guard = state.lock().map_err(|error| error.to_string())?;
                if guard.remote_cache_generation != generation {
                    continue 'resolve;
                }
                if guard.cache_enabled {
                    guard.remote_cache.insert(cache_key.clone(), file_id.clone());
                    guard
                        .remote_cache_validated_at
                        .insert(cache_key, Instant::now());
                    let max_entries = guard.cache_max_entries;
                    trim_remote_cache(&mut guard.remote_cache, max_entries);
                }
            }
            parent = file_id;
        }
        return Ok(parent);
    }
    Err("远程目录持续发生变化，请稍后重试".into())
}


#[tauri::command]
pub(crate) fn get_cache_settings(state: tauri::State<'_, SharedState>) -> Result<CacheSettings, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    Ok(cache_settings(&guard))
}
#[tauri::command]
pub(crate) fn update_cache_settings(
    state: tauri::State<'_, SharedState>,
    enabled: Option<bool>,
    max_entries: Option<usize>,
) -> Result<CacheSettings, String> {
    let mut guard = state.lock().map_err(|error| error.to_string())?;
    let next = CacheSettings {
        enabled: enabled.unwrap_or(guard.cache_enabled),
        max_entries: max_entries
            .map(validate_cache_max_entries)
            .transpose()?
            .unwrap_or(guard.cache_max_entries),
    };
    let db_path = guard.db_path.clone();
    let RuntimeState {
        remote_cache,
        remote_cache_validated_at,
        remote_cache_generation,
        ..
    } = &mut *guard;
    apply_cache_policy(&db_path, remote_cache, remote_cache_generation, next)?;
    remote_cache_validated_at.retain(|key, _| remote_cache.contains_key(key));
    save_app_state(&db_path, "cache_enabled", &next.enabled.to_string())?;
    save_app_state(&db_path, "cache_max_entries", &next.max_entries.to_string())?;
    guard.cache_enabled = next.enabled;
    guard.cache_max_entries = next.max_entries;
    Ok(next)
}
#[tauri::command]
pub(crate) fn get_metadata_cache_stats(
    state: tauri::State<'_, SharedState>,
) -> Result<MetadataCacheStats, String> {
    let guard = state.lock().map_err(|error| error.to_string())?;
    metadata_cache_stats(&guard.db_path, &guard.remote_cache, cache_settings(&guard))
}
#[tauri::command]
pub(crate) fn clear_metadata_cache(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
) -> Result<MetadataCacheStats, String> {
    let stats = {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        let db_path = guard.db_path.clone();
        let policy = cache_settings(&guard);
        let RuntimeState {
            remote_cache,
            remote_cache_validated_at,
            remote_cache_generation,
            ..
        } = &mut *guard;
        remote_cache_validated_at.clear();
        clear_metadata_cache_storage(&db_path, remote_cache, remote_cache_generation, policy)?
    };
    // "清理缓存"必须同时打掉挂载端目录快照并通知前端，否则用户感知不到生效。
    webdav::invalidate_all_directory_cache();
    webdav::publish_directory_invalidation(&app, Vec::new(), true, "clear-metadata-cache");
    Ok(stats)
}
