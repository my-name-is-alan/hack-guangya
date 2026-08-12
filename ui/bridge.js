import { readJsonResponse } from './httpResponse.js';

const tauriInvoke = window.__TAURI__?.core?.invoke;
const tauriListen = window.__TAURI__?.event?.listen;
export const isTauri = Boolean(tauriInvoke && tauriListen);
const authExpiredListeners = new Set();

const camelizeArgs = (args = {}) => Object.fromEntries(
  Object.entries(args).map(([key, value]) => [key.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase()), value]),
);

const errorMessage = (error) => String(error?.message || error || '');
const isAuthExpiredError = (error) => errorMessage(error).includes('登录态已失效');
const notifyAuthExpired = (error) => {
  if (!isAuthExpiredError(error)) return;
  for (const listener of authExpiredListeners) listener(errorMessage(error));
};
const subscribeAuthExpired = (callback) => {
  authExpiredListeners.add(callback);
  return () => authExpiredListeners.delete(callback);
};

async function webRequest(url, options = {}) {
  try {
    const response = await fetch(url, {
      credentials: 'same-origin',
      ...options,
      headers: { 'content-type': 'application/json', ...(options.headers || {}) },
    });
    return await readJsonResponse(response, `请求 ${url} 失败`);
  } catch (error) {
    notifyAuthExpired(error);
    throw error;
  }
}
export { webRequest };

async function invokeTauri(command, args = {}, allowRefresh = true) {
  try {
    return await tauriInvoke(command, camelizeArgs(args));
  } catch (error) {
    if (allowRefresh && command !== 'refresh_session' && command !== 'clear_expired_session' && isAuthExpiredError(error)) {
      try {
        await tauriInvoke('refresh_session');
      } catch {
        try { await tauriInvoke('clear_expired_session'); } catch {}
        notifyAuthExpired(error);
        throw error;
      }
      return invokeTauri(command, args, false);
    }
    if (isAuthExpiredError(error)) {
      try {
        await tauriInvoke('clear_expired_session');
      } catch {
        // 原始登录失效错误更有用；清理失败由下一次启动继续校验。
      }
      notifyAuthExpired(error);
    }
    throw error;
  }
}

