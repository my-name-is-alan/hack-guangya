use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_PARALLELISM: usize = 16;
const MAX_CACHE_SIZE_GB: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeMountOptions {
    #[serde(default)]
    pub rclone_path: String,
    #[serde(default = "default_target")]
    pub target: String,
    #[serde(default = "default_access_mode")]
    pub access_mode: String,
    #[serde(default = "default_vfs_cache_mode")]
    pub vfs_cache_mode: String,
    #[serde(default = "default_transfers")]
    pub transfers: usize,
    #[serde(default = "default_read_streams")]
    pub read_streams: usize,
    #[serde(default = "default_cache_size_gb")]
    pub cache_size_gb: usize,
}

impl Default for NativeMountOptions {
    fn default() -> Self {
        Self {
            rclone_path: String::new(),
            target: default_target(),
            access_mode: default_access_mode(),
            vfs_cache_mode: default_vfs_cache_mode(),
            transfers: default_transfers(),
            read_streams: default_read_streams(),
            cache_size_gb: default_cache_size_gb(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeMountInfo {
    pub supported: bool,
    pub available: bool,
    pub running: bool,
    pub engine: String,
    pub platform: String,
    pub rclone_available: bool,
    pub fuse_available: bool,
    pub version: String,
    pub prerequisite: String,
    pub target: String,
    pub access_mode: String,
    pub vfs_cache_mode: String,
    pub transfers: usize,
    pub read_streams: usize,
    pub cache_size_gb: usize,
    pub rclone_path: String,
    pub started_at: Option<u64>,
    pub error: Option<String>,
}

pub struct NativeMountManager {
    options: NativeMountOptions,
    child: Option<Child>,
    data_dir: PathBuf,
    resource_dir: PathBuf,
    started_at: Option<u64>,
    error: Option<String>,
}

impl NativeMountManager {
    pub fn new(options: NativeMountOptions, data_dir: PathBuf, resource_dir: PathBuf) -> Self {
        Self {
            options: normalize_options(options).unwrap_or_default(),
            child: None,
            data_dir,
            resource_dir,
            started_at: None,
            error: None,
        }
    }

    pub fn options(&self) -> NativeMountOptions {
        self.options.clone()
    }

    pub fn set_options(&mut self, options: NativeMountOptions) -> Result<(), String> {
        if self.child.is_some() && self.options != options {
            return Err("请先卸载当前原生挂载，再修改挂载参数".to_string());
        }
        self.options = normalize_options(options)?;
        Ok(())
    }

    pub fn info(&mut self) -> NativeMountInfo {
        self.refresh_process();
        let executable = self.resolve_rclone_path();
        let version = probe_rclone(&executable).unwrap_or_default();
        let rclone_available = !version.is_empty();
        let (fuse_available, prerequisite) = fuse_prerequisite();
        NativeMountInfo {
            supported: true,
            available: rclone_available && fuse_available,
            running: self.child.is_some(),
            engine: "rclone".to_string(),
            platform: platform_name().to_string(),
            rclone_available,
            fuse_available,
            version,
            prerequisite,
            target: self.options.target.clone(),
            access_mode: self.options.access_mode.clone(),
            vfs_cache_mode: self.options.vfs_cache_mode.clone(),
            transfers: self.options.transfers,
            read_streams: self.options.read_streams,
            cache_size_gb: self.options.cache_size_gb,
            rclone_path: self.options.rclone_path.clone(),
            started_at: self.started_at,
            error: self.error.clone(),
        }
    }

    pub fn start(
        &mut self,
        endpoint: &str,
        username: &str,
        password: &str,
    ) -> Result<NativeMountInfo, String> {
        self.refresh_process();
        if self.child.is_some() {
            return Ok(self.info());
        }
        self.options = normalize_options(self.options.clone())?;
        let executable = self.resolve_rclone_path();
        let version = probe_rclone(&executable).map_err(|error| {
            format!("未找到可用的 rclone：{error}。可在挂载设置中选择 rclone 可执行文件")
        })?;
        let (fuse_available, prerequisite) = fuse_prerequisite();
        if !fuse_available {
            return Err(prerequisite);
        }
        fs::create_dir_all(self.data_dir.join("native-mount-cache"))
            .map_err(|error| format!("创建原生挂载缓存目录失败：{error}"))?;
        fs::create_dir_all(self.data_dir.join("logs"))
            .map_err(|error| format!("创建原生挂载日志目录失败：{error}"))?;
        let log_path = self.log_path();
        fs::write(&log_path, b"").map_err(|error| format!("创建原生挂载日志失败：{error}"))?;
        let obscured_password = obscure_password(&executable, password)?;
        prepare_target(&self.options.target)?;

        let mut command = Command::new(&executable);
        command
            .args(build_mount_arguments(
                &self.options,
                &self.data_dir.join("native-mount-cache"),
                &log_path,
            ))
            .env("RCLONE_WEBDAV_URL", endpoint)
            .env("RCLONE_WEBDAV_VENDOR", "other")
            .env("RCLONE_WEBDAV_USER", username)
            .env("RCLONE_WEBDAV_PASS", obscured_password)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_command_window(&mut command);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                restore_windows_directory_target(&self.options.target);
                return Err(format!("启动 rclone 原生挂载失败：{error}"));
            }
        };
        self.child = Some(child);
        self.started_at = Some(unix_timestamp());
        self.error = None;

        thread::sleep(Duration::from_millis(1_200));
        self.refresh_process();
        if self.child.is_none() {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "rclone 在挂载完成前退出".to_string()));
        }
        let mut info = self.info();
        info.version = version;
        Ok(info)
    }

