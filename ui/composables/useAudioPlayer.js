// 内嵌音频播放器的共享状态：播放队列（同目录音频）与当前曲目。
// 播放机制（<audio> 元素、歌词加载）由 AudioPlayerBar 组件实现，
// 状态放在模块级单例里，播放不随视图切换中断。

import { reactive } from 'vue';
import { fileId, isFolder } from '../formatters.js';
import { OPEN_KIND, openKindOf } from '../fileOpen.js';

const state = reactive({
  visible: false,
  expanded: false,
  queue: [],
  // 同目录完整文件列表（含非音频），用于歌词等附属文件查找。
  siblings: [],
  index: -1,
  // 切歌序号：组件据此丢弃过期的异步结果（直链/歌词请求竞态）。
  requestId: 0,
});

function openQueue(record, siblings) {
  const pool = Array.isArray(siblings) && siblings.length ? siblings : [record];
  const tracks = pool.filter((item) => !isFolder(item) && openKindOf(item) === OPEN_KIND.AUDIO);
  const queue = tracks.length ? tracks : [record];
  const index = queue.findIndex((item) => String(fileId(item)) === String(fileId(record)));
  state.queue = queue;
  state.siblings = pool;
  state.index = Math.max(0, index);
  state.requestId += 1;
  state.visible = true;
}

function playAt(index) {
  if (index < 0 || index >= state.queue.length) return;
  state.index = index;
  state.requestId += 1;
}

function playPrevious() {
  playAt(state.index - 1);
}

function playNext() {
  playAt(state.index + 1);
}

function close() {
  state.visible = false;
  state.expanded = false;
  state.queue = [];
  state.siblings = [];
  state.index = -1;
  state.requestId += 1;
}

export function useAudioPlayer() {
  return { state, openQueue, playAt, playPrevious, playNext, close };
}