export const bridge = isTauri ? {
  invoke: invokeTauri,
  subscribe: (callback) => tauriListen('sync-event', ({ payload }) => callback(payload)),
  subscribeAuthExpired,
  subscribeDrag: async (callback) => {
    const unlisteners = await Promise.all([
      tauriListen('tauri://drag-enter', ({ payload }) => callback('enter', payload)),
      tauriListen('tauri://drag-over', ({ payload }) => callback('over', payload)),
      tauriListen('tauri://drag-leave', ({ payload }) => callback('leave', payload)),
      tauriListen('tauri://drag-drop', ({ payload }) => callback('drop', payload)),
    ]);
    return () => unlisteners.forEach((unlisten) => unlisten());
  },
  selectFolder: () => tauriInvoke('select_folder'),
  selectUploadFiles: () => tauriInvoke('select_upload_files'),
  selectUploadFolder: () => tauriInvoke('select_upload_folder'),
  login: () => tauriInvoke('start_device_login'),
} : {
  invoke: async (command, args = {}) => {
    if (command === 'get_state') return webRequest('/api/state');
    if (command === 'get_overview') return webRequest('/api/overview');
    if (command === 'get_mount_info') return webRequest('/api/mount');
    if (command === 'update_mount_credentials') return webRequest('/api/mount/credentials', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_native_mount_info') return webRequest('/api/mount/native');
    if (command === 'update_native_mount_options') return webRequest('/api/mount/native/options', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'start_native_mount') return webRequest('/api/mount/native/start', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'stop_native_mount') return webRequest('/api/mount/native/stop', { method: 'POST', body: '{}' });
    if (command === 'select_native_mount_target' || command === 'select_rclone_binary') return null;
    if (command === 'get_virtual_library_info') return webRequest('/api/virtual-library');
    if (command === 'update_virtual_library_settings') return webRequest('/api/virtual-library/settings', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'upsert_virtual_library_mapping') return webRequest('/api/virtual-library/mappings', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'remove_virtual_library_mapping') return webRequest(`/api/virtual-library/mappings/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'sync_virtual_library') return webRequest(`/api/virtual-library/mappings/${encodeURIComponent(args.id)}/sync`, { method: 'POST', body: '{}' });
    if (command === 'select_virtual_library_target') return null;
    if (command === 'get_access_status') return webRequest('/api/access/status');
    if (command === 'unlock_access') return webRequest('/api/access/unlock', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'update_access_code') return webRequest('/api/access/code', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_files') {
      const params = new URLSearchParams({
        page: String(Math.max(0, Number(args.page || 0))),
        parentId: String(args.parent_id || ''),
      });
      if (args.folders_only === true) params.set('resType', '2');
      if (args.force_refresh === true) params.set('refresh', '1');
      return webRequest(`/api/files?${params}`);
    }
    if (command === 'search_files') {
      const params = new URLSearchParams({ query: String(args.query || ''), type: String(args.file_type || ''), extension: String(args.extension || ''), page: String(args.page || 0) });
      return webRequest(`/api/search?${params}`);
    }
    if (command === 'copy_files') return webRequest('/api/files/copy', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'move_files') return webRequest('/api/files/move', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'create_folder') return webRequest('/api/files/create-folder', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_file_detail') {
      const params = new URLSearchParams({ fileId: String(args.file_id || '') });
      return webRequest(`/api/files/detail?${params}`);
    }
    if (command === 'list_recent_actions') {
      const params = new URLSearchParams({
        cursor: String(args.cursor || ''),
        pageSize: String(Math.max(1, Number(args.page_size || 50))),
      });
      if (args.file_types) params.set('fileTypes', Array.isArray(args.file_types) ? args.file_types.join(',') : String(args.file_types));
      if (args.exclude_file_types) params.set('excludeFileTypes', Array.isArray(args.exclude_file_types) ? args.exclude_file_types.join(',') : String(args.exclude_file_types));
      return webRequest(`/api/recent?${params}`);
    }
    if (command === 'delete_files') return webRequest('/api/files/delete', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_recycle_files') {
      const params = new URLSearchParams({
        page: String(Math.max(0, Number(args.page || 0))),
        pageSize: String(Math.max(1, Number(args.page_size || 100))),
      });
      return webRequest(`/api/recycle?${params}`);
    }
    if (command === 'restore_files') return webRequest('/api/recycle/restore', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'permanently_delete_files') return webRequest('/api/recycle/delete', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'clear_recycle_bin') return webRequest('/api/recycle/clear', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'batch_rename_files') return webRequest('/api/files/rename-batch', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'export_gcid_json') return webRequest('/api/files/export-gcid', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'export_gcid_diagnostic_log') return webRequest('/api/files/export-gcid-log');
    if (command === 'get_cloud_download') return webRequest('/api/files/download', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'create_share') return webRequest('/api/share', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_shares') return webRequest('/api/shares');
    if (command === 'delete_shares') return webRequest('/api/shares/delete', { method: 'POST', body: JSON.stringify({ ...args, ids: args.ids || args.share_ids }) });
    if (command === 'update_share') return webRequest('/api/shares/update', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'delete_invalid_shares') return webRequest('/api/shares/delete-invalid', { method: 'POST', body: '{}' });
    if (command === 'set_direct_link') return webRequest('/api/direct-link/set', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'unset_direct_link') return webRequest('/api/direct-link/unset', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_direct_link') return webRequest('/api/direct-link/get', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'open_received_share') return webRequest('/api/received-share/open', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_received_share_files') return webRequest('/api/received-share/files', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'restore_received_share') return webRequest('/api/received-share/restore', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_received_share_download') return webRequest('/api/received-share/download', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'pause_download') throw new Error('Docker Web 下载由浏览器接管，请在浏览器下载面板中暂停');
    if (command === 'resume_download') throw new Error('Docker Web 下载由浏览器接管，请在浏览器下载面板中继续');
    if (command === 'cancel_download') throw new Error('Docker Web 下载由浏览器接管，请在浏览器下载面板中取消');
    if (command === 'create_offline_task') return webRequest('/api/offline', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'resolve_offline_resource') return webRequest('/api/offline/resolve', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_offline_tasks') {
      const params = new URLSearchParams({
        cursor: String(args.cursor || ''),
        pageSize: String(Math.max(1, Number(args.page_size || 100))),
      });
      if (args.status !== undefined && args.status !== null && args.status !== '') params.set('status', String(args.status));
      return webRequest(`/api/offline?${params}`);
    }
    if (command === 'cancel_offline_tasks') return webRequest('/api/offline/cancel', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'delete_offline_tasks') return webRequest('/api/offline/delete', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'retry_offline_tasks') return webRequest('/api/offline/retry', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_offline_statistics') return webRequest('/api/offline/statistics');
    if (command === 'get_assets') return webRequest('/api/assets');
    if (command === 'get_global_config') return webRequest('/api/global-config');
    if (command === 'save_share_link') return webRequest('/api/share-links', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'remove_share_link') return webRequest(`/api/share-links/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'add_mapping') return webRequest('/api/mappings', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'remove_mapping') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'toggle_mapping') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ enabled: args.enabled }) });
    if (command === 'update_mapping_sync_types') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ sync_types: args.sync_types }) });
    if (command === 'update_mapping_monitor_mode') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ monitor_mode: args.monitor_mode }) });
    if (command === 'update_mapping_auto_share') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ auto_share: args.auto_share }) });
    if (command === 'update_mapping_organizer') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ organizer_mapping_id: args.organizer_mapping_id || '' }) });
    if (command === 'update_hdhive_config') return webRequest('/api/hdhive/config', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'backfill_auto_shares') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}/auto-share-backfill`, { method: 'POST', body: '{}' });
    if (command === 'retry_auto_share_event') return webRequest(`/api/auto-share/events/${encodeURIComponent(args.event_id)}/retry`, { method: 'POST', body: JSON.stringify({ tmdb_id: args.tmdb_id, media_type: args.media_type }) });
    if (command === 'pause_queue') return webRequest('/api/queue/pause', { method: 'POST' });
    if (command === 'resume_queue') return webRequest('/api/queue/resume', { method: 'POST' });
    if (command === 'pause_upload') return webRequest('/api/uploads/pause', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'resume_upload') return webRequest('/api/uploads/resume', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'cancel_upload') return webRequest('/api/uploads/cancel', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'retry_upload') return webRequest('/api/uploads/retry', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_transfer_settings') return webRequest('/api/settings');
    if (command === 'update_transfer_settings') return webRequest('/api/settings/transfer', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_offline_settings') return webRequest('/api/settings/offline');
    if (command === 'update_offline_settings') return webRequest('/api/settings/offline', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_network_preferences') return webRequest('/api/settings/network');
    if (command === 'update_network_preferences') return webRequest('/api/settings/network', { method: 'POST', body: JSON.stringify(args.input || args) });
    if (command === 'test_network') return webRequest('/api/network/test', { method: 'POST', body: JSON.stringify(args.input || args) });
    if (command === 'get_cache_settings') return webRequest('/api/settings/cache');
    if (command === 'update_cache_settings') return webRequest('/api/settings/cache', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_metadata_cache_stats') return webRequest('/api/cache');
    if (command === 'clear_metadata_cache') return webRequest('/api/cache/clear', { method: 'POST', body: '{}' });
    if (command === 'get_developer_settings') return webRequest('/api/developer/settings');
    if (command === 'update_developer_credentials') return webRequest('/api/developer/credentials', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'test_developer_credentials') return webRequest('/api/developer/test', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'update_developer_mode') return webRequest('/api/developer/mode', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'upsert_developer_target') return webRequest('/api/developer/targets', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'delete_developer_target') return webRequest(`/api/developer/targets/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'list_developer_transfers') {
      const params = new URLSearchParams({ limit: String(Math.max(1, Number(args.limit || 50))) });
      return webRequest(`/api/developer/transfers?${params}`);
    }
    if (command === 'start_developer_transfer') return webRequest('/api/developer/transfers', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_organizer_state') return webRequest('/api/organizer');
    if (command === 'update_organizer_settings') return webRequest('/api/organizer/settings', { method: 'PUT', body: JSON.stringify(args.input || args) });
    if (command === 'test_organizer_connection') return webRequest('/api/organizer/test', { method: 'POST', body: JSON.stringify(args.input || args) });
    if (command === 'add_organizer_mapping') return webRequest('/api/organizer/mappings', { method: 'POST', body: JSON.stringify(args.input || args) });
    if (command === 'update_organizer_mapping') return webRequest(`/api/organizer/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify(args.input || {}) });
    if (command === 'remove_organizer_mapping') return webRequest(`/api/organizer/mappings/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'remove_organizer_job') return webRequest(`/api/organizer/jobs/${encodeURIComponent(args.id)}`, { method: 'DELETE', body: JSON.stringify(args.input || {}) });
    if (command === 'scan_organizer_mapping') return webRequest(`/api/organizer/mappings/${encodeURIComponent(args.id)}/scan`, { method: 'POST', body: '{}' });
    if (command === 'run_organizer_job') return webRequest(`/api/organizer/jobs/${encodeURIComponent(args.id)}/run`, { method: 'POST', body: JSON.stringify(args.input || {}) });
    if (command === 'retry_organizer_job') return webRequest(`/api/organizer/jobs/${encodeURIComponent(args.id)}/retry`, { method: 'POST', body: JSON.stringify(args.input || {}) });
    if (command === 'rearchive_organizer_job') return webRequest(`/api/organizer/jobs/${encodeURIComponent(args.id)}/rearchive`, { method: 'POST', body: JSON.stringify(args.input || {}) });
    if (command === 'share_organizer_job') return webRequest(`/api/organizer/jobs/${encodeURIComponent(args.id)}/share`, { method: 'POST', body: '{}' });
    if (command === 'scrape_selected_files') return webRequest('/api/organizer/scrape-selected', { method: 'POST', body: JSON.stringify(args.input || args) });
    if (command === 'get_app_version') return { version: 'Docker Web' };
    if (command === 'fetch_app_update' || command === 'install_app_update') {
      throw new Error('Docker Web 版本随镜像更新，桌面自动更新仅在 Windows 客户端可用');
    }
    if (command === 'request_sms_code') return webRequest('/api/auth/sms/send', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'login_with_sms') return webRequest('/api/auth/sms/login', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'poll_device_login') return webRequest('/api/auth/device/poll', { method: 'POST', body: JSON.stringify(args) });
    throw new Error(`Docker Web 端暂不支持命令：${command}`);
  },
  subscribe: async (callback) => {
    const source = new EventSource('/api/events');
    source.onmessage = (event) => callback(JSON.parse(event.data));
    return () => source.close();
  },
  subscribeAuthExpired,
  subscribeDrag: async () => () => {},
  selectFolder: async () => null,
  selectUploadFiles: async () => [],
  selectUploadFolder: async () => null,
  login: () => webRequest('/api/auth/device/start', { method: 'POST', body: '{}' }),
};
