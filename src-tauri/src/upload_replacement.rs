//! 同名覆盖上传的安全替换流程。

use crate::prelude::*;

pub(crate) async fn find_remote_file(
    token: &str,
    device_id: &str,
    parent_id: &str,
    name: &str,
) -> Result<Option<(String, u64, i64)>, String> {
    for page in 0..1000 {
        let result = api_post(
            token,
            device_id,
            "/userres/v1/file/get_file_list",
            json!({
                "page": page,
                "pageSize": 100,
                "parentId": parent_id,
                "orderBy": 0,
                "sortType": 0,
                "needSubFolderStat": true
            }),
            &[],
        )
        .await?;
        let data = result.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(found) = list
            .iter()
            .find(|item| item.get("fileName").and_then(Value::as_str) == Some(name))
        {
            let file_id = found
                .get("fileId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let file_size = value_as_u64(found.get("fileSize")).unwrap_or_default();
            let res_type = found
                .get("resType")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if !file_id.is_empty() {
                return Ok(Some((file_id, file_size, res_type)));
            }
        }
        let total = data.get("total").and_then(Value::as_u64).unwrap_or(0);
        if list.is_empty() || ((page + 1) * 100) as u64 >= total {
            break;
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplacementRemoteEntry {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UploadReplacementState {
    Conflict,
    OldRenamedExternally(String),
    StageOld,
    PromoteNew { old_exists: bool },
    Promoted { old_exists: bool },
}

pub(crate) fn upload_replacement_state(
    entries: &[ReplacementRemoteEntry],
    replacement: &UploadReplacement,
    new_file_id: &str,
) -> UploadReplacementState {
    if entries.iter().any(|entry| {
        entry.name == replacement.original_name
            && entry.id != replacement.old_file_id
            && entry.id != new_file_id
    }) {
        return UploadReplacementState::Conflict;
    }
    let old = entries
        .iter()
        .find(|entry| entry.id == replacement.old_file_id);
    if entries
        .iter()
        .any(|entry| entry.id == new_file_id && entry.name == replacement.original_name)
    {
        return UploadReplacementState::Promoted {
            old_exists: old.is_some(),
        };
    }
    match old {
        Some(entry) if entry.name == replacement.original_name => UploadReplacementState::StageOld,
        Some(entry) if entry.name == replacement.backup_name => {
            UploadReplacementState::PromoteNew { old_exists: true }
        }
        Some(entry) => UploadReplacementState::OldRenamedExternally(entry.name.clone()),
        None => UploadReplacementState::PromoteNew { old_exists: false },
    }
}

pub(crate) async fn list_upload_replacement_entries(
    token: &str,
    device_id: &str,
    parent_id: &str,
) -> Result<Vec<ReplacementRemoteEntry>, String> {
    let mut entries = Vec::new();
    for page in 0..1000_u64 {
        let response = api_post(
            token,
            device_id,
            "/userres/v1/file/get_file_list",
            json!({
                "page": page,
                "pageSize": 100,
                "parentId": parent_id,
                "orderBy": 0,
                "sortType": 0,
                "needSubFolderStat": true
            }),
            &[],
        )
        .await?;
        let data = response.data.unwrap_or_default();
        let list = data
            .get("list")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let count = list.len();
        entries.extend(list.into_iter().filter_map(|entry| {
            let id = entry.get("fileId")?.as_str()?.to_string();
            let name = entry.get("fileName")?.as_str()?.to_string();
            (!id.is_empty() && !name.is_empty()).then_some(ReplacementRemoteEntry { id, name })
        }));
        match value_as_u64(data.get("total")) {
            Some(total) if entries.len() as u64 >= total => break,
            None if count < 100 => break,
            _ if count == 0 => break,
            _ => {}
        }
    }
    Ok(entries)
}

pub(crate) async fn rename_upload_replacement_entry(
    token: &str,
    device_id: &str,
    file_id: &str,
    new_name: &str,
) -> Result<(), String> {
    let response = api_post(
        token,
        device_id,
        "/userres/v1/file/rename",
        json!({ "fileId": file_id, "newName": new_name }),
        &[],
    )
    .await?;
    finish_operation_response(token, device_id, response)
        .await
        .map(|_| ())
}

pub(crate) async fn delete_upload_replacement_entry(
    token: &str,
    device_id: &str,
    file_id: &str,
) -> Result<(), String> {
    let response = api_post(
        token,
        device_id,
        "/userres/v1/file/delete_file",
        json!({ "fileIds": [file_id] }),
        &[],
    )
    .await?;
    finish_operation_response(token, device_id, response)
        .await
        .map(|_| ())
}

pub(crate) async fn safely_replace_uploaded_file(
    token: &str,
    device_id: &str,
    parent_id: &str,
    replacement: &UploadReplacement,
    new_file_id: &str,
) -> Result<(), String> {
    if new_file_id.is_empty() {
        return Err("新文件已入库，但缺少文件 ID，无法安全覆盖".into());
    }
    if new_file_id == replacement.old_file_id {
        return Ok(());
    }
    let entries = list_upload_replacement_entries(token, device_id, parent_id).await?;
    let mut state = upload_replacement_state(&entries, replacement, new_file_id);
    match &state {
        UploadReplacementState::Conflict => {
            return Err(format!(
                "云端“{}”已被其他文件占用；新版本保留为临时文件，未覆盖现有文件",
                replacement.original_name
            ));
        }
        UploadReplacementState::OldRenamedExternally(name) => {
            return Err(format!(
                "原云端文件已被改名为“{name}”；新版本保留为临时文件，未覆盖外部改动"
            ));
        }
        UploadReplacementState::StageOld => {
            rename_upload_replacement_entry(
                token,
                device_id,
                &replacement.old_file_id,
                &replacement.backup_name,
            )
            .await?;
            state = UploadReplacementState::PromoteNew { old_exists: true };
        }
        _ => {}
    }
    let old_exists = match state {
        UploadReplacementState::Promoted { old_exists } => old_exists,
        UploadReplacementState::PromoteNew { old_exists } => {
            if let Err(error) = rename_upload_replacement_entry(
                token,
                device_id,
                new_file_id,
                &replacement.original_name,
            )
            .await
            {
                if old_exists {
                    if let Err(rollback) = rename_upload_replacement_entry(
                        token,
                        device_id,
                        &replacement.old_file_id,
                        &replacement.original_name,
                    )
                    .await
                    {
                        return Err(format!("{error}；恢复旧文件名也失败：{rollback}"));
                    }
                }
                return Err(error);
            }
            old_exists
        }
        UploadReplacementState::Conflict
        | UploadReplacementState::OldRenamedExternally(_)
        | UploadReplacementState::StageOld => unreachable!("replacement state handled above"),
    };
    if old_exists {
        delete_upload_replacement_entry(token, device_id, &replacement.old_file_id).await?;
    }
    Ok(())
}

pub(crate) struct ActiveUploadReplacement {
    pub(crate) state: SharedState,
    pub(crate) key: String,
}

impl Drop for ActiveUploadReplacement {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.active_upload_replacements.remove(&self.key);
        }
    }
}

pub(crate) async fn complete_upload_replacement(
    app: &tauri::AppHandle,
    state: &SharedState,
    token: &str,
    device_id: &str,
    item: &UploadItem,
    outcome: &UploadOutcome,
) -> Result<(), String> {
    let Some(replacement) = item.replacement.as_ref() else {
        return Ok(());
    };
    let key = item_key(&item.mapping_id, &item.file_path);
    let gate_pool = state
        .lock()
        .map_err(|error| error.to_string())?
        .upload_replacement_gates
        .clone();
    let gate = gate_pool.gate(&key);
    let _gate = gate.lock().await;
    {
        let mut guard = state.lock().map_err(|error| error.to_string())?;
        if guard.cancelled_uploads.contains_key(&key) {
            return Err(UPLOAD_CANCELLED_MESSAGE.into());
        }
        guard.active_upload_replacements.insert(key.clone());
    }
    let _active = ActiveUploadReplacement {
        state: state.clone(),
        key,
    };
    let new_file_id = outcome
        .remote_file_id
        .as_deref()
        .ok_or_else(|| "新文件已入库，但缺少文件 ID，无法安全覆盖".to_string())?;
    emit(
        app,
        json!({
            "type": "progress",
            "file_path": item.file_path.to_string_lossy(),
            "percent": 100,
            "uploaded_bytes": item.size,
            "total_bytes": item.size,
            "bytes_per_second": 0,
            "stage": "新版本已入库，正在安全替换旧文件"
        }),
    );
    let parent_id = ensure_remote_path(
        app,
        state,
        token,
        device_id,
        &item.remote_parent_id,
        &item.remote_dir,
    )
    .await?;
    safely_replace_uploaded_file(token, device_id, &parent_id, replacement, new_file_id).await
}
