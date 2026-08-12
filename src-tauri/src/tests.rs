//! 单元测试（原 main.rs 内联测试模块）。

    use super::*;

    #[test]
    fn precise_remote_cache_invalidation_only_touches_affected_branches() {
        let mut cache = HashMap::from([
            (String::new(), String::new()),
            (remote_directory_cache_key("root", "电影"), "dir-a".to_string()),
            (remote_directory_cache_key("dir-a", "2024"), "dir-b".to_string()),
            (remote_directory_cache_key("dir-b", "深层"), "dir-c".to_string()),
            (remote_directory_cache_key("root", "剧集"), "dir-x".to_string()),
            (remote_directory_cache_key("dir-x", "S01"), "dir-y".to_string()),
        ]);
        let mut validated = cache
            .keys()
            .map(|key| (key.clone(), Instant::now()))
            .collect::<HashMap<_, _>>();

        // 删除 dir-a：应清掉 root→电影 的映射（value 命中）与 dir-a 的子映射
        // （parent 命中），不影响"剧集"分支。
        let removed = invalidate_remote_directory_children(
            &mut cache,
            &mut validated,
            &HashSet::new(),
            &["dir-a".to_string()],
        );
        assert!(removed);
        assert!(!cache.contains_key(&remote_directory_cache_key("root", "电影")));
        assert!(!cache.contains_key(&remote_directory_cache_key("dir-a", "2024")));
        assert!(cache.contains_key(&remote_directory_cache_key("root", "剧集")));
        assert!(cache.contains_key(&remote_directory_cache_key("dir-x", "S01")));
        assert!(validated.contains_key(&remote_directory_cache_key("dir-x", "S01")));
        assert!(!validated.contains_key(&remote_directory_cache_key("dir-a", "2024")));

        // 目标目录 dir-x 有新内容：按 parent 前缀清除其直接子映射。
        let removed = invalidate_remote_directory_children(
            &mut cache,
            &mut validated,
            &HashSet::from(["dir-x".to_string()]),
            &[],
        );
        assert!(removed);
        assert!(!cache.contains_key(&remote_directory_cache_key("dir-x", "S01")));
        assert!(cache.contains_key(&remote_directory_cache_key("root", "剧集")));

        // 无关目录不产生任何移除。
        let removed = invalidate_remote_directory_children(
            &mut cache,
            &mut validated,
            &HashSet::from(["unrelated".to_string()]),
            &[],
        );
        assert!(!removed);
    }

    #[test]
    fn endpoint_idempotency_classifies_reads_and_mutations() {
        for endpoint in [
            "/userres/v1/file/get_file_list",
            "/userres/v1/file/get_file_detail",
            "/userres/v1/file/search_files",
            "/userres/v1/get_share_list",
            "/userres/v1/get_task_status",
            "/userres/v1/check_can_flash_upload",
            "/cloudcollection/v1/list_task",
            "/cloudcollection/v1/resolve_res",
            "/developer/v1/upload_status",
            "/scheduler/v1/query_packaging_task",
            "/misc/v1/get_global_config",
            "/assets/v1/get_assets",
        ] {
            assert_eq!(
                endpoint_idempotency(endpoint),
                Idempotency::Read,
                "{endpoint} 应视为只读"
            );
        }
        for endpoint in [
            "/userres/v1/file/create_dir",
            "/userres/v1/file/copy_file",
            "/userres/v1/file/move_file",
            "/userres/v1/file/rename",
            "/userres/v1/file/delete_file",
            "/userres/v1/file/recycle_file",
            "/userres/v1/file/clear_recycle_bin",
            "/userres/v1/share_file",
            "/userres/v1/update_share",
            "/userres/v1/delete_share",
            "/cloudcollection/v1/create_task",
            "/cloudcollection/v2/delete_task",
            "/developer/v1/upload_by_fileid",
            "/developer/v1/pre_upload",
            "/scheduler/v1/create_packaging_task",
            "/unknown/v1/anything",
        ] {
            assert_eq!(
                endpoint_idempotency(endpoint),
                Idempotency::Mutation,
                "{endpoint} 应视为写操作"
            );
        }
    }

    #[test]
    fn retry_policy_backoff_grows_and_respects_the_cap() {
        let policy = RetryPolicy::READ;
        let first = policy.backoff(0).as_millis() as u64;
        // 首次退避在 base ± 25% 抖动范围内。
        assert!(
            (policy.base_delay_ms * 3 / 4..=policy.base_delay_ms * 5 / 4).contains(&first),
            "首次退避 {first}ms 超出抖动窗口"
        );
        // 高次退避不会超过上限 + 抖动。
        for attempt in 0..20 {
            let delay = policy.backoff(attempt).as_millis() as u64;
            assert!(
                delay <= policy.max_delay_ms * 5 / 4,
                "第 {attempt} 次退避 {delay}ms 超过上限"
            );
        }
        // 写策略比读策略更保守：尝试次数更少。
        assert!(RetryPolicy::MUTATION.attempts < RetryPolicy::READ.attempts);
    }

    #[test]
    fn developer_signature_hashes_binary_md5_with_sha512() {
        assert_eq!(
            developer_signature(
                "developer-client",
                "developer-secret",
                "0123456789abcdef",
                1_700_000_000,
            ),
            "217fb5d9f8a9b7c9c65e307cda0dea4f893b5e553e231f148b9b710a609d3aa643a78574605c1f9bdff14e267811ed04bec5f4e5674a67f81493c5c818d885ac"
        );
    }

    #[test]
    fn developer_account_binding_reads_supported_profile_ids() {
        assert_eq!(
            account_id_from_profile(&json!({ "data": { "user": { "user_id": "user-1" } } })),
            Some("user-1".to_string())
        );
        assert_eq!(
            account_id_from_profile(&json!({ "profile": { "id": 42 } })),
            Some("42".to_string())
        );
        assert_eq!(
            account_id_from_profile(&json!({ "nickname": "no-id" })),
            None
        );
    }

    #[test]
    fn business_headers_follow_the_live_windows_api_profile() {
        let headers = business_api_headers("access-token", "0123456789abcdef0123456789abcdef")
            .expect("headers should be valid");
        assert_eq!(headers.get("dt").unwrap(), "5");
        assert_eq!(headers.get("av").unwrap(), "1.0.2");
        assert_eq!(headers.get("vc").unwrap(), "1002");
        assert_eq!(headers.get("x-client-id").unwrap(), OAUTH_CLIENT_ID);
        assert_eq!(
            headers.get("x-device-id").unwrap(),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(headers.get("user-agent").unwrap(), API_USER_AGENT);
        assert!(business_auth_expired(200, 110));
        assert!(business_auth_expired(200, 117));
        assert!(business_auth_expired(200, 118));
        assert!(business_auth_expired(401, 0));
        assert!(!business_auth_expired(200, 112));
    }

    #[test]
    fn upload_credentials_follow_the_official_web_expiration_contract() {
        let token = |expiration: Option<&str>| UploadToken {
            task_id: "task-1".to_string(),
            object_path: Some("objects/file.bin".to_string()),
            bucket_name: Some("bucket".to_string()),
            end_point: Some("oss.example.test".to_string()),
            full_end_point: None,
            creds: Some(UploadCredentials {
                access_key_id: "access".to_string(),
                secret_access_key: "secret".to_string(),
                session_token: "session".to_string(),
                expiration: expiration.map(str::to_string),
            }),
            provider: Some(json!(1)),
        };
        assert!(upload_credentials_expired(&token(Some(
            "2000-01-01T00:00:00Z"
        ))));
        assert!(!upload_credentials_expired(&token(Some(
            "2100-01-01T00:00:00Z"
        ))));
        assert!(upload_credentials_expired(&token(None)));
        assert!(is_oss_security_token_expired(
            "<Code>SecurityTokenExpired</Code>"
        ));
        assert!(!is_oss_security_token_expired(
            "error sending request for url"
        ));
    }

    #[test]
    fn account_headers_follow_the_same_pc_device_profile() {
        let device_id = "0123456789abcdef0123456789abcdef";
        let headers = account_api_headers(device_id, Some("account-access-token"))
            .expect("account headers should be valid");
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(headers.get("accept").unwrap(), "application/json");
        assert_eq!(headers.get("x-client-id").unwrap(), OAUTH_CLIENT_ID);
        assert_eq!(headers.get("x-device-id").unwrap(), device_id);
        assert_eq!(headers.get("x-client-version").unwrap(), API_APP_VERSION);
        assert_eq!(headers.get("x-sdk-version").unwrap(), "9.0.2");
        assert_eq!(headers.get("x-protocol-version").unwrap(), "301");
        assert_eq!(headers.get("accept-language").unwrap(), "zh-CN");
        assert_eq!(headers.get("user-agent").unwrap(), API_USER_AGENT);
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap(),
            "Bearer account-access-token"
        );
        assert!(account_api_headers(device_id, None)
            .expect("anonymous account headers should be valid")
            .get(AUTHORIZATION)
            .is_none());
    }

    #[test]
    fn business_response_rejects_contradictory_failure_messages() {
        assert_eq!(
            parse_api_response(r#"{"msg":"success","data":{}}"#, 200, "/test")
                .expect("success message should be accepted")
                .code,
            0
        );
        assert_eq!(
            parse_api_response(r#"{"code":0,"msg":"OK","data":{}}"#, 200, "/test")
                .expect("code zero with an explicit success message should be accepted")
                .code,
            0
        );
        assert_eq!(
            parse_api_response(r#"{"data":{}}"#, 200, "/test")
                .expect("legacy data-only responses stay compatible")
                .code,
            0
        );
        assert_eq!(
            parse_api_response(r#"{"code":0,"data":{}}"#, 200, "/test")
                .expect("an explicit zero code is a success signal")
                .code,
            0
        );
        assert!(parse_api_response(r#"{"code":0,"msg":"参数错误"}"#, 200, "/test").is_err());
        assert!(parse_api_response(r#"{"msg":"参数错误"}"#, 200, "/test").is_err());
        assert_eq!(
            parse_api_response(r#"{"code":112,"msg":"参数错误"}"#, 200, "/test")
                .expect("non-zero business responses must remain inspectable")
                .code,
            112
        );
        assert!(parse_api_response("", 200, "/test").is_err());
    }

    #[test]
    fn file_management_payloads_match_the_official_pc_contract() {
        assert_eq!(
            recycle_file_list_request(Some(3)),
            json!({
                "page": 3,
                "pageSize": 100,
                "parentId": "",
                "dirType": 4,
                "orderBy": 12,
                "sortType": 1
            })
        );
        assert_eq!(
            clear_recycle_bin_request(),
            ("/userres/v1/file/clear_recycle_bin", json!({}))
        );
        assert_eq!(
            create_folder_request(" root-id ", " 新建目录 ", None).unwrap(),
            json!({ "parentId": "root-id", "dirName": "新建目录" })
        );
        assert_eq!(
            create_folder_request("", "目录", Some(true)).unwrap(),
            json!({ "parentId": "", "dirName": "目录", "failIfNameExist": true })
        );
        assert!(create_folder_request("", "../坏目录", None).is_err());
        assert_eq!(
            file_detail_request(" file-1 ").unwrap(),
            json!({ "fileId": "file-1" })
        );
        assert_eq!(
            recent_actions_request(None, None, None, None).unwrap(),
            json!({ "cursor": "", "pageSize": 20 })
        );
        assert_eq!(
            recent_actions_request(
                Some(" opaque-cursor "),
                Some(50),
                Some(&[1, 2, 1]),
                Some(&[4, 5, 4])
            )
            .unwrap(),
            json!({
                "cursor": " opaque-cursor ",
                "pageSize": 50,
                "fileTypes": [1, 2],
                "excludeFileTypes": [4, 5]
            })
        );
        assert!(recent_actions_request(Some("bad\ncursor"), None, None, None).is_err());
        assert!(recent_actions_request(None, Some(0), None, None).is_err());
        assert!(recent_actions_request(None, None, Some(&[12]), None).is_err());
    }

    #[test]
    fn recycle_bin_clear_planner_never_reposts_unknown_without_explicit_force() {
        assert_eq!(
            plan_recycle_bin_clear(None, false),
            RecycleBinClearAction::Submit
        );
        assert_eq!(
            plan_recycle_bin_clear(
                Some(&RecycleBinClearOperation::Task {
                    task_id: "clear-task-1".to_string(),
                    updated_at: 1,
                }),
                false,
            ),
            RecycleBinClearAction::ResumeTask
        );
        for updated_at in [i64::MIN / 2, 1, 950, i64::MAX / 2] {
            let unknown = RecycleBinClearOperation::Unknown { updated_at };
            assert_eq!(
                plan_recycle_bin_clear(Some(&unknown), false),
                RecycleBinClearAction::ProtectUnknown,
                "unknown state must never age into an automatic repost"
            );
            assert_eq!(
                plan_recycle_bin_clear(Some(&unknown), true),
                RecycleBinClearAction::Submit,
                "only explicit force_retry may clear unknown and submit again"
            );
        }
    }

    #[test]
    fn recycle_bin_clear_state_is_persistent_and_account_scoped() {
        let root = std::env::temp_dir().join(format!(
            "guangya-recycle-clear-state-{}",
            Uuid::new_v4().simple()
        ));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize recycle clear database");
        let account_a = "session:account-a".to_string();
        let account_b = "session:account-b".to_string();
        assert_ne!(account_a, account_b);

        save_recycle_bin_clear_operation(
            &database,
            &account_a,
            &RecycleBinClearOperation::Unknown { updated_at: 10 },
        )
        .expect("save unknown marker");
        assert_eq!(
            load_recycle_bin_clear_operation(&database, &account_a).expect("load unknown marker"),
            Some(RecycleBinClearOperation::Unknown { updated_at: 10 })
        );
        assert_eq!(
            load_recycle_bin_clear_operation(&database, &account_b)
                .expect("load other account state"),
            None
        );

        save_recycle_bin_clear_operation(
            &database,
            &account_a,
            &RecycleBinClearOperation::Task {
                task_id: "clear-task-1".to_string(),
                updated_at: 20,
            },
        )
        .expect("promote unknown marker to task");
        assert_eq!(
            load_recycle_bin_clear_operation(&database, &account_a).expect("load task marker"),
            Some(RecycleBinClearOperation::Task {
                task_id: "clear-task-1".to_string(),
                updated_at: 20,
            })
        );
        clear_recycle_bin_operation(&database, &account_a).expect("clear completed marker");
        assert_eq!(
            load_recycle_bin_clear_operation(&database, &account_a).expect("load cleared marker"),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recycle_bin_clear_scope_survives_token_refresh_for_the_same_jwt_subject() {
        let make_token = |claims: Value, signature: &str| {
            let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
            let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
            format!("{header}.{payload}.{signature}")
        };
        let old_token = make_token(
            json!({ "iss": "https://account.guangyapan.com", "sub": "account-1", "exp": 1 }),
            "old-signature",
        );
        let refreshed_token = make_token(
            json!({ "iss": "https://account.guangyapan.com", "sub": "account-1", "exp": 2 }),
            "new-signature",
        );
        let other_account_token = make_token(
            json!({ "iss": "https://account.guangyapan.com", "sub": "account-2", "exp": 2 }),
            "other-signature",
        );
        let account_scope = new_auth_account_scope(&old_token);
        assert_eq!(account_scope, new_auth_account_scope(&refreshed_token));
        assert_ne!(account_scope, new_auth_account_scope(&other_account_token));

        let root = std::env::temp_dir().join(format!(
            "guangya-recycle-refresh-scope-{}",
            Uuid::new_v4().simple()
        ));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize refreshed token state database");
        save_recycle_bin_clear_operation(
            &database,
            &account_scope,
            &RecycleBinClearOperation::Task {
                task_id: "clear-task-before-refresh".to_string(),
                updated_at: 10,
            },
        )
        .expect("persist task before token refresh");
        let refreshed_operation =
            load_recycle_bin_clear_operation(&database, &new_auth_account_scope(&refreshed_token))
                .expect("load task after token refresh");
        assert_eq!(
            plan_recycle_bin_clear(refreshed_operation.as_ref(), false),
            RecycleBinClearAction::ResumeTask
        );
        assert_eq!(
            load_recycle_bin_clear_operation(
                &database,
                &new_auth_account_scope(&other_account_token),
            )
            .expect("load isolated account state"),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn opaque_access_token_refresh_keeps_session_scope_and_explicit_login_isolates_accounts() {
        let root = std::env::temp_dir().join(format!(
            "guangya-opaque-auth-scope-{}",
            Uuid::new_v4().simple()
        ));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize opaque auth scope database");
        let first_scope = new_auth_account_scope("opaque-access-token-a");
        replace_auth_session(
            &database,
            Some("opaque-access-token-a"),
            Some("opaque-refresh-token"),
            Some(&first_scope),
        )
        .expect("persist explicit opaque login");
        save_recycle_bin_clear_operation(
            &database,
            &first_scope,
            &RecycleBinClearOperation::Task {
                task_id: "clear-task-before-opaque-refresh".to_string(),
                updated_at: 10,
            },
        )
        .expect("persist clear task for opaque login");

        save_auth_session(&database, Some("opaque-access-token-refreshed"), None)
            .expect("refresh only access token");
        let refreshed = load_auth_session(&database).expect("load refreshed opaque login");
        assert_eq!(
            refreshed.account_scope.as_deref(),
            Some(first_scope.as_str())
        );
        assert_eq!(
            load_recycle_bin_clear_operation(
                &database,
                refreshed
                    .account_scope
                    .as_deref()
                    .expect("persisted account scope"),
            )
            .expect("load clear task after opaque refresh"),
            Some(RecycleBinClearOperation::Task {
                task_id: "clear-task-before-opaque-refresh".to_string(),
                updated_at: 10,
            })
        );

        let second_scope = new_auth_account_scope("opaque-access-token-b");
        assert_ne!(first_scope, second_scope);
        replace_auth_session(
            &database,
            Some("opaque-access-token-b"),
            Some("other-refresh-token"),
            Some(&second_scope),
        )
        .expect("persist explicit account switch");
        let switched = load_auth_session(&database).expect("load switched account");
        assert_eq!(
            switched.account_scope.as_deref(),
            Some(second_scope.as_str())
        );
        assert_eq!(
            load_recycle_bin_clear_operation(&database, &second_scope)
                .expect("new account has isolated clear state"),
            None
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recycle_bin_task_status_only_completes_on_terminal_success() {
        assert_eq!(
            classify_recycle_bin_task_status(&json!({ "status": 1 })),
            RecycleBinTaskStatus::Pending
        );
        assert_eq!(
            classify_recycle_bin_task_status(&json!({
                "status": 2,
                "detail": { "code": 0 }
            })),
            RecycleBinTaskStatus::Succeeded
        );
        assert_eq!(
            classify_recycle_bin_task_status(&json!({
                "status": 2,
                "detail": { "code": 500, "msg": "clear failed" }
            })),
            RecycleBinTaskStatus::Failed("clear failed".to_string())
        );
    }

    #[test]
    fn initial_recycle_bin_clear_response_classification_preserves_ambiguous_posts() {
        use InitialRecycleBinClearResponseClass::{Accepted, Ambiguous, DefinitiveRejection};

        let cases = [
            (200, 0, Accepted, "normal success"),
            (
                408,
                0,
                Ambiguous,
                "request timeout may follow cloud acceptance",
            ),
            (429, 0, Ambiguous, "rate limit may follow cloud acceptance"),
            (
                500,
                0,
                Ambiguous,
                "gateway failure may follow cloud acceptance",
            ),
            (
                503,
                0,
                Ambiguous,
                "service failure may follow cloud acceptance",
            ),
            (200, 100, Ambiguous, "temporary business failure"),
            (200, 103, Ambiguous, "temporary business failure"),
            (200, 18010, Ambiguous, "explicit busy business response"),
            (400, 0, DefinitiveRejection, "invalid request"),
            (403, 0, DefinitiveRejection, "permission rejection"),
            (
                200,
                9001,
                DefinitiveRejection,
                "permanent business rejection",
            ),
            (401, 0, DefinitiveRejection, "HTTP authentication failure"),
            (
                200,
                110,
                DefinitiveRejection,
                "business authentication failure",
            ),
        ];
        for (http_status, business_code, expected, label) in cases {
            assert_eq!(
                classify_initial_recycle_bin_clear_response(http_status, business_code),
                expected,
                "{label}: HTTP {http_status}, business code {business_code}"
            );
        }
    }

    #[tokio::test]
    async fn recycle_bin_clear_singleflight_shares_one_operation_result() {
        let account_scope = format!("singleflight-{}", Uuid::new_v4().simple());
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let second_calls = Arc::clone(&calls);
        let first = run_recycle_bin_clear_singleflight(&account_scope, || async move {
            first_calls.fetch_add(1, Ordering::Relaxed);
            sleep(Duration::from_millis(25)).await;
            Ok(json!({ "completed": true, "source": "first" }))
        });
        let second = run_recycle_bin_clear_singleflight(&account_scope, || async move {
            second_calls.fetch_add(1, Ordering::Relaxed);
            Ok(json!({ "completed": true, "source": "second" }))
        });

        let (first_result, second_result) = futures_util::future::join(first, second).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            first_result.expect("first clear result"),
            json!({ "completed": true, "source": "first" })
        );
        assert_eq!(
            second_result.expect("joined clear result"),
            json!({ "completed": true, "source": "first" })
        );
    }

    #[tokio::test]
    async fn opaque_token_refresh_during_clear_stays_in_the_same_singleflight() {
        let root = std::env::temp_dir().join(format!(
            "guangya-opaque-clear-flight-{}",
            Uuid::new_v4().simple()
        ));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize opaque clear flight database");
        let account_scope = new_auth_account_scope("opaque-access-token-before-refresh");
        replace_auth_session(
            &database,
            Some("opaque-access-token-before-refresh"),
            Some("opaque-refresh-token"),
            Some(&account_scope),
        )
        .expect("persist opaque login before clear");

        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let second_calls = Arc::clone(&calls);
        let first_scope = account_scope.clone();
        let first = async move {
            run_recycle_bin_clear_singleflight(&first_scope, || async move {
                first_calls.fetch_add(1, Ordering::Relaxed);
                sleep(Duration::from_millis(30)).await;
                Ok(json!({ "completed": true, "source": "before-refresh" }))
            })
            .await
        };
        let second = async {
            sleep(Duration::from_millis(5)).await;
            save_auth_session(&database, Some("opaque-access-token-after-refresh"), None)
                .expect("persist refreshed access token during clear");
            let refreshed = load_auth_session(&database).expect("reload refreshed session");
            let refreshed_scope = refreshed.account_scope.expect("scope survives refresh");
            run_recycle_bin_clear_singleflight(&refreshed_scope, || async move {
                second_calls.fetch_add(1, Ordering::Relaxed);
                Ok(json!({ "completed": true, "source": "after-refresh" }))
            })
            .await
        };

        let (first_result, second_result) = futures_util::future::join(first, second).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            first_result.expect("first clear result"),
            json!({ "completed": true, "source": "before-refresh" })
        );
        assert_eq!(
            second_result.expect("refreshed clear joins existing result"),
            json!({ "completed": true, "source": "before-refresh" })
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn api_id_lists_are_trimmed_deduplicated_and_bounded() {
        assert_eq!(
            normalize_id_list(
                &[
                    " file-1 ".to_string(),
                    "file-2".to_string(),
                    "file-1".to_string()
                ],
                "文件"
            )
            .unwrap(),
            vec!["file-1".to_string(), "file-2".to_string()]
        );
        assert!(normalize_id_list(&[], "文件").is_err());
        assert!(normalize_id_list(&["   ".to_string()], "文件").is_err());
        assert!(normalize_id_list(&["bad\nid".to_string()], "文件").is_err());
        assert_eq!(
            operation_task_id(&json!({ "taskId": 12345 })).as_deref(),
            Some("12345")
        );
        assert_eq!(
            operation_task_id(&json!("task-1")).as_deref(),
            Some("task-1")
        );
        assert!(operation_task_id(&json!({})).is_none());
    }

    #[test]
    fn share_update_and_direct_link_payloads_match_the_official_pc_contract() {
        assert_eq!(
            update_share_request(" share-1 ", 604_800, 1, &json!(2048)).unwrap(),
            json!({
                "id": "share-1",
                "validateDuration": 604800,
                "downloadType": 1,
                "trafficLimit": "2048"
            })
        );
        assert_eq!(
            update_share_request("share-1", 0, 0, &json!(" 0 ")).unwrap()["trafficLimit"],
            json!("0")
        );
        assert!(update_share_request("share-1", -1, 1, &json!(0)).is_err());
        assert!(update_share_request("share-1", 86_400, 2, &json!(0)).is_err());
        assert!(update_share_request("share-1", 86_400, 1, &json!(1.5)).is_err());
        assert!(
            update_share_request("share-1", 0, 0, &json!(MAX_SHARE_TRAFFIC_BYTES + 1)).is_err()
        );
        assert_eq!(
            direct_link_file_request(" file-1 ").unwrap(),
            json!({ "fileId": "file-1" })
        );
        assert_eq!(
            get_direct_link_request("file-1", true).unwrap(),
            json!({ "fileId": "file-1", "shortLink": true })
        );
        assert_eq!(
            delete_shares_request(&[
                " share-1 ".to_string(),
                "share-2".to_string(),
                "share-1".to_string()
            ])
            .unwrap(),
            json!({ "ids": ["share-1", "share-2"] })
        );
    }

    #[test]
    fn registered_tauri_commands_and_acl_stay_in_sync() {
        use std::collections::BTreeSet;

        let source = include_str!("app.rs");
        let marker = ".invoke_handler(tauri::generate_handler![";
        let registry = source
            .split_once(marker)
            .expect("invoke handler registry must exist")
            .1
            .split_once("])")
            .expect("invoke handler registry must terminate")
            .0;
        let registered = registry
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.rsplit("::").next().unwrap_or(value).to_string())
            .collect::<Vec<_>>();

        let permissions = include_str!("../permissions/app.toml");
        let mut allowed = Vec::new();
        let mut remaining = permissions;
        while let Some((_, after_key)) = remaining.split_once("commands.allow") {
            let (_, after_open) = after_key
                .split_once('[')
                .expect("commands.allow must be an array");
            let (array, after_close) = after_open
                .split_once(']')
                .expect("commands.allow array must terminate");
            let mut quoted = array.split('"');
            while quoted.next().is_some() {
                let Some(command) = quoted.next() else {
                    break;
                };
                if !command.trim().is_empty() {
                    allowed.push(command.to_string());
                }
            }
            remaining = after_close;
        }

        let registered_set = registered.iter().cloned().collect::<BTreeSet<_>>();
        let allowed_set = allowed.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            registered.len(),
            registered_set.len(),
            "duplicate command registration"
        );
        assert_eq!(
            allowed.len(),
            allowed_set.len(),
            "duplicate command ACL entry"
        );
        assert_eq!(registered_set, allowed_set);
    }

    #[test]
    fn webdav_credentials_require_safe_explicit_values() {
        assert_eq!(
            normalize_webdav_username("  mount-user  ").unwrap(),
            "mount-user"
        );
        assert!(normalize_webdav_username("ab").is_err());
        assert!(normalize_webdav_username("bad:user").is_err());
        assert!(normalize_webdav_password("short").is_err());
        assert_eq!(
            normalize_webdav_password("correct horse battery staple").unwrap(),
            "correct horse battery staple"
        );
    }

    #[test]
    fn default_mapping_syncs_media_only() {
        let mapping: Mapping = serde_json::from_value(json!({
            "id": "mapping-1",
            "local_path": "C:/watch",
            "remote_path": "",
            "enabled": true
        }))
        .expect("mapping should deserialize");

        assert_eq!(mapping.sync_types, default_sync_types());
        assert!(!mapping.auto_share);
        assert!(should_sync(Path::new("photo.HEIC"), &mapping.sync_types));
        assert!(should_sync(Path::new("movie.mkv"), &mapping.sync_types));
        assert!(should_sync(Path::new("sound.flac"), &mapping.sync_types));
        assert!(!should_sync(Path::new("notes.pdf"), &mapping.sync_types));
    }

    #[test]
    fn auto_share_uses_sync_root_first_level() {
        let root_file = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: PathBuf::from("C:/watch/movie.mkv"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "movie.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 1,
            modified_ms: 1,
            replacement: None,
        };
        let episode = UploadItem {
            relative_path: "tvname/season 1/s01.mkv".to_string(),
            file_path: PathBuf::from("C:/watch/tvname/season 1/s01.mkv"),
            ..root_file.clone()
        };
        let next_season = UploadItem {
            relative_path: "tvname/season 2/s02.mkv".to_string(),
            file_path: PathBuf::from("C:/watch/tvname/season 2/s02.mkv"),
            ..root_file.clone()
        };
        let file_target = auto_share_target(&root_file).expect("root file target");
        assert_eq!(file_target.key, "movie.mkv");
        assert_eq!(file_target.target_type, "file");
        let episode_target = auto_share_target(&episode).expect("episode target");
        assert_eq!(episode_target.key, "tvname");
        assert_eq!(episode_target.target_type, "folder");
        assert_eq!(auto_share_target(&next_season).unwrap().key, "tvname");
    }

    #[test]
    fn auto_share_waits_for_pending_cloud_files_in_the_same_target() {
        let root =
            std::env::temp_dir().join(format!("guangya-auto-share-pending-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("state.sqlite3");
        init_database(&database).unwrap();
        let item = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: root.join("children").join("episode-02.mkv"),
            remote_parent_id: String::new(),
            remote_dir: "children".to_string(),
            relative_path: "children/episode-02.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 1024,
            modified_ms: 123,
            replacement: None,
        };
        save_upload_record(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-pending".to_string(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .unwrap();

        assert!(target_has_pending_cloud(&database, "mapping-1", "children").unwrap());
        assert!(!target_has_pending_cloud(&database, "mapping-1", "another-folder").unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hdhive_hmac_matches_node_and_backend() {
        assert_eq!(
            hdhive_signature(
                "secret",
                "post",
                "/api/integrations/guangya-sync/events",
                r#"{"a":1}"#,
                "1700000000",
            ),
            "v1=83db0943a113d8cdd5786f9447ebf125c764a64fb935b577f43aae6a2a8c5c5d"
        );
    }

    #[test]
    fn share_file_payload_matches_official_web_contract() {
        let payload = share_file_payload(&["file-1".to_string()], "测试分享", 0, "", false);

        assert_eq!(
            payload,
            json!({
                "fileIds": ["file-1"],
                "title": "测试分享",
                "validateDuration": 0,
                "shareType": 0,
                "code": "",
                "autoFillCode": false,
                "trafficLimit": "0",
                "maxRestoreCount": 0,
                "downloadType": 1,
                "shareTemplate": DEFAULT_SHARE_TEMPLATE
            })
        );
        assert_eq!(
            share_file_payload(&["file-1".to_string()], "   ", 0, "", false)["title"],
            "云盘分享"
        );
        assert_eq!(
            share_file_payload(&["file-1".to_string()], "私密分享", 2, "a1B2", false)["code"],
            "a1B2"
        );
        assert!(normalize_share_access(Some(2), Some("bad"), None).is_err());
    }

    #[test]
    fn parses_guangya_share_links_with_access_codes() {
        let parsed = parse_guangya_share_link(
            "https://www.guangyapan.com/s/1926585463106830337_al8cmYXLP9l33ld2?code=iv5k#/share",
        )
        .unwrap();
        assert_eq!(parsed.0, "1926585463106830337_al8cmYXLP9l33ld2");
        assert_eq!(parsed.1, "iv5k");
        assert!(parse_guangya_share_link("https://example.com/s/share-1").is_err());
    }

    #[test]
    fn uses_the_official_gcid_chunk_boundaries() {
        assert_eq!(gcid_chunk_size(128 * 1024 * 1024), 256 * 1024);
        assert_eq!(gcid_chunk_size(128 * 1024 * 1024 + 1), 512 * 1024);
        assert_eq!(gcid_chunk_size(256 * 1024 * 1024), 512 * 1024);
        assert_eq!(gcid_chunk_size(256 * 1024 * 1024 + 1), 1024 * 1024);
        assert_eq!(gcid_chunk_size(512 * 1024 * 1024 + 1), 2 * 1024 * 1024);
    }

    #[test]
    fn manual_share_event_is_a_new_hdhive_submission() {
        let share_data = json!({ "shareId": "1927007413038006365" });
        assert_eq!(
            share_id_for_hdhive(
                &share_data,
                "https://www.guangyapan.com/s/1927007413038006365_al3JUAaZz30d4FPe"
            ),
            "1927007413038006365_al3JUAaZz30d4FPe"
        );
        let payload = manual_share_event_payload(
            "00000000-0000-4000-8000-000000000001",
            &["folder-1".to_string()],
            "测试电视剧",
            "folder",
            "share-1",
            "https://www.guangyapan.com/s/share-1",
            "new",
        );

        assert_eq!(payload["mapping_id"], "__manual__");
        assert_eq!(payload["target_key"], "测试电视剧");
        assert_eq!(payload["target_type"], "folder");
        assert_eq!(payload["remote_target_id"], "folder-1");
        assert_eq!(payload["share_id"], "share-1");
        assert_eq!(payload["intent"], "new");

        let update_payload = manual_share_event_payload(
            "00000000-0000-4000-8000-000000000002",
            &["folder-1".to_string()],
            "测试电视剧",
            "folder",
            "share-1",
            "https://www.guangyapan.com/s/share-1",
            "update",
        );
        assert_eq!(update_payload["intent"], "update");
    }

    #[test]
    fn selected_sync_types_use_direct_extensions() {
        let selected = vec![".xlsx".to_string(), "srt".to_string(), "sqlite".to_string()];

        assert!(should_sync(Path::new("report.xlsx"), &selected));
        assert!(should_sync(Path::new("movie.srt"), &selected));
        assert!(should_sync(Path::new("database.sqlite"), &selected));
        assert!(!should_sync(Path::new("cover.jpg"), &selected));
    }

    #[test]
    fn directory_watch_events_expand_to_nested_syncable_files() {
        let root = std::env::temp_dir().join(format!("guangya-folder-event-{}", Uuid::new_v4()));
        let nested = root.join("season 1");
        fs::create_dir_all(&nested).expect("create nested fixture");
        fs::write(nested.join("episode-01.mp4"), b"video-1").expect("write first video");
        fs::write(nested.join("episode-02.mkv"), b"video-2").expect("write second video");
        fs::write(nested.join("notes.txt"), b"ignored").expect("write ignored fixture");

        let mut files = collect_watch_event_files(&root, &["mp4".to_string(), "mkv".to_string()]);
        files.sort();

        assert_eq!(
            files,
            vec![nested.join("episode-01.mp4"), nested.join("episode-02.mkv")]
        );
        fs::remove_dir_all(root).expect("remove directory event fixture");
    }

    #[test]
    fn invalid_or_empty_sync_types_fall_back_to_media() {
        assert_eq!(normalize_sync_types(&[]), default_sync_types());
        assert_eq!(
            normalize_sync_types(&["bad/name".to_string()]),
            default_sync_types()
        );
        assert_eq!(normalize_sync_types(&[".MP4".to_string()]), vec!["mp4"]);
    }

    #[test]
    fn duplicate_native_events_do_not_queue_an_inflight_file_again() {
        let item = UploadItem {
            mapping_id: "mapping-1".to_string(),
            file_path: PathBuf::from("C:/watch/photo.png"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "photo.png".to_string(),
            change_kind: "added".to_string(),
            size: 128,
            modified_ms: 42,
            replacement: None,
        };
        let history = HashMap::new();
        let mut pending_cloud = HashMap::new();
        let mut inflight = HashMap::new();
        let queue = VecDeque::new();
        let mut waiting_files = HashMap::new();
        let mut cancelled_uploads = HashMap::new();
        assert!(!upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &item
        ));

        inflight.insert(
            item_key(&item.mapping_id, &item.file_path),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &item
        ));

        let mut changed = item.clone();
        changed.modified_ms += 1;
        assert!(!upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &changed
        ));

        cancelled_uploads.insert(
            item_key(&item.mapping_id, &item.file_path),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &item
        ));
        assert!(!upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &changed
        ));

        inflight.clear();
        pending_cloud.insert(
            item_key(&item.mapping_id, &item.file_path),
            Stamp {
                size: item.size,
                modified_ms: item.modified_ms,
            },
        );
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &changed
        ));
        pending_cloud.clear();
        waiting_files.insert(item_key(&item.mapping_id, &item.file_path), item.clone());
        assert!(upload_already_scheduled(
            &history,
            &pending_cloud,
            &inflight,
            &queue,
            &waiting_files,
            &cancelled_uploads,
            &item
        ));
    }

    #[cfg(windows)]
    #[test]
    fn detects_a_file_exclusively_opened_by_another_program() {
        use std::os::windows::fs::OpenOptionsExt;

        let path = std::env::temp_dir().join(format!("guangya-locked-{}.tmp", Uuid::new_v4()));
        fs::write(&path, b"locked").expect("write fixture");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .expect("hold fixture exclusively");
        assert!(!file_available_for_upload(&path).expect("probe locked file"));
        drop(held);
        assert!(file_available_for_upload(&path).expect("probe released file"));
        fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn detects_source_growth_after_the_upload_snapshot() {
        let path =
            std::env::temp_dir().join(format!("guangya-growing-source-{}.tmp", Uuid::new_v4()));
        fs::write(&path, b"partial").expect("write fixture");
        let metadata = fs::metadata(&path).expect("read fixture metadata");
        let item = UploadItem {
            mapping_id: "mapping".into(),
            file_path: path.clone(),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "growing.tmp".into(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
            replacement: None,
        };
        assert!(!source_changed_since_upload(&item));
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b" remainder"))
            .expect("grow fixture");
        assert!(source_changed_since_upload(&item));
        fs::remove_file(path).expect("remove fixture");
    }

    #[test]
    fn upload_replacement_stages_the_old_file_before_promoting_the_new_version() {
        let replacement = UploadReplacement {
            old_file_id: "old".into(),
            original_name: "movie.mkv".into(),
            temporary_name: ".__gy_replace_new".into(),
            backup_name: ".__gy_replace_backup_old".into(),
            previous_size: 10,
            previous_modified_ms: 20,
        };
        assert_eq!(
            upload_replacement_state(
                &[
                    ReplacementRemoteEntry {
                        id: "old".into(),
                        name: "movie.mkv".into(),
                    },
                    ReplacementRemoteEntry {
                        id: "new".into(),
                        name: replacement.temporary_name.clone(),
                    },
                ],
                &replacement,
                "new",
            ),
            UploadReplacementState::StageOld
        );
        assert_eq!(
            upload_replacement_state(
                &[
                    ReplacementRemoteEntry {
                        id: "old".into(),
                        name: replacement.backup_name.clone(),
                    },
                    ReplacementRemoteEntry {
                        id: "new".into(),
                        name: replacement.temporary_name.clone(),
                    },
                ],
                &replacement,
                "new",
            ),
            UploadReplacementState::PromoteNew { old_exists: true }
        );
    }

    #[test]
    fn upload_replacement_refuses_to_overwrite_an_external_cloud_change() {
        let replacement = UploadReplacement {
            old_file_id: "old".into(),
            original_name: "movie.mkv".into(),
            temporary_name: ".__gy_replace_new".into(),
            backup_name: ".__gy_replace_backup_old".into(),
            previous_size: 10,
            previous_modified_ms: 20,
        };
        assert_eq!(
            upload_replacement_state(
                &[
                    ReplacementRemoteEntry {
                        id: "old".into(),
                        name: replacement.backup_name.clone(),
                    },
                    ReplacementRemoteEntry {
                        id: "new".into(),
                        name: replacement.temporary_name.clone(),
                    },
                    ReplacementRemoteEntry {
                        id: "external".into(),
                        name: "movie.mkv".into(),
                    },
                ],
                &replacement,
                "new",
            ),
            UploadReplacementState::Conflict
        );
    }

    #[test]
    fn cancelled_pending_replacement_restores_the_previous_confirmed_record() {
        let root =
            std::env::temp_dir().join(format!("guangya-upload-replacement-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create replacement fixture");
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize replacement database");
        let item = UploadItem {
            mapping_id: "mapping-1".into(),
            file_path: root.join("movie.mkv"),
            remote_parent_id: "parent".into(),
            remote_dir: String::new(),
            relative_path: "movie.mkv".into(),
            change_kind: "changed".into(),
            size: 20,
            modified_ms: 30,
            replacement: Some(UploadReplacement {
                old_file_id: "old-file".into(),
                original_name: "movie.mkv".into(),
                temporary_name: ".__gy_replace_new".into(),
                backup_name: ".__gy_replace_backup_old".into(),
                previous_size: 10,
                previous_modified_ms: 20,
            }),
        };
        save_upload_record(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-new".into(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .expect("save pending replacement");
        let pending = load_pending_uploads(&database)
            .expect("load pending replacement")
            .pop()
            .expect("pending replacement exists");
        assert!(delete_pending_upload(&database, &pending).expect("restore prior record"));
        let row = open_database(&database)
            .unwrap()
            .query_row(
                "SELECT size, modified_ms, remote_file_id, upload_state, replacement_json FROM uploaded_files",
                [],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, 10);
        assert_eq!(row.1, "20");
        assert_eq!(row.2, "old-file");
        assert_eq!(row.3, UPLOAD_STATE_CLOUD_CONFIRMED);
        assert_eq!(row.4, None);
        fs::remove_dir_all(root).expect("remove replacement fixture");
    }

    #[test]
    fn changed_upload_uses_a_persisted_unique_remote_name() {
        let root = std::env::temp_dir().join(format!(
            "guangya-upload-replacement-hydrate-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create replacement hydration fixture");
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize replacement hydration database");
        let file_path = root.join("movie.mkv");
        let original = UploadItem {
            mapping_id: "mapping-1".into(),
            file_path: file_path.clone(),
            remote_parent_id: "parent".into(),
            remote_dir: "movies".into(),
            relative_path: "movie.mkv".into(),
            change_kind: "added".into(),
            size: 10,
            modified_ms: 20,
            replacement: None,
        };
        save_upload_record(
            &database,
            &original,
            &UploadOutcome {
                task_id: "old-task".into(),
                remote_file_id: Some("old-file".into()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("save original confirmed upload");
        let mut changed = UploadItem {
            change_kind: "changed".into(),
            size: 20,
            modified_ms: 30,
            ..original
        };
        hydrate_upload_replacement(&database, &mut changed).expect("hydrate replacement");
        let replacement = changed.replacement.as_ref().expect("replacement exists");
        assert_eq!(replacement.old_file_id, "old-file");
        assert_eq!(replacement.previous_size, 10);
        assert_eq!(replacement.previous_modified_ms, 20);
        assert!(replacement.temporary_name.starts_with(".__gy_replace_"));
        assert_eq!(
            upload_remote_name(&changed).unwrap(),
            replacement.temporary_name
        );
        fs::remove_dir_all(root).expect("remove replacement hydration fixture");
    }

    #[test]
    fn monitor_mode_defaults_to_native_and_accepts_polling() {
        assert_eq!(normalize_monitor_mode(""), "native");
        assert_eq!(normalize_monitor_mode("local"), "native");
        assert_eq!(normalize_monitor_mode("POLLING"), "polling");
    }

    #[test]
    fn oss_parameters_are_normalized_for_the_rust_client() {
        assert_eq!(
            normalize_oss_endpoint_url(
                "https://bucket.oss-cn-shanghai.aliyuncs.com/path",
                "bucket"
            ),
            "https://oss-cn-shanghai.aliyuncs.com"
        );
        assert_eq!(
            normalize_oss_endpoint_url("http://oss-cn-hangzhou.aliyuncs.com", "bucket"),
            "http://oss-cn-hangzhou.aliyuncs.com"
        );
    }

    #[test]
    fn upload_token_preserves_numeric_provider_values() {
        let token: UploadToken = serde_json::from_value(json!({
            "taskId": "task-1",
            "objectPath": "objects/video.mkv",
            "bucketName": "bucket",
            "endPoint": "https://oss-cn-shanghai.aliyuncs.com",
            "provider": 1,
            "creds": {
                "accessKeyID": "access-key",
                "secretAccessKey": "secret-key",
                "sessionToken": "security-token"
            }
        }))
        .expect("numeric provider should deserialize");

        assert_eq!(token.provider, Some(json!(1)));
    }

    #[test]
    fn oss_signature_uses_security_token_and_multipart_subresource() {
        let checkpoint = OssUploadCheckpoint {
            task_id: "task-1".into(),
            object_path: "folder/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss-cn-shanghai.aliyuncs.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-1".into(),
            part_size: OSS_MIB,
            completed_parts: BTreeMap::new(),
        };
        assert_eq!(
            oss_string_to_sign(
                "PUT",
                "Sun, 26 Jul 2026 12:00:00 GMT",
                "security-token",
                &checkpoint,
                Some("partNumber=2&uploadId=upload-1")
            ),
            "PUT\n\n\nSun, 26 Jul 2026 12:00:00 GMT\nx-oss-security-token:security-token\n/bucket/folder/video.mkv?partNumber=2&uploadId=upload-1"
        );
    }

    #[test]
    fn upload_checkpoint_persists_parts_and_is_restored_after_restart() {
        let root =
            std::env::temp_dir().join(format!("guangya-upload-checkpoint-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create upload checkpoint fixture");
        let database = root.join("state.sqlite3");
        let file_path = root.join("video.mkv");
        fs::write(&file_path, b"video-content").expect("write upload checkpoint fixture");
        let metadata = fs::metadata(&file_path).expect("read upload checkpoint fixture metadata");
        let item = UploadItem {
            mapping_id: "__manual__".into(),
            file_path: file_path.clone(),
            remote_parent_id: "parent-1".into(),
            remote_dir: String::new(),
            relative_path: String::new(),
            change_kind: "added".into(),
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
            replacement: None,
        };
        init_database(&database).expect("initialize upload checkpoint database");
        let checkpoint = OssUploadCheckpoint {
            task_id: "task-1".into(),
            object_path: "objects/video.mkv".into(),
            bucket_name: "bucket".into(),
            end_point: "https://oss-cn-shanghai.aliyuncs.com".into(),
            provider: Some("oss".into()),
            upload_id: "upload-1".into(),
            part_size: 5,
            completed_parts: BTreeMap::from([(1, "\"etag-1\"".into())]),
        };
        save_upload_checkpoint(&database, &item, &checkpoint, 5).expect("save upload checkpoint");

        let loaded = load_upload_checkpoint(&database, &item)
            .expect("load upload checkpoint")
            .expect("upload checkpoint should exist");
        assert_eq!(loaded.uploaded_bytes, 5);
        assert_eq!(
            loaded.checkpoint.completed_parts.get(&1).unwrap(),
            "\"etag-1\""
        );
        assert_eq!(
            load_resumable_uploads(&database)
                .expect("restore upload checkpoints")
                .len(),
            1
        );

        fs::write(&file_path, b"changed-video-content").expect("change upload fixture");
        assert!(load_resumable_uploads(&database)
            .expect("clean stale upload checkpoints")
            .is_empty());
        fs::remove_dir_all(root).expect("remove upload checkpoint fixture");
    }

    #[test]
    fn multipart_part_size_uses_safe_tiers_and_stays_below_the_oss_part_limit() {
        assert_eq!(oss_part_size(100 * 1024 * 1024), 1024 * 1024);
        assert_eq!(oss_part_size(100 * 1024 * 1024 + 1), 2 * 1024 * 1024);
        assert_eq!(oss_part_size(1024 * 1024 * 1024), 2 * 1024 * 1024);
        assert_eq!(oss_part_size(1024 * 1024 * 1024 + 1), 4 * 1024 * 1024);
        assert_eq!(oss_part_size(10 * 1024 * 1024 * 1024), 4 * 1024 * 1024);
        assert_eq!(
            oss_part_size(10 * 1024 * 1024 * 1024 + 1),
            OSS_LARGE_FILE_PART_SIZE
        );

        let failed_file_size = 96_220_456_048;
        let part_size = oss_part_size(failed_file_size);
        assert_eq!(part_size, OSS_LARGE_FILE_PART_SIZE);
        assert_eq!(ceil_div_u64(failed_file_size, part_size), 5_736);
        assert!(ceil_div_u64(failed_file_size, part_size) <= OSS_MULTIPART_TARGET_PARTS);

        let tier_boundary = OSS_LARGE_FILE_PART_SIZE * OSS_MULTIPART_TARGET_PARTS;
        assert_eq!(oss_part_size(tier_boundary), OSS_LARGE_FILE_PART_SIZE);
        assert_eq!(
            oss_part_size(tier_boundary + 1),
            OSS_LARGE_FILE_PART_SIZE + OSS_MIB
        );

        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "auto"), OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "4m"), 4 * OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "8m"), 8 * OSS_MIB);
        assert_eq!(configured_oss_part_size(100 * OSS_MIB, "16m"), 16 * OSS_MIB);
        for tier in MULTIPART_PART_SIZE_OPTIONS {
            let configured = configured_oss_part_size(u64::MAX / 2, tier);
            assert!(ceil_div_u64(u64::MAX / 2, configured) <= OSS_MULTIPART_TARGET_PARTS);
            assert!(ceil_div_u64(u64::MAX / 2, configured) <= 10_000);
        }
    }

    #[test]
    fn multipart_part_size_validation_accepts_only_supported_tiers() {
        for tier in MULTIPART_PART_SIZE_OPTIONS {
            assert_eq!(validate_multipart_part_size(tier).unwrap(), *tier);
        }
        assert_eq!(validate_multipart_part_size(" 8M ").unwrap(), "8m");
        for invalid in ["", "1m", "2m", "32m", "custom"] {
            assert!(validate_multipart_part_size(invalid).is_err());
        }
        assert_eq!(normalize_multipart_part_size("invalid"), "auto");
    }

    #[test]
    fn transfer_concurrency_defaults_and_bounds_are_stable() {
        let config: AppConfig = serde_json::from_str("{}").expect("deserialize defaults");
        assert_eq!(config.upload_concurrency, DEFAULT_UPLOAD_CONCURRENCY);
        assert_eq!(config.download_concurrency, DEFAULT_DOWNLOAD_CONCURRENCY);
        assert_eq!(config.multipart_part_size, DEFAULT_MULTIPART_PART_SIZE);
        assert!(parse_cache_enabled(None));
        assert!(!parse_cache_enabled(Some("false")));
        assert_eq!(parse_cache_max_entries(None), DEFAULT_CACHE_MAX_ENTRIES);
        assert_eq!(
            parse_cache_max_entries(Some("99")),
            DEFAULT_CACHE_MAX_ENTRIES
        );
        assert_eq!(parse_cache_max_entries(Some("100")), 100);
        assert_eq!(parse_cache_max_entries(Some("100000")), 100_000);
        assert!(parse_hdhive_enabled(None));
        assert!(parse_hdhive_enabled(Some("true")));
        assert!(!parse_hdhive_enabled(Some("false")));
        assert_eq!(
            normalize_transfer_concurrency(0, DEFAULT_UPLOAD_CONCURRENCY),
            DEFAULT_UPLOAD_CONCURRENCY
        );
        assert_eq!(
            normalize_transfer_concurrency(MAX_TRANSFER_CONCURRENCY, DEFAULT_UPLOAD_CONCURRENCY),
            MAX_TRANSFER_CONCURRENCY
        );
    }

    #[test]
    fn hdhive_base_url_rejects_unsafe_parts_and_normalizes_paths() {
        let unrestricted = HashSet::new();
        assert_eq!(
            normalize_hdhive_base_url_with_allowed_hosts(
                "  https://Example.COM/integration///  ",
                &unrestricted,
            )
            .unwrap(),
            "https://example.com/integration"
        );
        assert_eq!(
            normalize_hdhive_base_url_with_allowed_hosts("", &unrestricted).unwrap(),
            ""
        );
        for unsafe_url in [
            "ftp://example.com",
            "https://user:secret@example.com",
            "https://example.com?next=https://evil.example",
            "https://example.com/#fragment",
        ] {
            assert!(
                normalize_hdhive_base_url_with_allowed_hosts(unsafe_url, &unrestricted).is_err(),
                "unsafe URL should be rejected: {unsafe_url}"
            );
        }
    }

    #[test]
    fn hdhive_base_url_honors_the_optional_host_allowlist() {
        let host_only = HashSet::from(["api.example.com".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:8443/root",
            &host_only,
        )
        .is_ok());
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://other.example.com/root",
            &host_only,
        )
        .is_err());

        let host_and_port = HashSet::from(["api.example.com:8443".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:8443/root",
            &host_and_port,
        )
        .is_ok());
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "https://api.example.com:9443/root",
            &host_and_port,
        )
        .is_err());

        let ipv6_host_and_port = HashSet::from(["[::1]:8080".to_string()]);
        assert!(normalize_hdhive_base_url_with_allowed_hosts(
            "http://[::1]:8080/root",
            &ipv6_host_and_port,
        )
        .is_ok());
        let ipv6_host = HashSet::from(["[::1]".to_string()]);
        assert!(
            normalize_hdhive_base_url_with_allowed_hosts("http://[::1]:8080/root", &ipv6_host,)
                .is_ok()
        );
    }

    #[test]
    fn hdhive_target_url_appends_only_a_structured_path() {
        let (target, signature_path) = build_hdhive_target_url(
            "https://api.example.com/integration",
            &["api", "guangya-sync", "events"],
        )
        .unwrap();
        assert_eq!(
            target.as_str(),
            "https://api.example.com/integration/api/guangya-sync/events"
        );
        assert_eq!(signature_path, "/api/guangya-sync/events");
        for unsafe_segment in [
            "event/id",
            r"event\id",
            ".",
            "..",
            "event?redirect=evil",
            "event#fragment",
        ] {
            assert!(build_hdhive_target_url(
                "https://api.example.com",
                &["api", "events", unsafe_segment],
            )
            .is_err());
        }
    }

    #[test]
    fn file_hash_cache_is_reused_only_for_an_unchanged_file_stamp() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-cache-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create gcid cache test root");
        let database = root.join("state.sqlite3");
        let file = root.join("movie.mkv");
        init_database(&database).expect("initialize gcid cache database");
        fs::write(&file, b"fixture").expect("write gcid fixture");
        let hashes = FileHashes {
            gcid: "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
            cid: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string(),
        };
        let policy = CacheSettings {
            enabled: true,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        };

        assert_eq!(
            load_cached_file_hashes(&database, &file, 7, 100, policy).expect("load empty cache"),
            None
        );
        save_cached_file_hashes(&database, &file, 7, 100, &hashes, policy)
            .expect("save hash cache");
        assert_eq!(
            load_cached_file_hashes(&database, &file, 7, 100, policy).expect("load cached hashes"),
            Some(hashes.clone())
        );
        assert_eq!(
            load_cached_file_hashes(&database, &file, 7, 101, policy)
                .expect("reject changed mtime"),
            None
        );
        assert_eq!(
            load_cached_file_hashes(&database, &file, 8, 100, policy).expect("reject changed size"),
            None
        );

        fs::remove_dir_all(root).expect("remove gcid cache test root");
    }

    #[test]
    fn gcid_export_snapshot_cache_is_account_scoped_and_preserves_root_aggregate_signature() {
        let root = std::env::temp_dir().join(format!(
            "guangya-gcid-export-snapshot-test-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("create export snapshot test root");
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize export snapshot database");
        let roots = vec![CloudSelectionEntry {
            file_id: "folder-1".to_string(),
            name: "媒体库".to_string(),
            folder: true,
            size: 0,
            gcid: String::new(),
            modified_at: 100,
            subtree_size: Some(2048),
            subtree_folders: Some(3),
            subtree_files: Some(7),
            ancestor_ids: Vec::new(),
            path: String::new(),
        }];
        let signatures = gcid_export_root_signatures(&roots);
        let export = GeneratedGcidExport {
            script_version: "guangya-gcid-export-2.0".to_string(),
            export_version: "2.0".to_string(),
            source: "guangya".to_string(),
            hash_type: "gcid".to_string(),
            uses_gcid_in_export: true,
            uses_cid_in_export: true,
            uses_base62_etags_in_export: false,
            common_path: "媒体库".to_string(),
            source_folder_id: "folder-1".to_string(),
            source_folder_name: "媒体库".to_string(),
            total_files_count: 1,
            total_size: Value::from(2048),
            formatted_total_size: "2.00 KB".to_string(),
            generated_at: 100,
            scanned_folders_count: 4,
            skipped_files_count: 0,
            skipped_files: Vec::new(),
            files: vec![GeneratedGcidExportFile {
                path: "movie.mkv".to_string(),
                size: "2048".to_string(),
                gcid: "0123456789abcdef0123456789abcdef01234567".to_string(),
                cid: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string(),
            }],
        };
        let key = gcid_export_selection_key(&["folder-1".to_string()]);
        save_gcid_export_snapshot(&database, "account-a", &key, &signatures, &export)
            .expect("save export snapshot");

        let cached = load_gcid_export_snapshot(&database, "account-a", &key)
            .expect("load export snapshot")
            .expect("snapshot exists");
        assert_eq!(cached.root_signatures, signatures);
        assert_eq!(cached.export.total_files_count, 1);
        assert_eq!(cached.root_signatures[0].subtree_files, Some(7));
        assert!(load_gcid_export_snapshot(&database, "account-b", &key)
            .expect("load isolated account")
            .is_none());
        save_gcid_export_file_hash(
            &database,
            "account-a",
            "file-1",
            2048,
            "0123456789abcdef0123456789abcdef01234567",
            "89ABCDEF0123456789ABCDEF0123456789ABCDEF",
        )
        .expect("save cloud export hash");
        assert_eq!(
            load_gcid_export_file_hash(
                &database,
                "account-a",
                "file-1",
                2048,
                "0123456789abcdef0123456789abcdef01234567",
            )
            .expect("load cloud export hash"),
            Some("89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string())
        );
        assert_eq!(
            load_gcid_export_file_hash(
                &database,
                "account-a",
                "file-1",
                2049,
                "0123456789abcdef0123456789abcdef01234567",
            )
            .expect("reject changed cloud file"),
            None
        );
        save_cached_file_hashes(
            &database,
            &root.join("locally-hashed.mkv"),
            4096,
            1,
            &FileHashes {
                gcid: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                cid: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string(),
            },
            CacheSettings {
                enabled: true,
                max_entries: DEFAULT_CACHE_MAX_ENTRIES,
            },
        )
        .expect("save existing local fingerprint");
        assert_eq!(
            load_gcid_export_file_hash(
                &database,
                "account-a",
                "cloud-file-2",
                4096,
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            .expect("reuse existing local fingerprint"),
            Some("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string())
        );

        fs::remove_dir_all(root).expect("remove export snapshot test root");
    }

    #[test]
    fn gcid_export_inventory_rebuilds_paths_from_full_parent_ids() {
        let root = cloud_selection_entry_from_value(
            &json!({
                "fileInfo": { "fileId": "root", "fileName": "媒体库", "resType": 2, "utime": 10 },
                "sizeInfo": { "subDirCount": 900, "subFileCount": 1, "size": 2048 }
            }),
            "",
            "",
        )
        .expect("parse selected root");
        let folder = cloud_selection_entry_from_value(
            &json!({
                "fileId": "year", "fileName": "2026", "resType": 2,
                "fullParentIds": "root/"
            }),
            "",
            "",
        )
        .expect("parse indexed folder");
        let file = cloud_selection_entry_from_value(
            &json!({
                "fileId": "movie", "fileName": "电影.mkv", "resType": 1,
                "fileSize": 2048, "gcid": "0123456789ABCDEF0123456789ABCDEF01234567",
                "fullParentIds": "root/year/"
            }),
            "",
            "",
        )
        .expect("parse indexed file");
        let folder_names = HashMap::from([(folder.file_id.clone(), folder.name.clone())]);

        assert!(should_use_gcid_export_inventory(std::slice::from_ref(
            &root
        )));
        assert_eq!(
            gcid_export_inventory_path(&file, &[root], &folder_names),
            Some("媒体库/2026/电影.mkv".to_string())
        );
    }

    #[test]
    fn metadata_cache_policy_persists_disables_and_bounds_each_cache() {
        let root =
            std::env::temp_dir().join(format!("guangya-cache-policy-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create cache policy test root");
        let database = root.join("state.sqlite3");
        init_database(&database).expect("initialize cache policy database");
        let enabled = CacheSettings {
            enabled: true,
            max_entries: MIN_CACHE_MAX_ENTRIES,
        };
        let hashes = FileHashes {
            gcid: "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
            cid: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string(),
        };

        for index in 0..105_u64 {
            save_cached_file_hashes(
                &database,
                &root.join(format!("cached-{index}.bin")),
                index + 1,
                u128::from(index),
                &hashes,
                enabled,
            )
            .expect("save bounded fingerprint");
        }
        assert_eq!(
            open_database(&database)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM file_fingerprints", [], |row| row
                    .get::<_, usize>(0))
                .unwrap(),
            MIN_CACHE_MAX_ENTRIES
        );

        let mut remote_cache = HashMap::from([(String::new(), String::new())]);
        let mut remote_cache_generation = 0;
        for index in 0..105 {
            remote_cache.insert(format!("root::{index}"), format!("folder-{index}"));
        }
        let bounded = apply_cache_policy(
            &database,
            &mut remote_cache,
            &mut remote_cache_generation,
            enabled,
        )
        .expect("apply enabled cache policy");
        assert_eq!(bounded.file_fingerprints_entries, 100);
        assert_eq!(bounded.remote_cache_entries, 100);
        assert_eq!(bounded.policy, enabled);
        assert_eq!(remote_cache.get(""), Some(&String::new()));

        let disabled = CacheSettings {
            enabled: false,
            max_entries: MIN_CACHE_MAX_ENTRIES,
        };
        let disabled_path = root.join("disabled.bin");
        save_cached_file_hashes(&database, &disabled_path, 1, 1, &hashes, disabled)
            .expect("disabled cache write is a no-op");
        assert!(
            load_cached_file_hashes(&database, &disabled_path, 1, 1, disabled)
                .expect("disabled cache read is a no-op")
                .is_none()
        );
        let cleared = apply_cache_policy(
            &database,
            &mut remote_cache,
            &mut remote_cache_generation,
            disabled,
        )
        .expect("disable and clear cache");
        assert_eq!(cleared.entries, 0);
        assert_eq!(cleared.policy, disabled);
        assert_eq!(
            remote_cache,
            HashMap::from([(String::new(), String::new())])
        );

        save_app_state(&database, "cache_enabled", &disabled.enabled.to_string())
            .expect("persist cache switch");
        save_app_state(
            &database,
            "cache_max_entries",
            &disabled.max_entries.to_string(),
        )
        .expect("persist cache limit");
        assert!(!parse_cache_enabled(
            load_app_state(&database, "cache_enabled")
                .unwrap()
                .as_deref()
        ));
        assert_eq!(
            parse_cache_max_entries(
                load_app_state(&database, "cache_max_entries")
                    .unwrap()
                    .as_deref()
            ),
            MIN_CACHE_MAX_ENTRIES
        );
        assert!(validate_cache_max_entries(MIN_CACHE_MAX_ENTRIES - 1).is_err());
        assert!(validate_cache_max_entries(MAX_CACHE_MAX_ENTRIES + 1).is_err());

        fs::remove_dir_all(root).expect("remove cache policy test root");
    }

    #[test]
    fn remote_directory_edge_cache_reconciles_only_complete_parent_snapshots() {
        let parent = "parent-a";
        let current_key = remote_directory_cache_key(parent, "当前目录");
        let stale_key = remote_directory_cache_key(parent, "已删除目录");
        let unrelated_key = remote_directory_cache_key("parent-b", "其它目录");
        let mut cache = HashMap::from([
            (String::new(), String::new()),
            (stale_key.clone(), "stale-id".to_string()),
            (unrelated_key.clone(), "other-id".to_string()),
        ]);

        assert!(reconcile_remote_directory_cache_entries(
            &mut cache,
            parent,
            0,
            &json!({
                "list": [{ "fileId": "current-id", "fileName": "当前目录", "resType": 2 }],
                "total": 1,
            }),
        ));
        assert_eq!(
            cache.get(&current_key).map(String::as_str),
            Some("current-id")
        );
        assert!(!cache.contains_key(&stale_key));
        assert!(cache.contains_key(&unrelated_key));

        cache.insert(stale_key.clone(), "new-stale-id".to_string());
        assert!(reconcile_remote_directory_cache_entries(
            &mut cache,
            parent,
            0,
            &json!({
                "list": [{ "fileId": "current-id-2", "fileName": "当前目录", "resType": 2 }],
                "total": 200,
            }),
        ));
        assert_eq!(
            cache.get(&current_key).map(String::as_str),
            Some("current-id-2")
        );
        assert!(cache.contains_key(&stale_key));

        let mut generation = 7;
        reset_remote_cache(&mut cache, &mut generation);
        assert_eq!(generation, 8);
        assert_eq!(cache, HashMap::from([(String::new(), String::new())]));
    }

    #[test]
    fn remote_directory_gates_coalesce_same_edge_without_blocking_other_edges() {
        let gates = RemoteCacheGates::default();
        let first = gates.gate("7\0parent\0folder");
        let same = gates.gate("7\0parent\0folder");
        let other = gates.gate("7\0parent\0other");
        let next_generation = gates.gate("8\0parent\0folder");
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &other));
        assert!(!Arc::ptr_eq(&first, &next_generation));
    }

    #[test]
    fn remote_folder_pagination_uses_seen_count_and_handles_missing_total() {
        assert!(!remote_folder_page_complete(50, 50, Some(150)));
        assert!(!remote_folder_page_complete(100, 50, Some(150)));
        assert!(remote_folder_page_complete(150, 50, Some(150)));
        assert!(!remote_folder_page_complete(100, 100, None));
        assert!(remote_folder_page_complete(150, 50, None));
        assert!(remote_folder_page_complete(100, 0, Some(200)));
    }

    #[test]
    fn metadata_cache_clear_preserves_upload_records_files_and_root_mapping() {
        let root =
            std::env::temp_dir().join(format!("guangya-cache-clear-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create metadata cache test root");
        let database = root.join("state.sqlite3");
        let file = root.join("movie.mkv");
        init_database(&database).expect("initialize metadata cache database");
        fs::write(&file, b"fixture").expect("write cache fixture");
        let policy = CacheSettings {
            enabled: true,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        };
        let hashes = FileHashes {
            gcid: "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
            cid: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string(),
        };
        save_cached_file_hashes(&database, &file, 7, 100, &hashes, policy)
            .expect("save fingerprint cache");
        let upload = UploadItem {
            mapping_id: "mapping-cache-test".to_string(),
            file_path: file.clone(),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "movie.mkv".to_string(),
            change_kind: "added".to_string(),
            size: 7,
            modified_ms: 100,
            replacement: None,
        };
        save_upload_record(
            &database,
            &upload,
            &UploadOutcome {
                task_id: "task-cache-test".to_string(),
                remote_file_id: Some("file-cache-test".to_string()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("save upload record");
        let mut remote_cache = HashMap::from([
            (String::new(), String::new()),
            ("root::Movies".to_string(), "folder-1".to_string()),
        ]);
        let mut remote_cache_generation = 0;
        let before =
            metadata_cache_stats(&database, &remote_cache, policy).expect("read cache stats");
        assert_eq!(before.file_fingerprints_entries, 1);
        assert_eq!(before.remote_cache_entries, 1);
        assert_eq!(before.entries, 2);
        assert!(before.bytes > 0);

        let after = clear_metadata_cache_storage(
            &database,
            &mut remote_cache,
            &mut remote_cache_generation,
            policy,
        )
        .expect("clear metadata cache");
        assert_eq!(after.entries, 0);
        assert_eq!(after.bytes, 0);
        assert_eq!(
            remote_cache,
            HashMap::from([(String::new(), String::new())])
        );
        assert!(file.exists());
        assert!(load_cached_file_hashes(&database, &file, 7, 100, policy)
            .expect("read cleared fingerprint")
            .is_none());
        assert!(load_upload_history(&database)
            .expect("load preserved upload history")
            .contains_key(&item_key(&upload.mapping_id, &upload.file_path)));

        fs::remove_dir_all(root).expect("remove metadata cache test root");
    }

    #[test]
    fn search_filters_use_cloud_item_suffixes_on_the_current_page() {
        let folder = json!({ "fileName": "相册", "resType": 2 });
        let image = json!({ "fileName": "封面.JPG", "fileSuffix": ".JPG", "resType": 1 });
        let video = json!({ "fileName": "电影.MKV", "resType": "1" });
        let document = json!({ "fileName": "说明.pdf", "resType": 1 });
        let archive = json!({ "fileName": "备份.7z", "resType": 1 });

        assert!(cloud_item_matches_search_filters(
            &folder,
            Some("folder"),
            None
        ));
        assert!(!cloud_item_matches_search_filters(
            &folder,
            Some("image"),
            None
        ));
        assert!(cloud_item_matches_search_filters(
            &image,
            Some("image"),
            Some("jpg")
        ));
        assert!(cloud_item_matches_search_filters(
            &video,
            Some("video"),
            Some("mkv")
        ));
        assert!(cloud_item_matches_search_filters(
            &document,
            Some("document"),
            None
        ));
        assert!(cloud_item_matches_search_filters(
            &archive,
            Some("archive"),
            Some("7z")
        ));
        assert!(!cloud_item_matches_search_filters(
            &archive,
            Some("document"),
            None
        ));
        assert_eq!(normalize_search_file_type(Some("ALL")).unwrap(), None);
        assert!(normalize_search_file_type(Some("executable")).is_err());
        assert_eq!(
            normalize_search_extension(Some(" .MP4 ")).as_deref(),
            Some("mp4")
        );

        let (search_endpoint, search_request) =
            cloud_search_request(" holiday ", Some("video"), None, 2);
        assert_eq!(search_endpoint, "/userres/v1/file/search_files");
        assert_eq!(
            search_request,
            json!({ "name": "holiday", "pageSize": 100, "page": 2 })
        );

        let (video_endpoint, video_request) = cloud_search_request("", Some("video"), None, 3);
        assert_eq!(video_endpoint, "/userres/v1/file/get_file_list");
        assert_eq!(
            video_request,
            json!({
                "parentId": "*",
                "pageSize": 100,
                "page": 3,
                "orderBy": 3,
                "sortType": 1,
                "resType": 1,
                "fileTypes": [CLOUD_FILE_TYPE_VIDEO]
            })
        );

        let (_, extension_request) = cloud_search_request("", None, Some("pdf"), 0);
        assert_eq!(
            extension_request.get("fileTypes"),
            Some(&json!([CLOUD_FILE_TYPE_DOCUMENT]))
        );
        let (_, folder_request) = cloud_search_request("", Some("folder"), None, 0);
        assert_eq!(folder_request.get("resType"), Some(&json!(2)));
        assert!(folder_request.get("fileTypes").is_none());
        let (_, unknown_extension_request) = cloud_search_request("", None, Some("blend"), 0);
        assert!(unknown_extension_request.get("fileTypes").is_none());
    }

    #[test]
    fn folder_picker_file_list_request_filters_before_remote_pagination() {
        let regular = file_list_request("folder-1", 3, false);
        assert_eq!(regular.get("parentId"), Some(&json!("folder-1")));
        assert_eq!(regular.get("page"), Some(&json!(3)));
        assert!(regular.get("resType").is_none());
        assert!(regular.get("needSubFolderStat").is_none());

        let folders = file_list_request("folder-1", 3, true);
        assert_eq!(folders.get("resType"), Some(&json!(2)));
    }

    #[test]
    fn filtered_search_pagination_uses_a_one_item_lookahead_until_remote_exhaustion() {
        let matches = (0..250)
            .map(|index| json!({ "index": index }))
            .collect::<Vec<_>>();
        let (middle_page, lower_bound_total) =
            paginate_filtered_search_results(matches.clone(), 1, 100, false);
        assert_eq!(middle_page.len(), 100);
        assert_eq!(middle_page[0].get("index"), Some(&json!(100)));
        assert_eq!(middle_page[99].get("index"), Some(&json!(199)));
        assert_eq!(lower_bound_total, 201);

        let (last_page, exact_total) = paginate_filtered_search_results(matches, 2, 100, true);
        assert_eq!(last_page.len(), 50);
        assert_eq!(last_page[0].get("index"), Some(&json!(200)));
        assert_eq!(exact_total, 250);
    }

    #[test]
    fn sms_phone_normalization_and_masked_signup_name_are_stable() {
        assert_eq!(
            normalize_china_phone("13800138000").unwrap(),
            "+86 13800138000"
        );
        assert_eq!(
            normalize_china_phone("+86 13800138000").unwrap(),
            "+86 13800138000"
        );
        assert_eq!(
            normalize_china_phone("86-138-0013-8000").unwrap(),
            "+86 13800138000"
        );
        assert_eq!(
            normalize_china_phone("0086 138 0013 8000").unwrap(),
            "+86 13800138000"
        );
        assert!(normalize_china_phone("23800138000").is_err());
        assert!(normalize_china_phone("12800138000").is_err());
        assert!(normalize_china_phone("1380013800").is_err());
        assert!(normalize_china_phone("+1 13800138000").is_err());
        assert!(normalize_china_phone("+86+13800138000").is_err());
        assert!(normalize_china_phone("+86 (13800138000").is_err());
        assert_eq!(masked_phone_name("+86 13800138000"), "用户138****8000");
    }

    #[test]
    fn cloud_index_polling_only_treats_business_code_147_as_pending() {
        assert!(matches!(
            classify_upload_task_response(
                200,
                ApiResponse {
                    code: 147,
                    msg: "任务处理中".to_string(),
                    data: None,
                },
            ),
            Ok(CloudTaskCheck::Pending)
        ));
        let confirmed = classify_upload_task_response(
            200,
            ApiResponse {
                code: 0,
                msg: "success".to_string(),
                data: Some(json!({ "fileId": "file-1" })),
            },
        )
        .expect("code zero with fileId should confirm the upload");
        assert!(matches!(confirmed, CloudTaskCheck::Confirmed(_)));
        assert!(matches!(
            classify_upload_task_response(
                200,
                ApiResponse {
                    code: 145,
                    msg: "任务不存在".to_string(),
                    data: None,
                },
            ),
            Err(CloudConfirmError::Permanent(_))
        ));
        assert!(matches!(
            classify_upload_task_response(
                200,
                ApiResponse {
                    code: 0,
                    msg: "success".to_string(),
                    data: Some(json!({})),
                },
            ),
            Err(CloudConfirmError::Permanent(_))
        ));
        assert!(matches!(
            classify_upload_task_response(
                200,
                ApiResponse {
                    code: 110,
                    msg: "token expired".to_string(),
                    data: None,
                },
            ),
            Err(CloudConfirmError::Retryable(_))
        ));
    }

    #[test]
    fn oauth_device_polling_distinguishes_pending_slow_down_and_fatal_errors() {
        let pending = device_login_wait_response(
            400,
            &json!({ "error": "authorization_pending", "error_description": "pending" }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(pending.get("pending"), Some(&json!(true)));
        assert!(pending.get("slow_down").is_none());

        let slow_down = device_login_wait_response(400, &json!({ "error": "slow_down" }))
            .unwrap()
            .unwrap();
        assert_eq!(slow_down.get("slow_down"), Some(&json!(true)));
        assert_eq!(slow_down.get("interval_increment"), Some(&json!(5)));

        assert!(device_login_wait_response(
            400,
            &json!({ "error": "expired_token", "error_description": "二维码已过期" }),
        )
        .is_err());
        assert!(device_login_wait_response(400, &json!({ "msg": "参数错误" })).is_err());
    }

    #[test]
    fn offline_requests_omit_blank_names_and_oss_prefers_full_endpoint() {
        let unnamed =
            offline_task_request("magnet:?xt=urn:btih:test", "root", "   ", None).unwrap();
        assert_eq!(
            unnamed,
            json!({ "url": "magnet:?xt=urn:btih:test", "parentId": "root" })
        );
        assert!(!unnamed.as_object().unwrap().contains_key("newName"));
        let named = offline_task_request("ed2k://fixture", "root", "  电影  ", None).unwrap();
        assert_eq!(named.get("newName"), Some(&json!("电影")));
        assert!(!named.as_object().unwrap().contains_key("resType"));
        assert_eq!(
            offline_task_request("magnet:?xt=urn:btih:test", "root", "", Some(&[2, 1, 2])).unwrap(),
            json!({
                "url": "magnet:?xt=urn:btih:test",
                "parentId": "root",
                "fileIndexes": [2, 1]
            })
        );
        assert!(offline_task_request("https://example.com/a", "", "", Some(&[0])).is_err());
        assert_eq!(
            offline_resolve_request(" https://example.com/a ").unwrap(),
            json!({ "url": "https://example.com/a" })
        );
        assert!(offline_resolve_request("file:///tmp/a.torrent").is_err());
        assert_eq!(
            offline_task_list_request(None, None, None, None).unwrap(),
            json!({ "cursor": "", "pageSize": 100 })
        );
        assert!(offline_task_list_request(Some(3), None, None, None).is_err());
        assert_eq!(
            offline_task_list_request(Some(0), Some("next"), Some(20), None).unwrap(),
            json!({ "cursor": "next", "pageSize": 20 })
        );
        assert!(offline_task_list_request(Some(3), Some(""), Some(20), None).is_err());
        assert_eq!(
            offline_task_list_request(None, Some(""), Some(20), Some(&[1, 3])).unwrap(),
            json!({ "cursor": "", "pageSize": 20, "status": [1, 3] })
        );
        assert_eq!(
            offline_task_list_request(
                None,
                Some(" opaque-next-cursor "),
                Some(50),
                Some(&[5, 2, 5])
            )
            .unwrap(),
            json!({ "cursor": " opaque-next-cursor ", "pageSize": 50, "status": [5, 2] })
        );
        assert!(offline_task_list_request(None, None, Some(101), None).is_err());
        assert!(offline_task_list_request(None, None, None, Some(&[6])).is_err());
        assert_eq!(
            offline_task_ids_request(&[
                " task-1 ".to_string(),
                "task-2".to_string(),
                "task-1".to_string()
            ])
            .unwrap(),
            json!({ "taskIds": ["task-1", "task-2"] })
        );

        let token: UploadToken = serde_json::from_value(json!({
            "taskId": "task",
            "endPoint": "oss-cn.example.com",
            "fullEndPoint": "https://bucket.oss-cn.example.com",
        }))
        .unwrap();
        assert_eq!(
            preferred_oss_endpoint(&token).as_deref(),
            Some("https://bucket.oss-cn.example.com")
        );
    }

    #[test]
    fn developer_pre_audit_plan_keeps_partial_success_and_legacy_resume() {
        let plan = DeveloperPreAuditPlan {
            version: 2,
            batches: vec![
                DeveloperPreAuditBatch {
                    task_id: "task-a".to_string(),
                    file_count: 20,
                    passed_count: 17,
                    rejected_count: 3,
                    done: true,
                    failed: false,
                },
                DeveloperPreAuditBatch {
                    task_id: String::new(),
                    file_count: 5,
                    passed_count: 0,
                    rejected_count: 5,
                    done: true,
                    failed: true,
                },
            ],
        };
        let encoded = encode_developer_pre_audit_plan(&plan).unwrap();
        let decoded = developer_pre_audit_plan(&encoded, 0);
        assert_eq!(
            summarize_developer_pre_audit_plan(&decoded),
            DeveloperPreAuditSummary {
                total_count: 25,
                passed_count: 17,
                rejected_count: 8,
                pending_count: 0,
                failed_batches: 1,
                done: true,
            }
        );

        let legacy = developer_pre_audit_plan("legacy-task", 7);
        assert_eq!(legacy.version, 1);
        assert_eq!(legacy.batches.len(), 1);
        assert_eq!(legacy.batches[0].task_id, "legacy-task");
        assert_eq!(legacy.batches[0].file_count, 7);
        assert!(!legacy.batches[0].done);
    }

    #[test]
    fn offline_name_obfuscation_preserves_extension_and_persists_restore_state() {
        assert_eq!(
            ed2k_file_name("ed2k://|file|Original%20Movie.mkv|1|ABC|/"),
            "Original Movie.mkv"
        );
        let temporary = offline_temporary_name(
            "Original Movie.mkv",
            "ed2k://|file|Original%20Movie.mkv|1|ABC|/",
        );
        assert!(temporary.starts_with("gy_"));
        assert!(temporary.ends_with(".mkv"));
        assert_eq!(temporary.len(), 27);
        let magnet_temporary = offline_temporary_name(
            "Original Magnet Folder",
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567",
        );
        assert!(magnet_temporary.starts_with("gy_"));
        assert_eq!(magnet_temporary.len(), 23);
        assert!(!magnet_temporary.contains('.'));
        let magnet_source = "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=Original%20Magnet+Folder&xl=1024";
        assert_eq!(magnet_display_name(magnet_source), "Original Magnet Folder");
        assert_eq!(offline_source_name(magnet_source), "Original Magnet Folder");
        assert_eq!(
            protected_offline_source(magnet_source, &magnet_temporary),
            "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&xl=1024"
        );
        assert_eq!(
            protected_offline_source("ed2k://|file|Original%20Movie.mkv|1|ABC|/", &temporary),
            format!("ed2k://|file|{temporary}|1|ABC|/")
        );
        assert_eq!(
            offline_resolved_name(&json!({
                "resType": 3,
                "emuleResInfo": { "fileName": "Original Movie.mkv" }
            })),
            "Original Movie.mkv"
        );

        let directory = std::env::temp_dir().join(format!(
            "guangya-offline-restore-test-{}",
            Uuid::new_v4().simple()
        ));
        let database = directory.join("state.sqlite3");
        init_database(&database).expect("initialize offline restore database");
        assert!(!offline_filename_obfuscation_enabled(&database).unwrap());
        save_app_state(&database, "offline_filename_obfuscation", "true").unwrap();
        assert!(offline_filename_obfuscation_enabled(&database).unwrap());
        save_offline_name_restore(&database, "task-1", "Original Movie.mkv", &temporary).unwrap();
        let settings = offline_settings_for_path(&database).unwrap();
        assert!(settings.filename_obfuscation_enabled);
        assert_eq!(settings.pending_restores, 1);
        remove_offline_name_restores(&database, &["task-1".to_string()]).unwrap();
        assert_eq!(pending_offline_name_restore_count(&database).unwrap(), 0);
        fs::remove_dir_all(directory).expect("remove offline restore test directory");
    }

    #[test]
    fn archive_collisions_never_overwrite_existing_files() {
        let root = std::env::temp_dir().join(format!("guangya-archive-test-{}", Uuid::new_v4()));
        let source_dir = root.join("source");
        let archive_dir = root.join("archive");
        fs::create_dir_all(&source_dir).expect("source directory");
        fs::create_dir_all(&archive_dir).expect("archive directory");
        let source = source_dir.join("episode.mkv");
        fs::write(&source, b"new upload").expect("source fixture");
        let metadata = fs::metadata(&source).expect("source metadata");
        let modified = modified_ms(&metadata);
        let requested = archive_dir.join("episode.mkv");
        let first_collision = archive_candidate(&requested, modified, 1);
        fs::write(&requested, b"old archive").expect("base collision");
        fs::write(&first_collision, b"older archive").expect("suffix collision");

        let archived =
            archive_file_without_overwrite(&source, &requested, metadata.len(), modified)
                .expect("archive should find a unique name");
        assert_eq!(archived, archive_candidate(&requested, modified, 2));
        assert_eq!(fs::read(&requested).unwrap(), b"old archive");
        assert_eq!(fs::read(&first_collision).unwrap(), b"older archive");
        assert_eq!(fs::read(&archived).unwrap(), b"new upload");
        assert!(!source.exists());
        fs::remove_dir_all(root).expect("archive fixture cleanup");
    }

    #[test]
    fn exclusive_archive_copy_preserves_source_on_collision_or_mismatch() {
        let root = std::env::temp_dir().join(format!("guangya-copy-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("copy directory");
        let source = root.join("source.bin");
        let destination = root.join("destination.bin");
        fs::write(&source, b"source bytes").expect("source fixture");
        fs::write(&destination, b"existing bytes").expect("destination fixture");
        let metadata = fs::metadata(&source).expect("source metadata");
        assert!(!copy_archive_exclusive(
            &source,
            &destination,
            metadata.len(),
            modified_ms(&metadata),
        )
        .expect("collision should not be an error"));
        assert_eq!(fs::read(&destination).unwrap(), b"existing bytes");
        assert!(source.exists());

        let mismatch_destination = root.join("mismatch.bin");
        assert!(copy_archive_exclusive(
            &source,
            &mismatch_destination,
            metadata.len() + 1,
            modified_ms(&metadata),
        )
        .is_err());
        assert!(source.exists());
        assert!(!mismatch_destination.exists());
        fs::remove_dir_all(root).expect("copy fixture cleanup");
    }

    #[test]
    fn download_names_are_safe_and_collisions_are_preserved() {
        assert_eq!(
            safe_download_name(" 剧集:S01/E01?.mkv "),
            "剧集_S01_E01_.mkv"
        );
        assert_eq!(safe_download_name("..."), "光鸭下载");

        let root = std::env::temp_dir().join(format!("guangya-download-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("download test directory should exist");
        fs::write(root.join("episode.mkv"), b"existing").expect("existing file should be created");
        assert_eq!(
            available_download_path(&root, "episode.mkv"),
            root.join("episode (1).mkv")
        );
        fs::remove_dir_all(root).expect("download test directory should be removable");
    }

    #[test]
    fn parallel_download_ranges_are_contiguous_bounded_and_complete() {
        let total_bytes = 200 * 1024 * 1024 + 17;
        let ranges = download_byte_ranges(total_bytes, 4);
        assert!(ranges.len() > 4);
        assert_eq!(ranges.first().map(|range| range.start), Some(0));
        assert_eq!(ranges.last().map(|range| range.end), Some(total_bytes - 1));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end + 1, pair[1].start);
        }
        assert!(ranges.iter().all(|range| {
            let length = range.end - range.start + 1;
            length <= DOWNLOAD_RANGE_MAX_BYTES
        }));
    }

    #[test]
    fn download_connection_budget_balances_files_and_segments() {
        assert_eq!(configured_download_connections(1), 4);
        assert_eq!(configured_download_connections(2), 4);
        assert_eq!(configured_download_connections(4), 2);
        assert_eq!(configured_download_connections(8), 2);
        assert_eq!(configured_download_connections(99), 2);
    }

    #[test]
    fn content_range_parser_rejects_incomplete_or_invalid_ranges() {
        assert_eq!(
            parse_content_range("bytes 8388608-16777215/33554432"),
            Some(ParsedContentRange {
                start: 8_388_608,
                end: 16_777_215,
                total: 33_554_432,
            })
        );
        assert!(parse_content_range("bytes */33554432").is_none());
        assert!(parse_content_range("bytes 10-9/20").is_none());
        assert!(parse_content_range("bytes 0-20/20").is_none());
    }

    #[tokio::test]
    async fn download_control_waits_while_paused_and_resumes() {
        let (sender, mut receiver) = watch::channel(DownloadControlState::Paused);
        let waiter = tokio::spawn(async move { wait_download_running(&mut receiver).await });
        sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        sender.send_replace(DownloadControlState::Running);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("paused download should resume promptly")
            .expect("download control task should join")
            .expect("resumed download should be runnable");
    }

    #[tokio::test]
    async fn download_control_cancellation_interrupts_waiting_tasks() {
        let (sender, mut receiver) = watch::channel(DownloadControlState::Paused);
        let waiter = tokio::spawn(async move { wait_download_running(&mut receiver).await });
        sender.send_replace(DownloadControlState::Cancelled);
        let error = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("cancelled download should stop promptly")
            .expect("download control task should join")
            .expect_err("cancelled download must not keep running");
        assert_eq!(error, "下载已取消");
    }

    #[test]
    fn download_registry_tracks_and_releases_active_tasks() {
        let registry = DownloadRegistry::default();
        let (receiver, registration) =
            begin_download_task(&registry, "download-1").expect("task should register");
        set_download_control(&registry, "download-1", DownloadControlState::Paused)
            .expect("task should pause");
        assert_eq!(*receiver.borrow(), DownloadControlState::Paused);
        assert!(begin_download_task(&registry, "download-1").is_err());
        drop(registration);
        assert!(
            set_download_control(&registry, "download-1", DownloadControlState::Running).is_err()
        );
    }

    #[test]
    fn packaging_failure_states_stop_polling_immediately() {
        assert!(ensure_packaging_task_active(&json!({ "status": "processing" })).is_ok());
        assert!(ensure_packaging_task_active(
            &json!({ "status": "failed", "message": "压缩失败" })
        )
        .is_err());
        assert!(
            ensure_packaging_task_active(&json!({ "errorCode": "42", "msg": "任务失效" })).is_err()
        );
    }

    #[test]
    fn sqlite_persists_auth_device_and_uploaded_file_history() {
        let root = std::env::temp_dir().join(format!("guangya-sqlite-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("database should initialize");
        save_auth_session(&database, Some("access-token"), Some("refresh-token"))
            .expect("auth should persist");
        let auth = load_auth_session(&database).expect("auth should load");
        assert_eq!(auth.access_token.as_deref(), Some("access-token"));
        assert_eq!(auth.refresh_token.as_deref(), Some("refresh-token"));
        assert!(auth.account_scope.is_none());
        save_auth_session(&database, Some("refreshed-access-token"), None)
            .expect("refresh should retain a non-rotated refresh token");
        let refreshed_auth = load_auth_session(&database).expect("refreshed auth should load");
        assert_eq!(
            refreshed_auth.access_token.as_deref(),
            Some("refreshed-access-token")
        );
        assert_eq!(
            refreshed_auth.refresh_token.as_deref(),
            Some("refresh-token")
        );
        assert!(refreshed_auth.account_scope.is_none());
        replace_auth_session(
            &database,
            Some("new-login-access-token"),
            None,
            Some("session:new-login"),
        )
        .expect("a fresh login should replace the complete session");
        let replaced_auth = load_auth_session(&database).expect("replacement auth should load");
        assert_eq!(
            replaced_auth.access_token.as_deref(),
            Some("new-login-access-token")
        );
        assert!(replaced_auth.refresh_token.is_none());
        assert_eq!(
            replaced_auth.account_scope.as_deref(),
            Some("session:new-login")
        );
        clear_persisted_auth_session(&database).expect("expired auth should clear");
        let cleared_auth = load_auth_session(&database).expect("cleared auth should load");
        assert!(cleared_auth.access_token.is_none());
        assert!(cleared_auth.refresh_token.is_none());
        assert!(cleared_auth.account_scope.is_none());
        let device_id = load_or_create_device_id(&database).expect("device id should persist");
        assert_eq!(device_id.len(), 32);
        assert!(device_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            load_or_create_device_id(&database).expect("device id should reload"),
            device_id
        );

        let item = UploadItem {
            mapping_id: "mapping-1".into(),
            file_path: PathBuf::from("H:/test/photo.png"),
            remote_parent_id: String::new(),
            remote_dir: String::new(),
            relative_path: "photo.png".to_string(),
            change_kind: "added".to_string(),
            size: 128,
            modified_ms: 42,
            replacement: None,
        };
        save_upload_record(
            &database,
            &item,
            &UploadOutcome {
                task_id: "task-1".into(),
                remote_file_id: Some("file-1".into()),
            },
            UPLOAD_STATE_CLOUD_CONFIRMED,
        )
        .expect("upload history should persist");
        let history = load_upload_history(&database).expect("upload history should load");
        assert_eq!(
            history.get(&item_key(&item.mapping_id, &item.file_path)),
            Some(&Stamp {
                size: 128,
                modified_ms: 42
            })
        );
        let mut pending_item = item.clone();
        pending_item.file_path = PathBuf::from("H:/test/pending.png");
        pending_item.relative_path = "pending.png".into();
        save_upload_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "task-pending".into(),
                remote_file_id: None,
            },
            UPLOAD_STATE_OSS_COMPLETE,
        )
        .expect("OSS-complete history should persist before cloud indexing");
        let connection = open_database(&database).expect("database should reopen");
        let (task_id, remote_file_id, upload_state): (String, Option<String>, String) = connection
            .query_row(
                "SELECT task_id, remote_file_id, upload_state FROM uploaded_files WHERE mapping_id = ?1 AND file_path = ?2",
                params![pending_item.mapping_id, pending_item.file_path.to_string_lossy()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("pending upload should be queryable");
        assert_eq!(task_id, "task-pending");
        assert_eq!(remote_file_id, None);
        assert_eq!(upload_state, UPLOAD_STATE_OSS_COMPLETE);
        drop(connection);
        let pending = load_pending_uploads(&database).expect("pending uploads should load");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, "task-pending");
        assert_eq!(pending[0].item.relative_path, "pending.png");
        let history = load_upload_history(&database).expect("confirmed history should load");
        assert!(history.contains_key(&item_key(&item.mapping_id, &item.file_path)));
        assert!(!history.contains_key(&item_key(&pending_item.mapping_id, &pending_item.file_path)));
        assert!(!confirm_pending_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "stale-task".into(),
                remote_file_id: Some("wrong-file".into()),
            },
        )
        .expect("a stale task must not replace the pending record"));
        assert!(confirm_pending_record(
            &database,
            &pending_item,
            &UploadOutcome {
                task_id: "task-pending".into(),
                remote_file_id: Some("file-pending".into()),
            },
        )
        .expect("pending record should transition to confirmed"));
        assert!(load_pending_uploads(&database)
            .expect("pending rows should reload")
            .is_empty());
        assert!(load_upload_history(&database)
            .expect("confirmed rows should reload")
            .contains_key(&item_key(&pending_item.mapping_id, &pending_item.file_path)));
        let mut recreated_item = item.clone();
        recreated_item.mapping_id = "mapping-2".into();
        let reused = reuse_matching_confirmed_upload(&database, &recreated_item)
            .expect("confirmed upload history should be reusable")
            .expect("matching history should exist");
        assert_eq!(reused.0, item.mapping_id);
        assert_eq!(reused.1.remote_file_id.as_deref(), Some("file-1"));
        assert!(load_upload_history(&database)
            .expect("reused history should reload")
            .contains_key(&item_key(
                &recreated_item.mapping_id,
                &recreated_item.file_path
            )));
        remove_mapping_transient_uploads(&database, &item.mapping_id)
            .expect("transient uploads should be removable");
        let history = load_upload_history(&database).expect("history should reload");
        assert!(history.contains_key(&item_key(&item.mapping_id, &item.file_path)));
        assert!(history.contains_key(&item_key(&pending_item.mapping_id, &pending_item.file_path)));
        fs::remove_dir_all(root).expect("test database should be removable");
    }

    #[test]
    fn sqlite_migrates_legacy_null_remote_ids_to_pending_state() {
        let root = std::env::temp_dir().join(format!("guangya-migration-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        fs::create_dir_all(&root).expect("migration directory");
        let connection = open_database(&database).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE uploaded_files (
                   mapping_id TEXT NOT NULL,
                   file_path TEXT NOT NULL,
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   task_id TEXT,
                   remote_file_id TEXT,
                   uploaded_at INTEGER NOT NULL,
                   PRIMARY KEY (mapping_id, file_path)
                 );
                 INSERT INTO uploaded_files VALUES
                   ('mapping-1', '/watch/confirmed.mkv', 10, '20', 'task-1', 'file-1', 1),
                   ('mapping-1', '/watch/pending.mkv', 11, '21', 'task-2', NULL, 1);",
            )
            .expect("legacy schema fixture");
        drop(connection);

        init_database(&database).expect("legacy database should migrate");
        let history = load_upload_history(&database).expect("confirmed history");
        assert!(history.contains_key(&item_key("mapping-1", Path::new("/watch/confirmed.mkv"))));
        assert!(!history.contains_key(&item_key("mapping-1", Path::new("/watch/pending.mkv"))));
        let pending = load_pending_uploads(&database).expect("pending migration");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, "task-2");
        fs::remove_dir_all(root).expect("migration fixture cleanup");
    }

    #[test]
    fn sqlite_adds_cid_to_legacy_flash_fingerprint_tables() {
        let root =
            std::env::temp_dir().join(format!("guangya-cid-migration-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        let file = root.join("movie.mkv");
        fs::create_dir_all(&root).expect("migration directory");
        let connection = open_database(&database).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE file_fingerprints (
                   file_path TEXT NOT NULL,
                   size INTEGER NOT NULL,
                   modified_ms TEXT NOT NULL,
                   gcid TEXT NOT NULL,
                   computed_at INTEGER NOT NULL,
                   PRIMARY KEY (file_path, size, modified_ms)
                 );
                 CREATE TABLE gcid_import_files (
                   job_id TEXT NOT NULL,
                   path TEXT NOT NULL,
                   folder_path TEXT NOT NULL,
                   file_name TEXT NOT NULL,
                   file_size INTEGER NOT NULL,
                   gcid TEXT NOT NULL,
                   status TEXT NOT NULL DEFAULT 'pending',
                   attempts INTEGER NOT NULL DEFAULT 0,
                   task_id TEXT,
                   file_id TEXT,
                   error TEXT,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY (job_id, path)
                 );",
            )
            .expect("legacy hash tables");
        connection
            .execute(
                "INSERT INTO file_fingerprints
                   (file_path, size, modified_ms, gcid, computed_at)
                 VALUES (?1, 7, '100', ?2, 1)",
                params![
                    file.to_string_lossy().as_ref(),
                    "0123456789ABCDEF0123456789ABCDEF01234567"
                ],
            )
            .expect("legacy fingerprint row");
        drop(connection);

        init_database(&database).expect("legacy hash tables should migrate");
        let connection = open_database(&database).expect("migrated database");
        for table in ["file_fingerprints", "gcid_import_files"] {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .expect("table info");
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .expect("column query")
                .collect::<Result<Vec<_>, _>>()
                .expect("columns");
            assert!(columns.iter().any(|column| column == "cid"));
        }
        drop(connection);
        let policy = CacheSettings {
            enabled: true,
            max_entries: DEFAULT_CACHE_MAX_ENTRIES,
        };
        assert!(load_cached_file_hashes(&database, &file, 7, 100, policy)
            .expect("legacy row should remain readable")
            .is_none());
        let hashes = FileHashes {
            gcid: "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
            cid: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_string(),
        };
        save_cached_file_hashes(&database, &file, 7, 100, &hashes, policy)
            .expect("migrated cache should accept CID");
        assert_eq!(
            load_cached_file_hashes(&database, &file, 7, 100, policy).expect("migrated hashes"),
            Some(hashes)
        );
        fs::remove_dir_all(root).expect("migration fixture cleanup");
    }

    #[test]
    fn guangya_gcid_and_cid_match_the_current_web_uploader_algorithm() {
        let content = (0..600_000)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let ranges = cid_byte_ranges(content.len() as u64);
        let mut gcid_hasher = Sha1::new();
        let mut cid_hasher = Sha1::new();
        let mut position = 0_u64;
        let mut sampled = 0_u64;
        for chunk in content.chunks(gcid_chunk_size(content.len() as u64)) {
            gcid_hasher.update(Sha1::digest(chunk));
            sampled += update_cid_hasher(&mut cid_hasher, &ranges, position, chunk)
                .expect("CID sample should be valid");
            position += chunk.len() as u64;
        }
        assert_eq!(sampled, 60 * 1024);
        assert_eq!(
            hex::encode_upper(gcid_hasher.finalize()),
            "3FC0617C331816DA4EE9C19C6F532F2D6D4FD6CC"
        );
        assert_eq!(
            hex::encode_upper(cid_hasher.finalize()),
            "ECDDF55803ED503C4DF219A5C9C847860A438CB8"
        );
    }

    #[test]
    fn gcid_export_diagnostics_redact_signed_urls_and_credentials() {
        let sanitized = sanitize_gcid_diagnostic_text(
            "request https://cdn.example/video.mkv?Signature=abc&token=def failed; Authorization: Bearer-secret",
        );
        assert!(!sanitized.contains("Signature=abc"));
        assert!(!sanitized.contains("token=def"));
        assert!(!sanitized.contains("Bearer-secret"));
        assert!(sanitized.contains("https://cdn.example/video.mkv?<redacted>"));
    }

    #[test]
    fn gcid_export_scan_retries_only_transient_transport_or_upstream_errors() {
        assert!(retryable_gcid_export_scan_error(
            "无法连接光鸭接口 /userres/v1/file/get_file_list：网络异常（error sending request）"
        ));
        assert!(retryable_gcid_export_scan_error(
            "光鸭接口返回了非 JSON 响应（HTTP 502）"
        ));
        assert!(retryable_gcid_export_scan_error("请求超时（HTTP 408）"));
        assert!(!retryable_gcid_export_scan_error(
            "登录态已失效，请重新打开官方登录页"
        ));
        assert!(!retryable_gcid_export_scan_error("没有访问该目录的权限"));
    }

    #[test]
    fn business_api_client_follows_global_proxy_updates() {
        // 共享客户端必须跟随"设置 → 网络"的全局代理：直连与代理都能构建，
        // 非法代理返回明确错误而不是悄悄回落直连。
        set_global_api_proxy("");
        assert!(business_api_client().is_ok());
        set_global_api_proxy("http://127.0.0.1:7890");
        assert!(business_api_client().is_ok());
        set_global_api_proxy("::这不是代理::");
        assert!(business_api_client().is_err());
        set_global_api_proxy("");
        assert!(business_api_client().is_ok());
    }

    #[tokio::test]
    async fn gcid_export_retries_only_the_failed_range_with_twenty_file_workers() {
        assert_eq!(GCID_EXPORT_FILE_CONCURRENCY, 20);
        assert_eq!(GCID_EXPORT_RANGE_ATTEMPTS, 3);
        let calls = Arc::new(Mutex::new([0_usize; 3]));
        let outcomes = stream::iter(0..3_usize)
            .map(|index| {
                let calls = Arc::clone(&calls);
                async move {
                    retry_gcid_export_range(
                        |_| {
                            let calls = Arc::clone(&calls);
                            async move {
                                let call = {
                                    let mut guard = calls.lock().expect("range call counts");
                                    guard[index] += 1;
                                    guard[index]
                                };
                                if index == 1 && call == 1 {
                                    Err(GcidExportRangeError::retryable("temporary range failure"))
                                } else {
                                    Ok(index)
                                }
                            }
                        },
                        GCID_EXPORT_RANGE_ATTEMPTS,
                        0,
                    )
                    .await
                }
            })
            .buffer_unordered(GCID_EXPORT_RANGE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        assert!(outcomes.iter().all(Result::is_ok));
        assert_eq!(*calls.lock().expect("final range call counts"), [1, 2, 1]);
    }

    #[tokio::test]
    async fn gcid_export_does_not_retry_an_explicit_range_rejection() {
        let calls = Arc::new(AtomicUsize::new(0));
        let result: Result<(), String> = retry_gcid_export_range(
            |_| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Err(GcidExportRangeError::permanent("range unsupported"))
                }
            },
            GCID_EXPORT_RANGE_ATTEMPTS,
            0,
        )
        .await;

        assert_eq!(
            result.expect_err("range rejection must fail"),
            "range unsupported"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn gcid_export_range_body_accepts_exact_short_chunks_and_rejects_short_total() {
        let exact = stream::iter(vec![Ok::<Vec<u8>, std::io::Error>(vec![1]), Ok(vec![2, 3])]);
        assert_eq!(
            read_bounded_gcid_range_stream(exact, 3, "exact.bin", Duration::from_secs(1))
                .await
                .expect("exact fragmented range body"),
            vec![1, 2, 3]
        );

        let short = stream::iter(vec![Ok::<Vec<u8>, std::io::Error>(vec![1, 2])]);
        let error = read_bounded_gcid_range_stream(short, 3, "short.bin", Duration::from_secs(1))
            .await
            .expect_err("short range body must fail");
        assert!(error.message.contains("字节数不完整"));
    }

    #[tokio::test]
    async fn gcid_export_range_body_stops_on_the_first_oversized_chunk() {
        let polls = Arc::new(AtomicUsize::new(0));
        let stream_polls = Arc::clone(&polls);
        let oversized = futures_util::stream::poll_fn(move |_| {
            let call = stream_polls.fetch_add(1, Ordering::Relaxed);
            match call {
                0 => std::task::Poll::Ready(Some(Ok::<Vec<u8>, std::io::Error>(vec![1, 2, 3, 4]))),
                _ => panic!("oversized range body must be dropped without polling its tail"),
            }
        });
        let error =
            read_bounded_gcid_range_stream(oversized, 3, "oversized.bin", Duration::from_secs(1))
                .await
                .expect_err("oversized range body must fail");
        assert!(error.message.contains("超出请求范围"));
        assert_eq!(polls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn gcid_export_range_body_uses_an_idle_timeout_per_chunk() {
        let chunks = stream::unfold(0_u8, |index| async move {
            if index == 3 {
                return None;
            }
            sleep(Duration::from_millis(20)).await;
            Some((Ok::<Vec<u8>, std::io::Error>(vec![index]), index + 1))
        });
        let started_at = Instant::now();
        assert_eq!(
            read_bounded_gcid_range_stream(chunks, 3, "slow.bin", Duration::from_millis(50),)
                .await
                .expect("continuous chunks must reset the idle timeout"),
            vec![0, 1, 2]
        );
        assert!(started_at.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn streamed_guangya_hashes_ignore_network_chunk_boundaries() {
        let content = (0..900_123)
            .map(|index| ((index * 17) % 251) as u8)
            .collect::<Vec<_>>();
        let mut accumulator = FlashHashAccumulator::new(content.len() as u64);
        let mut offset = 0_usize;
        for requested in [1_usize, 31_337, 262_143, 7, 400_001, 99_999, content.len()] {
            if offset >= content.len() {
                break;
            }
            let end = offset.saturating_add(requested).min(content.len());
            accumulator
                .update(&content[offset..end])
                .expect("stream chunk should hash");
            offset = end;
        }
        if offset < content.len() {
            accumulator
                .update(&content[offset..])
                .expect("remaining stream chunk should hash");
        }
        let streamed = accumulator.finish().expect("stream should finish");

        let mut gcid = Sha1::new();
        let mut cid = Sha1::new();
        let ranges = cid_byte_ranges(content.len() as u64);
        let mut position = 0_u64;
        for chunk in content.chunks(gcid_chunk_size(content.len() as u64)) {
            gcid.update(Sha1::digest(chunk));
            update_cid_hasher(&mut cid, &ranges, position, chunk).expect("CID sample");
            position += chunk.len() as u64;
        }
        assert_eq!(streamed.gcid, hex::encode_upper(gcid.finalize()));
        assert_eq!(streamed.cid, hex::encode_upper(cid.finalize()));
    }

    #[test]
    fn generated_export_size_matches_the_reference_json_format() {
        assert_eq!(format_export_bytes(889_837_161_511), "828.73 GB");
    }

    #[test]
    fn gcid_export_parser_accepts_numbers_and_strings() {
        let raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "usesCidInExport": true,
          "commonPath": "H:/Media",
          "totalFilesCount": 2,
          "totalSize": "30",
          "files": [
            {
              "path": "Movies/Film.mkv",
              "size": 10,
              "gcid": "0123456789ABCDEF0123456789ABCDEF01234567",
              "cid": "89ABCDEF0123456789ABCDEF0123456789ABCDEF"
            },
            {
              "path": "Shows\\Episode.mkv",
              "size": "20",
              "gcid": "89abcdef0123456789abcdef0123456789abcdef",
              "cid": "0123456789abcdef0123456789abcdef01234567"
            }
          ]
        }"#;
        let (files, total_size, common_path) =
            parse_gcid_export(raw).expect("valid Guangya export");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].folder_path, "Movies");
        assert_eq!(files[0].name, "Film.mkv");
        assert_eq!(files[0].gcid, "0123456789ABCDEF0123456789ABCDEF01234567");
        assert_eq!(files[0].cid, "89ABCDEF0123456789ABCDEF0123456789ABCDEF");
        assert_eq!(files[1].path, "Shows/Episode.mkv");
        assert_eq!(total_size, 30);
        assert_eq!(common_path, "H:/Media");
    }

    #[test]
    fn gcid_export_parser_rejects_unsafe_and_duplicate_paths() {
        let unsafe_raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "usesCidInExport": true,
          "files": [{
            "path": "../secret.mkv",
            "size": 1,
            "gcid": "0123456789abcdef0123456789abcdef01234567",
            "cid": "89abcdef0123456789abcdef0123456789abcdef"
          }]
        }"#;
        assert!(parse_gcid_export(unsafe_raw)
            .expect_err("parent traversal must be rejected")
            .contains("越界"));

        let duplicate_raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "usesCidInExport": true,
          "files": [
            {
              "path": "Movies/Film.mkv",
              "size": 1,
              "gcid": "0123456789abcdef0123456789abcdef01234567",
              "cid": "89abcdef0123456789abcdef0123456789abcdef"
            },
            {
              "path": "Movies\\Film.mkv",
              "size": 1,
              "gcid": "89abcdef0123456789abcdef0123456789abcdef",
              "cid": "0123456789abcdef0123456789abcdef01234567"
            }
          ]
        }"#;
        assert!(parse_gcid_export(duplicate_raw)
            .expect_err("normalized duplicate paths must be rejected")
            .contains("重复路径"));
    }

    #[test]
    fn gcid_import_jobs_are_scoped_to_destination() {
        let raw = br#"{"source":"guangya"}"#;
        let first = gcid_import_job_id(raw, "", "Media Library");
        assert_eq!(first, gcid_import_job_id(raw, "", "Media Library"));
        assert_ne!(first, gcid_import_job_id(raw, "parent-1", "Media Library"));
        assert_ne!(first, gcid_import_job_id(raw, "", "Other Library"));
        assert_eq!(first.len(), 32);
        assert!(validate_gcid_destination("Media Library").is_ok());
        assert!(validate_gcid_destination("../Media Library").is_err());
    }

    #[test]
    fn concurrent_gcid_workers_claim_distinct_files_without_exiting_on_sqlite_lock() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-claim-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("database should initialize");
        let connection = open_database(&database).expect("database should open");
        connection
            .execute(
                "INSERT INTO gcid_import_jobs
                   (job_id, source_path, source_name, destination_parent_id,
                    destination_name, total_files, total_size, status, created_at, updated_at)
                 VALUES ('job', 'source.json', 'source.json', '', 'Media Library',
                         ?1, '32', 'running', 1, 1)",
                params![MAX_GCID_IMPORT_CONCURRENCY],
            )
            .expect("job should be inserted");
        for index in 0..MAX_GCID_IMPORT_CONCURRENCY {
            let path = format!("Movies/{index:02}.mkv");
            connection
                .execute(
                    "INSERT INTO gcid_import_files
                       (job_id, path, folder_path, file_name, file_size, gcid, cid, updated_at)
                     VALUES ('job', ?1, 'Movies', ?2, 1, ?3, ?4, 1)",
                    params![
                        path,
                        format!("{index:02}.mkv"),
                        format!("{index:040X}"),
                        format!("{:040X}", index + 100)
                    ],
                )
                .expect("file should be inserted");
        }
        drop(connection);

        let barrier = Arc::new(std::sync::Barrier::new(MAX_GCID_IMPORT_CONCURRENCY + 1));
        let workers = (0..MAX_GCID_IMPORT_CONCURRENCY)
            .map(|_| {
                let database = database.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_gcid_import_file(&database, "job")
                        .expect("concurrent claim should wait for the writer")
                        .expect("a pending file should remain")
                        .path
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let claimed = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker should finish"))
            .collect::<HashSet<_>>();
        assert_eq!(claimed.len(), MAX_GCID_IMPORT_CONCURRENCY);
        let counts = load_gcid_import_status(&database, Some("job"))
            .expect("status should load")
            .expect("job should exist")
            .counts;
        assert_eq!(counts.processing, MAX_GCID_IMPORT_CONCURRENCY as u64);
        assert_eq!(counts.pending, 0);
        fs::remove_dir_all(root).expect("GCID fixture cleanup");
    }

    #[test]
    fn repreparing_completed_gcid_import_rechecks_every_file() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        let source = root.join("library.json");
        init_database(&database).expect("database should initialize");
        let raw = br#"{
          "source": "guangya",
          "hashType": "gcid",
          "usesGcidInExport": true,
          "usesCidInExport": true,
          "totalFilesCount": 1,
          "totalSize": 10,
          "files": [{
            "path": "Movies/Film.mkv",
            "size": 10,
            "gcid": "0123456789abcdef0123456789abcdef01234567",
            "cid": "89abcdef0123456789abcdef0123456789abcdef"
          }]
        }"#;
        let job_id = prepare_gcid_import_database(&database, raw, &source, "", "Media Library")
            .expect("job should be prepared");
        let connection = open_database(&database).expect("database should reopen");
        connection
            .execute(
                "UPDATE gcid_import_files
                 SET status = 'imported', attempts = 3, task_id = 'old-task',
                     file_id = 'old-file', error = 'old-error'
                 WHERE job_id = ?1",
                params![job_id],
            )
            .expect("file should become imported");
        connection
            .execute(
                "UPDATE gcid_import_jobs SET status = 'completed' WHERE job_id = ?1",
                params![job_id],
            )
            .expect("job should become completed");
        drop(connection);

        assert_eq!(
            prepare_gcid_import_database(&database, raw, &source, "", "Media Library")
                .expect("same job should be reusable"),
            job_id
        );
        let status = load_gcid_import_status(&database, Some(&job_id))
            .expect("status should load")
            .expect("status should exist");
        assert_eq!(status.status, "ready");
        assert_eq!(status.counts.imported, 0);
        assert_eq!(status.counts.pending, 1);
        let reset = open_database(&database)
            .unwrap()
            .query_row(
                "SELECT attempts, task_id, file_id, error
                 FROM gcid_import_files WHERE job_id = ?1",
                params![job_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .expect("re-import state should load");
        assert_eq!(reset, (0, None, None, None));
        fs::remove_dir_all(root).expect("GCID fixture cleanup");
    }

    #[test]
    fn retrying_gcid_import_requeues_missed_conflict_failed_and_processing_files() {
        let root = std::env::temp_dir().join(format!("guangya-gcid-retry-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("database should initialize");
        let connection = open_database(&database).expect("database should open");
        connection
            .execute(
                "INSERT INTO gcid_import_jobs
                   (job_id, source_path, source_name, destination_parent_id,
                    destination_name, total_files, total_size, status, created_at, updated_at)
                 VALUES ('retry-job', 'source.json', 'source.json', '', 'Media Library',
                         5, '5', 'completed_with_errors', 1, 1)",
                [],
            )
            .expect("job should be inserted");
        for (index, status) in ["imported", "missed", "conflict", "failed", "processing"]
            .into_iter()
            .enumerate()
        {
            connection
                .execute(
                    "INSERT INTO gcid_import_files
                       (job_id, path, folder_path, file_name, file_size, gcid, cid,
                        status, attempts, task_id, file_id, error, updated_at)
                     VALUES ('retry-job', ?1, '', ?1, 1, ?2, ?3, ?4, 4,
                             'old-task', 'old-file', 'old-error', 1)",
                    params![
                        format!("{index}.mkv"),
                        format!("{index:040X}"),
                        format!("{:040X}", index + 100),
                        status
                    ],
                )
                .expect("file should be inserted");
        }
        drop(connection);

        let before = load_gcid_import_status(&database, Some("retry-job"))
            .unwrap()
            .unwrap();
        assert!(gcid_import_has_retryable_work(&before.counts));
        reset_retryable_gcid_import_files(&database, "retry-job")
            .expect("retryable files should be reset");
        let after = load_gcid_import_status(&database, Some("retry-job"))
            .unwrap()
            .unwrap();
        assert_eq!(after.counts.imported, 1);
        assert_eq!(after.counts.pending, 4);
        assert_eq!(
            open_database(&database)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM gcid_import_files
                     WHERE job_id = 'retry-job' AND status = 'pending'
                       AND attempts = 0 AND task_id IS NULL AND file_id IS NULL AND error IS NULL",
                    [],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            4
        );
        fs::remove_dir_all(root).expect("GCID fixture cleanup");
    }

    #[test]
    fn restarting_terminal_gcid_import_rechecks_imported_and_existing_files() {
        let root =
            std::env::temp_dir().join(format!("guangya-gcid-reimport-test-{}", Uuid::new_v4()));
        let database = root.join("state.sqlite3");
        init_database(&database).expect("database should initialize");
        let connection = open_database(&database).expect("database should open");
        connection
            .execute(
                "INSERT INTO gcid_import_jobs
                   (job_id, source_path, source_name, destination_parent_id,
                    destination_name, total_files, total_size, status, created_at, updated_at)
                 VALUES ('reimport-job', 'source.json', 'source.json', '', 'Media Library',
                         2, '2', 'completed', 1, 1)",
                [],
            )
            .expect("job should be inserted");
        for (index, status) in ["imported", "existing"].into_iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO gcid_import_files
                       (job_id, path, folder_path, file_name, file_size, gcid, cid,
                        status, attempts, task_id, file_id, error, updated_at)
                     VALUES ('reimport-job', ?1, '', ?1, 1, ?2, ?3, ?4, 4,
                             'old-task', 'old-file', 'old-error', 1)",
                    params![
                        format!("{index}.mkv"),
                        format!("{index:040X}"),
                        format!("{:040X}", index + 100),
                        status
                    ],
                )
                .expect("file should be inserted");
        }
        drop(connection);

        assert!(gcid_import_is_terminal("completed"));
        assert!(gcid_import_is_terminal("completed_with_errors"));
        reset_all_gcid_import_files(&database, "reimport-job")
            .expect("terminal import should be reset");
        let after = load_gcid_import_status(&database, Some("reimport-job"))
            .unwrap()
            .unwrap();
        assert_eq!(after.counts.pending, 2);
        assert_eq!(after.counts.imported, 0);
        assert_eq!(after.counts.existing, 0);
        fs::remove_dir_all(root).expect("GCID fixture cleanup");
    }