    pub fn stop(&mut self) -> Result<NativeMountInfo, String> {
        self.refresh_process();
        if self.child.is_none() {
            self.started_at = None;
            return Ok(self.info());
        }
        attempt_unmount(&self.options.target);
        if let Some(child) = self.child.as_mut() {
            for _ in 0..20 {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.child = None;
        self.started_at = None;
        self.error = None;
        restore_windows_directory_target(&self.options.target);
        Ok(self.info())
    }

    pub fn shutdown(&mut self) {
        let _ = self.stop();
    }

    fn resolve_rclone_path(&self) -> PathBuf {
        if !self.options.rclone_path.trim().is_empty() {
            return PathBuf::from(self.options.rclone_path.trim());
        }
        if let Some(configured) = env::var("GUANGYA_RCLONE_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            return PathBuf::from(configured);
        }
        let file_name = if cfg!(windows) {
            "rclone.exe"
        } else {
            "rclone"
        };
        for candidate in [
            self.resource_dir.join("resources").join(file_name),
            self.resource_dir.join(file_name),
            env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|parent| parent.join(file_name)))
                .unwrap_or_default(),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
        PathBuf::from(file_name)
    }

    fn refresh_process(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.child = None;
                self.started_at = None;
                restore_windows_directory_target(&self.options.target);
                let log = read_log_tail(&self.log_path());
                self.error = Some(if log.is_empty() {
                    format!("rclone 原生挂载已退出（{status}）")
                } else {
                    format!("rclone 原生挂载已退出：{log}")
                });
            }
            Ok(None) => {}
            Err(error) => {
                self.child = None;
                self.started_at = None;
                restore_windows_directory_target(&self.options.target);
                self.error = Some(format!("读取 rclone 进程状态失败：{error}"));
            }
        }
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.join("logs").join("native-mount.log")
    }
}

fn normalize_options(mut options: NativeMountOptions) -> Result<NativeMountOptions, String> {
    options.rclone_path = options.rclone_path.trim().to_string();
    options.target = options.target.trim().to_string();
    if options.target.is_empty() {
        return Err("请选择盘符或挂载目录".to_string());
    }
    if !matches!(options.access_mode.as_str(), "read_only" | "read_write") {
        return Err("挂载权限必须是只读或读写".to_string());
    }
    if !matches!(
        options.vfs_cache_mode.as_str(),
        "off" | "minimal" | "writes" | "full"
    ) {
        return Err("VFS 缓存模式无效".to_string());
    }
    if !(1..=MAX_PARALLELISM).contains(&options.transfers) {
        return Err(format!("上传并行数必须为 1 到 {MAX_PARALLELISM}"));
    }
    if !(1..=MAX_PARALLELISM).contains(&options.read_streams) {
        return Err(format!("读取并行数必须为 1 到 {MAX_PARALLELISM}"));
    }
    if !(1..=MAX_CACHE_SIZE_GB).contains(&options.cache_size_gb) {
        return Err(format!("缓存上限必须为 1 到 {MAX_CACHE_SIZE_GB} GB"));
    }
    Ok(options)
}

