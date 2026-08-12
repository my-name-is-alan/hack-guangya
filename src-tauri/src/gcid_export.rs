//! GCID 导出：云端清点、哈希采样与诊断日志。

use crate::prelude::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GcidExport {
    pub(crate) source: String,
    pub(crate) hash_type: String,
    #[serde(default)]
    pub(crate) uses_gcid_in_export: bool,
    #[serde(default)]
    pub(crate) uses_cid_in_export: bool,
    #[serde(default)]
    pub(crate) contains_cid: bool,
    #[serde(default)]
    pub(crate) common_path: String,
    #[serde(default)]
    pub(crate) total_files_count: Option<u64>,
    #[serde(default)]
    pub(crate) total_size: Option<Value>,
    pub(crate) files: Vec<GcidExportFile>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GcidExportFile {
    pub(crate) path: String,
    pub(crate) size: Value,
    pub(crate) gcid: String,
    pub(crate) cid: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CloudSelectionEntry {
    pub(crate) file_id: String,
    pub(crate) name: String,
    pub(crate) folder: bool,
    pub(crate) size: u64,
    pub(crate) gcid: String,
    pub(crate) modified_at: u64,
    pub(crate) subtree_size: Option<u64>,
    pub(crate) subtree_folders: Option<u64>,
    pub(crate) subtree_files: Option<u64>,
    pub(crate) ancestor_ids: Vec<String>,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GcidExportRootSignature {
    pub(crate) file_id: String,
    pub(crate) name: String,
    pub(crate) folder: bool,
    pub(crate) size: u64,
    pub(crate) gcid: String,
    pub(crate) modified_at: u64,
    pub(crate) subtree_size: Option<u64>,
    pub(crate) subtree_folders: Option<u64>,
    pub(crate) subtree_files: Option<u64>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeneratedGcidExport {
    pub(crate) script_version: String,
    pub(crate) export_version: String,
    pub(crate) source: String,
    pub(crate) hash_type: String,
    pub(crate) uses_gcid_in_export: bool,
    pub(crate) uses_cid_in_export: bool,
    pub(crate) uses_base62_etags_in_export: bool,
    pub(crate) common_path: String,
    pub(crate) source_folder_id: String,
    pub(crate) source_folder_name: String,
    pub(crate) total_files_count: usize,
    pub(crate) total_size: Value,
    pub(crate) formatted_total_size: String,
    pub(crate) generated_at: i64,
    pub(crate) scanned_folders_count: usize,
    pub(crate) skipped_files_count: usize,
    pub(crate) skipped_files: Vec<String>,
    pub(crate) files: Vec<GeneratedGcidExportFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GeneratedGcidExportFile {
    pub(crate) path: String,
    pub(crate) size: String,
    pub(crate) gcid: String,
    pub(crate) cid: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GeneratedGcidExportResult {
    pub(crate) cancelled: bool,
    pub(crate) saved_path: Option<String>,
    pub(crate) file_name: String,
    pub(crate) total_files: usize,
    pub(crate) skipped_files_count: usize,
    pub(crate) total_size: String,
}


pub(crate) fn cloud_record_value(value: &Value) -> &Value {
    [
        "fileInfo",
        "file_info",
        "resourceInfo",
        "resource_info",
        "file",
    ]
    .into_iter()
    .find_map(|key| value.get(key).filter(|entry| entry.is_object()))
    .unwrap_or(value)
}

pub(crate) fn cloud_record_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(|entry| match entry {
                Value::String(text) => Some(text.trim().to_string()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
        })
        .unwrap_or_default()
}

pub(crate) fn cloud_selection_entry_from_value(
    value: &Value,
    fallback_id: &str,
    fallback_name: &str,
) -> Result<CloudSelectionEntry, String> {
    let container = value;
    let value = cloud_record_value(container);
    let file_id = {
        let value = cloud_record_text(value, &["fileId", "file_id", "id"]);
        if value.is_empty() {
            fallback_id.to_string()
        } else {
            value
        }
    };
    let name = {
        let value = cloud_record_text(value, &["fileName", "file_name", "name"]);
        if value.is_empty() {
            fallback_name.to_string()
        } else {
            value
        }
    };
    if file_id.trim().is_empty() || name.trim().is_empty() {
        return Err("光鸭返回的文件详情缺少文件 ID 或名称".to_string());
    }
    let resource_type = value_as_u64(
        value
            .get("resType")
            .or_else(|| value.get("res_type"))
            .or_else(|| value.get("type")),
    )
    .unwrap_or(0);
    let folder = resource_type == 2
        || value.get("isFolder").and_then(Value::as_bool) == Some(true)
        || value.get("is_folder").and_then(Value::as_bool) == Some(true)
        || value.get("isDir").and_then(Value::as_bool) == Some(true);
    let size = if folder {
        0
    } else {
        value_as_u64(
            value
                .get("fileSize")
                .or_else(|| value.get("file_size"))
                .or_else(|| value.get("totalSize"))
                .or_else(|| value.get("total_size"))
                .or_else(|| value.get("size")),
        )
        .ok_or_else(|| format!("文件大小无效：{name}"))?
    };
    let gcid = cloud_record_text(value, &["gcid", "GCID", "gCid"]);
    let modified_at = value_as_u64(
        value
            .get("utime")
            .or_else(|| value.get("updatedAt"))
            .or_else(|| value.get("updateTime"))
            .or_else(|| value.get("modifiedAt"))
            .or_else(|| value.get("modifyTime")),
    )
    .unwrap_or(0);
    let size_info = container
        .get("sizeInfo")
        .or_else(|| container.get("size_info"));
    let subtree_size = size_info.and_then(|item| {
        value_as_u64(
            item.get("size")
                .or_else(|| item.get("totalSize"))
                .or_else(|| item.get("total_size")),
        )
    });
    let subtree_folders = size_info.and_then(|item| {
        value_as_u64(
            item.get("subDirCount")
                .or_else(|| item.get("sub_dir_count"))
                .or_else(|| item.get("folderCount")),
        )
    });
    let subtree_files = size_info.and_then(|item| {
        value_as_u64(
            item.get("subFileCount")
                .or_else(|| item.get("sub_file_count"))
                .or_else(|| item.get("fileCount")),
        )
    });
    let ancestor_ids = cloud_record_text(value, &["fullParentIds", "full_parent_ids"])
        .split('/')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    Ok(CloudSelectionEntry {
        file_id,
        name,
        folder,
        size,
        gcid,
        modified_at,
        subtree_size,
        subtree_folders,
        subtree_files,
        ancestor_ids,
        path: String::new(),
    })
}

pub(crate) fn safe_cloud_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .filter_map(|character| {
            if character.is_control() {
                None
            } else if matches!(character, '/' | '\\') {
                Some('_')
            } else {
                Some(character)
            }
        })
        .collect::<String>();
    let segment = segment.trim();
    if segment.is_empty() {
        "未命名文件".to_string()
    } else {
        segment.to_string()
    }
}

pub(crate) async fn cloud_selection_entry_detail(
    token: &str,
    device_id: &str,
    file_id: &str,
    fallback_name: &str,
) -> Result<CloudSelectionEntry, String> {
    let response = api_post(
        token,
        device_id,
        "/userres/v1/file/get_file_detail",
        json!({ "fileId": file_id }),
        &[],
    )
    .await?;
    cloud_selection_entry_from_value(
        response.data.as_ref().unwrap_or(&Value::Null),
        file_id,
        fallback_name,
    )
}

pub(crate) async fn cloud_selection_children(
    token: &str,
    device_id: &str,
    parent_id: &str,
    relative_path: &str,
    diagnostics: Option<&GcidExportDiagnostics>,
) -> Result<Vec<CloudSelectionEntry>, String> {
    let mut entries = Vec::new();
    for page in 0..1000_u64 {
        let page_started = Instant::now();
        let request_body = json!({
            "page": page,
            "pageSize": 100,
            "parentId": parent_id,
            "orderBy": 0,
            "sortType": 0,
            "needSubFolderStat": true
        });
        let response = match retry_gcid_export_scan(|attempt| {
            let request_body = request_body.clone();
            async move {
                let attempt_started = Instant::now();
                match api_post(
                    token,
                    device_id,
                    "/userres/v1/file/get_file_list",
                    request_body,
                    &[],
                )
                .await
                {
                    Ok(response) => Ok(response),
                    Err(error) => {
                        if let Some(diagnostics) = diagnostics {
                            let retryable = retryable_gcid_export_scan_error(&error);
                            diagnostics.write(
                                if retryable { "warn" } else { "error" },
                                "scan_folder_page_attempt_failed",
                                json!({
                                    "path": relative_path,
                                    "file_id_suffix": parent_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                    "page": page,
                                    "attempt": attempt + 1,
                                    "max_attempts": GCID_EXPORT_SCAN_ATTEMPTS,
                                    "retrying": retryable && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
                                    "elapsed_ms_request": attempt_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                                    "error": error.clone()
                                }),
                            );
                        }
                        Err(error)
                    }
                }
            }
        })
        .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Some(diagnostics) = diagnostics {
                    diagnostics.write(
                        "error",
                        "scan_folder_page_failed",
                        json!({
                            "path": relative_path,
                            "file_id_suffix": parent_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                            "page": page,
                            "elapsed_ms_page": page_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                            "error": error.clone()
                        }),
                    );
                }
                return Err(error);
            }
        };
        let data = response.data.unwrap_or_else(|| json!({}));
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let count = list.len();
        for item in list {
            entries.push(cloud_selection_entry_from_value(&item, "", "")?);
        }
        let total = value_as_u64(data.get("total")).unwrap_or(entries.len() as u64);
        if let Some(diagnostics) = diagnostics {
            diagnostics.write(
                "info",
                "scan_folder_page_succeeded",
                json!({
                    "path": relative_path,
                    "file_id_suffix": parent_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                    "page": page,
                    "elapsed_ms_page": page_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                    "page_entries": count,
                    "collected_entries": entries.len(),
                    "reported_total": total
                }),
            );
        }
        if count == 0 || entries.len() as u64 >= total {
            break;
        }
    }
    Ok(entries)
}

pub(crate) async fn load_gcid_export_roots(
    token: &str,
    device_id: &str,
    file_ids: &[String],
    fallback_names: &[String],
    diagnostics: &GcidExportDiagnostics,
) -> Result<Vec<CloudSelectionEntry>, String> {
    let mut roots = stream::iter(file_ids.iter().cloned().enumerate())
        .map(|(index, file_id)| {
            let token = token.to_string();
            let device_id = device_id.to_string();
            let fallback_name = fallback_names.get(index).cloned().unwrap_or_default();
            let diagnostics = diagnostics.clone();
            async move {
                let started = Instant::now();
                diagnostics.write(
                    "info",
                    "scan_root_detail_started",
                    json!({
                        "root_index": index,
                        "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                        "path": fallback_name.clone()
                    }),
                );
                let entry = retry_gcid_export_scan(|attempt| {
                    let token = token.as_str();
                    let device_id = device_id.as_str();
                    let file_id = file_id.as_str();
                    let fallback_name = fallback_name.as_str();
                    let diagnostics = diagnostics.clone();
                    async move {
                        match cloud_selection_entry_detail(token, device_id, file_id, fallback_name)
                            .await
                        {
                            Ok(entry) => Ok(entry),
                            Err(error) => {
                                let retryable = retryable_gcid_export_scan_error(&error);
                                diagnostics.write(
                                    if retryable { "warn" } else { "error" },
                                    "scan_root_detail_attempt_failed",
                                    json!({
                                        "root_index": index,
                                        "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                        "path": fallback_name,
                                        "attempt": attempt + 1,
                                        "max_attempts": GCID_EXPORT_SCAN_ATTEMPTS,
                                        "retrying": retryable && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
                                        "error": error.clone()
                                    }),
                                );
                                Err(error)
                            }
                        }
                    }
                })
                .await?;
                diagnostics.write(
                    "info",
                    "scan_root_detail_succeeded",
                    json!({
                        "root_index": index,
                        "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                        "path": entry.name.clone(),
                        "is_folder": entry.folder,
                        "elapsed_ms_detail": started.elapsed().as_millis().min(u64::MAX as u128) as u64
                    }),
                );
                Ok::<_, String>((index, entry))
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
    roots.sort_by_key(|(index, _)| *index);
    Ok(roots.into_iter().map(|(_, entry)| entry).collect())
}

pub(crate) async fn collect_cloud_selection_entries(
    token: &str,
    device_id: &str,
    file_ids: &[String],
    fallback_names: &[String],
    include_folders: bool,
    diagnostics: Option<&GcidExportDiagnostics>,
    preloaded_roots: Option<Vec<CloudSelectionEntry>>,
) -> Result<(Vec<CloudSelectionEntry>, Vec<CloudSelectionEntry>, usize), String> {
    let roots = if let Some(roots) = preloaded_roots {
        roots
    } else {
        let mut detailed = stream::iter(file_ids.iter().cloned().enumerate())
        .map(|(index, file_id)| {
            let token = token.to_string();
            let device_id = device_id.to_string();
            let fallback_name = fallback_names.get(index).cloned().unwrap_or_default();
            let diagnostics = diagnostics.cloned();
            async move {
                let detail_started = Instant::now();
                let fields = json!({
                    "root_index": index,
                    "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                    "path": fallback_name.clone()
                });
                if let Some(diagnostics) = &diagnostics {
                    diagnostics.write("info", "scan_root_detail_started", fields.clone());
                }
                let result = retry_gcid_export_scan(|attempt| {
                    let diagnostics = diagnostics.clone();
                    let token = token.as_str();
                    let device_id = device_id.as_str();
                    let file_id = file_id.as_str();
                    let fallback_name = fallback_name.as_str();
                    async move {
                        let attempt_started = Instant::now();
                        match cloud_selection_entry_detail(token, device_id, file_id, fallback_name).await {
                            Ok(entry) => Ok(entry),
                            Err(error) => {
                                if let Some(diagnostics) = &diagnostics {
                                    let retryable = retryable_gcid_export_scan_error(&error);
                                    diagnostics.write(
                                        if retryable { "warn" } else { "error" },
                                        "scan_root_detail_attempt_failed",
                                        json!({
                                            "root_index": index,
                                            "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                            "path": fallback_name,
                                            "attempt": attempt + 1,
                                            "max_attempts": GCID_EXPORT_SCAN_ATTEMPTS,
                                            "retrying": retryable && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
                                            "elapsed_ms_request": attempt_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                                            "error": error.clone()
                                        }),
                                    );
                                }
                                Err(error)
                            }
                        }
                    }
                })
                .await;
                match result {
                    Ok(entry) => {
                        if let Some(diagnostics) = &diagnostics {
                            diagnostics.write(
                                "info",
                                "scan_root_detail_succeeded",
                                json!({
                                    "root_index": index,
                                    "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                    "path": entry.name.clone(),
                                    "is_folder": entry.folder,
                                    "elapsed_ms_detail": detail_started.elapsed().as_millis().min(u64::MAX as u128) as u64
                                }),
                            );
                        }
                        Ok((index, entry))
                    }
                    Err(error) => {
                        if let Some(diagnostics) = &diagnostics {
                            diagnostics.write(
                                "error",
                                "scan_root_detail_failed",
                                json!({
                                    "root_index": index,
                                    "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                    "path": fallback_name.clone(),
                                    "elapsed_ms_detail": detail_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                                    "error": error.clone()
                                }),
                            );
                        }
                        Err(error)
                    }
                }
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await?;
        detailed.sort_by_key(|(index, _)| *index);
        detailed
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
    };
    let mut queue = roots
        .iter()
        .cloned()
        .map(|entry| {
            let path = safe_cloud_path_segment(&entry.name);
            (entry, path)
        })
        .collect::<VecDeque<_>>();
    let mut visited = HashSet::new();
    let mut entries = Vec::new();
    let mut scanned_folders = 0_usize;
    while !queue.is_empty() {
        let mut folder_batch = Vec::new();
        while folder_batch.len() < GCID_EXPORT_SCAN_CONCURRENCY {
            let Some((mut entry, relative_path)) = queue.pop_front() else {
                break;
            };
            if !visited.insert(entry.file_id.clone()) {
                continue;
            }
            entry.path = relative_path.clone();
            if entry.folder {
                let batch_index = folder_batch.len();
                folder_batch.push((batch_index, entry, relative_path));
            } else {
                entries.push(entry);
            }
        }

        let mut loaded = stream::iter(folder_batch.into_iter())
            .map(|(batch_index, entry, relative_path)| async move {
                let children = cloud_selection_children(
                    token,
                    device_id,
                    &entry.file_id,
                    &relative_path,
                    diagnostics,
                )
                .await?;
                Ok::<_, String>((batch_index, entry, relative_path, children))
            })
            .buffer_unordered(GCID_EXPORT_SCAN_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;
        loaded.sort_by_key(|(batch_index, _, _, _)| *batch_index);
        for (_, entry, relative_path, children) in loaded {
            scanned_folders += 1;
            if include_folders {
                entries.push(entry);
            }
            for child in children {
                let child_path =
                    format!("{relative_path}/{}", safe_cloud_path_segment(&child.name));
                queue.push_back((child, child_path));
            }
        }
        if visited.len().saturating_add(queue.len()) > 100_000 {
            return Err("一次最多处理 100000 个云端文件或文件夹".to_string());
        }
    }
    Ok((entries, roots, scanned_folders))
}

pub(crate) fn should_use_gcid_export_inventory(roots: &[CloudSelectionEntry]) -> bool {
    roots.iter().filter(|entry| entry.folder).any(|entry| {
        entry
            .subtree_folders
            .is_none_or(|count| count.saturating_add(1) > GCID_EXPORT_INVENTORY_THRESHOLD)
    })
}

pub(crate) async fn cloud_gcid_export_inventory_page(
    token: &str,
    device_id: &str,
    resource_type: u64,
    page: u64,
    diagnostics: &GcidExportDiagnostics,
) -> Result<(Vec<Value>, u64), String> {
    let started = Instant::now();
    let response = retry_gcid_export_scan(|attempt| async move {
        let attempt_started = Instant::now();
        match api_post(
            token,
            device_id,
            "/userres/v1/file/get_file_list",
            json!({
                "parentId": "*",
                "page": page,
                "pageSize": GCID_EXPORT_INVENTORY_PAGE_SIZE,
                "orderBy": 0,
                "sortType": 0,
                "resType": resource_type
            }),
            &[],
        )
        .await
        {
            Ok(response) => Ok(response),
            Err(error) => {
                let retryable = retryable_gcid_export_scan_error(&error);
                diagnostics.write(
                    if retryable { "warn" } else { "error" },
                    "scan_inventory_page_attempt_failed",
                    json!({
                        "inventory_type": if resource_type == 1 { "file" } else { "folder" },
                        "page": page,
                        "attempt": attempt + 1,
                        "max_attempts": GCID_EXPORT_SCAN_ATTEMPTS,
                        "retrying": retryable && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
                        "elapsed_ms_request": attempt_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                        "error": error.clone()
                    }),
                );
                Err(error)
            }
        }
    })
    .await?;
    let data = response.data.unwrap_or_else(|| json!({}));
    let list = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = value_as_u64(data.get("total")).unwrap_or(list.len() as u64);
    diagnostics.write(
        "info",
        "scan_inventory_page_succeeded",
        json!({
            "inventory_type": if resource_type == 1 { "file" } else { "folder" },
            "page": page,
            "page_entries": list.len(),
            "reported_total": total,
            "elapsed_ms_page": started.elapsed().as_millis().min(u64::MAX as u128) as u64
        }),
    );
    Ok((list, total))
}

pub(crate) fn gcid_export_inventory_path(
    entry: &CloudSelectionEntry,
    roots: &[CloudSelectionEntry],
    folder_names: &HashMap<String, String>,
) -> Option<String> {
    if let Some(root) = roots
        .iter()
        .find(|root| !root.folder && root.file_id == entry.file_id)
    {
        return Some(safe_cloud_path_segment(&root.name));
    }
    for root in roots.iter().filter(|root| root.folder) {
        let Some(root_index) = entry
            .ancestor_ids
            .iter()
            .position(|ancestor| ancestor == &root.file_id)
        else {
            continue;
        };
        let mut parts = vec![safe_cloud_path_segment(&root.name)];
        for ancestor in entry.ancestor_ids.iter().skip(root_index + 1) {
            let name = folder_names.get(ancestor)?;
            parts.push(safe_cloud_path_segment(name));
        }
        parts.push(safe_cloud_path_segment(&entry.name));
        return Some(parts.join("/"));
    }
    None
}

pub(crate) async fn collect_gcid_export_entries(
    app: &tauri::AppHandle,
    token: &str,
    device_id: &str,
    file_ids: &[String],
    fallback_names: &[String],
    diagnostics: &GcidExportDiagnostics,
    roots: Vec<CloudSelectionEntry>,
) -> Result<(Vec<CloudSelectionEntry>, Vec<CloudSelectionEntry>, usize), String> {
    if !should_use_gcid_export_inventory(&roots) {
        diagnostics.write(
            "info",
            "scan_strategy_selected",
            json!({ "strategy": "directory" }),
        );
        return collect_cloud_selection_entries(
            token,
            device_id,
            file_ids,
            fallback_names,
            false,
            Some(diagnostics),
            Some(roots),
        )
        .await;
    }

    diagnostics.write(
        "info",
        "scan_strategy_selected",
        json!({
            "strategy": "global_inventory",
            "page_size": GCID_EXPORT_INVENTORY_PAGE_SIZE,
            "concurrency": GCID_EXPORT_SCAN_CONCURRENCY
        }),
    );
    let ((mut file_records, file_total), (mut folder_records, folder_total)) = tokio::try_join!(
        cloud_gcid_export_inventory_page(token, device_id, 1, 0, diagnostics),
        cloud_gcid_export_inventory_page(token, device_id, 2, 0, diagnostics)
    )?;
    let file_pages = file_total.div_ceil(GCID_EXPORT_INVENTORY_PAGE_SIZE).max(1);
    let folder_pages = folder_total
        .div_ceil(GCID_EXPORT_INVENTORY_PAGE_SIZE)
        .max(1);
    let total_pages =
        usize::try_from(file_pages.saturating_add(folder_pages)).unwrap_or(usize::MAX);
    let completed_pages = Arc::new(AtomicUsize::new(2));
    let scanned_entries = Arc::new(AtomicUsize::new(
        file_records.len().saturating_add(folder_records.len()),
    ));
    emit_gcid_export_scan_progress(
        app,
        "正在加载云端文件索引",
        completed_pages.load(Ordering::Relaxed),
        total_pages,
        scanned_entries.load(Ordering::Relaxed),
    );
    let page_jobs = (1..file_pages)
        .map(|page| (1_u64, page))
        .chain((1..folder_pages).map(|page| (2_u64, page)))
        .collect::<Vec<_>>();
    let mut pages = stream::iter(page_jobs)
        .map(|(resource_type, page)| {
            let app = app.clone();
            let completed_pages = completed_pages.clone();
            let scanned_entries = scanned_entries.clone();
            async move {
                let (records, total) = cloud_gcid_export_inventory_page(
                    token,
                    device_id,
                    resource_type,
                    page,
                    diagnostics,
                )
                .await?;
                let scanned =
                    scanned_entries.fetch_add(records.len(), Ordering::Relaxed) + records.len();
                let completed = completed_pages.fetch_add(1, Ordering::Relaxed) + 1;
                emit_gcid_export_scan_progress(
                    &app,
                    "正在加载云端文件索引",
                    completed,
                    total_pages,
                    scanned,
                );
                Ok::<_, String>((resource_type, page, records, total))
            }
        })
        .buffer_unordered(GCID_EXPORT_SCAN_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    pages.sort_by_key(|(resource_type, page, _, _)| (*resource_type, *page));
    for (resource_type, _, mut records, _) in pages {
        if resource_type == 1 {
            file_records.append(&mut records);
        } else {
            folder_records.append(&mut records);
        }
    }
    if file_records.len() < usize::try_from(file_total).unwrap_or(usize::MAX)
        || folder_records.len() < usize::try_from(folder_total).unwrap_or(usize::MAX)
    {
        return Err("光鸭全库文件索引返回不完整，请稍后重试".to_string());
    }
    if file_records.len().saturating_add(folder_records.len()) > 100_000 {
        return Err("一次最多处理 100000 个云端文件或文件夹".to_string());
    }

    let mut folders = Vec::with_capacity(folder_records.len());
    let mut folder_names = HashMap::with_capacity(folder_records.len().saturating_add(roots.len()));
    for record in folder_records {
        let entry = cloud_selection_entry_from_value(&record, "", "")?;
        folder_names.insert(entry.file_id.clone(), entry.name.clone());
        folders.push(entry);
    }
    for root in roots.iter().filter(|entry| entry.folder) {
        folder_names.insert(root.file_id.clone(), root.name.clone());
    }
    let scanned_folders = folders
        .iter()
        .filter(|entry| {
            roots.iter().filter(|root| root.folder).any(|root| {
                entry.file_id == root.file_id || entry.ancestor_ids.contains(&root.file_id)
            })
        })
        .map(|entry| entry.file_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let mut entries = Vec::new();
    let mut seen_files = HashSet::new();
    for record in file_records {
        let mut entry = cloud_selection_entry_from_value(&record, "", "")?;
        let Some(path) = gcid_export_inventory_path(&entry, &roots, &folder_names) else {
            continue;
        };
        if seen_files.insert(entry.file_id.clone()) {
            entry.path = path;
            entries.push(entry);
        }
    }
    for root in roots.iter().filter(|entry| !entry.folder) {
        if seen_files.insert(root.file_id.clone()) {
            let mut entry = root.clone();
            entry.path = safe_cloud_path_segment(&entry.name);
            entries.push(entry);
        }
    }
    if roots.len() == 1 && roots[0].folder {
        if let Some(expected) = roots[0].subtree_files {
            if entries.len() as u64 != expected {
                return Err(format!(
                    "光鸭全库索引与目录统计不一致（索引 {} / 目录 {expected}），请稍后重试",
                    entries.len()
                ));
            }
        }
    }
    diagnostics.write(
        "info",
        "scan_inventory_filtered",
        json!({
            "account_files": file_total,
            "account_folders": folder_total,
            "selected_files": entries.len(),
            "selected_folders": scanned_folders
        }),
    );
    Ok((entries, roots, scanned_folders))
}


pub(crate) fn format_export_bytes(bytes: u128) -> String {
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut unit = 0_usize;
    let mut divisor = 1_u128;
    while unit < units.len() - 1 && bytes >= divisor.saturating_mul(1024) {
        divisor = divisor.saturating_mul(1024);
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let hundredths = bytes.saturating_mul(100).saturating_add(divisor / 2) / divisor;
    format!(
        "{}.{:02} {}",
        hundredths / 100,
        hundredths % 100,
        units[unit]
    )
}

pub(crate) fn export_json_file_name(names: &[String]) -> String {
    let source = if names.is_empty() {
        "光鸭秒传".to_string()
    } else if names.len() == 1 {
        safe_cloud_path_segment(&names[0])
    } else {
        format!("光鸭秒传_{}项", names.len())
    };
    let stem = Path::new(&source)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("光鸭秒传")
        .chars()
        .filter_map(|character| {
            if character.is_control() {
                None
            } else if "\\/:*?\"<>|".contains(character) {
                Some('_')
            } else {
                Some(character)
            }
        })
        .take(120)
        .collect::<String>();
    format!(
        "{}_秒传.json",
        if stem.is_empty() {
            "光鸭秒传"
        } else {
            &stem
        }
    )
}

#[derive(Debug)]
pub(crate) struct CachedGcidExportSnapshot {
    pub(crate) root_signatures: Vec<GcidExportRootSignature>,
    pub(crate) export: GeneratedGcidExport,
    pub(crate) created_at: i64,
}

pub(crate) fn gcid_export_root_signatures(roots: &[CloudSelectionEntry]) -> Vec<GcidExportRootSignature> {
    roots
        .iter()
        .map(|entry| GcidExportRootSignature {
            file_id: entry.file_id.clone(),
            name: entry.name.clone(),
            folder: entry.folder,
            size: entry.size,
            gcid: entry.gcid.to_ascii_lowercase(),
            modified_at: entry.modified_at,
            subtree_size: entry.subtree_size,
            subtree_folders: entry.subtree_folders,
            subtree_files: entry.subtree_files,
        })
        .collect()
}

pub(crate) fn gcid_export_selection_key(file_ids: &[String]) -> String {
    hex::encode(Sha256::digest(file_ids.join("\0").as_bytes()))
}

pub(crate) fn load_gcid_export_snapshot(
    database: &Path,
    account_scope: &str,
    selection_key: &str,
) -> Result<Option<CachedGcidExportSnapshot>, String> {
    let row = open_database(database)?
        .query_row(
            "SELECT root_signatures_json, export_json, created_at
             FROM gcid_export_snapshots
             WHERE account_scope = ?1 AND selection_key = ?2",
            params![account_scope, selection_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("读取秒传 JSON 快照缓存失败：{error}"))?;
    let Some((root_signatures_json, export_json, created_at)) = row else {
        return Ok(None);
    };
    let root_signatures = serde_json::from_str(&root_signatures_json)
        .map_err(|error| format!("解析秒传 JSON 根目录缓存失败：{error}"))?;
    let export = serde_json::from_str(&export_json)
        .map_err(|error| format!("解析秒传 JSON 快照缓存失败：{error}"))?;
    Ok(Some(CachedGcidExportSnapshot {
        root_signatures,
        export,
        created_at,
    }))
}

pub(crate) fn save_gcid_export_snapshot(
    database: &Path,
    account_scope: &str,
    selection_key: &str,
    root_signatures: &[GcidExportRootSignature],
    export: &GeneratedGcidExport,
) -> Result<(), String> {
    let root_signatures_json = serde_json::to_string(root_signatures)
        .map_err(|error| format!("序列化秒传 JSON 根目录缓存失败：{error}"))?;
    let export_json = serde_json::to_string(export)
        .map_err(|error| format!("序列化秒传 JSON 快照缓存失败：{error}"))?;
    let now = unix_timestamp();
    open_database(database)?
        .execute(
            "INSERT INTO gcid_export_snapshots
               (account_scope, selection_key, root_signatures_json, export_json, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(account_scope, selection_key) DO UPDATE SET
               root_signatures_json = excluded.root_signatures_json,
               export_json = excluded.export_json,
               created_at = excluded.created_at,
               last_used_at = excluded.last_used_at",
            params![
                account_scope,
                selection_key,
                root_signatures_json,
                export_json,
                now
            ],
        )
        .map_err(|error| format!("保存秒传 JSON 快照缓存失败：{error}"))?;
    Ok(())
}

pub(crate) fn touch_gcid_export_snapshot(
    database: &Path,
    account_scope: &str,
    selection_key: &str,
) -> Result<(), String> {
    open_database(database)?
        .execute(
            "UPDATE gcid_export_snapshots SET last_used_at = ?1
             WHERE account_scope = ?2 AND selection_key = ?3",
            params![unix_timestamp(), account_scope, selection_key],
        )
        .map_err(|error| format!("更新秒传 JSON 快照缓存失败：{error}"))?;
    Ok(())
}

pub(crate) fn load_gcid_export_file_hash(
    database: &Path,
    account_scope: &str,
    file_id: &str,
    file_size: u64,
    gcid: &str,
) -> Result<Option<String>, String> {
    if !valid_sha1_hex(gcid) {
        return Ok(None);
    }
    let normalized_gcid = gcid.to_ascii_lowercase();
    let connection = open_database(database)?;
    let cloud_cid = connection
        .query_row(
            "SELECT cid FROM gcid_export_file_hashes
             WHERE account_scope = ?1 AND file_id = ?2 AND file_size = ?3 AND gcid = ?4",
            params![
                account_scope,
                file_id,
                file_size.to_string(),
                normalized_gcid
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取云端秒传指纹缓存失败：{error}"))?;
    let cid = if let Some(cid) = cloud_cid.filter(|value| valid_sha1_hex(value)) {
        Some((cid, true))
    } else if let Ok(file_size) = i64::try_from(file_size) {
        connection
            .query_row(
                "SELECT cid FROM file_fingerprints
                 WHERE size = ?1 AND LOWER(gcid) = ?2 AND LENGTH(cid) = 40
                 ORDER BY computed_at DESC LIMIT 1",
                params![file_size, normalized_gcid],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("读取已有秒传指纹缓存失败：{error}"))?
            .filter(|value| valid_sha1_hex(value))
            .map(|cid| (cid, false))
    } else {
        None
    };
    let Some((cid, cloud_hit)) = cid else {
        return Ok(None);
    };
    if cloud_hit {
        connection
            .execute(
                "UPDATE gcid_export_file_hashes SET last_used_at = ?1
                 WHERE account_scope = ?2 AND file_id = ?3 AND file_size = ?4 AND gcid = ?5",
                params![
                    unix_timestamp(),
                    account_scope,
                    file_id,
                    file_size.to_string(),
                    normalized_gcid
                ],
            )
            .map_err(|error| format!("更新云端秒传指纹缓存失败：{error}"))?;
    }
    Ok(Some(cid))
}

pub(crate) fn save_gcid_export_file_hash(
    database: &Path,
    account_scope: &str,
    file_id: &str,
    file_size: u64,
    gcid: &str,
    cid: &str,
) -> Result<(), String> {
    if !valid_sha1_hex(gcid) || !valid_sha1_hex(cid) {
        return Ok(());
    }
    open_database(database)?
        .execute(
            "INSERT INTO gcid_export_file_hashes
               (account_scope, file_id, file_size, gcid, cid, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_scope, file_id, file_size, gcid) DO UPDATE SET
               cid = excluded.cid,
               last_used_at = excluded.last_used_at",
            params![
                account_scope,
                file_id,
                file_size.to_string(),
                gcid.to_ascii_lowercase(),
                cid.to_ascii_uppercase(),
                unix_timestamp()
            ],
        )
        .map_err(|error| format!("保存云端秒传指纹缓存失败：{error}"))?;
    Ok(())
}

pub(crate) async fn cloud_download_url(token: &str, device_id: &str, file_id: &str) -> Result<String, String> {
    let response = api_post(
        token,
        device_id,
        "/userres/v1/get_res_download_url",
        json!({ "fileId": file_id }),
        &[],
    )
    .await?;
    let data = response.data.unwrap_or_else(|| json!({}));
    [
        "signedURL",
        "signedUrl",
        "downloadUrl",
        "downloadURL",
        "url",
    ]
    .into_iter()
    .find_map(|key| {
        data.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
    .ok_or_else(|| "光鸭没有返回文件下载地址".to_string())
}

pub(crate) fn gcid_export_diagnostic_path(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("logs")
        .join("gcid-export-latest.jsonl")
}

pub(crate) fn sanitize_gcid_diagnostic_text(value: &str) -> String {
    static URL_PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    static SECRET_PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let url_pattern = URL_PATTERN.get_or_init(|| {
        regex::Regex::new(r#"https?://[^\s\"'<>]+"#).expect("valid diagnostic URL regex")
    });
    let secret_pattern = SECRET_PATTERN.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)\b(authorization|cookie|access[_-]?token|refresh[_-]?token|client[_-]?secret|signature|security[_-]?token|x-oss-security-token)\b\s*[:=]\s*([^\s,;]+)"#,
        )
        .expect("valid diagnostic secret regex")
    });
    let urls_redacted = url_pattern.replace_all(value, |captures: &regex::Captures<'_>| {
        let raw = captures
            .get(0)
            .map(|value| value.as_str())
            .unwrap_or_default();
        raw.split_once('?')
            .map(|(base, _)| format!("{base}?<redacted>"))
            .unwrap_or_else(|| raw.to_string())
    });
    secret_pattern
        .replace_all(&urls_redacted, "$1=<redacted>")
        .chars()
        .take(2_000)
        .collect()
}

pub(crate) fn sanitize_gcid_diagnostic_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = sanitize_gcid_diagnostic_text(text),
        Value::Array(values) => {
            for item in values {
                sanitize_gcid_diagnostic_value(item);
            }
        }
        Value::Object(values) => {
            for item in values.values_mut() {
                sanitize_gcid_diagnostic_value(item);
            }
        }
        _ => {}
    }
}

#[derive(Clone)]
pub(crate) struct GcidExportDiagnostics {
    pub(crate) run_id: String,
    pub(crate) started_at: Instant,
    pub(crate) writer: Arc<Mutex<fs::File>>,
    pub(crate) written_bytes: Arc<AtomicU64>,
    pub(crate) info_suppressed: Arc<AtomicUsize>,
    pub(crate) details_suppressed: Arc<AtomicUsize>,
}

impl GcidExportDiagnostics {
    pub(crate) fn new(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建秒传 JSON 诊断日志目录失败：{error}"))?;
        }
        let writer = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|error| format!("初始化秒传 JSON 诊断日志失败：{error}"))?;
        Ok(Self {
            run_id: Uuid::new_v4().to_string(),
            started_at: Instant::now(),
            writer: Arc::new(Mutex::new(writer)),
            written_bytes: Arc::new(AtomicU64::new(0)),
            info_suppressed: Arc::new(AtomicUsize::new(0)),
            details_suppressed: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub(crate) fn write(&self, level: &str, event: &str, fields: Value) {
        if !matches!(
            event,
            "run_completed" | "run_failed" | "detail_log_limit_reached"
        ) && self.written_bytes.load(Ordering::Relaxed)
            >= GCID_EXPORT_DIAGNOSTIC_DETAIL_LIMIT_BYTES
        {
            if self.details_suppressed.fetch_add(1, Ordering::Relaxed) == 0 {
                self.write(
                    "warn",
                    "detail_log_limit_reached",
                    json!({
                        "limit_bytes": GCID_EXPORT_DIAGNOSTIC_DETAIL_LIMIT_BYTES,
                        "message": "诊断明细已达到大小上限，最终任务结果仍会写入"
                    }),
                );
            }
            return;
        }
        if level == "info"
            && self.written_bytes.load(Ordering::Relaxed) >= GCID_EXPORT_DIAGNOSTIC_INFO_LIMIT_BYTES
        {
            if self.info_suppressed.fetch_add(1, Ordering::Relaxed) == 0 {
                self.write(
                    "warn",
                    "info_log_limit_reached",
                    json!({
                        "limit_bytes": GCID_EXPORT_DIAGNOSTIC_INFO_LIMIT_BYTES,
                        "message": "普通成功明细已停止记录，后续警告和错误仍会继续写入"
                    }),
                );
            }
            return;
        }
        let mut record = fields.as_object().cloned().unwrap_or_default();
        record.insert(
            "timestamp_ms".to_string(),
            Value::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
                    .unwrap_or(0),
            ),
        );
        record.insert(
            "elapsed_ms".to_string(),
            Value::from(self.started_at.elapsed().as_millis().min(u64::MAX as u128) as u64),
        );
        record.insert("run_id".to_string(), Value::String(self.run_id.clone()));
        record.insert("runtime".to_string(), Value::String("tauri".to_string()));
        record.insert("level".to_string(), Value::String(level.to_string()));
        record.insert("event".to_string(), Value::String(event.to_string()));
        let mut value = Value::Object(record);
        sanitize_gcid_diagnostic_value(&mut value);
        let Ok(mut file) = self.writer.lock() else {
            return;
        };
        if let Ok(mut bytes) = serde_json::to_vec(&value) {
            bytes.push(b'\n');
            if file.write_all(&bytes).is_ok() {
                self.written_bytes
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
        }
    }
}

pub(crate) fn emit_gcid_export_progress(
    app: &tauri::AppHandle,
    stage: &str,
    current_path: &str,
    completed_files: usize,
    total_files: usize,
    read_bytes: u64,
    planned_sample_bytes: u64,
    source_total_bytes: u128,
) {
    emit(
        app,
        json!({
            "type": "gcid-export-progress",
            "phase": "hash",
            "stage": stage,
            "current_path": current_path,
            "completed_files": completed_files,
            "total_files": total_files,
            "sampled_bytes": read_bytes.to_string(),
            "planned_sample_bytes": planned_sample_bytes.to_string(),
            "source_total_bytes": source_total_bytes.to_string(),
            "downloaded_bytes": read_bytes.to_string(),
            "total_bytes": planned_sample_bytes.to_string(),
            "percent": (completed_files as u64).saturating_mul(100) / total_files.max(1) as u64
        }),
    );
}

pub(crate) fn emit_gcid_export_scan_progress(
    app: &tauri::AppHandle,
    stage: &str,
    completed_pages: usize,
    total_pages: usize,
    scanned_entries: usize,
) {
    emit(
        app,
        json!({
            "type": "gcid-export-progress",
            "phase": "scan",
            "stage": stage,
            "current_path": format!("已读取 {scanned_entries} 条云端索引"),
            "completed_files": 0,
            "total_files": 0,
            "scanned_pages": completed_pages,
            "total_pages": total_pages,
            "scanned_entries": scanned_entries,
            "percent": (completed_pages as u64).saturating_mul(100) / total_pages.max(1) as u64
        }),
    );
}

#[derive(Debug)]
pub(crate) struct GcidExportRangeError {
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

impl GcidExportRangeError {
    pub(crate) fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub(crate) fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}
pub(crate) fn retryable_gcid_export_range_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403 | 408 | 425 | 429) || status.is_server_error()
}

pub(crate) fn retryable_gcid_export_scan_error(message: &str) -> bool {
    if message.contains("登录态已失效") {
        return false;
    }
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("无法连接光鸭接口")
        || normalized.contains("网络异常")
        || normalized.contains("请求超时")
        || normalized.contains("error sending request")
        || normalized.contains("connection reset")
        || normalized.contains("connection closed")
    {
        return true;
    }
    static RETRYABLE_HTTP: OnceLock<regex::Regex> = OnceLock::new();
    RETRYABLE_HTTP
        .get_or_init(|| {
            regex::Regex::new(r"(?i)\bHTTP\s*(408|425|429|5[0-9]{2})\b")
                .expect("valid retryable scan HTTP regex")
        })
        .is_match(message)
}

pub(crate) async fn retry_gcid_export_scan<T, F, Fut>(mut operation: F) -> Result<T, String>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let mut last_error = "云端目录扫描失败".to_string();
    for attempt in 0..GCID_EXPORT_SCAN_ATTEMPTS {
        match operation(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
        if !retryable_gcid_export_scan_error(&last_error)
            || attempt + 1 >= GCID_EXPORT_SCAN_ATTEMPTS
        {
            break;
        }
        sleep(Duration::from_millis(
            400_u64.saturating_mul(1_u64 << attempt),
        ))
        .await;
    }
    Err(last_error)
}

pub(crate) async fn retry_gcid_export_range<T, F, Fut>(
    mut operation: F,
    attempts: usize,
    base_delay_ms: u64,
) -> Result<T, String>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, GcidExportRangeError>>,
{
    let attempts = attempts.max(1);
    let mut last_error = GcidExportRangeError::retryable("云端分段读取失败");
    for attempt in 0..attempts {
        match operation(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
        if !last_error.retryable || attempt + 1 >= attempts {
            break;
        }
        sleep(Duration::from_millis(
            base_delay_ms.saturating_mul(1_u64 << attempt),
        ))
        .await;
    }
    Err(last_error.message)
}

pub(crate) async fn read_bounded_gcid_range_stream<S, B, E>(
    stream: S,
    expected: usize,
    path: &str,
    idle_timeout: Duration,
) -> Result<Vec<u8>, GcidExportRangeError>
where
    S: futures_util::Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
{
    futures_util::pin_mut!(stream);
    let mut bytes = Vec::with_capacity(expected);
    loop {
        let next = tokio::time::timeout(idle_timeout, stream.next())
            .await
            .map_err(|_| {
                GcidExportRangeError::retryable(format!(
                    "分段读取 {path} 连续 {}ms 无数据",
                    idle_timeout.as_millis()
                ))
            })?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|error| {
            GcidExportRangeError::retryable(format!(
                "分段读取 {path} 失败：{}",
                sanitize_gcid_diagnostic_text(&error.to_string())
            ))
        })?;
        let chunk = chunk.as_ref();
        if chunk.len() > expected.saturating_sub(bytes.len()) {
            return Err(GcidExportRangeError::retryable(format!(
                "分段读取 {path} 返回的字节数超出请求范围"
            )));
        }
        bytes.extend_from_slice(chunk);
    }
    if bytes.len() != expected {
        return Err(GcidExportRangeError::retryable(format!(
            "分段读取 {path} 的字节数不完整"
        )));
    }
    Ok(bytes)
}

pub(crate) async fn read_cloud_cid_range_once(
    client: &reqwest::Client,
    range_limiter: &Semaphore,
    download_url: &str,
    path: &str,
    file_size: u64,
    index: usize,
    range: (u64, u64),
) -> Result<(usize, Vec<u8>), GcidExportRangeError> {
    let (start, end) = range;
    if start == end {
        return Ok((index, Vec::new()));
    }
    let _range_permit = range_limiter
        .acquire()
        .await
        .map_err(|_| GcidExportRangeError::retryable("Range 并发控制已关闭"))?;
    let expected = end.saturating_sub(start);
    let response = tokio::time::timeout(
        Duration::from_secs(GCID_EXPORT_REQUEST_TIMEOUT_SECS),
        client
            .get(download_url)
            .header("accept-encoding", "identity")
            .header(RANGE, format!("bytes={start}-{}", end - 1))
            .send(),
    )
    .await
    .map_err(|_| GcidExportRangeError::retryable(format!("分段读取 {path} 请求超时")))?
    .map_err(|error| {
        GcidExportRangeError::retryable(format!("分段读取 {path} 失败：{}", error.without_url()))
    })?;
    let status = response.status();
    let partial = status == reqwest::StatusCode::PARTIAL_CONTENT;
    let whole_file = start == 0 && end == file_size && status.is_success();
    if !partial && !whole_file {
        let message = format!("云端未接受分段读取（HTTP {status}）");
        return Err(if retryable_gcid_export_range_status(status) {
            GcidExportRangeError::retryable(message)
        } else {
            GcidExportRangeError::permanent(message)
        });
    }
    if partial {
        let expected_content_range = format!("bytes {start}-{}/{file_size}", end - 1);
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_range.eq_ignore_ascii_case(&expected_content_range) {
            return Err(GcidExportRangeError::retryable(
                "云端返回的分段范围与请求不一致",
            ));
        }
    }
    let expected = usize::try_from(expected)
        .map_err(|_| GcidExportRangeError::permanent("分段读取的预期字节数超出范围"))?;
    let bytes = read_bounded_gcid_range_stream(
        response.bytes_stream(),
        expected,
        path,
        Duration::from_secs(GCID_EXPORT_READ_IDLE_TIMEOUT_SECS),
    )
    .await?;
    Ok((index, bytes))
}

pub(crate) async fn read_cloud_cid_range_with_retry(
    client: &reqwest::Client,
    range_limiter: &Semaphore,
    token: &str,
    device_id: &str,
    file_id: &str,
    initial_download_url: &str,
    path: &str,
    file_size: u64,
    index: usize,
    range: (u64, u64),
    diagnostics: &GcidExportDiagnostics,
) -> Result<(usize, Vec<u8>), String> {
    retry_gcid_export_range(
        |attempt| async move {
            let request_started = Instant::now();
            let fields = json!({
                "path": path,
                "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                "range_index": index,
                "range_start": range.0,
                "range_end_exclusive": range.1,
                "expected_bytes": range.1.saturating_sub(range.0),
                "attempt": attempt + 1,
                "max_attempts": GCID_EXPORT_RANGE_ATTEMPTS
            });
            diagnostics.write("info", "range_request_started", fields.clone());
            let refreshed_download_url;
            let download_url = if attempt == 0 {
                initial_download_url
            } else {
                refreshed_download_url = match cloud_download_url(token, device_id, file_id).await {
                    Ok(value) => value,
                    Err(error) => {
                        diagnostics.write(
                            "error",
                            "range_download_url_failed",
                            json!({
                                "path": path,
                                "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                                "range_index": index,
                                "attempt": attempt + 1,
                                "elapsed_ms_request": request_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                                "error": error.clone()
                            }),
                        );
                        return Err(GcidExportRangeError::retryable(error));
                    }
                };
                &refreshed_download_url
            };
            match read_cloud_cid_range_once(
                client,
                range_limiter,
                download_url,
                path,
                file_size,
                index,
                range,
            )
            .await
            {
                Ok((result_index, bytes)) => {
                    diagnostics.write(
                        "info",
                        "range_request_succeeded",
                        json!({
                            "path": path,
                            "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                            "range_index": index,
                            "range_start": range.0,
                            "range_end_exclusive": range.1,
                            "attempt": attempt + 1,
                            "elapsed_ms_request": request_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                            "received_bytes": bytes.len()
                        }),
                    );
                    Ok((result_index, bytes))
                }
                Err(error) => {
                    diagnostics.write(
                        "error",
                        "range_request_failed",
                        json!({
                            "path": path,
                            "file_id_suffix": file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                            "range_index": index,
                            "range_start": range.0,
                            "range_end_exclusive": range.1,
                            "attempt": attempt + 1,
                            "elapsed_ms_request": request_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                            "retryable": error.retryable,
                            "error": error.message.clone()
                        }),
                    );
                    Err(error)
                }
            }
        },
        GCID_EXPORT_RANGE_ATTEMPTS,
        400_u64.saturating_add((index as u64).saturating_mul(125)),
    )
    .await
}

pub(crate) async fn sample_cloud_selection_cid(
    client: &reqwest::Client,
    range_limiter: &Semaphore,
    token: &str,
    device_id: &str,
    download_url: &str,
    entry: &CloudSelectionEntry,
    diagnostics: &GcidExportDiagnostics,
) -> Result<(String, u64), String> {
    let ranges = cid_byte_ranges(entry.size);
    let mut parts = stream::iter(ranges.into_iter().enumerate())
        .map(|(index, range)| {
            read_cloud_cid_range_with_retry(
                client,
                range_limiter,
                token,
                device_id,
                &entry.file_id,
                download_url,
                &entry.path,
                entry.size,
                index,
                range,
                diagnostics,
            )
        })
        .buffer_unordered(GCID_EXPORT_RANGE_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await?;
    parts.sort_by_key(|(index, _)| *index);
    let sampled_bytes = parts
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .sum::<u64>();
    let mut hasher = Sha1::new();
    for (_, bytes) in parts {
        hasher.update(bytes);
    }
    Ok((hex::encode_upper(hasher.finalize()), sampled_bytes))
}

pub(crate) async fn sample_cloud_selection_cid_with_retry(
    client: &reqwest::Client,
    range_limiter: &Semaphore,
    token: &str,
    device_id: &str,
    entry: &CloudSelectionEntry,
    diagnostics: &GcidExportDiagnostics,
) -> Result<(String, u64), String> {
    let download_url = retry_gcid_export_range(
        |attempt| async move {
            let request_started = Instant::now();
            let fields = json!({
                "path": entry.path,
                "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                "attempt": attempt + 1,
                "max_attempts": GCID_EXPORT_RANGE_ATTEMPTS
            });
            diagnostics.write("info", "sample_download_url_started", fields.clone());
            match cloud_download_url(token, device_id, &entry.file_id).await {
                Ok(value) => {
                    diagnostics.write(
                        "info",
                        "sample_download_url_succeeded",
                        json!({
                            "path": entry.path,
                            "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                            "attempt": attempt + 1,
                            "elapsed_ms_request": request_started.elapsed().as_millis().min(u64::MAX as u128) as u64
                        }),
                    );
                    Ok(value)
                }
                Err(error) => {
                    diagnostics.write(
                        "error",
                        "sample_download_url_failed",
                        json!({
                            "path": entry.path,
                            "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                            "attempt": attempt + 1,
                            "elapsed_ms_request": request_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                            "error": error.clone()
                        }),
                    );
                    Err(GcidExportRangeError::retryable(error))
                }
            }
        },
        GCID_EXPORT_RANGE_ATTEMPTS,
        250,
    )
    .await?;
    sample_cloud_selection_cid(
        client,
        range_limiter,
        token,
        device_id,
        &download_url,
        entry,
        diagnostics,
    )
    .await
}
pub(crate) async fn hash_cloud_selection_entry(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    range_limiter: Arc<Semaphore>,
    token: &str,
    device_id: &str,
    mut entry: CloudSelectionEntry,
    read_bytes: Arc<AtomicU64>,
    completed_files: Arc<AtomicUsize>,
    planned_sample_bytes: u64,
    source_total_bytes: u128,
    total_files: usize,
    diagnostics: &GcidExportDiagnostics,
) -> Result<GeneratedGcidExportFile, String> {
    emit_gcid_export_progress(
        app,
        "正在生成秒传指纹（Range 采样）",
        &entry.path,
        completed_files.load(Ordering::Relaxed),
        total_files,
        read_bytes.load(Ordering::Relaxed),
        planned_sample_bytes,
        source_total_bytes,
    );
    if !valid_sha1_hex(&entry.gcid) {
        match cloud_selection_entry_detail(token, device_id, &entry.file_id, &entry.name).await {
            Ok(detail) => entry.gcid = detail.gcid,
            Err(error) => diagnostics.write(
                "warn",
                "file_detail_refresh_failed",
                json!({
                    "path": entry.path,
                    "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                    "error": error
                }),
            ),
        }
    }
    if !valid_sha1_hex(&entry.gcid) {
        return Err("光鸭文件详情缺少有效 GCID，无法进行 Range 采样".to_string());
    }
    let (cid, sampled) = match sample_cloud_selection_cid_with_retry(
        client,
        range_limiter.as_ref(),
        token,
        device_id,
        &entry,
        diagnostics,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            diagnostics.write(
                "error",
                "sample_mode_failed",
                json!({
                    "path": entry.path,
                    "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                    "fallback_to_full_download": false,
                    "error": error.clone()
                }),
            );
            return Err(format!("CID Range 采样失败：{error}"));
        }
    };
    let read = read_bytes.fetch_add(sampled, Ordering::Relaxed) + sampled;
    let completed = completed_files.fetch_add(1, Ordering::Relaxed) + 1;
    emit_gcid_export_progress(
        app,
        "正在生成秒传指纹（Range 采样）",
        &entry.path,
        completed,
        total_files,
        read,
        planned_sample_bytes,
        source_total_bytes,
    );
    Ok(GeneratedGcidExportFile {
        path: entry.path,
        size: entry.size.to_string(),
        gcid: entry.gcid.to_ascii_lowercase(),
        cid,
    })
}

#[tauri::command]
pub(crate) async fn export_gcid_json(
    app: tauri::AppHandle,
    state: tauri::State<'_, SharedState>,
    file_ids: Vec<String>,
    file_names: Option<Vec<String>>,
) -> Result<GeneratedGcidExportResult, String> {
    let file_ids = normalize_id_list(&file_ids, "文件或文件夹")?;
    let file_names = file_names
        .unwrap_or_default()
        .into_iter()
        .take(file_ids.len())
        .map(|value| value.chars().take(255).collect::<String>())
        .collect::<Vec<_>>();
    let suggested_name = export_json_file_name(&file_names);
    let Some(save_path) = rfd::FileDialog::new()
        .add_filter("光鸭 GCID JSON", &["json"])
        .set_file_name(&suggested_name)
        .save_file()
    else {
        return Ok(GeneratedGcidExportResult {
            cancelled: true,
            saved_path: None,
            file_name: suggested_name,
            total_files: 0,
            skipped_files_count: 0,
            total_size: "0".to_string(),
        });
    };
    let (database_path, account_scope, cache_enabled, cache_max_entries) = {
        let guard = state.lock().map_err(|error| error.to_string())?;
        (
            guard.db_path.clone(),
            guard.auth_account_scope.clone(),
            guard.cache_enabled,
            guard.cache_max_entries,
        )
    };
    let diagnostics = GcidExportDiagnostics::new(gcid_export_diagnostic_path(&database_path))?;
    diagnostics.write(
        "info",
        "run_started",
        json!({
            "selected_roots": file_ids.len(),
            "scan_concurrency": GCID_EXPORT_SCAN_CONCURRENCY,
            "file_concurrency": GCID_EXPORT_FILE_CONCURRENCY,
            "range_concurrency_per_file": GCID_EXPORT_RANGE_CONCURRENCY,
            "scan_attempts": GCID_EXPORT_SCAN_ATTEMPTS,
            "range_attempts": GCID_EXPORT_RANGE_ATTEMPTS,
            "global_range_concurrency": GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY,
            "request_timeout_ms": GCID_EXPORT_REQUEST_TIMEOUT_SECS * 1_000,
            "read_idle_timeout_ms": GCID_EXPORT_READ_IDLE_TIMEOUT_SECS * 1_000
        }),
    );
    let (token, device_id) = match auth_context(&state) {
        Ok(context) => context,
        Err(error) => {
            diagnostics.write("error", "run_failed", json!({ "error": error.clone() }));
            return Err(error);
        }
    };
    diagnostics.write(
        "info",
        "scan_started",
        json!({ "selected_roots": file_ids.len() }),
    );
    let roots = match load_gcid_export_roots(
        &token,
        &device_id,
        &file_ids,
        &file_names,
        &diagnostics,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            diagnostics.write(
                "error",
                "run_failed",
                json!({ "stage": "scan_root", "error": error.clone() }),
            );
            return Err(error);
        }
    };
    let root_signatures = gcid_export_root_signatures(&roots);
    let selection_key = gcid_export_selection_key(&file_ids);
    let mut cached_snapshot_files = HashMap::new();
    if cache_enabled {
        if let Some(account_scope) = account_scope.as_deref() {
            match load_gcid_export_snapshot(&database_path, account_scope, &selection_key) {
                Ok(Some(snapshot)) => {
                    let cache_age = unix_timestamp().saturating_sub(snapshot.created_at).max(0);
                    cached_snapshot_files = snapshot
                        .export
                        .files
                        .iter()
                        .cloned()
                        .map(|file| (file.path.clone(), file))
                        .collect();
                    if cache_age <= GCID_EXPORT_SNAPSHOT_FRESH_SECS
                        && snapshot.root_signatures == root_signatures
                        && snapshot.export.skipped_files_count == 0
                    {
                        let mut export = snapshot.export;
                        export.generated_at = unix_timestamp();
                        let bytes = serde_json::to_vec_pretty(&export)
                            .map_err(|error| format!("生成缓存秒传 JSON 失败：{error}"))?;
                        tokio::fs::write(&save_path, bytes)
                            .await
                            .map_err(|error| format!("保存秒传 JSON 失败：{error}"))?;
                        if let Err(error) = touch_gcid_export_snapshot(
                            &database_path,
                            account_scope,
                            &selection_key,
                        ) {
                            diagnostics.write(
                                "warn",
                                "snapshot_cache_touch_failed",
                                json!({ "error": error }),
                            );
                        }
                        diagnostics.write(
                            "info",
                            "snapshot_cache_hit",
                            json!({
                                "cache_age_seconds": cache_age,
                                "total_files": export.total_files_count,
                                "fresh_window_seconds": GCID_EXPORT_SNAPSHOT_FRESH_SECS
                            }),
                        );
                        diagnostics.write(
                            "info",
                            "run_completed",
                            json!({
                                "cache_hit": true,
                                "total_files": export.total_files_count,
                                "skipped_files": 0,
                                "source_total_bytes": export.total_size.to_string(),
                                "sampled_or_downloaded_bytes": 0
                            }),
                        );
                        emit_gcid_export_progress(
                            &app,
                            "已命中缓存，秒传 JSON 已生成",
                            "",
                            export.total_files_count,
                            export.total_files_count,
                            0,
                            0,
                            value_as_u64(Some(&export.total_size)).unwrap_or(0) as u128,
                        );
                        return Ok(GeneratedGcidExportResult {
                            cancelled: false,
                            saved_path: Some(save_path.to_string_lossy().to_string()),
                            file_name: suggested_name,
                            total_files: export.total_files_count,
                            skipped_files_count: 0,
                            total_size: export.total_size.to_string().trim_matches('"').to_string(),
                        });
                    }
                    diagnostics.write(
                        "info",
                        "snapshot_cache_miss",
                        json!({
                            "reason": if cache_age > GCID_EXPORT_SNAPSHOT_FRESH_SECS {
                                "expired"
                            } else if snapshot.root_signatures != root_signatures {
                                "root_signature_changed"
                            } else {
                                "partial_snapshot"
                            },
                            "cache_age_seconds": cache_age,
                            "fresh_window_seconds": GCID_EXPORT_SNAPSHOT_FRESH_SECS
                        }),
                    );
                }
                Ok(None) => diagnostics.write(
                    "info",
                    "snapshot_cache_miss",
                    json!({ "reason": "not_found" }),
                ),
                Err(error) => diagnostics.write(
                    "warn",
                    "snapshot_cache_read_failed",
                    json!({ "error": error }),
                ),
            }
        }
    }
    let (entries, roots, scanned_folders) = match collect_gcid_export_entries(
        &app,
        &token,
        &device_id,
        &file_ids,
        &file_names,
        &diagnostics,
        roots,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            diagnostics.write(
                "error",
                "run_failed",
                json!({ "stage": "scan", "error": error.clone() }),
            );
            return Err(error);
        }
    };
    diagnostics.write(
        "info",
        "scan_completed",
        json!({
            "roots": roots.len(),
            "folders": scanned_folders,
            "discovered_entries": entries.len()
        }),
    );
    let single_folder = (roots.len() == 1 && roots[0].folder).then(|| roots[0].clone());
    let root_prefix = single_folder
        .as_ref()
        .map(|entry| format!("{}/", safe_cloud_path_segment(&entry.name)))
        .unwrap_or_default();
    let files = entries
        .into_iter()
        .filter(|entry| !entry.folder)
        .map(|mut entry| {
            if !root_prefix.is_empty() && entry.path.starts_with(&root_prefix) {
                entry.path = entry.path[root_prefix.len()..].to_string();
            }
            entry
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        diagnostics.write(
            "error",
            "run_failed",
            json!({ "stage": "scan", "error": "所选内容中没有可生成秒传 JSON 的文件" }),
        );
        return Err("所选内容中没有可生成秒传 JSON 的文件".to_string());
    }
    let total_size = files
        .iter()
        .try_fold(0_u128, |total, entry| total.checked_add(entry.size as u128))
        .ok_or_else(|| "所选文件总大小溢出".to_string())?;
    let planned_sample_bytes = files.iter().fold(0_u64, |total, entry| {
        total.saturating_add(
            cid_byte_ranges(entry.size)
                .into_iter()
                .map(|(start, end)| end.saturating_sub(start))
                .sum::<u64>(),
        )
    });
    let read_bytes = Arc::new(AtomicU64::new(0));
    let completed_files = Arc::new(AtomicUsize::new(0));
    emit_gcid_export_progress(
        &app,
        "正在生成秒传指纹（Range 采样）",
        &files[0].path,
        0,
        files.len(),
        0,
        planned_sample_bytes,
        total_size,
    );
    let proxy = load_global_network_proxy(&database_path)?;
    diagnostics.write(
        "info",
        "hash_plan_ready",
        json!({
            "total_files": files.len(),
            "folders": scanned_folders,
            "source_total_bytes": total_size.to_string(),
            "planned_sample_bytes": planned_sample_bytes,
            "global_range_concurrency": GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY,
            "proxy_configured": !proxy.trim().is_empty()
        }),
    );
    let mut client_builder =
        reqwest::Client::builder().connect_timeout(Duration::from_secs(API_CONNECT_TIMEOUT_SECS));
    if !proxy.trim().is_empty() {
        client_builder = client_builder.proxy(
            reqwest::Proxy::all(proxy.trim())
                .map_err(|error| format!("初始化秒传 JSON 下载代理失败：{error}"))?,
        );
    }
    let client = client_builder
        .build()
        .map_err(|error| format!("创建秒传 JSON 下载客户端失败：{error}"))?;
    let total_files = files.len();
    let range_limiter = Arc::new(Semaphore::new(GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY));
    let cached_snapshot_files = Arc::new(cached_snapshot_files);
    let mut outcomes = stream::iter(files.into_iter().enumerate())
        .map(|(index, entry)| {
            let app = app.clone();
            let client = client.clone();
            let range_limiter = range_limiter.clone();
            let token = token.clone();
            let device_id = device_id.clone();
            let read_bytes = read_bytes.clone();
            let completed_files = completed_files.clone();
            let diagnostics = diagnostics.clone();
            let database_path = database_path.clone();
            let account_scope = account_scope.clone();
            let cached_snapshot_files = cached_snapshot_files.clone();
            async move {
                let file_started = Instant::now();
                let path = entry.path.clone();
                let file_id = entry.file_id.clone();
                let file_size = entry.size;
                let scanned_gcid = entry.gcid.clone();
                diagnostics.write(
                    "info",
                    "file_started",
                    json!({
                        "file_index": index,
                        "path": path.clone(),
                        "file_id_suffix": entry.file_id.chars().rev().take(8).collect::<String>().chars().rev().collect::<String>(),
                        "size": entry.size,
                        "gcid_available_from_scan": valid_sha1_hex(&entry.gcid)
                    }),
                );
                let cached_cid = if cache_enabled {
                    cached_snapshot_files
                        .get(&path)
                        .and_then(|file| {
                            (file.size == file_size.to_string()
                                && file.gcid.eq_ignore_ascii_case(&scanned_gcid)
                                && valid_sha1_hex(&file.cid))
                            .then(|| file.cid.clone())
                        })
                        .or_else(|| {
                            account_scope.as_deref().and_then(|scope| {
                                match load_gcid_export_file_hash(
                                    &database_path,
                                    scope,
                                    &file_id,
                                    file_size,
                                    &scanned_gcid,
                                ) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        diagnostics.write(
                                            "warn",
                                            "file_cache_read_failed",
                                            json!({ "path": path.clone(), "error": error }),
                                        );
                                        None
                                    }
                                }
                            })
                        })
                } else {
                    None
                };
                let reused_cached_hash = cached_cid.is_some();
                let result = if let Some(cid) = cached_cid {
                    let completed = completed_files.fetch_add(1, Ordering::Relaxed) + 1;
                    diagnostics.write(
                        "info",
                        "file_cache_hit",
                        json!({ "path": path.clone(), "file_index": index }),
                    );
                    emit_gcid_export_progress(
                        &app,
                        "正在生成秒传指纹（Range 采样）",
                        &path,
                        completed,
                        total_files,
                        read_bytes.load(Ordering::Relaxed),
                        planned_sample_bytes,
                        total_size,
                    );
                    Ok(GeneratedGcidExportFile {
                        path: path.clone(),
                        size: file_size.to_string(),
                        gcid: scanned_gcid.to_ascii_lowercase(),
                        cid,
                    })
                } else {
                    hash_cloud_selection_entry(
                        &app,
                        &client,
                        range_limiter,
                        &token,
                        &device_id,
                        entry,
                        read_bytes.clone(),
                        completed_files.clone(),
                        planned_sample_bytes,
                        total_size,
                        total_files,
                        &diagnostics,
                    )
                    .await
                };
                if cache_enabled && !reused_cached_hash {
                    if let (Some(scope), Ok(file)) = (account_scope.as_deref(), &result) {
                        if let Err(error) = save_gcid_export_file_hash(
                            &database_path,
                            scope,
                            &file_id,
                            file_size,
                            &file.gcid,
                            &file.cid,
                        ) {
                            diagnostics.write(
                                "warn",
                                "file_cache_save_failed",
                                json!({ "path": path.clone(), "error": error }),
                            );
                        }
                    }
                }
                match &result {
                    Ok(_) => diagnostics.write(
                        "info",
                        "file_succeeded",
                        json!({
                            "file_index": index,
                            "path": path.clone(),
                            "elapsed_ms_file": file_started.elapsed().as_millis().min(u64::MAX as u128) as u64
                        }),
                    ),
                    Err(error) => {
                        diagnostics.write(
                            "error",
                            "file_failed",
                            json!({
                                "file_index": index,
                                "path": path.clone(),
                                "elapsed_ms_file": file_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                                "error": error
                            }),
                        );
                        let completed = completed_files.fetch_add(1, Ordering::Relaxed) + 1;
                        emit_gcid_export_progress(
                            &app,
                            "正在生成秒传指纹（Range 采样）",
                            &path,
                            completed,
                            total_files,
                            read_bytes.load(Ordering::Relaxed),
                            planned_sample_bytes,
                            total_size,
                        );
                    }
                }
                (index, path, result)
            }
        })
        .buffer_unordered(GCID_EXPORT_FILE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    outcomes.sort_by_key(|(index, _, _)| *index);
    let mut hashed_files = Vec::new();
    let mut skipped_files = Vec::new();
    for (_, path, outcome) in outcomes {
        match outcome {
            Ok(file) => hashed_files.push(file),
            Err(error) => skipped_files.push(format!("{path}：{error}")),
        }
    }
    if hashed_files.is_empty() {
        let reason = skipped_files
            .first()
            .cloned()
            .unwrap_or_else(|| "没有文件可导出".to_string());
        let error = format!("秒传 JSON 生成失败：{reason}");
        diagnostics.write(
            "error",
            "run_failed",
            json!({ "stage": "hash", "skipped_files": skipped_files.len(), "error": error.clone() }),
        );
        return Err(error);
    }
    let export = GeneratedGcidExport {
        script_version: "guangya-gcid-export-2.0".to_string(),
        export_version: "2.0".to_string(),
        source: "guangya".to_string(),
        hash_type: "gcid".to_string(),
        uses_gcid_in_export: true,
        uses_cid_in_export: true,
        uses_base62_etags_in_export: false,
        common_path: single_folder
            .as_ref()
            .map(|entry| entry.name.clone())
            .unwrap_or_default(),
        source_folder_id: single_folder
            .as_ref()
            .map(|entry| entry.file_id.clone())
            .unwrap_or_default(),
        source_folder_name: single_folder
            .as_ref()
            .map(|entry| entry.name.clone())
            .unwrap_or_default(),
        total_files_count: hashed_files.len(),
        total_size: u64::try_from(total_size)
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(total_size.to_string())),
        formatted_total_size: format_export_bytes(total_size),
        generated_at: unix_timestamp(),
        scanned_folders_count: scanned_folders,
        skipped_files_count: skipped_files.len(),
        skipped_files,
        files: hashed_files,
    };
    let bytes = serde_json::to_vec_pretty(&export)
        .map_err(|error| format!("生成秒传 JSON 失败：{error}"))?;
    tokio::fs::write(&save_path, bytes)
        .await
        .map_err(|error| format!("保存秒传 JSON 失败：{error}"))?;
    if cache_enabled {
        if let Err(error) = trim_gcid_export_file_hash_cache(&database_path, cache_max_entries) {
            diagnostics.write("warn", "file_cache_trim_failed", json!({ "error": error }));
        }
    }
    if cache_enabled && export.skipped_files_count == 0 {
        if let Some(account_scope) = account_scope.as_deref() {
            match save_gcid_export_snapshot(
                &database_path,
                account_scope,
                &selection_key,
                &root_signatures,
                &export,
            ) {
                Ok(()) => diagnostics.write(
                    "info",
                    "snapshot_cache_saved",
                    json!({
                        "total_files": export.total_files_count,
                        "fresh_window_seconds": GCID_EXPORT_SNAPSHOT_FRESH_SECS
                    }),
                ),
                Err(error) => diagnostics.write(
                    "warn",
                    "snapshot_cache_save_failed",
                    json!({ "error": error }),
                ),
            }
        }
    }
    diagnostics.write(
        "info",
        "run_completed",
        json!({
            "total_files": export.total_files_count,
            "skipped_files": export.skipped_files_count,
            "source_total_bytes": total_size.to_string(),
            "sampled_or_downloaded_bytes": read_bytes.load(Ordering::Relaxed)
        }),
    );
    emit_gcid_export_progress(
        &app,
        if export.skipped_files_count > 0 {
            "秒传 JSON 已生成，部分文件已跳过"
        } else {
            "秒传 JSON 已生成"
        },
        "",
        export.total_files_count + export.skipped_files_count,
        export.total_files_count + export.skipped_files_count,
        read_bytes.load(Ordering::Relaxed),
        planned_sample_bytes,
        total_size,
    );
    Ok(GeneratedGcidExportResult {
        cancelled: false,
        saved_path: Some(save_path.to_string_lossy().to_string()),
        file_name: suggested_name,
        total_files: export.total_files_count,
        skipped_files_count: export.skipped_files_count,
        total_size: total_size.to_string(),
    })
}

#[derive(Serialize)]
pub(crate) struct GcidExportDiagnosticLogResult {
    pub(crate) cancelled: bool,
    pub(crate) saved_path: Option<String>,
    pub(crate) file_name: String,
}

#[tauri::command]
pub(crate) async fn export_gcid_diagnostic_log(
    state: tauri::State<'_, SharedState>,
) -> Result<GcidExportDiagnosticLogResult, String> {
    let database_path = state
        .lock()
        .map_err(|error| error.to_string())?
        .db_path
        .clone();
    let source_path = gcid_export_diagnostic_path(&database_path);
    let content = tokio::fs::read(&source_path).await.map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            "还没有秒传 JSON 诊断日志，请先运行一次生成任务".to_string()
        } else {
            format!("读取秒传 JSON 诊断日志失败：{error}")
        }
    })?;
    if content.is_empty() {
        return Err("秒传 JSON 诊断日志为空，请重新运行一次生成任务".to_string());
    }
    let file_name = "光鸭秒传诊断日志.jsonl".to_string();
    let Some(save_path) = rfd::FileDialog::new()
        .add_filter("JSON Lines 日志", &["jsonl", "log"])
        .set_file_name(&file_name)
        .save_file()
    else {
        return Ok(GcidExportDiagnosticLogResult {
            cancelled: true,
            saved_path: None,
            file_name,
        });
    };
    tokio::fs::write(&save_path, content)
        .await
        .map_err(|error| format!("保存秒传 JSON 诊断日志失败：{error}"))?;
    Ok(GcidExportDiagnosticLogResult {
        cancelled: false,
        saved_path: Some(save_path.to_string_lossy().to_string()),
        file_name,
    })
}
