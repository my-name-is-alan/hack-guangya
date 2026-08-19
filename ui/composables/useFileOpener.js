// 按类型打开云盘文件的统一入口：视频→外部播放器、图片→内嵌查看器、
// 音频→内嵌播放器、文本→预览弹窗；其余类型返回 false 由调用方兜底。
// 播放直链走两端共用的 /strm/{fileId}?sign= 通道（桌面为本机 STRM 服务，
// Web 为同源路径），签名是稳定 HMAC，可在会话内长期缓存。

import { reactive } from 'vue';
import { message } from 'antdv-next';
import { bridge } from '../bridge.js';
import { errorText, fileId, isFolder, unwrapData } from '../formatters.js';
import { OPEN_KIND, externalPlayerOptions, openKindOf } from '../fileOpen.js';
import { useAudioPlayer } from './useAudioPlayer.js';

const SIBLING_PAGE_SIZE = 100;
const MAX_SIBLING_ITEMS = 2000;
const VIDEO_PLAYER_STORAGE_KEY = 'guangya.open.video-player';

const playUrlCache = new Map();

const imageViewer = reactive({ open: false, items: [], index: 0 });
const textPreview = reactive({ open: false, record: null });
const videoPicker = reactive({ open: false, record: null });

export async function getPlayUrls(ids) {
  const wanted = [...new Set((Array.isArray(ids) ? ids : []).map((value) => String(value || '').trim()).filter(Boolean))];
  const missing = wanted.filter((id) => !playUrlCache.has(id));
  if (missing.length) {
    const data = unwrapData(await bridge.invoke('get_play_urls', { file_ids: missing }));
    for (const entry of Array.isArray(data.urls) ? data.urls : []) {
      const id = String(entry.file_id ?? entry.fileId ?? '');
      const url = String(entry.url || '');
      if (id && url) playUrlCache.set(id, url);
    }
  }
  return new Map(wanted.map((id) => [id, playUrlCache.get(id) || '']));
}

export async function getPlayUrl(record) {
  const id = String(fileId(record));
  const urls = await getPlayUrls([id]);
  const url = urls.get(id);
  if (!url) throw new Error('未能获取播放直链');
  return url;
}

/** 外部播放器需要绝对地址；Web 端把同源相对路径补全为完整 URL。 */
export async function getAbsolutePlayUrl(record) {
  return new URL(await getPlayUrl(record), window.location.href).href;
}

export async function readCloudText(record, maxBytes = 512 * 1024) {
  const data = unwrapData(await bridge.invoke('read_cloud_text', {
    file_id: String(fileId(record)),
    max_bytes: maxBytes,
  }));
  return {
    text: String(data.text ?? ''),
    truncated: Boolean(data.truncated),
    size: Number(data.size || 0),
  };
}

/**
 * 汇总同目录条目：优先用调用方已加载的当前页，目录有多页时
 * 走 list_files 补齐（封顶 MAX_SIBLING_ITEMS），失败退回已加载内容。
 */
async function collectSiblings(context = {}) {
  const loaded = Array.isArray(context.siblings) ? context.siblings : [];
  const total = Number(context.total ?? loaded.length);
  const dirId = context.dirId;
  if (dirId === undefined || dirId === null || total <= loaded.length) return loaded;
  const collected = [];
  try {
    const target = Math.min(total, MAX_SIBLING_ITEMS);
    for (let page = 0; collected.length < target; page += 1) {
      const data = unwrapData(await bridge.invoke('list_files', { page, parent_id: dirId }));
      const list = Array.isArray(data.list) ? data.list : [];
      if (!list.length) break;
      collected.push(...list);
      if (list.length < SIBLING_PAGE_SIZE) break;
    }
  } catch {
    return loaded;
  }
  return collected.length ? collected : loaded;
}

export function readRememberedPlayer() {
  try {
    const raw = window.localStorage?.getItem(VIDEO_PLAYER_STORAGE_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

export function rememberPlayer(player) {
  try {
    if (player) window.localStorage?.setItem(VIDEO_PLAYER_STORAGE_KEY, JSON.stringify(player));
    else window.localStorage?.removeItem(VIDEO_PLAYER_STORAGE_KEY);
  } catch {
    // localStorage 不可用时只影响“记住选择”。
  }
}

/**
 * 用指定播放器打开：local = 桌面端本地可执行文件，scheme = URL 协议唤起。
 */
export async function launchVideo(record, player) {
  const url = await getAbsolutePlayUrl(record);
  if (player?.type === 'local') {
    await bridge.invoke('open_in_player', { player_path: String(player.path || ''), url });
    return;
  }
  const option = externalPlayerOptions().find((item) => item.id === player?.id);
  if (!option) throw new Error('未知的播放器，请重新选择');
  // 自定义协议不会离开当前页面；无对应处理程序时浏览器静默忽略。
  window.location.href = option.buildUrl(url);
}

async function openVideo(record) {
  const remembered = readRememberedPlayer();
  if (remembered) {
    try {
      await launchVideo(record, remembered);
      message.success(`已调用${remembered.name ? ` ${remembered.name}` : '播放器'}播放`);
      return;
    } catch (error) {
      rememberPlayer(null);
      message.warning(`调用已记住的播放器失败，请重新选择：${errorText(error)}`);
    }
  }
  videoPicker.record = record;
  videoPicker.open = true;
}

async function openImageViewer(record, context) {
  // 先展示当前图片保证响应速度，同目录集合就绪后再扩展成图集。
  imageViewer.items = [record];
  imageViewer.index = 0;
  imageViewer.open = true;
  const currentId = String(fileId(record));
  const siblings = await collectSiblings(context);
  const images = siblings.filter((item) => !isFolder(item) && openKindOf(item) === OPEN_KIND.IMAGE);
  const index = images.findIndex((item) => String(fileId(item)) === currentId);
  if (!imageViewer.open || index < 0) return;
  const shown = imageViewer.items[imageViewer.index];
  if (String(fileId(shown)) !== currentId) return;
  imageViewer.items = images;
  imageViewer.index = index;
}

const audioPlayer = useAudioPlayer();

async function openAudioPlayer(record, context) {
  const siblings = await collectSiblings(context);
  audioPlayer.openQueue(record, siblings.length ? siblings : [record]);
}

function openTextPreview(record) {
  textPreview.record = record;
  textPreview.open = true;
}

/**
 * 按类型打开文件。返回 true 表示已接管（含失败提示），
 * 返回 false 表示类型不支持，由调用方兜底（如详情抽屉）。
 * context: { siblings, total, dirId } 描述文件所在目录的已加载列表。
 */
async function openFile(record, context = {}) {
  if (!record || isFolder(record)) return false;
  const kind = openKindOf(record);
  if (kind === OPEN_KIND.OTHER) return false;
  try {
    if (kind === OPEN_KIND.VIDEO) await openVideo(record);
    else if (kind === OPEN_KIND.IMAGE) await openImageViewer(record, context);
    else if (kind === OPEN_KIND.AUDIO) await openAudioPlayer(record, context);
    else if (kind === OPEN_KIND.TEXT) openTextPreview(record);
  } catch (error) {
    message.error(errorText(error));
  }
  return true;
}

/** 供不支持内嵌播放的音频等场景复用“外部播放器”选择弹窗。 */
function openExternalPlayerPicker(record) {
  videoPicker.record = record;
  videoPicker.open = true;
}

export function useFileOpener() {
  return {
    imageViewer,
    textPreview,
    videoPicker,
    openFile,
    openExternalPlayerPicker,
    launchVideo,
    readRememberedPlayer,
    rememberPlayer,
  };
}
