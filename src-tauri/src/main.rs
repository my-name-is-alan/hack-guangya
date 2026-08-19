#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod native_mount;
mod organizer;
mod organizer_core;
mod virtual_library;
mod webdav;

mod api;
mod app;
mod auth;
mod auto_share;
mod cache;
mod constants;
mod db;
mod developer;
mod download_url_cache;
mod downloads;
mod files;
mod gcid_export;
mod gcid_import;
mod hashes;
mod mappings;
mod mounts;
mod offline;
mod open_file;
mod oss;
mod queue;
mod recycle;
mod settings;
mod shares;
mod state;
mod tasks;
mod telegram;
mod updates;
mod upload_replacement;
mod upload_store;
mod uploads;
mod util;
mod watcher;
mod prelude;

#[cfg(test)]
mod tests;

pub(crate) use prelude::*;

fn main() {
    run();
}