fn build_mount_arguments(
    options: &NativeMountOptions,
    cache_dir: &Path,
    log_path: &Path,
) -> Vec<String> {
    let mut arguments = vec![
        "mount".to_string(),
        ":webdav:".to_string(),
        options.target.clone(),
        "--config".to_string(),
        if cfg!(windows) {
            "NUL".to_string()
        } else {
            "/dev/null".to_string()
        },
        "--vfs-cache-mode".to_string(),
        options.vfs_cache_mode.clone(),
        "--transfers".to_string(),
        options.transfers.to_string(),
        "--vfs-read-chunk-streams".to_string(),
        options.read_streams.to_string(),
        "--cache-dir".to_string(),
        cache_dir.to_string_lossy().to_string(),
        "--vfs-cache-max-size".to_string(),
        format!("{}G", options.cache_size_gb),
        "--vfs-cache-max-age".to_string(),
        "24h".to_string(),
        "--vfs-cache-poll-interval".to_string(),
        "1m".to_string(),
        "--vfs-write-back".to_string(),
        "5s".to_string(),
        "--dir-cache-time".to_string(),
        "5m".to_string(),
        "--poll-interval".to_string(),
        "0".to_string(),
        "--buffer-size".to_string(),
        "4M".to_string(),
        "--vfs-read-ahead".to_string(),
        "16M".to_string(),
        "--vfs-read-chunk-size".to_string(),
        "4M".to_string(),
        "--log-file".to_string(),
        log_path.to_string_lossy().to_string(),
        "--log-level".to_string(),
        "INFO".to_string(),
    ];
    if options.access_mode == "read_only" {
        arguments.push("--read-only".to_string());
    }
    if cfg!(windows) {
        arguments.push("--no-console".to_string());
        arguments.push("--volname".to_string());
        arguments.push("光鸭云盘".to_string());
    }
    arguments
}

fn obscure_password(executable: &Path, password: &str) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .args(["obscure", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_command_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 rclone 密码处理失败：{error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "无法写入 rclone 密码处理进程".to_string())?
        .write_all(format!("{password}\n").as_bytes())
        .map_err(|error| format!("写入 rclone 密码失败：{error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("等待 rclone 密码处理失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rclone 密码处理失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        return Err("rclone 没有返回处理后的密码".to_string());
    }
    Ok(value)
}

fn probe_rclone(executable: &Path) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_command_window(&mut command);
    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "rclone version 没有输出版本".to_string())
}

fn prepare_target(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        if is_windows_drive(target) {
            let drive_root = PathBuf::from(format!("{target}\\"));
            if drive_root.exists() {
                return Err(format!("盘符 {target} 已被占用，请选择未使用的盘符"));
            }
            return Ok(());
        }
        let path = Path::new(target);
        if !path.is_absolute() {
            return Err("挂载目录必须使用绝对路径；Windows 也可填写 X: 形式的盘符".to_string());
        }
        let parent = path
            .parent()
            .filter(|parent| parent.is_dir())
            .ok_or_else(|| "Windows 挂载目录的上级目录必须已存在".to_string())?;
        if !parent.is_dir() {
            return Err("Windows 挂载目录的上级目录必须已存在".to_string());
        }
        if path.exists() {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| format!("检查 Windows 挂载目录失败：{error}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("Windows 挂载目标必须是未使用的盘符或普通空目录".to_string());
            }
            let mut entries = fs::read_dir(path)
                .map_err(|error| format!("读取 Windows 挂载目录失败：{error}"))?;
            if entries
                .next()
                .transpose()
                .map_err(|error| format!("读取 Windows 挂载目录失败：{error}"))?
                .is_some()
            {
                return Err(
                    "Windows 挂载目录必须为空；为避免覆盖现有文件，程序不会使用非空目录"
                        .to_string(),
                );
            }
            fs::remove_dir(path).map_err(|error| format!("准备 Windows 挂载目录失败：{error}"))?;
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let path = Path::new(target);
        if !path.is_absolute() {
            return Err("挂载目录必须使用绝对路径；Windows 也可填写 X: 形式的盘符".to_string());
        }
        fs::create_dir_all(path).map_err(|error| format!("创建挂载目录失败：{error}"))
    }
}

fn restore_windows_directory_target(_target: &str) {
    #[cfg(windows)]
    if !is_windows_drive(_target) {
        let path = Path::new(_target);
        if path.is_absolute() && !path.exists() {
            let _ = fs::create_dir(path);
        }
    }
}

