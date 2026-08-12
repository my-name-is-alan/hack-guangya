//! 全局 prelude：外部依赖导入与各业务模块的统一再导出。
//!
//! 每个业务模块通过 `use crate::prelude::*;` 获得与拆分前单文件相同的可见性。

#![allow(unused_imports)]

pub(crate) use base64::{
    engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
pub(crate) use futures_util::{stream, StreamExt, TryStreamExt};
pub(crate) use hmac::{Hmac, Mac};
pub(crate) use md5::Md5;
pub(crate) use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
pub(crate) use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
pub(crate) use reqwest::header::{
    HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_RANGE, CONTENT_TYPE, DATE, ETAG, RANGE,
};
pub(crate) use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use serde_json::{json, Value};
pub(crate) use sha1::Sha1;
pub(crate) use sha2::{Digest, Sha256, Sha512};
pub(crate) use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, OpenOptions},
    future::Future,
    io::{self, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
};
pub(crate) use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
pub(crate) use tauri_plugin_updater::{Update, UpdaterExt};
pub(crate) use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        watch, Notify, Semaphore,
    },
    time::{sleep, Duration, Instant},
};
pub(crate) use uuid::Uuid;

pub(crate) use crate::{native_mount, organizer, virtual_library, webdav};

pub(crate) use crate::native_mount::{NativeMountInfo, NativeMountManager, NativeMountOptions};
pub(crate) use crate::organizer::{
    add_organizer_mapping, get_organizer_state, rearchive_organizer_job, remove_organizer_job,
    remove_organizer_mapping, retry_organizer_job, run_organizer_job, scan_organizer_mapping,
    scrape_selected_files, start as start_organizer, test_organizer_connection,
    update_organizer_mapping, update_organizer_settings,
};
pub(crate) use crate::virtual_library::{
    VirtualLibraryInfo, VirtualLibraryManager, VirtualLibraryMapping, VirtualLibraryOptions,
};

pub(crate) use crate::api::*;
pub(crate) use crate::app::*;
pub(crate) use crate::auth::*;
pub(crate) use crate::auto_share::*;
pub(crate) use crate::cache::*;
pub(crate) use crate::constants::*;
pub(crate) use crate::db::*;
pub(crate) use crate::developer::*;
pub(crate) use crate::downloads::*;
pub(crate) use crate::files::*;
pub(crate) use crate::gcid_export::*;
pub(crate) use crate::gcid_import::*;
pub(crate) use crate::hashes::*;
pub(crate) use crate::mappings::*;
pub(crate) use crate::mounts::*;
pub(crate) use crate::offline::*;
pub(crate) use crate::oss::*;
pub(crate) use crate::queue::*;
pub(crate) use crate::recycle::*;
pub(crate) use crate::settings::*;
pub(crate) use crate::shares::*;
pub(crate) use crate::state::*;
pub(crate) use crate::tasks::*;
pub(crate) use crate::updates::*;
pub(crate) use crate::upload_replacement::*;
pub(crate) use crate::upload_store::*;
pub(crate) use crate::uploads::*;
pub(crate) use crate::util::*;
pub(crate) use crate::watcher::*;
