//! Tauri 应用装配：初始化、后台任务与命令注册。

use crate::prelude::*;

pub(crate) fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let (event_tx, event_rx) = mpsc::unbounded_channel();
            let config_path = app
                .path()
                .app_config_dir()
                .map_err(|e| e.to_string())?
                .join("config.json");
            let db_path = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("state.sqlite3");
            let native_mount_data_dir = db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
            init_database(&db_path)?;
            let organizer_state = organizer::initialize(app.handle().clone(), db_path.clone())?;
            let mut auth_session = load_auth_session(&db_path)?;
            ensure_auth_account_scope(&db_path, &mut auth_session)?;
            let upload_history = load_upload_history(&db_path)?;
            let pending_cloud = pending_upload_stamps(&db_path)?;
            let device_id = load_or_create_device_id(&db_path)?;
            let cache_policy = CacheSettings {
                enabled: parse_cache_enabled(load_app_state(&db_path, "cache_enabled")?.as_deref()),
                max_entries: parse_cache_max_entries(
                    load_app_state(&db_path, "cache_max_entries")?.as_deref(),
                ),
            };
            save_app_state(&db_path, "cache_enabled", &cache_policy.enabled.to_string())?;
            save_app_state(
                &db_path,
                "cache_max_entries",
                &cache_policy.max_entries.to_string(),
            )?;
            let mut remote_cache = HashMap::from([(String::new(), String::new())]);
            let mut remote_cache_generation = 0;
            apply_cache_policy(
                &db_path,
                &mut remote_cache,
                &mut remote_cache_generation,
                cache_policy,
            )?;
            let hdhive_enabled =
                parse_hdhive_enabled(load_app_state(&db_path, "hdhive_enabled")?.as_deref());
            let raw_hdhive_base_url = std::env::var("HDHIVE_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_base_url")?)
                .unwrap_or_default();
            let hdhive_base_url = normalize_hdhive_base_url(&raw_hdhive_base_url)?;
            let hdhive_secret = std::env::var("HDHIVE_GUANGYA_SYNC_SECRET")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_secret")?)
                .unwrap_or_default();
            let hdhive_instance_id = std::env::var("HDHIVE_GUANGYA_SYNC_INSTANCE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or(load_app_state(&db_path, "hdhive_instance_id")?)
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            save_app_state(&db_path, "hdhive_instance_id", &hdhive_instance_id)?;
            let mut config = load_config(&config_path);
            if let Some(port) = std::env::var("GUANGYA_WEBDAV_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
            {
                if port > 0 {
                    config.webdav_port = port;
                }
            }
            if config.webdav_port == 0 {
                config.webdav_port = DEFAULT_WEBDAV_PORT;
            }
            if config.webdav_username.trim().is_empty() {
                config.webdav_username = default_webdav_username();
            }
            if config.webdav_password.trim().is_empty() {
                config.webdav_password = Uuid::new_v4().simple().to_string();
            }
            let webdav_enabled = config.webdav_enabled;
            let webdav_port = config.webdav_port;
            let webdav_username = config.webdav_username.clone();
            let webdav_password = config.webdav_password.clone();
            let native_mount_options = config.native_mount.clone();
            let virtual_library = VirtualLibraryManager::new(config.virtual_library.clone());
            let strm_sign_secret = load_app_state(&db_path, "strm_sign_secret")?
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    format!(
                        "{}{}",
                        Uuid::new_v4().simple(),
                        Uuid::new_v4().simple()
                    )
                });
            save_app_state(&db_path, "strm_sign_secret", &strm_sign_secret)?;
            let (strm_rebind_tx, strm_rebind_rx) = watch::channel(0_u64);
            let strm_rebind = Arc::new(strm_rebind_tx);
            let upload_concurrency = normalize_transfer_concurrency(
                config.upload_concurrency,
                DEFAULT_UPLOAD_CONCURRENCY,
            );
            let download_concurrency = normalize_transfer_concurrency(
                config.download_concurrency,
                DEFAULT_DOWNLOAD_CONCURRENCY,
            );
            let multipart_part_size = normalize_multipart_part_size(&config.multipart_part_size);
            let mappings = config
                .mappings
                .into_iter()
                .map(|mut mapping| {
                    mapping.sync_types = normalize_sync_types(&mapping.sync_types);
                    mapping.monitor_mode = normalize_monitor_mode(&mapping.monitor_mode);
                    mapping
                })
                .collect::<Vec<_>>();
            let mut resumable_uploads = load_resumable_uploads(&db_path)?;
            resumable_uploads.retain(|item| {
                item.mapping_id.starts_with("__")
                    || mappings
                        .iter()
                        .any(|mapping| mapping.id == item.mapping_id && mapping.enabled)
            });
            let state = Arc::new(Mutex::new(RuntimeState {
                token: auth_session.access_token,
                refresh_token: auth_session.refresh_token,
                auth_account_scope: auth_session.account_scope,
                config_path,
                db_path,
                mappings: mappings.clone(),
                saved_shares: config.saved_shares,
                queue: resumable_uploads,
                flash_preflight_cache: HashMap::new(),
                waiting_files: HashMap::new(),
                history: upload_history,
                pending_cloud,
                recovering_pending: HashSet::new(),
                inflight: HashMap::new(),
                inflight_items: HashMap::new(),
                failed_uploads: HashMap::new(),
                cancelled_uploads: HashMap::new(),
                paused_uploads: HashSet::new(),
                queue_pause_requests: HashSet::new(),
                remote_cache,
                remote_cache_validated_at: HashMap::new(),
                remote_cache_generation,
                remote_cache_gates: Arc::new(RemoteCacheGates::default()),
                upload_replacement_gates: Arc::new(RemoteCacheGates::default()),
                active_upload_replacements: HashSet::new(),
                watchers: HashMap::new(),
                event_tx,
                paused: false,
                active_uploads: 0,
                active_flash_preflights: 0,
                upload_concurrency,
                download_concurrency,
                multipart_part_size,
                cache_enabled: cache_policy.enabled,
                cache_max_entries: cache_policy.max_entries,
                device_id,
                hdhive_enabled,
                hdhive_base_url,
                hdhive_secret,
                hdhive_instance_id,
                auto_share_processing: HashSet::new(),
                gcid_import_running: HashSet::new(),
                developer_transfer_running: HashSet::new(),
                sms_verifications: HashMap::new(),
                webdav_enabled,
                webdav_port,
                webdav_username,
                webdav_password,
                webdav_running: false,
                webdav_error: None,
                native_mount: NativeMountManager::new(
                    native_mount_options,
                    native_mount_data_dir,
                    resource_dir,
                ),
                virtual_library,
                strm_sign_secret,
                strm_rebind,
            }));
            app.manage(DownloadRegistry::default());
            app.manage(PendingAppUpdate::default());
            app.manage(state.clone());
            install_auth_broker(app.handle().clone(), state.clone());
            if let Ok(guard) = state.lock() {
                if let Ok(proxy) = load_global_network_proxy(&guard.db_path) {
                    set_global_api_proxy(&proxy);
                }
            }
            app.manage(organizer_state);
            start_organizer(
                app.handle().clone(),
                app.state::<organizer::OrganizerSharedState>()
                    .inner()
                    .clone(),
            );
            let app_handle = app.handle().clone();
            if webdav_enabled {
                tauri::async_runtime::spawn(webdav::serve(
                    app_handle.clone(),
                    state.clone(),
                    webdav_port,
                ));
            }
            tauri::async_runtime::spawn(virtual_library::serve_strm(
                app_handle.clone(),
                state.clone(),
                strm_rebind_rx,
            ));
            if let Ok(guard) = state.lock() {
                save_config(&guard);
            }
            tauri::async_runtime::spawn(event_loop(app_handle.clone(), state.clone(), event_rx));
            if state
                .lock()
                .ok()
                .and_then(|guard| guard.refresh_token.clone())
                .is_some()
            {
                let refresh_app = app_handle.clone();
                let refresh_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        refresh_saved_session(refresh_app.clone(), refresh_state).await
                    {
                        status(
                            &refresh_app,
                            "warning",
                            format!("已恢复上次登录，但刷新会话失败：{error}"),
                        );
                    }
                });
            }
            for mapping in mappings {
                if mapping.enabled {
                    match install_watcher(&state, &mapping) {
                        Ok(()) => {
                            if mapping.scan_existing {
                                enqueue_existing_files(&app_handle, &state, &mapping);
                            } else if mapping.monitor_mode == "polling" {
                                seed_existing_files(&state, &mapping);
                            }
                        }
                        Err(error) => {
                            if let Ok(mut guard) = state.lock() {
                                if let Some(current) = guard
                                    .mappings
                                    .iter_mut()
                                    .find(|current| current.id == mapping.id)
                                {
                                    current.enabled = false;
                                    current.watch_error = Some(error.clone());
                                }
                                save_config(&guard);
                            }
                            status(
                                &app_handle,
                                "error",
                                format!("备份任务监控启动失败：{}：{}", mapping.local_path, error),
                            );
                        }
                    }
                }
            }
            tauri::async_runtime::spawn(polling_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(pending_upload_recovery_loop(
                app_handle.clone(),
                state.clone(),
            ));
            tauri::async_runtime::spawn(auto_share_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(token_refresh_loop(app_handle.clone(), state.clone()));
            tauri::async_runtime::spawn(offline_name_restore_loop(state.clone()));
            tauri::async_runtime::spawn(virtual_library_refresh_loop(
                app_handle.clone(),
                state.clone(),
            ));
            resume_developer_transfer_jobs(app_handle.clone(), state.clone())?;
            drain_queue(app_handle.clone(), state.clone());
            emit_state(&app_handle, &state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::state::get_state,
            crate::mounts::get_mount_info,
            crate::mounts::update_mount_credentials,
            crate::mounts::get_native_mount_info,
            crate::mounts::update_native_mount_options,
            crate::mounts::start_native_mount,
            crate::mounts::stop_native_mount,
            crate::mounts::select_native_mount_target,
            crate::mounts::select_rclone_binary,
            crate::mounts::get_virtual_library_info,
            crate::mounts::update_virtual_library_settings,
            crate::mounts::upsert_virtual_library_mapping,
            crate::mounts::remove_virtual_library_mapping,
            crate::mounts::sync_virtual_library,
            crate::mounts::select_virtual_library_target,
            crate::auth::clear_expired_session,
            crate::auth::refresh_session,
            crate::auth::start_device_login,
            crate::auth::request_sms_code,
            crate::auth::login_with_sms,
            crate::auth::poll_device_login,
            crate::auth::get_overview,
            crate::auth::get_assets,
            crate::auth::get_global_config,
            crate::files::list_files,
            crate::recycle::list_recycle_files,
            crate::files::search_files,
            crate::files::create_folder,
            crate::files::get_file_detail,
            crate::files::list_recent_actions,
            crate::gcid_export::export_gcid_json,
            crate::gcid_export::export_gcid_diagnostic_log,
            crate::gcid_import::select_gcid_import_file,
            crate::gcid_import::stage_gcid_import_text,
            crate::gcid_import::prepare_gcid_import,
            crate::gcid_import::get_gcid_import_status,
            crate::gcid_import::start_gcid_import,
            crate::queue::select_upload_files,
            crate::queue::select_upload_folder,
            crate::queue::queue_upload_paths,
            crate::queue::pause_upload,
            crate::queue::resume_upload,
            crate::queue::cancel_upload,
            crate::queue::retry_upload,
            crate::files::copy_files,
            crate::files::move_files,
            crate::files::delete_files,
            crate::files::restore_files,
            crate::files::permanently_delete_files,
            crate::recycle::clear_recycle_bin,
            crate::files::batch_rename_files,
            crate::shares::create_share,
            crate::shares::list_shares,
            crate::shares::delete_shares,
            crate::shares::update_share,
            crate::shares::delete_invalid_shares,
            crate::shares::set_direct_link,
            crate::shares::unset_direct_link,
            crate::shares::get_direct_link,
            crate::shares::open_received_share,
            crate::shares::list_received_share_files,
            crate::shares::restore_received_share,
            crate::downloads::get_received_share_download,
            crate::downloads::get_cloud_download,
            crate::downloads::pause_download,
            crate::downloads::resume_download,
            crate::downloads::cancel_download,
            crate::offline::resolve_offline_resource,
            crate::offline::create_offline_task,
            crate::offline::list_offline_tasks,
            crate::offline::delete_offline_tasks,
            crate::offline::cancel_offline_tasks,
            crate::offline::retry_offline_tasks,
            crate::offline::get_offline_statistics,
            crate::offline::get_offline_settings,
            crate::offline::update_offline_settings,
            crate::shares::save_share_link,
            crate::shares::remove_share_link,
            crate::auth::open_login,
            crate::auth::capture_token,
            crate::mappings::select_folder,
            crate::mappings::add_mapping,
            crate::mappings::remove_mapping,
            crate::mappings::toggle_mapping,
            crate::mappings::update_mapping_sync_types,
            crate::mappings::update_mapping_monitor_mode,
            crate::mappings::update_mapping_auto_share,
            crate::mappings::update_mapping_organizer,
            crate::auto_share::update_hdhive_config,
            crate::auto_share::backfill_auto_shares,
            crate::auto_share::retry_auto_share_event,
            crate::queue::pause_queue,
            crate::settings::get_transfer_settings,
            crate::settings::update_transfer_settings,
            crate::settings::get_network_preferences,
            crate::settings::update_network_preferences,
            crate::settings::test_network,
            crate::cache::get_cache_settings,
            crate::cache::update_cache_settings,
            crate::cache::get_metadata_cache_stats,
            crate::cache::clear_metadata_cache,
            crate::developer::get_developer_settings,
            crate::developer::update_developer_credentials,
            crate::developer::test_developer_credentials,
            crate::developer::update_developer_mode,
            crate::developer::upsert_developer_target,
            crate::developer::delete_developer_target,
            crate::developer::list_developer_transfers,
            crate::developer::start_developer_transfer,
            crate::organizer::get_organizer_state,
            crate::organizer::update_organizer_settings,
            crate::organizer::test_organizer_connection,
            crate::organizer::add_organizer_mapping,
            crate::organizer::update_organizer_mapping,
            crate::organizer::remove_organizer_mapping,
            crate::organizer::remove_organizer_job,
            crate::organizer::scan_organizer_mapping,
            crate::organizer::run_organizer_job,
            crate::organizer::retry_organizer_job,
            crate::organizer::rearchive_organizer_job,
            crate::organizer::share_organizer_job,
            crate::organizer::scrape_selected_files,
            crate::queue::resume_queue,
            crate::updates::get_app_version,
            crate::updates::fetch_app_update,
            crate::updates::install_app_update
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app_handle.state::<SharedState>();
                if let Ok(mut guard) = state.lock() {
                    guard.native_mount.shutdown();
                };
            }
        });
}
