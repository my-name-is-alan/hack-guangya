import { readJsonResponse } from './httpResponse.js';

const tauriInvoke = window.__TAURI__?.core?.invoke;
const tauriListen = window.__TAURI__?.event?.listen;
export const isTauri = Boolean(tauriInvoke && tauriListen);

const camelizeArgs = (args = {}) => Object.fromEntries(
  Object.entries(args).map(([key, value]) => [key.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase()), value]),
);

async function webRequest(url, options = {}) {
  const response = await fetch(url, {
    credentials: 'same-origin',
    ...options,
    headers: { 'content-type': 'application/json', ...(options.headers || {}) },
  });
  return readJsonResponse(response, `请求 ${url} 失败`);
}
export { webRequest };

export const bridge = isTauri ? {
  invoke: (command, args = {}) => tauriInvoke(command, camelizeArgs(args)),
  subscribe: (callback) => tauriListen('sync-event', ({ payload }) => callback(payload)),
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
    if (command === 'get_access_status') return webRequest('/api/access/status');
    if (command === 'unlock_access') return webRequest('/api/access/unlock', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'update_access_code') return webRequest('/api/access/code', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_files') return webRequest(`/api/files?page=${args.page || 0}&parentId=${encodeURIComponent(args.parent_id || '')}`);
    if (command === 'search_files') {
      const params = new URLSearchParams({ query: String(args.query || ''), type: String(args.file_type || ''), extension: String(args.extension || ''), page: String(args.page || 0) });
      return webRequest(`/api/search?${params}`);
    }
    if (command === 'copy_files') return webRequest('/api/files/copy', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'move_files') return webRequest('/api/files/move', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'delete_files') return webRequest('/api/files/delete', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'batch_rename_files') return webRequest('/api/files/rename-batch', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_cloud_download') return webRequest('/api/files/download', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'create_share') return webRequest('/api/share', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_shares') return webRequest('/api/shares');
    if (command === 'delete_shares') return webRequest('/api/shares/delete', { method: 'POST', body: JSON.stringify({ ...args, ids: args.ids || args.share_ids }) });
    if (command === 'open_received_share') return webRequest('/api/received-share/open', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_received_share_files') return webRequest('/api/received-share/files', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'restore_received_share') return webRequest('/api/received-share/restore', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_received_share_download') return webRequest('/api/received-share/download', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'create_offline_task') return webRequest('/api/offline', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_offline_tasks') return webRequest('/api/offline');
    if (command === 'save_share_link') return webRequest('/api/share-links', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'remove_share_link') return webRequest(`/api/share-links/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'add_mapping') return webRequest('/api/mappings', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'remove_mapping') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'DELETE' });
    if (command === 'toggle_mapping') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ enabled: args.enabled }) });
    if (command === 'update_mapping_sync_types') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ sync_types: args.sync_types }) });
    if (command === 'update_mapping_monitor_mode') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ monitor_mode: args.monitor_mode }) });
    if (command === 'update_mapping_auto_share') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}`, { method: 'PATCH', body: JSON.stringify({ auto_share: args.auto_share }) });
    if (command === 'update_hdhive_config') return webRequest('/api/hdhive/config', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'backfill_auto_shares') return webRequest(`/api/mappings/${encodeURIComponent(args.id)}/auto-share-backfill`, { method: 'POST', body: '{}' });
    if (command === 'retry_auto_share_event') return webRequest(`/api/auto-share/events/${encodeURIComponent(args.event_id)}/retry`, { method: 'POST', body: JSON.stringify({ tmdb_id: args.tmdb_id, media_type: args.media_type }) });
    if (command === 'pause_queue') return webRequest('/api/queue/pause', { method: 'POST' });
    if (command === 'resume_queue') return webRequest('/api/queue/resume', { method: 'POST' });
    if (command === 'get_transfer_settings' || command === 'get_settings') return webRequest('/api/settings');
    if (command === 'update_transfer_settings') return webRequest('/api/settings/transfer', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_cache_settings') return webRequest('/api/settings/cache');
    if (command === 'update_cache_settings') return webRequest('/api/settings/cache', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_metadata_cache_stats' || command === 'get_cache_stats') return webRequest('/api/cache');
    if (command === 'clear_metadata_cache' || command === 'clear_cache') return webRequest('/api/cache/clear', { method: 'POST', body: '{}' });
    if (command === 'request_sms_code') return webRequest('/api/auth/sms/send', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'login_with_sms') return webRequest('/api/auth/sms/login', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'poll_device_login') return webRequest('/api/auth/device/poll', { method: 'POST', body: JSON.stringify(args) });
    return null;
  },
  subscribe: async (callback) => {
    const source = new EventSource('/api/events');
    source.onmessage = (event) => callback(JSON.parse(event.data));
    return () => source.close();
  },
  subscribeDrag: async () => () => {},
  selectFolder: async () => null,
  selectUploadFiles: async () => [],
  selectUploadFolder: async () => null,
  login: () => webRequest('/api/auth/device/start', { method: 'POST', body: '{}' }),
};
