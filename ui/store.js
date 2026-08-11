import { computed } from 'vue';
import { storeToRefs } from 'pinia';
import { formatSize, formatTime, pick } from './formatters.js';
import { useFilesStore } from './stores/files.ts';
import { useSessionStore } from './stores/session.ts';
import { useTransfersStore } from './stores/transfers.ts';
import { classifyVipStatus } from './vipStatus.js';

export { formatSize, formatTime };

const sessionStore = useSessionStore();
const filesStore = useFilesStore();
const transfersStore = useTransfersStore();
const fileRefs = storeToRefs(filesStore);
const sessionRefs = storeToRefs(sessionStore);

export const appState = sessionStore.state;
Object.defineProperty(appState, 'logs', { get: () => sessionStore.logs });
export const overview = sessionStore.overview;
export const bootLoading = sessionRefs.bootLoading;
export const filesLoading = fileRefs.loading;
export const files = fileRefs.files;
export const filesPage = fileRefs.page;
export const filesTotal = fileRefs.total;
export const filesPageSize = 100;
export const currentPath = fileRefs.currentPath;
export const currentFolderId = fileRefs.currentFolderId;
export const currentFolderName = fileRefs.currentFolderName;

export const userName = sessionRefs.userName;
export const userAvatar = sessionRefs.userAvatar;
export const profileId = computed(() => pick(overview.profile, ['sub', 'userId', 'id'], '—'));
export const profilePhone = computed(() => pick(overview.profile, ['phone_number', 'phoneNumber', 'phone', 'mobile'], '未绑定'));
export const usedSpace = sessionRefs.usedSpace;
export const totalSpace = sessionRefs.totalSpace;
export const quotaPercent = sessionRefs.quotaPercent;
export const vipExpireTime = computed(() => Number(pick(overview.assets, ['vipExpireTime', 'vip_expire_time'], 0)));
const vipState = computed(() => classifyVipStatus(pick(overview.assets, ['vipStatus', 'svipStatus', 'isVip'], 1)));
export const isVip = computed(() => vipState.value.active);
export const vipExpired = computed(() => vipState.value.expired);
export const vipLabel = computed(() => isVip.value ? 'VIP会员' : vipExpired.value ? 'VIP已过期' : '普通用户');
export const vipExpireLabel = computed(() => vipExpireTime.value ? formatTime(vipExpireTime.value) : isVip.value ? '未返回到期时间' : '未开通 VIP');

export const queueText = computed(() => {
  if (appState.paused) return '队列已暂停';
  if (appState.pending || appState.active_uploads) return '正在同步';
  return '队列空闲';
});
export const totalUploadSpeed = transfersStore.uploadSpeed;
export function formatUploadSpeed(bytesPerSecond) {
  const value = Number(bytesPerSecond || 0);
  if (!value) return '';
  return `${formatSize(value)}/s`;
}

export function applyState(next = {}) {
  sessionStore.applyState(next);
}

export async function loadOverview() {
  return sessionStore.loadOverview();
}

export async function loadFiles(page = 0, options = {}) {
  return filesStore.loadFiles(page, options);
}

export async function refreshState() {
  try { await sessionStore.refreshState(); } catch { /* 忽略 */ }
}
