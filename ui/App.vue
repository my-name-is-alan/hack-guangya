<script setup>
import { computed, h, onBeforeUnmount, onMounted, reactive, ref } from 'vue';
import { message } from 'antdv-next';
import appLogo from '../src-tauri/icons/128x128.png';
import { buildRenamePreview } from './renameRules.js';
import { formatUploadSpeed, nextUploadProgress } from './uploadProgress.js';
import { readJsonResponse } from './httpResponse.js';
import { parseGuangyaShareLink } from './shareLink.js';
import {
  ArrowDownOutlined,
  ArrowUpOutlined,
  CheckCircleFilled,
  CheckOutlined,
  ClockCircleOutlined,
  CloudOutlined,
  CloudSyncOutlined,
  CopyOutlined,
  DeleteOutlined,
  DownloadOutlined,
  DragOutlined,
  EditOutlined,
  FileAddOutlined,
  FileOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  HomeOutlined,
  InboxOutlined,
  LinkOutlined,
  LoginOutlined,
  MoreOutlined,
  PauseCircleOutlined,
  PlayCircleOutlined,
  PlusOutlined,
  QrcodeOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
  ScissorOutlined,
  ShareAltOutlined,
  SwapOutlined,
  SyncOutlined,
  UploadOutlined,
  UserOutlined,
} from '@antdv-next/icons';

const tauriInvoke = window.__TAURI__?.core?.invoke;
const tauriListen = window.__TAURI__?.event?.listen;
const isTauri = Boolean(tauriInvoke && tauriListen);
const camelizeArgs = (args = {}) => Object.fromEntries(Object.entries(args).map(([key, value]) => [key.replace(/_([a-z])/g, (_, letter) => letter.toUpperCase()), value]));

async function webRequest(url, options = {}) {
  const response = await fetch(url, { headers: { 'content-type': 'application/json' }, ...options });
  return readJsonResponse(response, `请求 ${url} 失败`);
}

