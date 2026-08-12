//! 文件哈希：MD5、SHA1、CID/GCID 计算。

use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileHashes {
    pub(crate) gcid: String,
    pub(crate) cid: String,
}


pub(crate) fn gcid_chunk_size(file_size: u64) -> usize {
    match file_size {
        0..=0x0800_0000 => 256 * 1024,
        0x0800_0001..=0x1000_0000 => 512 * 1024,
        0x1000_0001..=0x2000_0000 => 1024 * 1024,
        _ => 2 * 1024 * 1024,
    }
}

pub(crate) fn cid_byte_ranges(file_size: u64) -> Vec<(u64, u64)> {
    if file_size < 60 * 1024 {
        return vec![(0, file_size)];
    }
    let middle = file_size / 3;
    vec![
        (0, 20 * 1024),
        (middle, middle + 20 * 1024),
        (file_size - 20 * 1024, file_size),
    ]
}

pub(crate) fn update_cid_hasher(
    hasher: &mut Sha1,
    ranges: &[(u64, u64)],
    chunk_start: u64,
    chunk: &[u8],
) -> Result<u64, String> {
    let chunk_end = chunk_start.saturating_add(chunk.len() as u64);
    let mut sampled = 0_u64;
    for (start, end) in ranges {
        let overlap_start = chunk_start.max(*start);
        let overlap_end = chunk_end.min(*end);
        if overlap_start >= overlap_end {
            continue;
        }
        let local_start = usize::try_from(overlap_start - chunk_start)
            .map_err(|_| "CID 采样位置超出范围".to_string())?;
        let local_end = usize::try_from(overlap_end - chunk_start)
            .map_err(|_| "CID 采样位置超出范围".to_string())?;
        hasher.update(&chunk[local_start..local_end]);
        sampled = sampled.saturating_add(overlap_end - overlap_start);
    }
    Ok(sampled)
}

pub(crate) struct FlashHashAccumulator {
    pub(crate) file_size: u64,
    pub(crate) chunk_size: usize,
    pub(crate) gcid_chunk: Vec<u8>,
    pub(crate) gcid_chunk_bytes: usize,
    pub(crate) gcid_hasher: Sha1,
    pub(crate) cid_hasher: Sha1,
    pub(crate) cid_ranges: Vec<(u64, u64)>,
    pub(crate) expected_cid_bytes: u64,
    pub(crate) cid_bytes: u64,
    pub(crate) position: u64,
}

impl FlashHashAccumulator {
    pub(crate) fn new(file_size: u64) -> Self {
        let chunk_size = gcid_chunk_size(file_size);
        let cid_ranges = cid_byte_ranges(file_size);
        let expected_cid_bytes = cid_ranges
            .iter()
            .map(|(start, end)| end - start)
            .sum::<u64>();
        Self {
            file_size,
            chunk_size,
            gcid_chunk: vec![0_u8; chunk_size],
            gcid_chunk_bytes: 0,
            gcid_hasher: Sha1::new(),
            cid_hasher: Sha1::new(),
            cid_ranges,
            expected_cid_bytes,
            cid_bytes: 0,
            position: 0,
        }
    }

    pub(crate) fn flush_gcid_chunk(&mut self) {
        if self.gcid_chunk_bytes == 0 {
            return;
        }
        self.gcid_hasher
            .update(Sha1::digest(&self.gcid_chunk[..self.gcid_chunk_bytes]));
        self.gcid_chunk_bytes = 0;
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) -> Result<u64, String> {
        let chunk_start = self.position;
        let chunk_end = chunk_start
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "秒传指纹读取位置溢出".to_string())?;
        if chunk_end > self.file_size {
            return Err("下载内容超过云端文件声明大小".to_string());
        }
        self.cid_bytes = self.cid_bytes.saturating_add(update_cid_hasher(
            &mut self.cid_hasher,
            &self.cid_ranges,
            chunk_start,
            bytes,
        )?);
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let copied = (self.chunk_size - self.gcid_chunk_bytes).min(bytes.len() - offset);
            self.gcid_chunk[self.gcid_chunk_bytes..self.gcid_chunk_bytes + copied]
                .copy_from_slice(&bytes[offset..offset + copied]);
            self.gcid_chunk_bytes += copied;
            offset += copied;
            if self.gcid_chunk_bytes == self.chunk_size {
                self.flush_gcid_chunk();
            }
        }
        self.position = chunk_end;
        Ok(self.position)
    }

    pub(crate) fn finish(mut self) -> Result<FileHashes, String> {
        if self.position != self.file_size || self.cid_bytes != self.expected_cid_bytes {
            return Err("下载内容与云端文件声明大小不一致".to_string());
        }
        self.flush_gcid_chunk();
        Ok(FileHashes {
            gcid: hex::encode_upper(self.gcid_hasher.finalize()),
            cid: hex::encode_upper(self.cid_hasher.finalize()),
        })
    }
}

pub(crate) async fn calculate_file_md5(path: &Path) -> Result<String, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("读取秒传文件失败：{error}"))?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0_u8; 2 * 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("计算文件 MD5 失败：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) async fn calculate_file_flash_hashes(
    app: &tauri::AppHandle,
    path: &Path,
    file_size: u64,
) -> Result<FileHashes, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("读取秒传文件失败：{error}"))?;
    let chunk_size = gcid_chunk_size(file_size);
    let mut buffer = vec![0_u8; chunk_size];
    let mut accumulator = FlashHashAccumulator::new(file_size);
    let mut hashed = 0_u64;
    while hashed < file_size {
        let read = usize::try_from((file_size - hashed).min(chunk_size as u64))
            .map_err(|_| "秒传指纹分块大小超出范围".to_string())?;
        file.read_exact(&mut buffer[..read])
            .await
            .map_err(|error| format!("计算文件 GCID 失败：{error}"))?;
        hashed = accumulator.update(&buffer[..read])?;
        let percent = if file_size == 0 {
            100
        } else {
            hashed.saturating_mul(100) / file_size
        };
        emit(
            app,
            json!({
                "type": "progress",
                "file_path": path.to_string_lossy(),
                "percent": 0,
                "bytes_per_second": 0,
                "stage": format!("正在计算秒传指纹 {percent}%")
            }),
        );
    }
    accumulator
        .finish()
        .map_err(|_| "计算秒传指纹时文件大小发生变化".to_string())
}
