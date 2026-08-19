<script setup>
import { computed, reactive, ref, watch, onBeforeUnmount } from 'vue';
import { message } from 'antdv-next';
import {
  CloseOutlined,
  DownloadOutlined,
  LeftOutlined,
  MinusOutlined,
  PlusOutlined,
  ReloadOutlined,
  RightOutlined,
  RotateRightOutlined,
} from '@antdv-next/icons';
import { errorText, fileId, formatSize, pick } from '../../formatters.js';
import { getPlayUrls, useFileOpener } from '../../composables/useFileOpener.js';
import { useTransfersStore } from '../../stores/transfers.ts';

const MIN_SCALE = 0.1;
const MAX_SCALE = 8;

const { imageViewer } = useFileOpener();
const transfers = useTransfersStore();

// fileId → 播放直链 / 加载状态（'loading' | 'ready' | 'error'）
const urls = reactive({});
const status = reactive({});
const transform = reactive({ scale: 1, rotate: 0, x: 0, y: 0 });
const dragging = ref(false);
let dragStart = null;

const current = computed(() => imageViewer.items[imageViewer.index] || null);
const currentId = computed(() => (current.value ? String(fileId(current.value)) : ''));
const currentName = computed(() => String(pick(current.value || {}, ['fileName', 'name'], '')));
const currentSize = computed(() => formatSize(pick(current.value || {}, ['fileSize', 'size'], 0)));
const counter = computed(() => `${imageViewer.index + 1} / ${imageViewer.items.length}`);
const currentUrl = computed(() => urls[currentId.value] || '');
const currentStatus = computed(() => status[currentId.value] || 'loading');
// 云端列表自带的小缩略图：原图加载期间先模糊展示。
const currentThumbnail = computed(() => String(current.value?.thumbnail || '').trim());
const hasPrevious = computed(() => imageViewer.index > 0);
const hasNext = computed(() => imageViewer.index < imageViewer.items.length - 1);
const imageStyle = computed(() => ({
  transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.scale}) rotate(${transform.rotate}deg)`,
}));

function resetTransform() {
  transform.scale = 1;
  transform.rotate = 0;
  transform.x = 0;
  transform.y = 0;
}

async function ensureUrls() {
  const items = imageViewer.items;
  if (!items.length) return;
  // 当前图 + 前后相邻图一起取直链，切换时无需等待。
  const targets = [imageViewer.index, imageViewer.index - 1, imageViewer.index + 1]
    .filter((index) => index >= 0 && index < items.length)
    .map((index) => String(fileId(items[index])))
    .filter((id) => id && !urls[id]);
  if (!targets.length) return;
  for (const id of targets) status[id] = 'loading';
  try {
    const resolved = await getPlayUrls(targets);
    for (const [id, url] of resolved) {
      if (url) {
        urls[id] = url;
      } else {
        status[id] = 'error';
      }
    }
  } catch (error) {
    for (const id of targets) status[id] = 'error';
    message.error(errorText(error));
  }
}

watch(
  () => [imageViewer.open, imageViewer.index, imageViewer.items],
  ([open]) => {
    if (!open) return;
    resetTransform();
    void ensureUrls();
  },
  { immediate: true },
);

watch(() => imageViewer.open, (open) => {
  if (open) window.addEventListener('keydown', handleKeydown, true);
  else window.removeEventListener('keydown', handleKeydown, true);
});
onBeforeUnmount(() => window.removeEventListener('keydown', handleKeydown, true));

function close() {
  imageViewer.open = false;
  imageViewer.items = [];
  imageViewer.index = 0;
}

function showPrevious() {
  if (hasPrevious.value) imageViewer.index -= 1;
}

function showNext() {
  if (hasNext.value) imageViewer.index += 1;
}

function handleKeydown(event) {
  if (!imageViewer.open) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    close();
  } else if (event.key === 'ArrowLeft') {
    event.preventDefault();
    showPrevious();
  } else if (event.key === 'ArrowRight') {
    event.preventDefault();
    showNext();
  }
}

function zoomBy(factor, originEvent = null) {
  const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, transform.scale * factor));
  if (originEvent && next !== transform.scale) {
    // 以光标位置为缩放锚点，保持指向的内容不动。
    const ratio = next / transform.scale;
    const rect = originEvent.currentTarget.getBoundingClientRect();
    const originX = originEvent.clientX - rect.left - rect.width / 2;
    const originY = originEvent.clientY - rect.top - rect.height / 2;
    transform.x = originX - (originX - transform.x) * ratio;
    transform.y = originY - (originY - transform.y) * ratio;
  }
  transform.scale = next;
}

function handleWheel(event) {
  event.preventDefault();
  zoomBy(event.deltaY < 0 ? 1.2 : 1 / 1.2, event);
}

function handlePointerDown(event) {
  if (event.button !== 0) return;
  dragging.value = true;
  dragStart = { x: event.clientX - transform.x, y: event.clientY - transform.y };
  event.currentTarget.setPointerCapture?.(event.pointerId);
}

function handlePointerMove(event) {
  if (!dragging.value || !dragStart) return;
  transform.x = event.clientX - dragStart.x;
  transform.y = event.clientY - dragStart.y;
}

function handlePointerUp() {
  dragging.value = false;
  dragStart = null;
}

function toggleActualSize() {
  if (transform.scale === 1 && !transform.x && !transform.y) transform.scale = 2;
  else resetTransform();
}

function rotate() {
  transform.rotate = (transform.rotate + 90) % 360;
}

function markLoaded() {
  if (currentId.value) status[currentId.value] = 'ready';
}

function markFailed() {
  if (currentId.value) status[currentId.value] = 'error';
}

function retryCurrent() {
  const id = currentId.value;
  if (!id) return;
  delete urls[id];
  status[id] = 'loading';
  void ensureUrls();
}

async function downloadCurrent() {
  if (!current.value) return;
  try {
    await transfers.downloadRecords([current.value]);
    message.success('已发起下载');
  } catch (error) {
    message.error(errorText(error));
  }
}
</script>

<template>
  <teleport to="body">
    <div v-if="imageViewer.open" class="image-viewer" role="dialog" aria-label="图片查看器">
      <div class="viewer-backdrop" @click="close" />

      <header class="viewer-head">
        <div class="viewer-title">
          <strong :title="currentName">{{ currentName }}</strong>
          <span>{{ counter }} · {{ currentSize }}</span>
        </div>
        <div class="viewer-actions">
          <button type="button" title="下载" @click="downloadCurrent"><DownloadOutlined /></button>
          <button type="button" title="关闭 (Esc)" @click="close"><CloseOutlined /></button>
        </div>
      </header>

      <div
        class="viewer-stage"
        :class="{ dragging }"
        @wheel="handleWheel"
        @pointerdown="handlePointerDown"
        @pointermove="handlePointerMove"
        @pointerup="handlePointerUp"
        @pointercancel="handlePointerUp"
        @dblclick="toggleActualSize"
      >
        <img
          v-if="currentStatus === 'loading' && currentThumbnail"
          :key="`thumb-${currentId}`"
          class="viewer-placeholder"
          :src="currentThumbnail"
          alt=""
          draggable="false"
          referrerpolicy="no-referrer"
        >
        <img
          v-if="currentUrl && currentStatus !== 'error'"
          :key="currentId"
          class="viewer-image"
          :src="currentUrl"
          :style="imageStyle"
          alt=""
          draggable="false"
          @load="markLoaded"
          @error="markFailed"
        >
        <div v-if="currentStatus === 'loading'" class="viewer-state"><a-spin size="large" /></div>
        <div v-else-if="currentStatus === 'error'" class="viewer-state">
          <p>图片加载失败</p>
          <a-flex gap="small">
            <a-button size="small" @click="retryCurrent">重试</a-button>
            <a-button size="small" @click="downloadCurrent">下载查看</a-button>
          </a-flex>
        </div>
      </div>

      <button v-if="hasPrevious" type="button" class="viewer-nav previous" title="上一张 (←)" @click="showPrevious"><LeftOutlined /></button>
      <button v-if="hasNext" type="button" class="viewer-nav next" title="下一张 (→)" @click="showNext"><RightOutlined /></button>

      <footer class="viewer-toolbar">
        <button type="button" title="缩小" @click="zoomBy(1 / 1.2)"><MinusOutlined /></button>
        <span class="zoom-label">{{ Math.round(transform.scale * 100) }}%</span>
        <button type="button" title="放大" @click="zoomBy(1.2)"><PlusOutlined /></button>
        <button type="button" title="旋转 90°" @click="rotate"><RotateRightOutlined /></button>
        <button type="button" title="重置" @click="resetTransform"><ReloadOutlined /></button>
      </footer>
    </div>
  </teleport>
</template>

<style scoped>
.image-viewer { position: fixed; z-index: 1060; inset: 0; display: flex; flex-direction: column; }
.viewer-backdrop { position: absolute; inset: 0; background: rgb(0 0 0 / 82%); }
.viewer-head { display: flex; position: relative; z-index: 2; align-items: center; justify-content: space-between; gap: 16px; padding: 12px 16px; color: #fff; }
.viewer-title { display: flex; min-width: 0; align-items: baseline; gap: 10px; }
.viewer-title strong { max-width: min(56vw, 560px); overflow: hidden; font-size: 14px; text-overflow: ellipsis; white-space: nowrap; }
.viewer-title span { color: rgb(255 255 255 / 65%); font-size: 12px; white-space: nowrap; }
.viewer-actions { display: flex; gap: 6px; }
.viewer-head button, .viewer-toolbar button, .viewer-nav { display: grid; width: 34px; height: 34px; place-items: center; border: 0; border-radius: 8px; color: #fff; background: rgb(255 255 255 / 12%); font-size: 15px; cursor: pointer; }
.viewer-head button:hover, .viewer-toolbar button:hover, .viewer-nav:hover { background: rgb(255 255 255 / 24%); }
.viewer-stage { position: relative; z-index: 1; display: grid; flex: 1; min-height: 0; place-items: center; overflow: hidden; cursor: grab; touch-action: none; }
.viewer-stage.dragging { cursor: grabbing; }
.viewer-image { max-width: 92%; max-height: 92%; user-select: none; transition: transform .08s ease-out; will-change: transform; }
.viewer-placeholder { position: absolute; max-width: 60%; max-height: 60%; filter: blur(14px); opacity: .55; transform: scale(1.06); user-select: none; pointer-events: none; }
.viewer-stage.dragging .viewer-image { transition: none; }
.viewer-state { display: flex; position: absolute; align-items: center; flex-direction: column; gap: 12px; color: rgb(255 255 255 / 85%); }
.viewer-state p { margin: 0; }
.viewer-nav { position: absolute; z-index: 2; top: 50%; width: 42px; height: 42px; transform: translateY(-50%); font-size: 18px; }
.viewer-nav.previous { left: 18px; }
.viewer-nav.next { right: 18px; }
.viewer-toolbar { display: flex; position: relative; z-index: 2; align-items: center; justify-content: center; gap: 8px; padding: 12px 0 18px; }
.zoom-label { min-width: 48px; color: rgb(255 255 255 / 85%); font-size: 12px; font-variant-numeric: tabular-nums; text-align: center; }
</style>