#[cfg(windows)]
fn is_windows_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn fuse_prerequisite() -> (bool, String) {
    #[cfg(windows)]
    {
        let installed = [
            Path::new(r"C:\Program Files (x86)\WinFsp\bin\winfsp-x64.dll"),
            Path::new(r"C:\Program Files\WinFsp\bin\winfsp-x64.dll"),
            Path::new(r"C:\Program Files (x86)\WinFsp\bin\winfsp-a64.dll"),
            Path::new(r"C:\Program Files\WinFsp\bin\winfsp-a64.dll"),
        ]
        .iter()
        .any(|path| path.is_file());
        return (
            installed,
            if installed {
                "WinFsp 已就绪".to_string()
            } else {
                "原生挂载需要先安装 WinFsp".to_string()
            },
        );
    }
    #[cfg(target_os = "macos")]
    {
        let installed = [
            Path::new("/Library/Filesystems/macfuse.fs"),
            Path::new("/Library/Filesystems/fuse-t.fs"),
        ]
        .iter()
        .any(|path| path.exists());
        return (
            installed,
            if installed {
                "macFUSE/FUSE-T 已就绪".to_string()
            } else {
                "原生挂载需要先安装 macFUSE 或 FUSE-T".to_string()
            },
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let installed = Path::new("/dev/fuse").exists();
        (
            installed,
            if installed {
                "FUSE 设备已就绪".to_string()
            } else {
                "原生挂载需要 fuse3 与可访问的 /dev/fuse".to_string()
            },
        )
    }
}

fn attempt_unmount(_target: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("umount")
            .arg(_target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for helper in ["fusermount3", "fusermount"] {
            if Command::new(helper)
                .args(["-u", _target])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                break;
            }
        }
    }
}

fn read_log_tail(path: &Path) -> String {
    let Ok(content) = fs::read_to_string(path) else {
        return String::new();
    };
    let mut tail = content.lines().rev().take(4).collect::<Vec<_>>();
    tail.reverse();
    tail.join(" | ")
}

fn hide_command_window(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn default_target() -> String {
    if cfg!(windows) {
        "X:".to_string()
    } else {
        env::var("HOME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|home| {
                PathBuf::from(home)
                    .join("Guangya")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| "/mnt/guangya".to_string())
    }
}

fn default_access_mode() -> String {
    "read_write".to_string()
}

fn default_vfs_cache_mode() -> String {
    "full".to_string()
}

fn default_transfers() -> usize {
    4
}

fn default_read_streams() -> usize {
    4
}

fn default_cache_size_gb() -> usize {
    20
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_mount_options_validate_permissions_parallelism_and_cache() {
        let valid = normalize_options(NativeMountOptions::default()).unwrap();
        assert_eq!(valid.access_mode, "read_write");
        assert_eq!(valid.vfs_cache_mode, "full");

        let mut invalid = valid.clone();
        invalid.access_mode = "owner".to_string();
        assert!(normalize_options(invalid).is_err());
        let mut invalid = valid.clone();
        invalid.transfers = 17;
        assert!(normalize_options(invalid).is_err());
        let mut invalid = valid;
        invalid.read_streams = 0;
        assert!(normalize_options(invalid).is_err());
    }

    #[test]
    fn native_mount_arguments_map_read_only_and_parallel_settings() {
        let options = NativeMountOptions {
            target: "X:".to_string(),
            access_mode: "read_only".to_string(),
            transfers: 6,
            read_streams: 3,
            cache_size_gb: 32,
            ..NativeMountOptions::default()
        };
        let arguments = build_mount_arguments(&options, Path::new("cache"), Path::new("mount.log"));
        assert!(arguments.iter().any(|value| value == "--read-only"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--transfers", "6"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--vfs-read-chunk-streams", "3"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--vfs-cache-max-size", "32G"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--dir-cache-time", "5m"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--vfs-cache-max-age", "24h"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--vfs-read-chunk-size", "4M"]));
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_mount_removes_only_an_empty_leaf_and_restores_it() {
        let root = env::temp_dir().join(format!(
            "guangya-native-mount-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let target = root.join("mount");
        fs::create_dir_all(&target).unwrap();

        prepare_target(target.to_str().unwrap()).unwrap();
        assert!(!target.exists());
        restore_windows_directory_target(target.to_str().unwrap());
        assert!(target.is_dir());

        fs::write(target.join("keep.txt"), b"keep").unwrap();
        assert!(prepare_target(target.to_str().unwrap()).is_err());
        assert!(target.join("keep.txt").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
