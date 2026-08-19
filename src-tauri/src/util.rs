//! 通用小工具：时间戳、扩展名、路径规范化。

use crate::prelude::*;

pub(crate) fn file_extension(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    extension
}

pub(crate) fn normalize_remote_path(input: &str) -> String {
    input
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}


pub(crate) fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| match value {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.parse::<u64>().ok(),
        _ => None,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct UploadScanSkip {
    pub path: String,
    pub reason: String,
}

impl UploadScanSkip {
    pub(crate) fn new(path: &Path, reason: impl Into<String>) -> Self {
        Self {
            path: user_visible_path(path).to_string_lossy().into_owned(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PathIdentity(String);

pub(crate) fn extended_length_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if raw.starts_with(r"\\?\") || raw.starts_with(r"\\.\") {
            return path.to_path_buf();
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };
        let text = absolute.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\") {
            PathBuf::from(format!(r"\\?\UNC\{rest}"))
        } else {
            PathBuf::from(format!(r"\\?\{text}"))
        }
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

pub(crate) fn user_visible_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
        path.to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

pub(crate) fn readable_fs_path(path: &Path) -> PathBuf {
    extended_length_path(path)
}

pub(crate) fn path_identity(path: &Path) -> PathIdentity {
    let readable = extended_length_path(path);
    PathIdentity(
        fs::canonicalize(&readable)
            .unwrap_or(readable)
            .to_string_lossy()
            .to_lowercase(),
    )
}

pub(crate) fn inspect_local_entry(
    path: &Path,
    skips: &mut Vec<UploadScanSkip>,
) -> Option<(LocalEntryKind, PathBuf)> {
    let readable = extended_length_path(path);
    let symlink_meta = match fs::symlink_metadata(&readable) {
        Ok(meta) => meta,
        Err(error) => {
            skips.push(UploadScanSkip::new(path, format!("无法读取路径：{error}")));
            return None;
        }
    };
    let target_meta = if symlink_meta.file_type().is_symlink() {
        match fs::metadata(&readable) {
            Ok(meta) => meta,
            Err(error) => {
                skips.push(UploadScanSkip::new(
                    path,
                    format!("无法解析链接目标：{error}"),
                ));
                return None;
            }
        }
    } else {
        symlink_meta
    };
    if target_meta.is_file() {
        return Some((LocalEntryKind::File, readable));
    }
    if target_meta.is_dir() {
        return Some((LocalEntryKind::Directory, readable));
    }
    skips.push(UploadScanSkip::new(path, "不是普通文件或目录"));
    None
}
