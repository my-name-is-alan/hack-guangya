import { computed, reactive, ref } from 'vue';
import { bridge } from './bridge.js';
import { formatSize, formatTime, normalizeAvatarUrl, pick, unwrapData } from './formatters.js';

export { formatSize, formatTime };

export const appState = reactive({
  logged_in: false,
  paused: false,
  pending: 0,
  active_uploads: 0,
  upload_concurrency: 1,
  download_concurrency: 1,
  upload_speed_bps: 0,
  mappings: [],
  share_links: [],
  auto_share_events: [],
  logs: [],
});

export const overview = reactive({ profile: {}, quota: {}, vip: {} });
export const bootLoading = ref(true);
export const filesLoading = ref(false);
export const files = ref([]);
export const currentPath = ref([{ id: '', name: '全部文件' }]);

export const currentFolderId = computed(() => currentPath.value[currentPath.value.length - 1]?.id || '');
export const currentFolderName = computed(() => currentPath.value[currentPath.value.length - 1]?.name || '全部文件');

export const userName = computed(() => pick(overview.profile, ['nickname', 'nickName', 'name', 'userName'], appState.logged_in ? '光鸭云盘用户' : '未登录'));
export const userAvatar = computed(() => normalizeAvatarUrl(pick(overview.profile, ['avatarUrl', 'avatar_url', 'avatar', 'headImageUrl', 'headImg', 'portrait', 'icon'], '')));
export const profileId = computed(() => pick(overview.profile, ['sub', 'userId', 'id'], '—'));
export const profilePhone = computed(() => pick(overview.profile, ['phone_number', 'phoneNumber', 'phone', 'mobile'], '未绑定'));
export const usedSpace = computed(() => Number(pick(overview.quota, ['usedSize', 'used_size'], 0)));
export const totalSpace = computed(() => Number(pick(overview.quota, ['totalSize', 'total_size'], 0)));
export const quotaPercent = computed(() => totalSpace.value ? Math.min(100, Math.round((usedSpace.value / totalSpace.value) * 100)) : 0);
export const vipExpireTime = computed(() => Number(pick(overview.vip, ['vipExpireTime', 'vip_expire_time'], 0)));
export const isVip = computed(() => Number(pick(overview.vip, ['isVip', 'is_vip'], 0)) === 1);
export const vipExpired = computed(() => Boolean(vipExpireTime.value && vipExpireTime.value <= Date.now()));
export const vipLabel = computed(() => isVip.value ? 'VIP会员' : vipExpired.value ? 'VIP已过期' : '普通用户');
export const vipExpireLabel = computed(() => vipExpireTime.value ? formatTime(vipExpireTime.value) : isVip.value ? '未返回到期时间' : '未开通 VIP');

export const queueText = computed(() => {
  if (appState.paused) return '队列已暂停';
  if (appState.pending || appState.active_uploads) return '正在同步';
  return '队列空闲';
});
export const totalUploadSpeed = computed(() => Number(appState.upload_speed_bps || 0));
export function formatUploadSpeed(bytesPerSecond) {
  const value = Number(bytesPerSecond || 0);
  if (!value) return '';
  return `${formatSize(value)}/s`;
}

export function applyState(next = {}) {
  Object.assign(appState, next);
  if (!appState.logged_in) {
    Object.assign(overview, { profile: {}, quota: {}, vip: {} });
    files.value = [];
    currentPath.value = [{ id: '', name: '全部文件' }];
  }
}

export async function loadOverview() {
  if (!appState.logged_in) return;
  try {
    const data = unwrapData(await bridge.invoke('get_overview'));
    Object.assign(overview, {
      profile: data.profile || {},
      quota: data.quota || {},
      vip: data.vip || {},
    });
  } catch {
    // 静默失败，避免轮询刷屏
  }
}

export async function loadFiles() {
  if (!appState.logged_in) return;
  filesLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_files', { page: 0, parent_id: currentFolderId.value }));
    files.value = data.list || [];
  } catch (error) {
    throw error;
  } finally {
    filesLoading.value = false;
  }
}

export async function refreshState() {
  try { applyState(unwrapData(await bridge.invoke('get_state'))); } catch { /* 忽略 */ }
}