const bridge = isTauri ? {
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
    if (command === 'list_files') return webRequest(`/api/files?page=${args.page || 0}&parentId=${encodeURIComponent(args.parent_id || '')}`);
    if (command === 'copy_files') return webRequest('/api/files/copy', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'move_files') return webRequest('/api/files/move', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'delete_files') return webRequest('/api/files/delete', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'batch_rename_files') return webRequest('/api/files/rename-batch', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'get_cloud_download') return webRequest('/api/files/download', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'create_share') return webRequest('/api/share', { method: 'POST', body: JSON.stringify(args) });
    if (command === 'list_shares') return webRequest('/api/shares');
    if (command === 'delete_shares') return webRequest('/api/shares/delete', { method: 'POST', body: JSON.stringify(args) });
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

const theme = {
  token: {
    colorPrimary: '#1677ff',
    colorInfo: '#1677ff',
    colorSuccess: '#16a672',
    colorWarning: '#f59e0b',
    colorError: '#e5484d',
    borderRadius: 8,
    borderRadiusLG: 12,
    controlHeight: 28,
    controlHeightSM: 22,
    controlHeightLG: 34,
    fontSize: 13,
    colorBgLayout: '#f5f7fb',
    colorText: '#172033',
    colorTextSecondary: '#6f7a8a',
    fontFamily: "Inter, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
  },
  components: {
    Layout: { bodyBg: '#f5f7fb', siderBg: '#ffffff', headerBg: '#f5f7fb' },
    Menu: { itemBorderRadius: 8, itemHeight: 38, itemMarginInline: 10 },
    Table: { headerBg: '#fafbfc', headerColor: '#667085', rowHoverBg: '#f5f9ff' },
  },
};

const navigation = [
  { key: 'cloud', label: '云盘文件', icon: () => h(CloudOutlined) },
  { key: 'backup', label: '备份任务', icon: () => h(CloudSyncOutlined) },
  { key: 'downloads', label: '下载管理', icon: () => h(DownloadOutlined) },
  { key: 'offline', label: '离线下载', icon: () => h(DownloadOutlined) },
  { key: 'shares', label: '分享管理', icon: () => h(ShareAltOutlined) },
];
const pageMeta = {
  cloud: ['云盘文件', '浏览、整理和分享云端内容'],
  backup: ['备份任务', '持续监控本地文件夹并自动上传'],
  downloads: ['下载管理', '查看保存到本机的下载任务和实时进度'],
  offline: ['离线下载', '让云端代你下载链接资源'],
  shares: ['分享管理', '查询、复制和取消当前账号创建的分享'],
};

const extensionPresets = [
  { key: 'video', label: '视频', extensions: ['mp4', 'mov', 'mkv', 'avi', 'wmv', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts', '3gp'] },
  { key: 'image', label: '图片', extensions: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff', 'raw', 'cr2', 'nef', 'arw', 'dng'] },
  { key: 'subtitle', label: '字幕', extensions: ['srt', 'ass', 'ssa', 'vtt', 'sub', 'idx', 'sup', 'lrc'] },
  { key: 'audio', label: '音频', extensions: ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'wma', 'aiff'] },
];
const defaultSyncExtensions = [...extensionPresets.find((item) => item.key === 'image').extensions, ...extensionPresets.find((item) => item.key === 'video').extensions, ...extensionPresets.find((item) => item.key === 'audio').extensions];

const activeView = ref('cloud');
const appState = reactive({ logged_in: false, paused: false, pending: 0, active_uploads: 0, mappings: [], saved_shares: [], hdhive: { configured: false, base_url: '', instance_id: '' }, auto_share_receipts: [] });
const overview = reactive({ profile: {}, assets: {} });
const files = ref([]);
const filesLoading = ref(false);
const downloadTasks = ref([]);
const currentParentId = ref('');
const currentFolderName = ref('根目录');
const currentPath = ref([]);
const selectedFileIds = ref([]);
const offlineTasks = ref([]);
const offlineLoading = ref(false);
const cloudShares = ref([]);
const cloudSharesLoading = ref(false);
const events = ref([]);
const uploadProgress = ref({});
const backupOpen = ref(false);
const shareOpen = ref(false);
const loginOpen = ref(false);
const shareResultOpen = ref(false);
const loginToken = ref('');
const lastShare = reactive({ label: '', url: '', code: '', reused: false, hdhiveStatus: '', hdhiveMessage: '', hdhiveEventId: '' });
const backupForm = reactive({ local_path: '', remote_path: '', remote_parent_id: '', source_policy: 'keep', archive_path: '', scan_existing: true, sync_types: [...defaultSyncExtensions], monitor_mode: 'native', auto_share: false });
const hdhiveForm = reactive({ base_url: '', secret: '' });
const hdhiveSubmitting = ref(false);
const autoShareBusy = reactive({});
const receiptReview = reactive({});
const offlineForm = reactive({ url: '', parent_id: '', parent_label: '根目录', new_name: '' });
const shareForm = reactive({ label: '', url: '' });
const receivedShare = reactive({ open: false, link: '', loading: false, restoring: false, downloading: false, shareId: '', code: '', accessToken: '', items: [], selected: [], stack: [], targetId: '', targetLabel: '根目录' });
const login = reactive({ loading: false, qr: '', userCode: '—', verificationUrl: '', message: '等待扫码确认', remaining: 0 });
const folderPicker = reactive({ open: false, title: '选择云端目录', loading: false, items: [], stack: [], onConfirm: null });
const clipboard = reactive({ mode: '', items: [] });
const contextMenu = reactive({ visible: false, x: 0, y: 0, record: null });
const deleteDialog = reactive({ open: false, items: [], loading: false });
const renameOpen = ref(false);
const renameTargets = ref([]);
const renameRules = ref([]);
const preserveExtension = ref(true);
const renaming = ref(false);
const operationBusy = ref(false);
const cloudDownloadBusy = ref(false);
const backupSubmitting = ref(false);
const shareCreating = ref(false);
const dragActive = ref(false);
const uploadingCount = ref(0);
const fileInput = ref(null);
const folderInput = ref(null);
const uploadSourceOpen = ref(false);
const uploadSourceKind = ref('files');
const serverFilePicker = reactive({ open: false, loading: false, submitting: false, mode: 'upload', targetField: '', roots: [], path: '', parent: '', displayPath: '/', items: [], selected: [] });
const activeServerRoot = computed(() => [...serverFilePicker.roots]
  .sort((left, right) => right.length - left.length)
  .find((root) => serverFilePicker.path === root || serverFilePicker.path.startsWith(root.endsWith('/') || root.endsWith('\\') ? root : `${root}${root.includes('\\') ? '\\' : '/'}`)) || serverFilePicker.roots[0]);
let devicePollTimer = null;
let deviceExpiryTimer = null;
let unsubscribe = null;
let unsubscribeDrag = null;
let ruleId = 0;
let refreshTimer = null;
const uploadRemovalTimers = new Map();

const pageTitle = computed(() => pageMeta[activeView.value][0]);
const pageSubtitle = computed(() => pageMeta[activeView.value][1]);
const userName = computed(() => pick(overview.profile, ['nickname', 'nickName', 'userName', 'name'], appState.logged_in ? '光鸭云盘用户' : '尚未登录'));
const userAvatar = computed(() => normalizeAvatarUrl(pick(overview.profile, [
  'picture',
  'avatar',
  'avatarUrl',
  'avatar_url',
  'photoUrl',
  'photo_url',
  'headImgUrl',
  'headImageUrl',
  'headPic',
], '')));
const usedSpace = computed(() => pick(overview.assets, ['usedSpaceSize', 'usedSpace', 'useSpace', 'used', 'usedSize'], 0));
const totalSpace = computed(() => pick(overview.assets, ['totalSpaceSize', 'totalSpace', 'capacity', 'total', 'totalSize'], 0));
const quotaPercent = computed(() => totalSpace.value ? Math.min(100, Math.round(Number(usedSpace.value) / Number(totalSpace.value) * 100)) : 0);
const profileId = computed(() => pick(overview.profile, ['sub', 'userId', 'id'], '—'));
const profilePhone = computed(() => pick(overview.profile, ['phone_number', 'phoneNumber', 'phone', 'mobile'], '未绑定'));
const vipStatus = computed(() => Number(pick(overview.assets, ['vipStatus', 'svipStatus'], 0)));
const isVip = computed(() => [2].includes(vipStatus.value));
const vipExpired = computed(() => vipStatus.value === 3);
const vipExpireTime = computed(() => pick(overview.assets, ['vipExpireTime', 'vipExpireAt', 'vipEndTime'], 0));
const vipLabel = computed(() => isVip.value ? 'VIP会员' : vipExpired.value ? 'VIP已过期' : '普通用户');
const vipExpireLabel = computed(() => vipExpireTime.value ? formatTime(vipExpireTime.value) : isVip.value ? '未返回到期时间' : '未开通 VIP');
const queueText = computed(() => appState.paused ? '队列已暂停' : appState.active_uploads ? '正在上传' : appState.pending ? '等待上传' : '队列空闲');
const recentUploads = computed(() => Object.values(uploadProgress.value).sort((left, right) => right.updatedAt - left.updatedAt).slice(0, 8));
const totalUploadSpeed = computed(() => recentUploads.value.reduce((total, upload) => total + (upload.state === 'uploading' ? Number(upload.bytesPerSecond || 0) : 0), 0));
const selectedFiles = computed(() => files.value.filter((item) => selectedFileIds.value.includes(fileId(item))));
const activeDownloadCount = computed(() => downloadTasks.value.filter((task) => ['preparing', 'downloading'].includes(task.status)).length);
const currentFolderPath = computed(() => currentPath.value.length ? `根目录 / ${currentPath.value.map((item) => item.name).join(' / ')}` : '根目录');
const clipboardLabel = computed(() => clipboard.items.length ? `${clipboard.mode === 'move' ? '剪切' : '复制'} ${clipboard.items.length} 项` : '');
const lastShareReceipt = computed(() => appState.auto_share_receipts.find((receipt) => receipt.event_id === lastShare.hdhiveEventId) || null);
const lastShareHdhiveStatus = computed(() => lastShareReceipt.value?.status || lastShare.hdhiveStatus);
const lastShareHdhiveMessage = computed(() => receiptDisplayMessage(lastShareReceipt.value) || lastShare.hdhiveMessage);
const receivedSharePath = computed(() => receivedShare.stack.length ? `分享根目录 / ${receivedShare.stack.map((item) => item.name).join(' / ')}` : '分享根目录');
const sourcePolicyLabel = (value) => ({ keep: '保留源文件', archive: '上传后归档', delete: '上传后删除' }[value] || value);
const sourcePolicyColor = (value) => ({ keep: 'blue', archive: 'gold', delete: 'red' }[value] || 'default');

const OFFLINE_STATUS_MAP = {
  0: ['排队等待', 'default'],
  1: ['下载中', 'processing'],
  2: ['已完成', 'success'],
  3: ['下载失败', 'error'],
  4: ['已取消', 'warning'],
  5: ['资源违规', 'error'],
};
function offlineStatus(record) {
  const raw = pick(record, ['status', 'taskStatus', 'state'], null);
  if (raw === null || raw === '') {
    const err = Number(pick(record, ['errCode', 'errorCode'], 0));
    return err ? ['下载失败', 'error'] : ['处理中', 'processing'];
  }
  const text = String(raw).trim();
  if (/^-?\d+$/.test(text)) return OFFLINE_STATUS_MAP[Number(text)] || ['处理中', 'processing'];
  const lowered = text.toLowerCase();
  if (['success', 'done', 'complete', 'completed', 'finish', 'finished'].includes(lowered)) return ['已完成', 'success'];
  if (['fail', 'failed', 'error'].includes(lowered)) return ['下载失败', 'error'];
  if (['cancel', 'canceled', 'cancelled'].includes(lowered)) return ['已取消', 'warning'];
  if (['pending', 'waiting', 'queue', 'queued'].includes(lowered)) return ['排队等待', 'default'];
  return ['下载中', 'processing'];
}

const fileColumns = [
  { title: '名称', key: 'name', dataIndex: 'fileName' },
  { title: '类型', key: 'type', width: 110 },
  { title: '大小', key: 'size', width: 120 },
  { title: '更新时间', key: 'time', width: 180 },
];
const offlineColumns = [
  { title: '任务名称', key: 'name' },
  { title: '大小', key: 'size', width: 130 },
  { title: '状态', key: 'status', width: 120 },
];
const receivedShareColumns = [
  { title: '名称', key: 'name', dataIndex: 'fileName' },
  { title: '类型', key: 'type', width: 100 },
  { title: '大小', key: 'size', width: 130 },
];
const cloudShareColumns = [
  { title: '分享名称', key: 'title', dataIndex: 'title' },
  { title: '类型', key: 'type', width: 100 },
  { title: '状态', key: 'status', width: 110 },
  { title: '创建时间', key: 'time', width: 180 },
  { title: '操作', key: 'actions', width: 230 },
];
const sourcePolicyOptions = [
  { value: 'keep', label: '保留源文件（推荐）' },
  { value: 'archive', label: '移动到归档目录' },
  { value: 'delete', label: '删除源文件（谨慎）' },
];
const monitorModeOptions = [
  { value: 'native', label: '本地文件夹（系统事件）' },
  { value: 'polling', label: '网盘挂载（每 5 秒轮询）' },
];
const monitorModeLabel = (value) => value === 'polling' ? '轮询监控' : '系统事件';
function normalizeExtensions(values) {
  return [...new Set((Array.isArray(values) ? values : []).map((value) => String(value).trim().replace(/^\./, '').toLowerCase()).filter((value) => /^[a-z0-9]{1,16}$/.test(value)))];
}
function mappingExtensions(mapping) {
  const values = normalizeExtensions(mapping.sync_types);
  return values.length ? values : [...defaultSyncExtensions];
}
function syncTypeSummary(mapping) {
  const values = mappingExtensions(mapping);
  const preview = values.slice(0, 5).map((value) => `.${value}`).join('、');
  return values.length > 5 ? `${preview} 等 ${values.length} 种` : preview;
}
const renameRuleOptions = [
  { value: 'set', label: '设置名称' },
  { value: 'replace', label: '文本替换' },
  { value: 'regex', label: '正则替换' },
  { value: 'prefix', label: '添加前缀' },
  { value: 'suffix', label: '添加后缀' },
  { value: 'sequence', label: '追加序号' },
  { value: 'upper', label: '转为大写' },
  { value: 'lower', label: '转为小写' },
];
const rowSelection = computed(() => ({
  selectedRowKeys: selectedFileIds.value,
  onChange: (keys) => { selectedFileIds.value = keys; },
}));
const receivedShareRowSelection = computed(() => ({
  selectedRowKeys: receivedShare.selected,
  onChange: (keys) => { receivedShare.selected = keys; },
}));
const renamePreview = computed(() => buildRenamePreview(renameTargets.value, renameRules.value, preserveExtension.value));
const renameChangedCount = computed(() => renamePreview.value.rows.filter((row) => row.currentName !== row.newName).length);

function pick(object, keys, fallback = '') {
  for (const key of keys) if (object && object[key] !== undefined && object[key] !== null && object[key] !== '') return object[key];
  return fallback;
}
function normalizeAvatarUrl(value) {
  const source = value && typeof value === 'object' ? pick(value, ['url', 'src', 'original', 'large'], '') : value;
  const url = String(source || '').trim();
  return url.startsWith('//') ? `https:${url}` : url;
}
function unwrapData(payload) { return payload?.data || payload || {}; }
function errorText(error) { return error instanceof Error ? error.message : String(error?.message || error || '未知错误'); }
function formatSize(size) {
  const number = Number(size || 0);
  if (!number) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let index = 0;
  let value = number;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
  return `${value >= 10 || index === 0 ? Math.round(value) : value.toFixed(1)} ${units[index]}`;
}
function formatTime(value) {
  if (!value) return '—';
  const number = Number(value);
  const date = new Date(number < 10 ** 12 ? number * 1000 : number);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
}
function fileId(record) { return pick(record, ['fileId', 'id']); }
function isFolder(record) { return Number(record.resType) === 2; }
function appendEvent(level, text) {
  events.value.unshift({ id: `${Date.now()}-${Math.random()}`, level, text, time: new Date().toLocaleTimeString() });
  events.value = events.value.slice(0, 50);
}
function uploadFileName(filePath) {
  return String(filePath || '').split(/[\\/]/).filter(Boolean).pop() || '未命名文件';
}
function updateUploadProgress(payload) {
  const filePath = String(payload?.file_path || '');
  if (!filePath) return;
  const previous = uploadProgress.value[filePath] || { percent: 0, state: 'queued', stage: '排队等待' };
  const next = nextUploadProgress(previous, payload);
  if (next === previous) return;
  const pendingRemoval = uploadRemovalTimers.get(filePath);
  if (pendingRemoval) {
    clearTimeout(pendingRemoval);
    uploadRemovalTimers.delete(filePath);
  }
  uploadProgress.value = {
    ...uploadProgress.value,
    [filePath]: {
      filePath,
      fileName: uploadFileName(filePath),
      ...next,
    },
  };
  const entries = Object.entries(uploadProgress.value).sort((left, right) => right[1].updatedAt - left[1].updatedAt).slice(0, 30);
  uploadProgress.value = Object.fromEntries(entries);
  if (next.state === 'done') {
    const completedAt = next.updatedAt;
    uploadRemovalTimers.set(filePath, setTimeout(() => {
      const current = uploadProgress.value[filePath];
      uploadRemovalTimers.delete(filePath);
      if (current?.state !== 'done' || current.updatedAt !== completedAt) return;
      const remaining = { ...uploadProgress.value };
      delete remaining[filePath];
      uploadProgress.value = remaining;
    }, 3000));
  }
}
function applyState(next = {}) {
  Object.assign(appState, next);
  if (next.mappings) appState.mappings = next.mappings;
  if (next.saved_shares) appState.saved_shares = next.saved_shares;
  if (next.auto_share_receipts) {
    appState.auto_share_receipts = next.auto_share_receipts;
    for (const receipt of next.auto_share_receipts) if (!receiptReview[receipt.event_id]) receiptReview[receipt.event_id] = { tmdb_id: '', media_type: 'tv' };
  }
  if (next.hdhive) {
    appState.hdhive = next.hdhive;
    if (!hdhiveForm.base_url || hdhiveForm.base_url === appState.hdhive.base_url) hdhiveForm.base_url = next.hdhive.base_url || '';
  }
}
function selectView({ key }) {
  activeView.value = key;
  if (key === 'cloud' && appState.logged_in) loadFiles();
  if (key === 'offline' && appState.logged_in) loadOffline();
  if (key === 'shares' && appState.logged_in) loadCloudShares();
}

function fileBaseName(record) {
  const name = pick(record, ['fileName', 'name'], '');
  if (isFolder(record)) return name;
  const index = name.lastIndexOf('.');
  return index > 0 ? name.slice(0, index) : name;
}
function selectedOr(record) {
  if (record && !selectedFileIds.value.includes(fileId(record))) selectedFileIds.value = [fileId(record)];
  return record ? files.value.filter((item) => selectedFileIds.value.includes(fileId(item))) : selectedFiles.value;
}
function scheduleFileRefresh() {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => { if (activeView.value === 'cloud' && appState.logged_in) loadFiles(); }, 1200);
}

async function loadOverview() {
  if (!appState.logged_in) return;
  try {
    const data = unwrapData(await bridge.invoke('get_overview'));
    const profile = data.profile || {};
    const assets = data.assets || {};
    overview.profile = profile.data || profile.user || profile;
    overview.assets = assets.data || assets;
  } catch {
    overview.profile = {};
    overview.assets = {};
  }
}
async function loadFiles() {
  if (!appState.logged_in) return;
  filesLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_files', { parent_id: currentParentId.value, page: 0 }));
    files.value = data.list || [];
    selectedFileIds.value = [];
  } catch (error) {
    message.error(errorText(error));
  } finally {
    filesLoading.value = false;
  }
}
async function loadCloudShares() {
  if (!appState.logged_in) return;
  cloudSharesLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_shares'));
    cloudShares.value = Array.isArray(data.list) ? data.list : [];
  } catch (error) {
    message.error(`分享列表加载失败：${errorText(error)}`);
  } finally {
    cloudSharesLoading.value = false;
  }
}
function cloudShareStatus(record) {
  return ({ 1: ['分享中', 'success'], 2: ['已过期', 'warning'], 3: ['已取消', 'default'], 4: ['已封禁', 'error'] })[Number(record.shareStatus)] || ['未知', 'default'];
}
async function deleteCloudShare(record) {
  try {
    await bridge.invoke('delete_shares', { ids: [record.id] });
    message.success('已取消分享');
    await loadCloudShares();
  } catch (error) {
    message.error(errorText(error));
  }
}
async function openFolder(record) {
  if (!isFolder(record)) return;
  currentParentId.value = fileId(record);
  currentFolderName.value = pick(record, ['fileName', 'name'], '文件夹');
  currentPath.value = [...currentPath.value, { id: currentParentId.value, name: currentFolderName.value }];
  await loadFiles();
}
async function goRoot() {
  currentParentId.value = '';
  currentFolderName.value = '根目录';
  currentPath.value = [];
  await loadFiles();
}
async function goToPath(index) {
  if (index < 0) return goRoot();
  currentPath.value = currentPath.value.slice(0, index + 1);
  const target = currentPath.value[index];
  currentParentId.value = target.id;
  currentFolderName.value = target.name;
  await loadFiles();
}
async function createShare() {
  if (!selectedFileIds.value.length || shareCreating.value) return;
  shareCreating.value = true;
  let closeProgress;
  try {
    closeProgress = message.loading('正在创建分享，请稍候…', 0);
    const names = selectedFiles.value
      .map((item) => String(pick(item, ['fileName', 'name'], '')).trim())
      .filter(Boolean);
    const title = names.length > 1 ? `${names[0]} 等 ${names.length} 项` : names[0] || '云盘分享';
    const targetType = selectedFiles.value.length === 1 && isFolder(selectedFiles.value[0]) ? 'folder' : 'file';
    const data = unwrapData(await bridge.invoke('create_share', { file_ids: selectedFileIds.value, title, target_type: targetType }));
    lastShare.url = pick(data, ['shareUrl', 'share_url', 'url']);
    lastShare.code = pick(data, ['code', 'extractCode']);
    lastShare.reused = data.reused_existing === true;
    lastShare.label = files.value.find((item) => selectedFileIds.value.includes(fileId(item)))?.fileName || '云盘分享';
    lastShare.hdhiveStatus = pick(data, ['hdhive_status'], 'delivery_failed');
    lastShare.hdhiveMessage = pick(data, ['hdhive_message'], '光鸭分享成功，但未收到影巢回执');
    lastShare.hdhiveEventId = pick(data, ['hdhive_event_id']);
    if (!lastShare.url) throw new Error('光鸭没有返回分享链接');
    shareResultOpen.value = true;
    if (['accepted', 'processing', 'completed'].includes(lastShare.hdhiveStatus)) message.success(lastShare.reused ? '已复用已有分享，影巢将只更新备注' : '光鸭分享成功，已提交影巢处理');
    else message.warning(lastShare.hdhiveMessage);
  } catch (error) { message.error(errorText(error)); }
  finally {
    closeProgress?.();
    shareCreating.value = false;
  }
}
async function saveCreatedShare() {
  try {
    await bridge.invoke('save_share_link', { label: lastShare.label, url: lastShare.url });
    shareResultOpen.value = false;
    message.success('分享链接已收藏');
  } catch (error) { message.error(errorText(error)); }
}
function openShareForm() {
  shareForm.label = '';
  shareForm.url = '';
  shareOpen.value = true;
}
async function saveShareLink() {
  if (!shareForm.url.trim()) return;
  try {
    await bridge.invoke('save_share_link', { label: shareForm.label || '分享链接', url: shareForm.url });
    shareOpen.value = false;
    message.success('分享链接已保存');
  } catch (error) { message.error(errorText(error)); }
}
async function removeShare(id) {
  try { await bridge.invoke('remove_share_link', { id }); message.success('已移除收藏'); }
  catch (error) { message.error(errorText(error)); }
}
async function copyText(value) {
  try { await navigator.clipboard.writeText(value); message.success('已复制到剪贴板'); }
  catch { message.info(value); }
}
function localDownloadName(items, packaged) {
  const name = String(pick(items[0], ['fileName', 'name'], '') || '').trim();
  if (!packaged) return name || '光鸭下载';
  if (items.length === 1 && name) return /\.zip$/i.test(name) ? name : `${name}.zip`;
  return `光鸭批量下载-${new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19)}.zip`;
}
function newDownloadId() {
  return globalThis.crypto?.randomUUID?.() || `download-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
function updateDownloadTask(id, changes) {
  downloadTasks.value = downloadTasks.value.map((task) => task.id === id ? { ...task, ...changes, updatedAt: Date.now() } : task);
}
function downloadStatus(task) {
  if (task.status === 'completed') return ['已完成', 'success'];
  if (task.status === 'failed') return ['下载失败', 'error'];
  if (task.status === 'downloading') return ['下载中', 'processing'];
  return [task.packaged ? '云端打包中' : '准备下载', 'warning'];
}
function clearFinishedDownloads() {
  downloadTasks.value = downloadTasks.value.filter((task) => !['completed', 'failed'].includes(task.status));
}
async function chooseDownloadDirectory() {
  if (!isTauri) {
    message.warning('服务器 Web 模式不能直接写入访问者电脑，请使用桌面版下载到本机');
    return '';
  }
  return bridge.selectFolder();
}
function queueLocalDownload(command, args, task) {
  downloadTasks.value = [task, ...downloadTasks.value];
  activeView.value = 'downloads';
  bridge.invoke(command, { ...args, destination_dir: task.destination, download_id: task.id })
    .then((payload) => {
      const data = unwrapData(payload);
      if (!data.file_path) throw new Error('客户端下载完成，但没有返回本地文件位置');
      updateDownloadTask(task.id, { status: 'completed', progress: 100, filePath: data.file_path, downloadedBytes: data.bytes || task.downloadedBytes });
      message.success(`下载完成：${data.file_path}`, 8);
    })
    .catch((error) => {
      const text = errorText(error);
      updateDownloadTask(task.id, { status: 'failed', error: text });
      message.error(text);
    });
}
async function downloadCloudFiles(items = selectedFiles.value) {
  const targets = [...items];
  if (!targets.length) { message.warning('请先选择要下载的文件或文件夹'); return; }
  const packaged = targets.length !== 1 || isFolder(targets[0]);
  cloudDownloadBusy.value = true;
  try {
    const destination = await chooseDownloadDirectory();
    if (!destination) return;
    const fileName = localDownloadName(targets, packaged);
    const task = { id: newDownloadId(), fileName, destination, source: '我的文件', packaged, status: 'preparing', progress: 0, downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0, filePath: '', error: '', createdAt: Date.now(), updatedAt: Date.now() };
    queueLocalDownload('get_cloud_download', { file_ids: targets.map(fileId), packaged, file_name: fileName }, task);
    message.success('已加入下载管理');
  } catch (error) { message.error(errorText(error)); }
  finally { cloudDownloadBusy.value = false; }
}

function openReceivedShare() {
  Object.assign(receivedShare, { open: true, link: '', loading: false, restoring: false, downloading: false, shareId: '', code: '', accessToken: '', items: [], selected: [], stack: [], targetId: currentParentId.value, targetLabel: currentFolderPath.value });
}
async function pasteReceivedShareLink() {
  try {
    const value = await navigator.clipboard.readText();
    receivedShare.link = parseGuangyaShareLink(value).url;
  } catch (error) {
    message.warning(errorText(error) || '无法读取剪贴板，请在输入框中粘贴链接');
  }
}
async function loadReceivedShare() {
  let parsed;
  try { parsed = parseGuangyaShareLink(receivedShare.link); }
  catch (error) { message.error(errorText(error)); return; }
  receivedShare.loading = true;
  try {
    receivedShare.link = parsed.url;
    const data = unwrapData(await bridge.invoke('open_received_share', { url: parsed.url }));
    Object.assign(receivedShare, {
      shareId: data.share_id || parsed.shareId,
      code: data.code || parsed.code,
      accessToken: data.access_token || '',
      items: data.files?.list || [],
      selected: [],
      stack: [],
    });
    if (!receivedShare.items.length) message.info('这个分享目录为空');
  } catch (error) { message.error(errorText(error)); }
  finally { receivedShare.loading = false; }
}
async function loadReceivedShareFolder(parentId, stack) {
  receivedShare.loading = true;
  try {
    const data = unwrapData(await bridge.invoke('list_received_share_files', { access_token: receivedShare.accessToken, parent_id: parentId }));
    receivedShare.items = data.list || [];
    receivedShare.selected = [];
    receivedShare.stack = stack;
  } catch (error) { message.error(errorText(error)); }
  finally { receivedShare.loading = false; }
}
function enterReceivedShareFolder(record) {
  if (!isFolder(record)) return;
  loadReceivedShareFolder(fileId(record), [...receivedShare.stack, { id: fileId(record), name: pick(record, ['fileName', 'name'], '文件夹') }]);
}
function goToReceivedSharePath(index) {
  const stack = index < 0 ? [] : receivedShare.stack.slice(0, index + 1);
  loadReceivedShareFolder(stack.at(-1)?.id || '', stack);
}
function receivedShareRowProps(record) {
  return { onDblclick: () => enterReceivedShareFolder(record) };
}
function chooseReceivedShareTarget() {
  openRemotePicker('选择分享转存目录', (target) => {
    receivedShare.targetId = target.id;
    receivedShare.targetLabel = target.path ? `根目录 / ${target.path}` : '根目录';
  });
}
async function restoreReceivedShare() {
  if (!receivedShare.selected.length) { message.warning('请先选择要转存的文件或文件夹'); return; }
  receivedShare.restoring = true;
  try {
    await bridge.invoke('restore_received_share', { access_token: receivedShare.accessToken, file_ids: receivedShare.selected, parent_id: receivedShare.targetId });
    receivedShare.open = false;
    message.success(`已将 ${receivedShare.selected.length} 项转存到 ${receivedShare.targetLabel}`);
    if (activeView.value === 'cloud') await loadFiles();
  } catch (error) { message.error(errorText(error)); }
  finally { receivedShare.restoring = false; }
}
async function downloadReceivedShare() {
  if (!receivedShare.selected.length) { message.warning('请先选择要下载的文件或文件夹'); return; }
  const selectedItems = receivedShare.items.filter((item) => receivedShare.selected.includes(fileId(item)));
  const packaged = selectedItems.length !== 1 || isFolder(selectedItems[0]);
  receivedShare.downloading = true;
  try {
    const destination = await chooseDownloadDirectory();
    if (!destination) return;
    const fileName = localDownloadName(selectedItems, packaged);
    const task = { id: newDownloadId(), fileName, destination, source: '接收分享', packaged, status: 'preparing', progress: 0, downloadedBytes: 0, totalBytes: 0, bytesPerSecond: 0, filePath: '', error: '', createdAt: Date.now(), updatedAt: Date.now() };
    queueLocalDownload('get_received_share_download', {
      access_token: receivedShare.accessToken,
      file_ids: receivedShare.selected,
      packaged,
      file_name: fileName,
    }, task);
    receivedShare.open = false;
    message.success('已加入下载管理');
  } catch (error) { message.error(errorText(error)); }
  finally { receivedShare.downloading = false; }
}

function openBackupForm() {
  Object.assign(backupForm, { local_path: '', remote_path: '', remote_parent_id: '', source_policy: 'keep', archive_path: '', scan_existing: true, sync_types: [...defaultSyncExtensions], monitor_mode: 'native', auto_share: false });
  backupOpen.value = true;
}
function presetExtensions(key) {
  return [...(extensionPresets.find((item) => item.key === key)?.extensions || [])];
}
function applyBackupExtensionPreset(key) {
  backupForm.sync_types = presetExtensions(key);
}
async function chooseFolder(field) {
  if (!isTauri) {
    await openServerFolderPicker(field);
    return;
  }
  try {
    const value = await bridge.selectFolder();
    if (value) backupForm[field] = value;
  } catch (error) { message.error(errorText(error)); }
}
async function addBackup() {
  if (!backupForm.local_path) { message.warning('请先选择本地监控文件夹'); return; }
  if (backupForm.source_policy === 'archive' && !backupForm.archive_path) { message.warning('归档策略需要选择归档目录'); return; }
  const extensions = normalizeExtensions(backupForm.sync_types);
  if (!extensions.length) { message.warning('请至少填写一个同步文件后缀'); return; }
  backupSubmitting.value = true;
  try {
    await bridge.invoke('add_mapping', {
      local_path: backupForm.local_path,
      remote_path: backupForm.remote_path,
      remote_parent_id: backupForm.remote_parent_id,
      source_policy: backupForm.source_policy,
      archive_path: backupForm.source_policy === 'archive' ? backupForm.archive_path : null,
      scan_existing: backupForm.scan_existing,
      sync_types: extensions,
      monitor_mode: backupForm.monitor_mode,
      auto_share: backupForm.auto_share,
    });
    backupOpen.value = false;
    activeView.value = 'backup';
    applyState(await bridge.invoke('get_state'));
    message.success('备份任务已创建');
  } catch (error) { message.error(`创建失败：${errorText(error)}`); }
  finally { backupSubmitting.value = false; }
}
async function toggleMapping(mapping, enabled) {
  try { await bridge.invoke('toggle_mapping', { id: mapping.id, enabled }); }
  catch (error) { message.error(errorText(error)); }
}
async function updateMappingSyncTypes(mapping, values) {
  const extensions = normalizeExtensions(values);
  if (!extensions.length) { message.warning('至少保留一个同步文件后缀'); return; }
  try {
    await bridge.invoke('update_mapping_sync_types', { id: mapping.id, sync_types: extensions });
    message.success('同步文件后缀已更新');
  } catch (error) { message.error(errorText(error)); }
}
function applyMappingExtensionPreset(mapping, key) {
  updateMappingSyncTypes(mapping, presetExtensions(key));
}
async function updateMappingMonitorMode(mapping, value) {
  try {
    await bridge.invoke('update_mapping_monitor_mode', { id: mapping.id, monitor_mode: value });
    message.success(value === 'polling' ? '已切换为网盘轮询监控' : '已切换为本地事件监控');
  } catch (error) { message.error(errorText(error)); }
}
async function saveHdhiveConfig() {
  if (hdhiveForm.base_url && !/^https?:\/\//i.test(hdhiveForm.base_url)) { message.warning('请输入完整的 Hdhive HTTP(S) 地址'); return; }
  hdhiveSubmitting.value = true;
  try {
    await bridge.invoke('update_hdhive_config', { base_url: hdhiveForm.base_url, secret: hdhiveForm.secret || null });
    hdhiveForm.secret = '';
    applyState(await bridge.invoke('get_state'));
    message.success('Hdhive 接入设置已保存，密钥不会返回到界面');
  } catch (error) { message.error(errorText(error)); }
  finally { hdhiveSubmitting.value = false; }
}
async function updateMappingAutoShare(mapping, value) {
  autoShareBusy[mapping.id] = true;
  try {
    await bridge.invoke('update_mapping_auto_share', { id: mapping.id, auto_share: value });
    applyState(await bridge.invoke('get_state'));
    message.success(value ? '上传完成自动分享已开启' : '上传完成自动分享已关闭');
  } catch (error) { message.error(errorText(error)); }
  finally { autoShareBusy[mapping.id] = false; }
}
async function backfillAutoShares(mapping) {
  autoShareBusy[mapping.id] = true;
  try {
    const count = unwrapData(await bridge.invoke('backfill_auto_shares', { id: mapping.id }));
    message.success(`已有内容已加入补建队列（${typeof count === 'number' ? count : count?.scheduled || 0} 条记录）`);
  } catch (error) { message.error(errorText(error)); }
  finally { autoShareBusy[mapping.id] = false; }
}
function receiptStatusLabel(receipt) {
  return ({ accepted: 'Hdhive 已接收', processing: 'Hdhive 处理中', completed: '处理完成', needs_review: '待人工处理', failed: '处理失败', delivery_failed: '等待重新投递', waiting_upload: '等待失败文件重传', sending: '正在通知 Hdhive' })[receipt.status] || receipt.status;
}
function receiptActionLabel(action) {
  return ({ created: '已投稿', updated: '已更新', no_change: '内容未变化', baseline_initialized: '已建立内容基线' })[action] || action || '';
}
function receiptDisplayMessage(receipt) {
  if (!receipt) return '';
  if (receipt.status === 'completed') {
    const outcome = ({ created: '影巢投稿完成', updated: '影巢内容更新完成', no_change: '影巢确认内容没有变化', baseline_initialized: '影巢已建立内容基线' })[receipt.action] || '影巢处理完成';
    return receipt.notification_status === 'sent' ? `${outcome}，消息已推送` : outcome;
  }
  if (receipt.status === 'needs_review') return receipt.message || '影巢需要人工补充信息';
  if (['failed', 'delivery_failed'].includes(receipt.status)) return receipt.message || '影巢处理失败，请重试';
  return receipt.message || (receipt.status === 'processing' ? '影巢正在解析并投稿' : '影巢已接收，等待处理');
}
function receiptAlertType(status) {
  if (status === 'completed') return 'success';
  if (['failed', 'delivery_failed'].includes(status)) return 'error';
  if (status === 'needs_review') return 'warning';
  return 'info';
}
function receiptColor(status) {
  return status === 'completed' ? 'green' : status === 'needs_review' ? 'orange' : ['failed', 'delivery_failed'].includes(status) ? 'red' : status === 'waiting_upload' ? 'gold' : 'blue';
}
async function retryAutoShareReceipt(receipt) {
  const review = receiptReview[receipt.event_id] || {};
  autoShareBusy[receipt.event_id] = true;
  try {
    await bridge.invoke('retry_auto_share_event', { event_id: receipt.event_id, tmdb_id: review.tmdb_id || null, media_type: review.media_type || null });
    applyState(await bridge.invoke('get_state'));
    message.success('Hdhive 已重新接收事件');
  } catch (error) { message.error(errorText(error)); }
  finally { autoShareBusy[receipt.event_id] = false; }
}
async function removeMapping(mapping) {
  try { await bridge.invoke('remove_mapping', { id: mapping.id }); message.success('备份任务已移除'); }
  catch (error) { message.error(errorText(error)); }
}
async function toggleQueue() {
  try { await bridge.invoke(appState.paused ? 'resume_queue' : 'pause_queue'); }
  catch (error) { message.error(errorText(error)); }
}

async function loadOffline() {
  if (!appState.logged_in) return;
  offlineLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_offline_tasks'));
    const list = data.list || data.taskList || data.tasks || data.items || [];
    offlineTasks.value = Array.isArray(list) ? list : [];
  } catch (error) { message.error(errorText(error)); }
  finally { offlineLoading.value = false; }
}
async function createOffline() {
  if (!offlineForm.url.trim()) return;
  try {
    await bridge.invoke('create_offline_task', { url: offlineForm.url, parent_id: offlineForm.parent_id, new_name: offlineForm.new_name });
    Object.assign(offlineForm, { url: '', parent_id: '', new_name: '' });
    await loadOffline();
    message.success('离线任务已创建');
  } catch (error) { message.error(errorText(error)); }
}

async function loadPickerFolder() {
  folderPicker.loading = true;
  try {
    const parentId = folderPicker.stack.at(-1)?.id || '';
    const data = unwrapData(await bridge.invoke('list_files', { parent_id: parentId, page: 0 }));
    folderPicker.items = (data.list || []).filter(isFolder);
  } catch (error) { message.error(errorText(error)); }
  finally { folderPicker.loading = false; }
}
async function openRemotePicker(title, onConfirm, initialStack = []) {
  if (!appState.logged_in) { message.warning('请先登录光鸭云盘'); return; }
  Object.assign(folderPicker, { open: true, title, loading: false, items: [], stack: [...initialStack], onConfirm });
  await loadPickerFolder();
}
async function enterPickerFolder(record) {
  folderPicker.stack.push({ id: fileId(record), name: pick(record, ['fileName', 'name'], '文件夹') });
  await loadPickerFolder();
}
async function pickerGoTo(index) {
  folderPicker.stack = index < 0 ? [] : folderPicker.stack.slice(0, index + 1);
  await loadPickerFolder();
}
async function confirmPicker() {
  const target = folderPicker.stack.at(-1) || { id: '', name: '根目录' };
  const result = { id: target.id, name: target.name, path: folderPicker.stack.map((item) => item.name).join('/') };
  const callback = folderPicker.onConfirm;
  folderPicker.open = false;
  folderPicker.onConfirm = null;
  if (callback) await callback(result);
}
function chooseBackupRemote() {
  openRemotePicker('选择备份目标目录', (target) => {
    backupForm.remote_parent_id = target.id;
    backupForm.remote_path = target.path;
  });
}
function chooseOfflineRemote() {
  openRemotePicker('选择离线下载目录', (target) => {
    offlineForm.parent_id = target.id;
    offlineForm.parent_label = target.path || '根目录';
  });
}

async function runFileOperation(command, items, parentId = '') {
  const ids = items.map(fileId).filter(Boolean);
  if (!ids.length) return;
  operationBusy.value = true;
  try {
    await bridge.invoke(command, { file_ids: ids, parent_id: parentId });
    const label = command === 'copy_files' ? '复制任务已提交' : command === 'move_files' ? '移动任务已提交' : '操作已提交';
    message.success(label);
    if (command === 'move_files') selectedFileIds.value = [];
    await loadFiles();
    scheduleFileRefresh();
  } catch (error) { message.error(errorText(error)); }
  finally { operationBusy.value = false; }
}
function chooseTransferTarget(mode, items = selectedFiles.value) {
  const targets = [...items];
  if (!targets.length) { message.warning('请先选择文件'); return; }
  openRemotePicker(mode === 'copy' ? '复制到云端目录' : '移动到云端目录', (target) => runFileOperation(mode === 'copy' ? 'copy_files' : 'move_files', targets, target.id));
}
function setFileClipboard(mode, items = selectedFiles.value) {
  if (!items.length) { message.warning('请先选择文件'); return; }
  clipboard.mode = mode;
  clipboard.items = items.map((item) => ({ id: fileId(item), fileName: pick(item, ['fileName', 'name']), resType: item.resType }));
  message.success(`${mode === 'move' ? '已剪切' : '已复制'} ${items.length} 项，可进入目标目录粘贴`);
}
async function pasteClipboard() {
  if (!clipboard.items.length) { message.info('内部剪贴板为空'); return; }
  const command = clipboard.mode === 'move' ? 'move_files' : 'copy_files';
  await runFileOperation(command, clipboard.items.map((item) => ({ fileId: item.id })), currentParentId.value);
  if (clipboard.mode === 'move') Object.assign(clipboard, { mode: '', items: [] });
}
function requestDelete(items = selectedFiles.value) {
  if (!items.length) { message.warning('请先选择文件'); return; }
  deleteDialog.items = [...items];
  deleteDialog.open = true;
}
async function confirmDelete() {
  deleteDialog.loading = true;
  try {
    await bridge.invoke('delete_files', { file_ids: deleteDialog.items.map(fileId) });
    message.success('已移入回收站');
    deleteDialog.open = false;
    selectedFileIds.value = [];
    await loadFiles();
    scheduleFileRefresh();
  } catch (error) { message.error(errorText(error)); }
  finally { deleteDialog.loading = false; }
}

function createRenameRule(type = 'replace', seed = '') {
  return { id: ++ruleId, type, value: seed, search: '', replacement: '', ignoreCase: false, start: 1, padding: 2 };
}
function openRename(items = selectedFiles.value) {
  if (!items.length) { message.warning('请先选择文件'); return; }
  renameTargets.value = [...items];
  preserveExtension.value = true;
  renameRules.value = items.length === 1 ? [createRenameRule('set', fileBaseName(items[0]))] : [createRenameRule('replace')];
  renameOpen.value = true;
}
function addRenameRule() { renameRules.value.push(createRenameRule('replace')); }
function removeRenameRule(index) { if (renameRules.value.length > 1) renameRules.value.splice(index, 1); }
function moveRenameRule(index, direction) {
  const target = index + direction;
  if (target < 0 || target >= renameRules.value.length) return;
  const [rule] = renameRules.value.splice(index, 1);
  renameRules.value.splice(target, 0, rule);
}
async function executeRename() {
  if (renamePreview.value.error) { message.error(renamePreview.value.error); return; }
  const renames = renamePreview.value.rows.filter((row) => row.currentName !== row.newName);
  if (!renames.length) { message.info('名称没有变化'); return; }
  renaming.value = true;
  try {
    await bridge.invoke('batch_rename_files', { renames });
    message.success(`已重命名 ${renames.length} 项`);
    renameOpen.value = false;
    selectedFileIds.value = [];
    await loadFiles();
  } catch (error) { message.error(errorText(error)); }
  finally { renaming.value = false; }
}

function fileRowProps(record) {
  return {
    onDblclick: () => openFolder(record),
    onContextmenu: (event) => showContextMenu(event, record),
  };
}
function handleFileTableContextMenu(event) {
  const row = event.target?.closest?.('tr');
  if (!row || row.parentElement?.tagName !== 'TBODY') return;
  const key = row?.getAttribute('data-row-key');
  const fallbackIndex = row && row.parentElement ? Array.from(row.parentElement.children).indexOf(row) : -1;
  const record = key
    ? files.value.find((item) => String(fileId(item)) === String(key))
    : fallbackIndex >= 0 ? files.value[fallbackIndex] : null;
  if (record) {
    event.preventDefault();
    event.stopPropagation();
    showContextMenu(event, record);
  }
}
function showContextMenu(event, record = null) {
  event.preventDefault();
  if (record) selectedOr(record);
  Object.assign(contextMenu, { visible: true, x: Math.min(event.clientX, window.innerWidth - 190), y: Math.min(event.clientY, window.innerHeight - 330), record });
}
function hideContextMenu() { contextMenu.visible = false; }
function contextItems() { return contextMenu.record ? selectedOr(contextMenu.record) : selectedFiles.value; }
function contextAction(action) {
  const items = contextItems();
  hideContextMenu();
  if (action === 'open' && contextMenu.record) openFolder(contextMenu.record);
  else if (action === 'copy') setFileClipboard('copy', items);
  else if (action === 'cut') setFileClipboard('move', items);
  else if (action === 'copyTo') chooseTransferTarget('copy', items);
  else if (action === 'moveTo') chooseTransferTarget('move', items);
  else if (action === 'rename') openRename(items);
  else if (action === 'download') downloadCloudFiles(items);
  else if (action === 'share') createShare();
  else if (action === 'delete') requestDelete(items);
  else if (action === 'paste') pasteClipboard();
  else if (action === 'uploadFile') triggerUpload('files');
  else if (action === 'uploadFolder') triggerUpload('folder');
  else if (action === 'refresh') loadFiles();
}

async function queueNativeUpload(paths) {
  if (!paths?.length) return;
  try {
    const count = await bridge.invoke('queue_upload_paths', { paths, parent_id: currentParentId.value });
    message.success(`已加入上传队列：${count} 个文件`);
  } catch (error) { message.error(errorText(error)); }
}
async function triggerUpload(kind) {
  if (!appState.logged_in) { message.warning('请先登录光鸭云盘'); return; }
  if (isTauri) {
    const selected = kind === 'files' ? await bridge.selectUploadFiles() : await bridge.selectUploadFolder();
    await queueNativeUpload(Array.isArray(selected) ? selected : selected ? [selected] : []);
  } else {
    uploadSourceKind.value = kind;
    uploadSourceOpen.value = true;
  }
}
function chooseBrowserUpload() {
  uploadSourceOpen.value = false;
  (uploadSourceKind.value === 'files' ? fileInput.value : folderInput.value)?.click();
}
async function loadServerDirectory(relativePath = '') {
  serverFilePicker.loading = true;
  try {
    const query = new URLSearchParams({ path: relativePath });
    const response = await fetch(`/api/server-files?${query}`);
    const payload = await readJsonResponse(response, '读取服务器目录失败');
    Object.assign(serverFilePicker, { roots: payload.roots || [], path: payload.path || '', parent: payload.parent || '', displayPath: payload.display_path || '/', items: payload.items || [] });
  } catch (error) { message.error(errorText(error)); }
  finally { serverFilePicker.loading = false; }
}
async function chooseServerUpload() {
  uploadSourceOpen.value = false;
  serverFilePicker.mode = 'upload';
  serverFilePicker.targetField = '';
  serverFilePicker.open = true;
  serverFilePicker.selected = [];
  await loadServerDirectory('');
}
async function openServerFolderPicker(field) {
  serverFilePicker.mode = 'folder';
  serverFilePicker.targetField = field;
  serverFilePicker.open = true;
  serverFilePicker.selected = [];
  await loadServerDirectory(backupForm[field] || '');
}
function toggleServerSelection(item, checked) {
  const selected = new Set(serverFilePicker.selected);
  if (checked) selected.add(item.path); else selected.delete(item.path);
  serverFilePicker.selected = [...selected];
}
async function confirmServerUpload() {
  if (!serverFilePicker.selected.length) { message.warning('请至少选择一个服务器文件或文件夹'); return; }
  serverFilePicker.submitting = true;
  try {
    const response = await fetch('/api/server-upload', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ paths: serverFilePicker.selected, parent_id: currentParentId.value }) });
    const payload = await readJsonResponse(response, '加入服务器上传队列失败');
    serverFilePicker.open = false;
    if (payload.queued) message.success(`已加入上传队列：${payload.queued} 个文件${payload.skipped ? `，跳过已上传 ${payload.skipped} 个` : ''}`);
    else message.info(`没有需要上传的文件，已跳过 ${payload.skipped || 0} 个已上传文件`);
  } catch (error) { message.error(errorText(error)); }
  finally { serverFilePicker.submitting = false; }
}
async function confirmServerPicker() {
  if (serverFilePicker.mode === 'folder') {
    if (!serverFilePicker.path) { message.warning('请选择一个服务器文件夹'); return; }
    backupForm[serverFilePicker.targetField] = serverFilePicker.path;
    serverFilePicker.open = false;
    return;
  }
  await confirmServerUpload();
}
async function readDirectoryEntry(entry, prefix, result) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    result.push({ file, relativePath: `${prefix}${file.name}` });
    return;
  }
  if (!entry.isDirectory) return;
  const reader = entry.createReader();
  const entries = [];
  while (true) {
    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    if (!batch.length) break;
    entries.push(...batch);
  }
  for (const child of entries) await readDirectoryEntry(child, `${prefix}${entry.name}/`, result);
}
async function filesFromTransfer(dataTransfer) {
  const result = [];
  const entries = [...(dataTransfer.items || [])].map((item) => item.webkitGetAsEntry?.()).filter(Boolean);
  if (entries.length) {
    for (const entry of entries) await readDirectoryEntry(entry, '', result);
    return result;
  }
  return [...(dataTransfer.files || [])].map((file) => ({ file, relativePath: file.webkitRelativePath || file.name }));
}
async function uploadWebFiles(entries) {
  if (!entries.length) return;
  uploadingCount.value = entries.length;
  try {
    let cursor = 0;
    let queued = 0;
    let skipped = 0;
    const worker = async () => {
      while (cursor < entries.length) {
        const entry = entries[cursor++];
        const query = new URLSearchParams({ parentId: currentParentId.value, fileName: entry.file.name, relativePath: entry.relativePath || entry.file.name, lastModified: String(entry.file.lastModified || 0) });
        const eventPath = `[浏览器]/${entry.relativePath || entry.file.name}`;
        const payload = await new Promise((resolve, reject) => {
          const request = new XMLHttpRequest();
          const startedAt = performance.now();
          request.open('POST', `/api/upload?${query}`);
          request.setRequestHeader('content-type', entry.file.type || 'application/octet-stream');
          request.upload.onprogress = (event) => {
            const elapsedSeconds = Math.max((performance.now() - startedAt) / 1000, 0.001);
            const total = event.lengthComputable ? event.total : entry.file.size;
            updateUploadProgress({ type: 'progress', file_path: eventPath, percent: total ? Math.round(event.loaded / total * 100) : 0, bytes_per_second: event.loaded / elapsedSeconds, stage: '正在传到服务器' });
          };
          request.onerror = () => { const error = new Error(`上传接口网络错误：${entry.file.name}`); updateUploadProgress({ type: 'file', state: 'error', file_path: eventPath, error: error.message }); reject(error); };
          request.onabort = () => { const error = new Error(`上传已取消：${entry.file.name}`); updateUploadProgress({ type: 'file', state: 'error', file_path: eventPath, error: error.message }); reject(error); };
          request.onload = async () => {
            try {
              const response = new Response(request.responseText || '', { status: request.status || 500, headers: { 'content-type': request.getResponseHeader('content-type') || 'text/plain' } });
              const result = await readJsonResponse(response, `上传接口失败：${entry.file.name}`);
              if (result.skipped) updateUploadProgress({ type: 'file', state: 'done', file_path: eventPath, stage: '文件未变化，已跳过' });
              resolve(result);
            } catch (error) {
              updateUploadProgress({ type: 'file', state: 'error', file_path: eventPath, error: error.message });
              reject(error);
            }
          };
          request.send(entry.file);
        });
        queued += Number(payload.queued || 0);
        skipped += Number(payload.skipped || 0);
        uploadingCount.value -= 1;
      }
    };
    await Promise.all([worker(), worker()]);
    if (queued) message.success(`已传到服务器并加入云端上传队列：${queued} 个${skipped ? `，跳过已上传 ${skipped} 个` : ''}`);
    else message.info(`没有需要上传的文件，已跳过 ${skipped} 个已上传文件`);
    await loadFiles();
  } catch (error) { message.error(errorText(error)); }
  finally { uploadingCount.value = 0; }
}
async function handleWebInput(event) {
  const entries = [...event.target.files].map((file) => ({ file, relativePath: file.webkitRelativePath || file.name }));
  event.target.value = '';
  await uploadWebFiles(entries);
}
async function handleWebDrop(event) {
  dragActive.value = false;
  if (isTauri || activeView.value !== 'cloud') return;
  await uploadWebFiles(await filesFromTransfer(event.dataTransfer));
}
async function handleNativeDrag(type, payload) {
  if (activeView.value !== 'cloud') return;
  if (type === 'enter' || type === 'over') dragActive.value = true;
  if (type === 'leave') dragActive.value = false;
  if (type === 'drop') {
    dragActive.value = false;
    await queueNativeUpload(Array.isArray(payload) ? payload : payload?.paths || []);
  }
}
function isTypingTarget(target) {
  return target instanceof HTMLElement && (['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) || target.isContentEditable);
}
function handleShortcut(event) {
  if (activeView.value !== 'cloud' || isTypingTarget(event.target) || renameOpen.value || folderPicker.open || backupOpen.value) return;
  if (event.key === 'F2' && selectedFiles.value.length) { event.preventDefault(); openRename(); }
  else if (event.key === 'Delete' && selectedFiles.value.length) { event.preventDefault(); requestDelete(); }
}

function clearLoginTimers() {
  clearInterval(devicePollTimer);
  clearInterval(deviceExpiryTimer);
  devicePollTimer = null;
  deviceExpiryTimer = null;
}
function closeLogin() { clearLoginTimers(); loginOpen.value = false; }
async function openLogin() {
  loginOpen.value = true;
  await refreshLogin();
}
async function setWebToken() {
  if (!loginToken.value.trim()) return;
  try {
    await webRequest('/api/auth', { method: 'POST', body: JSON.stringify({ token: loginToken.value.replace(/^Bearer\s+/i, '') }) });
    applyState(await bridge.invoke('get_state'));
    loginToken.value = '';
    loginOpen.value = false;
    await Promise.all([loadOverview(), loadFiles()]);
    message.success('会话令牌已设置');
  } catch (error) { message.error(errorText(error)); }
}
async function refreshLogin() {
  clearLoginTimers();
  Object.assign(login, { loading: true, qr: '', userCode: '—', verificationUrl: '', message: '正在获取二维码…', remaining: 0 });
  try {
    const data = unwrapData(await bridge.login());
    const deviceCode = pick(data, ['device_code', 'deviceCode']);
    const verificationUri = pick(data, ['verification_uri_complete', 'verificationUriComplete', 'verification_url', 'verificationUrl', 'verification_uri', 'verificationUri']);
    if (!deviceCode || !verificationUri) throw new Error('官方没有返回完整扫码信息');
    Object.assign(login, { loading: false, qr: verificationUri, userCode: pick(data, ['user_code', 'userCode'], '—'), verificationUrl: verificationUri, message: '等待扫码确认', remaining: Number(data.expires_in || 120) });
    devicePollTimer = setInterval(async () => {
      try {
        const result = await bridge.invoke('poll_device_login', { device_code: deviceCode });
        if (result?.authenticated) {
          clearLoginTimers();
          loginOpen.value = false;
          applyState(await bridge.invoke('get_state'));
          await Promise.all([loadOverview(), loadFiles()]);
          message.success('登录成功');
        } else if (result?.message) login.message = result.message;
      } catch (error) {
        clearLoginTimers();
        login.message = errorText(error);
      }
    }, Number(data.interval || 3) * 1000);
    deviceExpiryTimer = setInterval(() => {
      login.remaining -= 1;
      if (login.remaining <= 0) refreshLogin();
    }, 1000);
  } catch (error) {
    login.loading = false;
    login.message = errorText(error);
    message.error(errorText(error));
  }
}

async function initialize() {
  try {
    unsubscribe = await bridge.subscribe((payload) => {
      if (payload.type === 'state') applyState(payload.state);
      if (payload.type === 'status') appendEvent(payload.level || 'info', payload.message);
      if (payload.type === 'progress' || payload.type === 'file') updateUploadProgress(payload);
      if (payload.type === 'download' && payload.download_id) {
        const changes = {
          status: payload.state === 'done' ? 'completed' : payload.state === 'error' ? 'failed' : 'downloading',
        };
        if (payload.percent !== null && payload.percent !== undefined) changes.progress = Number(payload.percent);
        if (payload.downloaded_bytes !== null && payload.downloaded_bytes !== undefined) changes.downloadedBytes = Number(payload.downloaded_bytes);
        if (Number(payload.total_bytes || 0) > 0) changes.totalBytes = Number(payload.total_bytes);
        if (payload.bytes_per_second !== null && payload.bytes_per_second !== undefined) changes.bytesPerSecond = Number(payload.bytes_per_second);
        if (payload.file_path) changes.filePath = payload.file_path;
        if (payload.error) changes.error = payload.error;
        updateDownloadTask(payload.download_id, changes);
      }
      if (payload.type === 'file' && payload.state === 'queued') appendEvent('info', `已加入上传队列：${payload.file_path}`);
      if (payload.type === 'file' && payload.state === 'waiting-login') appendEvent('warning', `等待登录后上传：${payload.file_path}`);
      if (payload.type === 'file' && payload.state === 'waiting-file') appendEvent('warning', `${payload.file_path}：另外的程序正在使用该文件，释放后将自动上传`);
      if (payload.type === 'file' && payload.state === 'preparing') appendEvent('info', `准备上传：${payload.file_path}`);
      if (payload.type === 'file' && payload.state === 'uploading') appendEvent('info', `开始上传：${payload.file_path}`);
      if (payload.type === 'file' && payload.state === 'done') appendEvent('success', `上传完成：${payload.file_path}`);
      if (payload.type === 'file' && payload.state === 'error') appendEvent('error', `${payload.file_path}：${payload.error}`);
    });
    unsubscribeDrag = await bridge.subscribeDrag(handleNativeDrag);
  } catch (error) { appendEvent('error', `状态订阅失败：${errorText(error)}`); }
  try { applyState(await bridge.invoke('get_state')); }
  catch (error) { message.error(errorText(error)); }
  if (appState.logged_in) await Promise.all([loadOverview(), loadFiles()]);
}

onMounted(() => {
  document.addEventListener('click', hideContextMenu);
  window.addEventListener('keydown', handleShortcut);
  initialize();
});
onBeforeUnmount(() => {
  clearLoginTimers();
  clearTimeout(refreshTimer);
  uploadRemovalTimers.forEach((timer) => clearTimeout(timer));
  uploadRemovalTimers.clear();
  document.removeEventListener('click', hideContextMenu);
  window.removeEventListener('keydown', handleShortcut);
  if (typeof unsubscribe === 'function') unsubscribe();
  if (typeof unsubscribeDrag === 'function') unsubscribeDrag();
});
</script>

<template>
  <a-config-provider :theme="theme">
    <a-app>
      <a-layout class="app-shell">
        <a-layout-sider :width="240" class="sidebar" theme="light">
          <div class="brand">
            <div class="brand-mark"><img :src="appLogo" alt="光鸭文件夹同步" /></div>
            <div><strong>光鸭云盘</strong><span>GUANGYA SYNC</span></div>
          </div>

          <div class="side-profile">
            <div class="side-profile-head">
              <a-avatar :size="36" :src="userAvatar || undefined"><template #icon><UserOutlined /></template></a-avatar>
              <div class="side-profile-name">
                <strong>{{ userName }}</strong>
                <span>{{ appState.logged_in ? (profilePhone !== '未绑定' ? profilePhone : `ID：${profileId}`) : '登录后同步云盘文件' }}</span>
              </div>
              <a-tag v-if="appState.logged_in" class="side-vip-tag" :color="isVip ? 'gold' : vipExpired ? 'red' : 'default'" :title="`VIP 到期：${vipExpireLabel}`">{{ vipLabel }}</a-tag>
            </div>
            <div class="side-quota">
              <div class="side-quota-row"><span>剩余空间</span><strong>{{ totalSpace ? formatSize(Math.max(Number(totalSpace) - Number(usedSpace), 0)) : (appState.logged_in ? '读取中…' : '—') }}</strong></div>
              <a-progress :percent="quotaPercent" :show-info="false" size="small" />
              <small>{{ totalSpace ? `已用 ${formatSize(usedSpace)} / ${formatSize(totalSpace)}` : (appState.logged_in ? '正在读取空间信息' : '登录后显示空间用量') }}</small>
            </div>
          </div>

          <a-menu class="side-menu" mode="inline" :items="navigation" :selected-keys="[activeView]" @click="selectView" />

          <div class="sidebar-bottom">
            <a-button type="primary" block @click="openLogin"><template #icon><LoginOutlined /></template>{{ appState.logged_in ? '重新登录' : '登录云盘' }}</a-button>
            <div class="connection-card" :class="{ online: appState.logged_in }">
              <div class="connection-icon"><CheckCircleFilled v-if="appState.logged_in" /><ClockCircleOutlined v-else /></div>
              <div><strong>{{ appState.logged_in ? '云盘已连接' : '等待登录' }}</strong><span>{{ isTauri ? '桌面监控服务运行中' : 'Docker Web 控制台' }}</span></div>
            </div>
            <div class="version-row"><SafetyCertificateOutlined /><span>会话与上传记录已本地持久化</span></div>
          </div>
        </a-layout-sider>

        <a-layout class="main-layout">
          <a-layout-header class="topbar">
            <div class="title-block"><h1>{{ pageTitle }}</h1><p>{{ pageSubtitle }}</p></div>
            <a-tag v-if="appState.paused || appState.pending || appState.active_uploads" class="queue-tag" :color="appState.paused ? 'warning' : 'processing'">
              <SyncOutlined :spin="!appState.paused && Boolean(appState.active_uploads)" />{{ queueText }} · {{ appState.active_uploads }} 上传中 / {{ appState.pending }} 等待<span v-if="totalUploadSpeed"> · {{ formatUploadSpeed(totalUploadSpeed) }}</span>
            </a-tag>
          </a-layout-header>

          <a-layout-content class="content">
            <a-alert v-if="!isTauri" class="web-notice" type="info" show-icon message="服务器 Web 模式" description="文件夹浏览范围由服务进程的系统权限和 GUANGYA_FILE_ROOTS 配置决定。" />

            <template v-if="activeView === 'cloud'">
              <div class="cloud-view">
              <a-card class="content-card file-card file-drop-surface" :class="{ 'is-dragging': dragActive }" :bordered="false" @dragenter.prevent="dragActive = true" @dragover.prevent="dragActive = true" @dragleave.prevent="dragActive = false" @drop.prevent="handleWebDrop" @contextmenu="(event) => { if (event.target.closest('tbody')) return; showContextMenu(event) }">
                <div v-if="dragActive" class="drop-overlay"><div><UploadOutlined /><strong>拖到这里上传</strong><span>文件和文件夹将上传到 {{ currentFolderPath }}</span></div></div>
                <template #title>
                  <a-flex align="center" gap="small"><div class="section-icon"><FolderOpenOutlined /></div><div><strong>我的文件</strong><span class="section-subtitle">{{ currentFolderPath }}</span></div></a-flex>
                </template>
                <template #extra>
                  <a-space>
                    <a-button @click="openReceivedShare"><template #icon><InboxOutlined /></template>接收分享</a-button>
                    <a-button @click="openShareForm"><template #icon><LinkOutlined /></template>收藏链接</a-button>
                    <a-button type="primary" @click="triggerUpload('files')" :disabled="!appState.logged_in"><template #icon><UploadOutlined /></template>上传文件</a-button>
                  </a-space>
                </template>

                <div class="file-breadcrumb">
                  <button :class="{ active: !currentPath.length }" @click="goRoot"><HomeOutlined />根目录</button>
                  <template v-for="(part, index) in currentPath" :key="part.id"><span>/</span><button :class="{ active: index === currentPath.length - 1 }" @click="goToPath(index)">{{ part.name }}</button></template>
                </div>

                <div class="file-toolbar">
                  <a-flex wrap="wrap" gap="small" align="center">
                    <a-button @click="triggerUpload('folder')"><template #icon><FolderOpenOutlined /></template>上传文件夹</a-button>
                    <a-button :disabled="!selectedFileIds.length" @click="setFileClipboard('copy')"><template #icon><CopyOutlined /></template>复制</a-button>
                    <a-button :disabled="!selectedFileIds.length" @click="setFileClipboard('move')"><template #icon><ScissorOutlined /></template>剪切</a-button>
                    <a-button :disabled="!selectedFileIds.length" :loading="operationBusy" @click="chooseTransferTarget('move')"><template #icon><SwapOutlined /></template>移动到</a-button>
                    <a-button :disabled="!selectedFileIds.length" @click="openRename()"><template #icon><EditOutlined /></template>{{ selectedFileIds.length > 1 ? '批量重命名' : '重命名' }}</a-button>
                    <a-button :disabled="!selectedFileIds.length" :loading="cloudDownloadBusy" @click="downloadCloudFiles()"><template #icon><DownloadOutlined /></template>下载</a-button>
                    <a-button :disabled="!selectedFileIds.length" :loading="shareCreating" @click="createShare"><template #icon><ShareAltOutlined /></template>{{ shareCreating ? '创建中' : '分享' }}</a-button>
                    <a-button danger :disabled="!selectedFileIds.length" @click="requestDelete()"><template #icon><DeleteOutlined /></template>删除</a-button>
                  </a-flex>
                  <a-flex gap="small" align="center">
                    <a-tag v-if="clipboard.items.length" color="blue" class="clipboard-tag">{{ clipboardLabel }}</a-tag>
                    <a-button v-if="clipboard.items.length" type="primary" ghost @click="pasteClipboard"><template #icon><CheckOutlined /></template>粘贴到此处</a-button>
                    <a-button @click="loadFiles" :loading="filesLoading"><template #icon><ReloadOutlined /></template></a-button>
                  </a-flex>
                </div>

                <div v-if="selectedFileIds.length" class="selection-tip">已选择 {{ selectedFileIds.length }} 项 · 可使用工具栏或右键菜单操作 · F2 重命名 · Delete 删除</div>
                <div v-else class="drop-hint"><DragOutlined /> 拖入文件或文件夹即可上传，双击打开目录，右键查看更多操作</div>

                <input ref="fileInput" class="hidden-file-input" type="file" multiple @change="handleWebInput" />
                <input ref="folderInput" class="hidden-file-input" type="file" multiple webkitdirectory directory @change="handleWebInput" />

                <a-table :columns="fileColumns" :data-source="files" :loading="filesLoading" :row-key="fileId" :row-selection="rowSelection" :custom-row="fileRowProps" :pagination="false" :scroll="{ y: 'clamp(240px, calc(100vh - 330px), 640px)' }" size="small" @contextmenu="handleFileTableContextMenu">
                  <template #emptyText><a-empty :description="appState.logged_in ? '这个目录还没有文件' : '请先登录光鸭云盘'" /></template>
                  <template #bodyCell="{ column, record }">
                    <template v-if="column.key === 'name'">
                      <button type="button" class="file-name-button" :class="{ clickable: isFolder(record) }" @dblclick.stop="openFolder(record)" @contextmenu.stop="showContextMenu($event, record)">
                        <span class="file-icon" :class="isFolder(record) ? 'folder' : 'file'"><FolderOutlined v-if="isFolder(record)" /><FileOutlined v-else /></span>
                        <span>{{ pick(record, ['fileName', 'name'], '未命名') }}</span>
                      </button>
                    </template>
                    <template v-else-if="column.key === 'type'"><a-tag :color="isFolder(record) ? 'blue' : undefined">{{ isFolder(record) ? '文件夹' : (record.ext || '文件') }}</a-tag></template>
                    <template v-else-if="column.key === 'size'">{{ isFolder(record) ? `${record.subFolderCount || 0} 项` : formatSize(record.fileSize) }}</template>
                    <template v-else-if="column.key === 'time'">{{ formatTime(record.utime || record.ctime) }}</template>
                  </template>
                </a-table>
              </a-card>
              <a-card v-if="recentUploads.length" class="content-card upload-progress-card" :bordered="false" title="上传进度">
                <div class="upload-progress-list">
                  <div v-for="upload in recentUploads" :key="upload.filePath" class="upload-progress-item">
                    <div class="upload-progress-heading">
                      <div><strong>{{ upload.fileName }}</strong><span :title="upload.filePath">{{ upload.filePath }}</span></div>
                      <span>{{ upload.stage }}<template v-if="upload.state === 'uploading'"> · {{ formatUploadSpeed(upload.bytesPerSecond) }}</template></span>
                    </div>
                    <a-progress :percent="upload.percent" :status="upload.state === 'error' ? 'exception' : upload.state === 'done' ? 'success' : 'active'" size="small" />
                  </div>
                </div>
              </a-card>
              </div>
            </template>

            <template v-else-if="activeView === 'backup'">
              <div class="section-toolbar">
                <div><h2>自动备份</h2><p>每个任务独立监控一个文件夹，并保留原始目录结构。</p></div>
                <a-space><a-button @click="toggleQueue"><template #icon><PlayCircleOutlined v-if="appState.paused" /><PauseCircleOutlined v-else /></template>{{ appState.paused ? '继续队列' : '暂停队列' }}</a-button><a-button type="primary" @click="openBackupForm"><template #icon><PlusOutlined /></template>新建备份任务</a-button></a-space>
              </div>

              <a-card class="content-card hdhive-config-card" :bordered="false" title="Hdhive 自动投稿">
                <template #extra><a-tag :color="appState.hdhive?.configured ? 'green' : 'default'">{{ appState.hdhive?.configured ? '已配置' : '未配置' }}</a-tag></template>
                <a-row :gutter="12" align="bottom">
                  <a-col :xs="24" :lg="10"><label>Hdhive 地址</label><a-input v-model:value="hdhiveForm.base_url" placeholder="https://hdhive.example.com" /></a-col>
                  <a-col :xs="24" :lg="9"><label>同步专用 HMAC 密钥</label><a-input-password v-model:value="hdhiveForm.secret" :placeholder="appState.hdhive?.configured ? '留空表示不修改' : '输入服务端 guangya_sync 密钥'" /></a-col>
                  <a-col :xs="24" :lg="5"><a-button type="primary" block :loading="hdhiveSubmitting" @click="saveHdhiveConfig">保存接入设置</a-button></a-col>
                </a-row>
                <div class="hdhive-instance">实例 ID：{{ appState.hdhive?.instance_id || '保存后生成' }}。密钥仅保存在本机 SQLite，不会回传明文。</div>
              </a-card>

              <a-alert v-if="!appState.logged_in" class="backup-login-alert" type="warning" show-icon message="备份任务正在监控，但需要登录后才会上传；已发现的文件会留在等待队列中。" />

              <a-row :gutter="16" class="backup-grid">
                <a-col v-for="mapping in appState.mappings" :key="mapping.id" :xs="24" :xl="12">
                  <a-card class="backup-card" :bordered="false">
                    <a-flex justify="space-between" align="flex-start">
                      <a-flex gap="middle" align="center"><div class="task-icon"><SyncOutlined :spin="mapping.enabled" /></div><div class="task-title"><strong>{{ mapping.local_path }}</strong><span><UploadOutlined /> 云端 /{{ mapping.remote_path || '根目录' }}</span></div></a-flex>
                      <a-switch :checked="mapping.enabled" @change="(value) => toggleMapping(mapping, value)" />
                    </a-flex>
                    <a-divider />
                    <a-flex wrap="wrap" gap="small"><a-tag :color="mapping.watch_error ? 'red' : mapping.enabled ? 'green' : undefined">{{ mapping.watch_error ? '监控失败' : mapping.enabled ? '监控中' : '已暂停' }}</a-tag><a-tag :color="mapping.monitor_mode === 'polling' ? 'purple' : 'cyan'">{{ monitorModeLabel(mapping.monitor_mode) }}</a-tag><a-tag :color="sourcePolicyColor(mapping.source_policy)">{{ sourcePolicyLabel(mapping.source_policy) }}</a-tag><a-tag>{{ mapping.scan_existing ? '包含已有文件' : '仅监控新文件' }}</a-tag><a-tag :color="mapping.auto_share ? 'geekblue' : undefined">{{ mapping.auto_share ? '自动分享' : '不自动分享' }}</a-tag><a-tag>后缀：{{ syncTypeSummary(mapping) }}</a-tag></a-flex>
                    <a-alert v-if="mapping.watch_error" class="mapping-error" type="error" show-icon :message="mapping.watch_error" />
                    <div class="mapping-type-editor"><span>监控方式</span><a-select :value="mapping.monitor_mode || 'native'" :options="monitorModeOptions" @change="(value) => updateMappingMonitorMode(mapping, value)" /></div>
                    <div class="mapping-type-editor"><span>同步后缀</span><div><a-select mode="tags" :value="mappingExtensions(mapping)" :token-separators="[',', '，', ' ']" :max-tag-count="4" placeholder="输入后缀，如 mp4、srt" @change="(values) => updateMappingSyncTypes(mapping, values)" /><div class="extension-presets compact"><a-button v-for="preset in extensionPresets" :key="preset.key" size="small" @click="applyMappingExtensionPreset(mapping, preset.key)">{{ preset.label }}</a-button></div></div></div>
                    <div class="mapping-type-editor"><span>上传后分享</span><a-flex justify="space-between" align="center"><a-switch :checked="Boolean(mapping.auto_share)" :loading="autoShareBusy[mapping.id]" @change="(value) => updateMappingAutoShare(mapping, value)" /><a-popconfirm v-if="mapping.auto_share" title="将已上传的顶层文件/目录加入补建队列，不会处理未上传内容。确定继续？" @confirm="backfillAutoShares(mapping)"><a-button size="small" :loading="autoShareBusy[mapping.id]">补建已有内容</a-button></a-popconfirm></a-flex></div>
                    <div v-if="mapping.archive_path" class="archive-line">归档到：{{ mapping.archive_path }}</div>
                    <div class="card-actions"><a-popconfirm title="移除任务不会删除已经上传的文件，确定继续？" ok-text="移除" cancel-text="取消" @confirm="removeMapping(mapping)"><a-button danger type="text"><template #icon><DeleteOutlined /></template>移除任务</a-button></a-popconfirm></div>
                  </a-card>
                </a-col>
                <a-col v-if="!appState.mappings.length" :span="24"><a-card class="empty-card" :bordered="false"><a-empty description="还没有备份任务"><a-button type="primary" @click="openBackupForm"><template #icon><PlusOutlined /></template>创建第一个任务</a-button></a-empty></a-card></a-col>
              </a-row>

              <a-card v-if="appState.auto_share_receipts?.length" class="content-card auto-share-receipts" :bordered="false" title="分享与 Hdhive 回执">
                <div v-for="receipt in appState.auto_share_receipts" :key="receipt.event_id" class="receipt-row">
                  <div class="receipt-main"><strong>{{ receipt.target_key }}</strong><a-tag color="green">光鸭分享成功</a-tag><a-tag :color="receiptColor(receipt.status)">{{ receiptStatusLabel(receipt) }}</a-tag><a-tag v-if="receipt.action" color="purple">{{ receiptActionLabel(receipt.action) }}</a-tag><a-tag v-if="receipt.notification_status">通知：{{ receipt.notification_status }}</a-tag><span>{{ receiptDisplayMessage(receipt) }}</span></div>
                  <a-flex gap="small" align="center" wrap="wrap">
                    <a v-if="receipt.share_url" :href="receipt.share_url" target="_blank" rel="noreferrer">分享链接</a><a v-if="receipt.resource_url" :href="receipt.resource_url" target="_blank" rel="noreferrer">Hdhive 资源</a>
                    <template v-if="receipt.status === 'needs_review' && String(receipt.message || '').includes('TMDB')"><a-input v-model:value="receiptReview[receipt.event_id].tmdb_id" size="small" placeholder="TMDB ID" class="receipt-tmdb" /><a-select v-model:value="receiptReview[receipt.event_id].media_type" size="small" placeholder="类型" :options="[{ label: '电视剧', value: 'tv' }, { label: '电影', value: 'movie' }]" class="receipt-media" /></template>
                    <a-button v-if="['needs_review', 'failed', 'delivery_failed'].includes(receipt.status)" size="small" :loading="autoShareBusy[receipt.event_id]" @click="retryAutoShareReceipt(receipt)">重试</a-button>
                  </a-flex>
                </div>
              </a-card>

              <a-card v-if="recentUploads.length" class="content-card upload-progress-card" :bordered="false" title="上传进度">
                <div class="upload-progress-list">
                  <div v-for="upload in recentUploads" :key="upload.filePath" class="upload-progress-item">
                    <div class="upload-progress-heading">
                      <div><strong>{{ upload.fileName }}</strong><span :title="upload.filePath">{{ upload.filePath }}</span></div>
                      <span>{{ upload.stage }}<template v-if="upload.state === 'uploading'"> · {{ formatUploadSpeed(upload.bytesPerSecond) }}</template></span>
                    </div>
                    <a-progress
                      :percent="upload.percent"
                      :status="upload.state === 'error' ? 'exception' : upload.state === 'done' ? 'success' : 'active'"
                      size="small"
                    />
                  </div>
                </div>
              </a-card>

              <a-card class="content-card event-card" :bordered="false" title="最近活动">
                <template #extra><a-badge :count="events.length" :number-style="{ backgroundColor: '#e8f1ff', color: '#1677ff' }" /></template>
                <a-empty v-if="!events.length" description="文件变化和上传进度会显示在这里" />
                <a-timeline v-else :items="events.slice(0, 8).map(item => ({ color: item.level === 'error' ? 'red' : item.level === 'success' ? 'green' : 'blue', children: `${item.time}  ${item.text}` }))" />
              </a-card>
            </template>

            <template v-else-if="activeView === 'downloads'">
              <div class="section-toolbar">
                <div><h2>本机下载</h2><p>下载前选择保存目录，任务由客户端直接代理并写入本地文件。</p></div>
                <a-space><a-tag v-if="activeDownloadCount" color="processing">{{ activeDownloadCount }} 个进行中</a-tag><a-button :disabled="!downloadTasks.some(task => ['completed', 'failed'].includes(task.status))" @click="clearFinishedDownloads">清除已结束</a-button></a-space>
              </div>
              <a-card class="content-card download-manager-card" :bordered="false">
                <a-empty v-if="!downloadTasks.length" description="还没有本机下载任务；请在云盘文件或接收分享中点击下载。" />
                <div v-else class="download-task-list">
                  <div v-for="task in downloadTasks" :key="task.id" class="download-task-item">
                    <div class="download-task-icon"><DownloadOutlined /></div>
                    <div class="download-task-content">
                      <div class="download-task-heading">
                        <div><strong>{{ task.fileName }}</strong><a-tag>{{ task.source }}</a-tag></div>
                        <a-tag :color="downloadStatus(task)[1]">{{ downloadStatus(task)[0] }}</a-tag>
                      </div>
                      <div class="download-task-path"><FolderOutlined /><span :title="task.filePath || task.destination">{{ task.filePath || task.destination }}</span></div>
                      <div v-if="task.status === 'downloading' && !task.totalBytes" class="download-indeterminate" title="服务器未返回总大小，正在持续下载"><span></span></div>
                      <a-progress v-else :percent="task.progress" :status="task.status === 'failed' ? 'exception' : task.status === 'completed' ? 'success' : 'active'" size="small" />
                      <div class="download-task-meta">
                        <span v-if="task.status === 'preparing'">{{ task.packaged ? '等待光鸭完成云端打包' : '正在获取下载地址' }}</span>
                        <span v-else-if="task.status === 'downloading'">{{ task.totalBytes ? `${formatSize(task.downloadedBytes)} / ${formatSize(task.totalBytes)}` : `已下载 ${formatSize(task.downloadedBytes)}` }}<template v-if="task.bytesPerSecond"> · {{ formatUploadSpeed(task.bytesPerSecond) }}</template></span>
                        <span v-else>{{ formatSize(task.downloadedBytes) }}</span>
                        <span>{{ formatTime(task.createdAt) }}</span>
                      </div>
                      <a-alert v-if="task.error" type="error" :message="task.error" show-icon />
                    </div>
                  </div>
                </div>
              </a-card>
            </template>

            <template v-else-if="activeView === 'offline'">
              <a-row :gutter="18">
                <a-col :xs="24" :lg="9">
                  <a-card class="content-card" :bordered="false" title="新建离线任务">
                    <a-form layout="vertical" @finish="createOffline">
                      <a-form-item label="资源链接" required><a-textarea v-model:value="offlineForm.url" :rows="5" placeholder="支持 HTTP、HTTPS、Magnet 或 ED2K 链接" /></a-form-item>
                      <a-form-item label="保存到云端目录"><a-flex gap="small"><a-input :value="offlineForm.parent_label" readonly><template #prefix><FolderOutlined /></template></a-input><a-button @click="chooseOfflineRemote">选择</a-button></a-flex></a-form-item>
                      <a-form-item label="重命名"><a-input v-model:value="offlineForm.new_name" placeholder="可选"><template #prefix><FileOutlined /></template></a-input></a-form-item>
                      <a-button type="primary" block html-type="submit"><template #icon><DownloadOutlined /></template>开始云端下载</a-button>
                    </a-form>
                  </a-card>
                </a-col>
                <a-col :xs="24" :lg="15">
                  <a-card class="content-card" :bordered="false" title="离线任务">
                    <template #extra><a-button type="text" :loading="offlineLoading" @click="loadOffline"><template #icon><ReloadOutlined /></template>刷新</a-button></template>
                    <a-table :columns="offlineColumns" :data-source="offlineTasks" :loading="offlineLoading" :row-key="(item) => pick(item, ['taskId', 'id', 'fileId'], item.fileName || item.name)" :pagination="false" size="small">
                      <template #emptyText><a-empty description="暂无离线任务" /></template>
                      <template #bodyCell="{ column, record }">
                        <template v-if="column.key === 'name'"><a-flex align="center" gap="small"><a-avatar class="list-avatar"><InboxOutlined /></a-avatar><strong>{{ pick(record, ['fileName', 'name', 'taskName', 'title'], '离线任务') }}</strong></a-flex></template>
                        <template v-else-if="column.key === 'size'">{{ formatSize(pick(record, ['totalSize', 'fileSize', 'size'], 0)) }}</template>
                        <template v-else-if="column.key === 'status'"><a-tag :color="offlineStatus(record)[1]">{{ offlineStatus(record)[0] }}</a-tag></template>
                      </template>
                    </a-table>
                  </a-card>
                </a-col>
              </a-row>
            </template>

            <template v-else>
              <div class="section-toolbar"><div><h2>我的分享</h2><p>直接查询光鸭账号中的已有分享；相同文件或文件夹会自动复用，不会重复创建。</p></div><a-space><a-button @click="openReceivedShare"><template #icon><InboxOutlined /></template>接收分享</a-button><a-button :loading="cloudSharesLoading" @click="loadCloudShares"><template #icon><ReloadOutlined /></template>刷新</a-button></a-space></div>
              <a-card class="content-card" :bordered="false">
                <a-table :columns="cloudShareColumns" :data-source="cloudShares" :loading="cloudSharesLoading" :row-key="(item) => item.id || item.shareId" :pagination="{ pageSize: 20, hideOnSinglePage: true }" size="small">
                  <template #emptyText><a-empty description="当前账号还没有分享记录" /></template>
                  <template #bodyCell="{ column, record }">
                    <template v-if="column.key === 'title'"><a-flex align="center" gap="small"><a-avatar class="list-avatar"><ShareAltOutlined /></a-avatar><div><strong>{{ record.title || '未命名分享' }}</strong><div class="table-secondary">{{ record.shareUrl }}</div></div></a-flex></template>
                    <template v-else-if="column.key === 'type'"><a-tag :color="Number(record.resType) === 2 ? 'blue' : undefined">{{ Number(record.resType) === 2 ? '文件夹' : '文件' }}</a-tag></template>
                    <template v-else-if="column.key === 'status'"><a-tag :color="cloudShareStatus(record)[1]">{{ cloudShareStatus(record)[0] }}</a-tag></template>
                    <template v-else-if="column.key === 'time'">{{ formatTime(record.createTime) }}</template>
                    <template v-else-if="column.key === 'actions'"><a-space><a-button size="small" @click="copyText(record.shareUrl)"><CopyOutlined />复制</a-button><a-button size="small" type="link" :href="record.shareUrl" target="_blank">打开</a-button><a-popconfirm v-if="Number(record.shareStatus) === 1" title="确定取消这个分享？链接将立即失效。" @confirm="deleteCloudShare(record)"><a-button size="small" danger type="text">取消分享</a-button></a-popconfirm></a-space></template>
                  </template>
                </a-table>
              </a-card>
              <div class="section-toolbar"><div><h2>本地收藏</h2><p>额外保存常用的外部分享链接，不影响光鸭账号中的分享。</p></div><a-button type="primary" @click="openShareForm"><template #icon><PlusOutlined /></template>保存分享链接</a-button></div>
              <a-row :gutter="16" class="share-grid">
                <a-col v-for="item in appState.saved_shares" :key="item.id" :xs="24" :lg="12" :xl="8">
                  <a-card class="share-card" :bordered="false">
                    <div class="share-card-icon"><ShareAltOutlined /></div>
                    <strong>{{ item.label }}</strong><p>{{ item.url }}</p><span>{{ formatTime(item.created_at) }}</span>
                    <a-flex gap="small" class="share-actions"><a-button type="primary" ghost @click="copyText(item.url)"><template #icon><CopyOutlined /></template>复制链接</a-button><a-popconfirm title="确定移除这个收藏？" @confirm="removeShare(item.id)"><a-button danger type="text"><template #icon><DeleteOutlined /></template></a-button></a-popconfirm></a-flex>
                  </a-card>
                </a-col>
                <a-col v-if="!appState.saved_shares.length" :span="24"><a-card class="empty-card" :bordered="false"><a-empty description="还没有收藏的分享链接"><a-button type="primary" @click="openShareForm">保存一个链接</a-button></a-empty></a-card></a-col>
              </a-row>
            </template>
          </a-layout-content>
          <a-layout-footer class="footer"><span>光鸭云盘非官方客户端</span><span>{{ isTauri ? 'Tauri 桌面端 · 后台文件监控已启用' : 'Linux Web 服务 · 系统目录监控' }}</span></a-layout-footer>
        </a-layout>
      </a-layout>

      <a-drawer v-model:open="backupOpen" title="新建备份任务" :width="480" :destroy-on-close="false">
        <template #extra><CloudSyncOutlined class="drawer-title-icon" /></template>
        <a-alert type="info" show-icon message="文件夹结构会原样同步到云端" class="drawer-alert" />
        <a-form layout="vertical">
          <a-form-item label="本地监控文件夹" required><a-flex gap="small"><a-input v-model:value="backupForm.local_path" readonly :placeholder="isTauri ? '选择需要监控的文件夹' : '例如 /mnt/media'"><template #prefix><FolderOutlined /></template></a-input><a-button @click="chooseFolder('local_path')">选择</a-button></a-flex></a-form-item>
          <a-form-item label="云端目录" required><a-flex gap="small"><a-input :value="backupForm.remote_path || '根目录'" readonly><template #prefix><CloudOutlined /></template></a-input><a-button @click="chooseBackupRemote">选择</a-button></a-flex></a-form-item>
          <a-form-item label="监控文件夹类型"><a-select v-model:value="backupForm.monitor_mode" :options="monitorModeOptions" /><div class="form-help">本地磁盘使用系统事件；网盘映射盘、NAS 或同步盘使用轮询。</div></a-form-item>
          <a-form-item label="同步文件后缀"><div class="extension-presets"><a-button v-for="preset in extensionPresets" :key="preset.key" @click="applyBackupExtensionPreset(preset.key)">{{ preset.label }}</a-button></div><a-select v-model:value="backupForm.sync_types" mode="tags" :token-separators="[',', '，', ' ']" :max-tag-count="8" placeholder="直接输入后缀，如 mp4、mkv、srt" /><div class="form-help">快捷按钮会填入常用后缀；也可以直接输入任意后缀，输入时可带或不带点。</div></a-form-item>
          <a-form-item label="上传完成后的源文件策略"><a-select v-model:value="backupForm.source_policy" :options="sourcePolicyOptions" /></a-form-item>
          <a-form-item v-if="backupForm.source_policy === 'archive'" label="归档目录" required><a-flex gap="small"><a-input v-model:value="backupForm.archive_path" readonly placeholder="选择归档文件夹"><template #prefix><InboxOutlined /></template></a-input><a-button @click="chooseFolder('archive_path')">选择</a-button></a-flex></a-form-item>
          <a-form-item><a-flex justify="space-between" align="center" class="switch-row"><div><strong>扫描已有文件</strong><span>创建任务后立即上传文件夹中的现有内容</span></div><a-switch v-model:checked="backupForm.scan_existing" /></a-flex></a-form-item>
          <a-form-item><a-flex justify="space-between" align="center" class="switch-row"><div><strong>上传完毕自动分享并投稿 Hdhive</strong><span>根文件分享文件；多层路径统一分享第一层目录。需先配置 Hdhive。</span></div><a-switch v-model:checked="backupForm.auto_share" /></a-flex></a-form-item>
          <a-button type="primary" block :loading="backupSubmitting" @click="addBackup"><template #icon><PlusOutlined /></template>创建备份任务</a-button>
        </a-form>
      </a-drawer>

      <a-modal v-model:open="folderPicker.open" :title="folderPicker.title" :width="620" ok-text="选择当前目录" cancel-text="取消" :confirm-loading="folderPicker.loading" @ok="confirmPicker" @cancel="folderPicker.onConfirm = null">
        <div class="folder-picker-path">
          <button :class="{ active: !folderPicker.stack.length }" @click="pickerGoTo(-1)"><HomeOutlined />根目录</button>
          <template v-for="(part, index) in folderPicker.stack" :key="part.id"><span>/</span><button :class="{ active: index === folderPicker.stack.length - 1 }" @click="pickerGoTo(index)">{{ part.name }}</button></template>
        </div>
        <div class="folder-picker-list" :class="{ loading: folderPicker.loading }">
          <a-skeleton v-if="folderPicker.loading" active :paragraph="{ rows: 5 }" />
          <button v-for="item in folderPicker.items" v-else :key="fileId(item)" @click="enterPickerFolder(item)"><span class="file-icon folder"><FolderOutlined /></span><span>{{ pick(item, ['fileName', 'name'], '文件夹') }}</span><span>{{ item.subFolderCount || 0 }} 个子目录</span></button>
          <a-empty v-if="!folderPicker.loading && !folderPicker.items.length" description="当前目录没有子文件夹，可直接选择这里" />
        </div>
        <div class="picker-current"><CheckOutlined />将使用：{{ folderPicker.stack.length ? `根目录 / ${folderPicker.stack.map(item => item.name).join(' / ')}` : '根目录' }}</div>
      </a-modal>

      <a-modal v-model:open="renameOpen" :width="780" :footer="null" title="链式批量重命名">
        <div class="rename-summary"><div><strong>{{ renameTargets.length }} 个项目</strong><span>规则会从上到下依次执行</span></div><a-checkbox v-model:checked="preserveExtension">保留文件扩展名</a-checkbox></div>
        <div class="rename-rules">
          <div v-for="(rule, index) in renameRules" :key="rule.id" class="rename-rule" :class="{ compact: ['set', 'prefix', 'suffix', 'upper', 'lower'].includes(rule.type) }">
            <div class="rule-order"><DragOutlined /><span>{{ index + 1 }}</span></div>
            <a-select v-model:value="rule.type" :options="renameRuleOptions" class="rule-type" />
            <a-input v-if="rule.type === 'set' || rule.type === 'prefix' || rule.type === 'suffix'" v-model:value="rule.value" :placeholder="rule.type === 'set' ? '新名称' : rule.type === 'prefix' ? '前缀内容' : '后缀内容'" />
            <template v-else-if="rule.type === 'replace' || rule.type === 'regex'"><a-input v-model:value="rule.search" :placeholder="rule.type === 'regex' ? '正则表达式' : '查找文本'" /><a-input v-model:value="rule.replacement" placeholder="替换为" /><a-tooltip title="忽略大小写"><a-switch v-model:checked="rule.ignoreCase" checked-children="Aa" un-checked-children="Aa" /></a-tooltip></template>
            <template v-else-if="rule.type === 'sequence'"><a-input v-model:value="rule.value" placeholder="模板，如 -{n}" /><a-input-number v-model:value="rule.start" :min="0" addon-before="起始" /><a-input-number v-model:value="rule.padding" :min="1" :max="12" addon-before="位数" /></template>
            <span v-else class="rule-description">{{ rule.type === 'upper' ? '将当前结果转为大写' : '将当前结果转为小写' }}</span>
            <div class="rule-actions"><a-button type="text" size="small" :disabled="index === 0" @click="moveRenameRule(index, -1)"><ArrowUpOutlined /></a-button><a-button type="text" size="small" :disabled="index === renameRules.length - 1" @click="moveRenameRule(index, 1)"><ArrowDownOutlined /></a-button><a-button type="text" danger size="small" :disabled="renameRules.length === 1" @click="removeRenameRule(index)"><DeleteOutlined /></a-button></div>
          </div>
          <a-button block type="dashed" @click="addRenameRule"><PlusOutlined />追加规则</a-button>
        </div>
        <a-alert v-if="renamePreview.error" type="error" show-icon :message="renamePreview.error" />
        <div v-else class="rename-preview"><div class="preview-head"><strong>实时预览</strong><span>{{ renameChangedCount }} 项将改变</span></div><div class="preview-list"><div v-for="row in renamePreview.rows" :key="row.fileId"><span>{{ row.currentName }}</span><SwapOutlined /><strong :class="{ unchanged: row.currentName === row.newName }">{{ row.newName }}</strong></div></div></div>
        <a-flex justify="flex-end" gap="small" class="modal-actions"><a-button @click="renameOpen = false">取消</a-button><a-button type="primary" :disabled="Boolean(renamePreview.error) || !renameChangedCount" :loading="renaming" @click="executeRename"><EditOutlined />执行重命名</a-button></a-flex>
      </a-modal>

      <a-modal v-model:open="deleteDialog.open" title="移入回收站" ok-text="删除" cancel-text="取消" ok-type="danger" :confirm-loading="deleteDialog.loading" @ok="confirmDelete">
        <a-alert type="warning" show-icon :message="`确定将选中的 ${deleteDialog.items.length} 项移入回收站？`" description="此操作不会立即永久删除，可在光鸭云盘回收站中恢复。" />
        <div class="delete-preview"><div v-for="item in deleteDialog.items.slice(0, 8)" :key="fileId(item)"><FileOutlined />{{ pick(item, ['fileName', 'name'], '未命名') }}</div><span v-if="deleteDialog.items.length > 8">另有 {{ deleteDialog.items.length - 8 }} 项</span></div>
      </a-modal>

      <Teleport to="body">
        <div v-if="contextMenu.visible" class="file-context-menu" :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }" @click.stop>
          <template v-if="contextMenu.record">
            <button v-if="isFolder(contextMenu.record)" @click="contextAction('open')"><FolderOpenOutlined />打开</button>
            <button @click="contextAction('copy')"><CopyOutlined />复制</button>
            <button @click="contextAction('cut')"><ScissorOutlined />剪切</button>
            <button @click="contextAction('copyTo')"><CopyOutlined />复制到…</button>
            <button @click="contextAction('moveTo')"><SwapOutlined />移动到…</button>
            <button @click="contextAction('rename')"><EditOutlined />重命名 <kbd>F2</kbd></button>
            <button @click="contextAction('download')"><DownloadOutlined />下载</button>
            <button :disabled="shareCreating" @click="contextAction('share')"><ShareAltOutlined />{{ shareCreating ? '正在创建分享…' : '创建分享' }}</button>
            <div class="context-separator"></div>
            <button class="danger" @click="contextAction('delete')"><DeleteOutlined />移入回收站 <kbd>Del</kbd></button>
          </template>
          <template v-else>
            <button @click="contextAction('uploadFile')"><FileAddOutlined />上传文件</button>
            <button @click="contextAction('uploadFolder')"><FolderOpenOutlined />上传文件夹</button>
            <button :disabled="!clipboard.items.length" @click="contextAction('paste')"><CheckOutlined />粘贴</button>
            <div class="context-separator"></div>
            <button @click="contextAction('refresh')"><ReloadOutlined />刷新</button>
          </template>
        </div>
      </Teleport>

      <a-modal v-if="!isTauri" v-model:open="uploadSourceOpen" title="选择上传来源" :footer="null" :width="560">
        <div class="upload-source-grid">
          <button type="button" class="upload-source-option" @click="chooseBrowserUpload">
            <UploadOutlined />
            <strong>{{ uploadSourceKind === 'folder' ? '浏览器本地文件夹' : '浏览器本地文件' }}</strong>
            <span>从当前电脑选择，浏览器先把文件传到当前 Web 服务</span>
          </button>
          <button type="button" class="upload-source-option" @click="chooseServerUpload">
            <FolderOpenOutlined />
            <strong>服务器文件</strong>
            <span>直接选择服务进程有权限访问的文件或文件夹，不经过浏览器中转</span>
          </button>
        </div>
      </a-modal>

      <a-modal
        v-if="!isTauri"
        v-model:open="serverFilePicker.open"
        :title="serverFilePicker.mode === 'folder' ? '选择服务器文件夹' : '选择服务器文件'"
        :ok-text="serverFilePicker.mode === 'folder' ? '选择当前目录' : '加入上传队列'"
        cancel-text="取消"
        :width="720"
        :confirm-loading="serverFilePicker.submitting"
        :ok-button-props="{ disabled: serverFilePicker.mode === 'upload' && !serverFilePicker.selected.length }"
        @ok="confirmServerPicker"
      >
        <div class="server-picker-toolbar">
          <a-button size="small" :disabled="!serverFilePicker.parent" @click="loadServerDirectory(serverFilePicker.parent)"><ArrowUpOutlined />上一级</a-button>
          <a-select v-if="serverFilePicker.roots.length > 1" :value="activeServerRoot" size="small" style="min-width: 160px" :options="serverFilePicker.roots.map((root) => ({ label: root, value: root }))" @change="loadServerDirectory" />
          <span>{{ serverFilePicker.displayPath }}</span>
          <a-tag v-if="serverFilePicker.mode === 'upload'" color="blue">已选 {{ serverFilePicker.selected.length }} 项</a-tag><span v-else></span>
        </div>
        <a-spin :spinning="serverFilePicker.loading">
          <div v-if="serverFilePicker.items.length" class="server-file-list">
            <div v-for="item in serverFilePicker.items.filter((entry) => serverFilePicker.mode === 'upload' || entry.type === 'directory')" :key="item.path" class="server-file-row" :class="{ 'folder-only': serverFilePicker.mode === 'folder' }">
              <a-checkbox v-if="serverFilePicker.mode === 'upload'" :checked="serverFilePicker.selected.includes(item.path)" @change="(event) => toggleServerSelection(item, event.target.checked)" />
              <span class="file-icon" :class="item.type === 'directory' ? 'folder' : 'file'"><FolderOutlined v-if="item.type === 'directory'" /><FileOutlined v-else /></span>
              <button type="button" class="server-file-name" @dblclick="item.type === 'directory' && loadServerDirectory(item.path)">{{ item.name }}</button>
              <span class="server-file-size">{{ item.type === 'directory' ? '文件夹' : formatSize(item.size) }}</span>
              <a-button v-if="item.type === 'directory'" type="link" size="small" @click="loadServerDirectory(item.path)">打开</a-button>
            </div>
          </div>
          <a-empty v-else description="这个服务器目录为空" />
        </a-spin>
        <a-alert class="server-picker-tip" type="info" show-icon :message="serverFilePicker.mode === 'folder' ? '只能选择服务进程实际有权限访问的目录。' : '文件夹会递归上传并保留目录结构；未修改且已上传的文件会自动跳过。'" />
      </a-modal>

      <a-modal v-model:open="shareOpen" title="保存分享链接" ok-text="保存" cancel-text="取消" @ok="saveShareLink">
        <a-form layout="vertical"><a-form-item label="名称"><a-input v-model:value="shareForm.label" placeholder="例如：项目资料"><template #prefix><ShareAltOutlined /></template></a-input></a-form-item><a-form-item label="分享链接" required><a-input v-model:value="shareForm.url" placeholder="https://..."><template #prefix><LinkOutlined /></template></a-input></a-form-item></a-form>
      </a-modal>

      <a-modal v-model:open="receivedShare.open" title="接收光鸭分享" :width="820" :footer="null" :mask-closable="!receivedShare.restoring && !receivedShare.downloading">
        <a-flex vertical gap="middle">
          <a-alert type="info" show-icon message="不会自动监听剪贴板；只有点击“粘贴剪贴板”时才会读取。" />
          <a-flex gap="small">
            <a-input v-model:value="receivedShare.link" placeholder="粘贴 https://www.guangyapan.com/s/... 分享链接" @press-enter="loadReceivedShare"><template #prefix><LinkOutlined /></template></a-input>
            <a-button @click="pasteReceivedShareLink"><template #icon><CopyOutlined /></template>粘贴剪贴板</a-button>
            <a-button type="primary" :loading="receivedShare.loading" @click="loadReceivedShare">读取分享</a-button>
          </a-flex>
          <template v-if="receivedShare.accessToken">
            <div class="file-breadcrumb">
              <button :class="{ active: !receivedShare.stack.length }" @click="goToReceivedSharePath(-1)"><HomeOutlined />分享根目录</button>
              <template v-for="(part, index) in receivedShare.stack" :key="part.id"><span>/</span><button :class="{ active: index === receivedShare.stack.length - 1 }" @click="goToReceivedSharePath(index)">{{ part.name }}</button></template>
            </div>
            <a-flex justify="space-between" align="center" wrap="wrap" gap="small">
              <span>{{ receivedSharePath }} · 共 {{ receivedShare.items.length }} 项</span>
              <a-flex align="center" gap="small"><span>转存到：{{ receivedShare.targetLabel }}</span><a-button size="small" @click="chooseReceivedShareTarget">选择目录</a-button></a-flex>
            </a-flex>
            <a-table :columns="receivedShareColumns" :data-source="receivedShare.items" :loading="receivedShare.loading" :row-key="fileId" :row-selection="receivedShareRowSelection" :custom-row="receivedShareRowProps" :pagination="false" :scroll="{ y: 360 }" size="small">
              <template #emptyText><a-empty description="当前分享目录为空" /></template>
              <template #bodyCell="{ column, record }">
                <template v-if="column.key === 'name'"><button type="button" class="file-name-button" :class="{ clickable: isFolder(record) }" @dblclick.stop="enterReceivedShareFolder(record)"><span class="file-icon" :class="isFolder(record) ? 'folder' : 'file'"><FolderOutlined v-if="isFolder(record)" /><FileOutlined v-else /></span><span>{{ pick(record, ['fileName', 'name'], '未命名') }}</span></button></template>
                <template v-else-if="column.key === 'type'"><a-tag :color="isFolder(record) ? 'blue' : undefined">{{ isFolder(record) ? '文件夹' : (record.ext || '文件') }}</a-tag></template>
                <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
              </template>
            </a-table>
            <a-flex justify="flex-end" gap="small"><a-button :disabled="receivedShare.restoring || receivedShare.downloading" @click="receivedShare.open = false">取消</a-button><a-button :disabled="!receivedShare.selected.length || receivedShare.restoring" :loading="receivedShare.downloading" @click="downloadReceivedShare"><template #icon><DownloadOutlined /></template>选择目录并下载</a-button><a-button type="primary" :disabled="!receivedShare.selected.length || receivedShare.downloading" :loading="receivedShare.restoring" @click="restoreReceivedShare">转存选中 {{ receivedShare.selected.length }} 项</a-button></a-flex>
          </template>
        </a-flex>
      </a-modal>

      <a-modal v-model:open="shareResultOpen" :title="lastShare.reused ? '已复用已有分享' : '分享创建成功'" :footer="null">
        <a-result status="success" :title="lastShare.reused ? '相同内容已存在，未重复创建' : '分享链接已生成'" :sub-title="lastShare.code ? `提取码：${lastShare.code}` : '此分享不需要提取码'">
          <template #extra><a-flex vertical gap="middle"><a-input :value="lastShare.url" readonly><template #prefix><LinkOutlined /></template><template #suffix><a-button type="text" @click="copyText(lastShare.url)"><CopyOutlined /></a-button></template></a-input><a-alert :type="receiptAlertType(lastShareHdhiveStatus)" show-icon :message="lastShareHdhiveMessage" /><a v-if="lastShareReceipt?.resource_url" :href="lastShareReceipt.resource_url" target="_blank" rel="noreferrer">查看影巢资源</a><a-flex justify="center" gap="small"><a-button @click="shareResultOpen = false">完成</a-button><a-button type="primary" @click="saveCreatedShare">加入收藏</a-button></a-flex></a-flex></template>
        </a-result>
      </a-modal>

      <a-modal :open="loginOpen" :footer="null" :width="680" :closable="true" @cancel="closeLogin">
        <div class="login-panel">
          <div class="login-heading"><div class="login-heading-icon"><QrcodeOutlined /></div><div><h2>登录光鸭云盘</h2><p>使用光鸭云盘 App 扫码确认授权，登录状态会保存在本机 SQLite</p></div></div>
          <a-divider />
          <a-flex class="login-content" gap="large" align="center">
            <div class="qr-frame"><a-skeleton v-if="login.loading" active :paragraph="{ rows: 5 }" /><a-qrcode v-else-if="login.qr" :value="login.qr" :size="210" :bordered="false" /><div v-else class="qr-error"><QrcodeOutlined /><span>二维码加载失败</span></div></div>
            <div class="login-guide"><a-steps direction="vertical" size="small" :current="0" :items="[{ title: '打开光鸭云盘 App', description: '进入扫一扫功能' }, { title: '扫描左侧二维码', description: '确认本应用的授权请求' }, { title: '自动完成登录', description: '无需复制任何令牌' }]" /><div class="device-code"><span>设备验证码</span><strong>{{ login.userCode }}</strong></div><a-flex gap="small"><a-button type="primary" ghost :href="login.verificationUrl" target="_blank" :disabled="!login.verificationUrl">打开官方授权页</a-button><a-button :loading="login.loading" @click="refreshLogin"><template #icon><ReloadOutlined /></template>刷新二维码</a-button></a-flex></div>
          </a-flex>
          <div class="login-status"><a-badge status="processing" :text="login.message" /><span v-if="login.remaining">{{ login.remaining }} 秒后自动刷新</span></div>
          <template v-if="!isTauri">
            <a-divider>备用登录方式</a-divider>
            <a-collapse ghost><a-collapse-panel key="token" header="手动填写 Bearer Token"><div class="web-login"><a-input-password v-model:value="loginToken" placeholder="Bearer Token" @press-enter="setWebToken"><template #prefix><SafetyCertificateOutlined /></template></a-input-password><a-button type="primary" block @click="setWebToken">使用令牌连接</a-button></div></a-collapse-panel></a-collapse>
          </template>
        </div>
      </a-modal>
    </a-app>
  </a-config-provider>
</template>
