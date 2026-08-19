<script setup>
import { computed, nextTick, ref, shallowRef, watch, onBeforeUnmount } from 'vue';
import { message } from 'antdv-next';
import {
  CloseOutlined,
  CustomerServiceOutlined,
  DownloadOutlined,
  PauseCircleFilled,
  PlayCircleFilled,
  SoundOutlined,
  StepBackwardOutlined,
  StepForwardOutlined,
  UpOutlined,
} from '@antdv-next/icons';
import { errorText, fileId, pick } from '../../formatters.js';
import {
  browserCanPlayAudio,
  fileExtensionOf,
  findLyricsSibling,
  formatPlayTime,
  lrcIndexAt,
  parseLrc,
} from '../../fileOpen.js';
import { getPlayUrl, readCloudText, useFileOpener } from '../../composables/useFileOpener.js';
import { useAudioPlayer } from '../../composables/useAudioPlayer.js';
import { useTransfersStore } from '../../stores/transfers.ts';

const VOLUME_STORAGE_KEY = 'guangya.open.audio-volume';
const LYRICS_MAX_BYTES = 256 * 1024;

const audio = useAudioPlayer();
const { openExternalPlayerPicker } = useFileOpener();
const transfers = useTransfersStore();

const audioElement = ref(null);
const lyricsPanel = ref(null);
const playing = shallowRef(false);
const currentTime = shallowRef(0);
const duration = shallowRef(0);
const seekPreview = shallowRef(null);
const trackError = shallowRef('');
const lyrics = shallowRef([]);
const volume = shallowRef(readVolume());

const currentTrack = computed(() => audio.state.queue[audio.state.index] || null);
const trackName = computed(() => String(pick(currentTrack.value || {}, ['fileName', 'name'], '')));
const queueLabel = computed(() => (audio.state.queue.length > 1 ? `${audio.state.index + 1} / ${audio.state.queue.length}` : ''));
const hasPrevious = computed(() => audio.state.index > 0);
const hasNext = computed(() => audio.state.index < audio.state.queue.length - 1);
const sliderTime = computed(() => (seekPreview.value ?? currentTime.value));
const activeLyricIndex = computed(() => lrcIndexAt(lyrics.value, currentTime.value));
const activeLyricText = computed(() => {
  const line = lyrics.value[activeLyricIndex.value];
  return line?.text || '';
});

function readVolume() {
  try {
    const raw = Number(window.localStorage?.getItem(VOLUME_STORAGE_KEY));
    return Number.isFinite(raw) && raw >= 0 && raw <= 1 ? raw : 0.8;
  } catch {
    return 0.8;
  }
}

function persistVolume(value) {
  try {
    window.localStorage?.setItem(VOLUME_STORAGE_KEY, String(value));
  } catch {
    // 忽略：仅影响下次启动的默认音量。
  }
}

watch(() => audio.state.requestId, () => void loadTrack());

async function loadTrack() {
  const track = currentTrack.value;
  const requestId = audio.state.requestId;
  const element = audioElement.value;
  trackError.value = '';
  lyrics.value = [];
  currentTime.value = 0;
  duration.value = 0;
  seekPreview.value = null;
  if (!audio.state.visible || !track) {
    if (element) {
      element.pause();
      element.removeAttribute('src');
      element.load();
    }
    playing.value = false;
    return;
  }
  if (!browserCanPlayAudio(track)) {
    trackError.value = `浏览器无法直接解码 ${fileExtensionOf(track).toUpperCase() || '该'} 格式，请用外部播放器打开或下载后播放`;
    element?.pause();
    playing.value = false;
    return;
  }
  try {
    const url = await getPlayUrl(track);
    if (requestId !== audio.state.requestId) return;
    await nextTick();
    const target = audioElement.value;
    if (!target) return;
    target.src = url;
    target.volume = volume.value;
    await target.play();
  } catch (error) {
    if (requestId !== audio.state.requestId) return;
    // 自动播放被拦截时不算错误，用户点播放即可。
    if (error?.name !== 'NotAllowedError') trackError.value = errorText(error);
  }
  void loadLyrics(track, requestId);
}

async function loadLyrics(track, requestId) {
  const sibling = findLyricsSibling(track, audio.state.siblings);
  if (!sibling) return;
  try {
    const { text } = await readCloudText(sibling, LYRICS_MAX_BYTES);
    if (requestId !== audio.state.requestId) return;
    const parsed = parseLrc(text);
    if (parsed.length) lyrics.value = parsed;
  } catch {
    // 歌词加载失败不影响播放。
  }
}

function togglePlay() {
  const element = audioElement.value;
  if (!element || trackError.value) return;
  if (element.paused) void element.play().catch((error) => { trackError.value = errorText(error); });
  else element.pause();
}

function handleTimeUpdate() {
  if (seekPreview.value === null) currentTime.value = audioElement.value?.currentTime || 0;
}

function handleLoadedMetadata() {
  duration.value = audioElement.value?.duration || 0;
}

function handleEnded() {
  if (hasNext.value) audio.playNext();
  else playing.value = false;
}

function handleElementError() {
  if (!audioElement.value?.currentSrc) return;
  trackError.value = '音频加载失败，可能是格式不受支持或网络异常';
  playing.value = false;
}

function previewSeek(value) {
  seekPreview.value = Number(value) || 0;
}

function commitSeek(value) {
  const element = audioElement.value;
  const target = Number(value) || 0;
  seekPreview.value = null;
  if (!element || !Number.isFinite(element.duration)) return;
  element.currentTime = Math.min(Math.max(0, target), element.duration);
  currentTime.value = element.currentTime;
}

function setVolume(value) {
  const normalized = Math.min(1, Math.max(0, Number(value) / 100));
  volume.value = normalized;
  if (audioElement.value) audioElement.value.volume = normalized;
  persistVolume(normalized);
}

function toggleExpanded() {
  audio.state.expanded = !audio.state.expanded;
}

function openExternally() {
  if (currentTrack.value) openExternalPlayerPicker(currentTrack.value);
}

async function downloadTrack() {
  if (!currentTrack.value) return;
  try {
    await transfers.downloadRecords([currentTrack.value]);
    message.success('已发起下载');
  } catch (error) {
    message.error(errorText(error));
  }
}

function close() {
  const element = audioElement.value;
  if (element) {
    element.pause();
    element.removeAttribute('src');
    element.load();
  }
  playing.value = false;
  audio.close();
}

watch(activeLyricIndex, async (index) => {
  if (!audio.state.expanded || index < 0) return;
  await nextTick();
  lyricsPanel.value
    ?.querySelector(`[data-lyric-index="${index}"]`)
    ?.scrollIntoView({ block: 'center', behavior: 'smooth' });
});

watch(() => audio.state.expanded, async (expanded) => {
  if (!expanded) return;
  await nextTick();
  const index = Math.max(0, activeLyricIndex.value);
  lyricsPanel.value
    ?.querySelector(`[data-lyric-index="${index}"]`)
    ?.scrollIntoView({ block: 'center' });
});

onBeforeUnmount(() => audioElement.value?.pause());
</script>

<template>
  <teleport to="body">
    <div v-if="audio.state.visible" class="audio-player" role="region" aria-label="音频播放器">
      <div v-if="audio.state.expanded && lyrics.length" ref="lyricsPanel" class="lyrics-panel">
        <p
          v-for="(line, index) in lyrics"
          :key="`${index}-${line.time}`"
          :data-lyric-index="index"
          class="lyric-line"
          :class="{ active: index === activeLyricIndex }"
          @click="commitSeek(line.time)"
        >{{ line.text || '♪' }}</p>
      </div>

      <div class="player-bar">
        <audio
          ref="audioElement"
          preload="metadata"
          @play="playing = true"
          @pause="playing = false"
          @timeupdate="handleTimeUpdate"
          @loadedmetadata="handleLoadedMetadata"
          @ended="handleEnded"
          @error="handleElementError"
        />

        <div class="track-meta">
          <div class="track-cover"><CustomerServiceOutlined /></div>
          <div class="track-copy">
            <strong :title="trackName">{{ trackName }}</strong>
            <span v-if="trackError" class="track-error">{{ trackError }}</span>
            <span v-else class="track-lyric" :title="activeLyricText">{{ activeLyricText || (lyrics.length ? '' : '暂无歌词') }}</span>
          </div>
        </div>

        <div class="player-center">
          <div class="player-controls">
            <button type="button" title="上一首" :disabled="!hasPrevious" @click="audio.playPrevious()"><StepBackwardOutlined /></button>
            <button type="button" class="play-toggle" :title="playing ? '暂停' : '播放'" :disabled="Boolean(trackError)" @click="togglePlay">
              <PauseCircleFilled v-if="playing" />
              <PlayCircleFilled v-else />
            </button>
            <button type="button" title="下一首" :disabled="!hasNext" @click="audio.playNext()"><StepForwardOutlined /></button>
          </div>
          <div class="player-progress">
            <span class="time-label">{{ formatPlayTime(sliderTime) }}</span>
            <a-slider
              class="progress-slider"
              :value="sliderTime"
              :max="Math.max(1, duration)"
              :step="1"
              :disabled="!duration || Boolean(trackError)"
              :tooltip="{ formatter: formatPlayTime }"
              @change="previewSeek"
              @change-complete="commitSeek"
            />
            <span class="time-label">{{ formatPlayTime(duration) }}</span>
          </div>
        </div>

        <div class="player-side">
          <template v-if="trackError">
            <a-button size="small" @click="openExternally">外部播放器</a-button>
            <a-button size="small" @click="downloadTrack"><template #icon><DownloadOutlined /></template>下载</a-button>
          </template>
          <template v-else>
            <span v-if="queueLabel" class="queue-label">{{ queueLabel }}</span>
            <div class="volume-control">
              <SoundOutlined />
              <a-slider class="volume-slider" :value="Math.round(volume * 100)" :max="100" :step="1" @change="setVolume" />
            </div>
            <button
              v-if="lyrics.length"
              type="button"
              class="bar-icon-button"
              :class="{ active: audio.state.expanded }"
              title="歌词"
              @click="toggleExpanded"
            ><UpOutlined :rotate="audio.state.expanded ? 180 : 0" /></button>
          </template>
          <button type="button" class="bar-icon-button" title="关闭播放器" @click="close"><CloseOutlined /></button>
        </div>
      </div>
    </div>
  </teleport>
</template>

<style scoped>
.audio-player { position: fixed; z-index: 980; right: 16px; bottom: 16px; left: 90px; display: flex; flex-direction: column; gap: 8px; pointer-events: none; }
.audio-player > * { pointer-events: auto; }
.lyrics-panel { align-self: center; width: min(560px, 100%); max-height: 42vh; overflow: auto; padding: 18px 22px; border: 1px solid var(--line, #e5e5e5); border-radius: 14px; background: color-mix(in srgb, var(--surface, #fff) 96%, transparent); box-shadow: 0 12px 32px rgb(0 0 0 / 14%); backdrop-filter: blur(10px); }
.lyric-line { margin: 0; padding: 5px 0; color: var(--text-3, #737373); font-size: 13px; line-height: 1.6; text-align: center; cursor: pointer; transition: color .2s ease, font-size .2s ease; }
.lyric-line:hover { color: var(--text-2, #525252); }
.lyric-line.active { color: var(--primary-strong, #171717); font-size: 15px; font-weight: 600; }
.player-bar { display: grid; grid-template-columns: minmax(160px, 26%) minmax(0, 1fr) auto; align-items: center; gap: 16px; padding: 10px 16px; border: 1px solid var(--line, #e5e5e5); border-radius: 14px; background: color-mix(in srgb, var(--surface, #fff) 97%, transparent); box-shadow: 0 10px 28px rgb(0 0 0 / 14%); backdrop-filter: blur(10px); }
.track-meta { display: flex; min-width: 0; align-items: center; gap: 10px; }
.track-cover { display: grid; width: 40px; height: 40px; flex: 0 0 40px; place-items: center; border-radius: 10px; color: var(--text-2, #525252); background: var(--surface-hover, #f5f5f5); font-size: 18px; }
.track-copy { display: grid; min-width: 0; gap: 2px; }
.track-copy strong { overflow: hidden; font-size: 13px; text-overflow: ellipsis; white-space: nowrap; }
.track-lyric, .track-error { overflow: hidden; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.track-lyric { color: var(--text-3, #737373); }
.track-error { color: #d4380d; }
.player-center { display: grid; min-width: 0; gap: 2px; }
.player-controls { display: flex; align-items: center; justify-content: center; gap: 14px; }
.player-controls button { display: grid; place-items: center; border: 0; color: var(--text-2, #525252); background: transparent; font-size: 18px; cursor: pointer; }
.player-controls button:hover:not(:disabled) { color: var(--primary-strong, #171717); }
.player-controls button:disabled { opacity: .35; cursor: not-allowed; }
.play-toggle { font-size: 30px !important; color: var(--primary-strong, #171717) !important; }
.play-toggle:disabled { color: var(--text-3, #737373) !important; }
.player-progress { display: flex; align-items: center; gap: 10px; }
.progress-slider { flex: 1; margin: 0; }
.time-label { min-width: 36px; color: var(--text-3, #737373); font-size: 11px; font-variant-numeric: tabular-nums; text-align: center; }
.player-side { display: flex; align-items: center; gap: 10px; }
.queue-label { color: var(--text-3, #737373); font-size: 11px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.volume-control { display: flex; align-items: center; gap: 6px; color: var(--text-2, #525252); }
.volume-slider { width: 72px; margin: 0; }
.bar-icon-button { display: grid; width: 28px; height: 28px; place-items: center; border: 0; border-radius: 8px; color: var(--text-2, #525252); background: transparent; font-size: 14px; cursor: pointer; }
.bar-icon-button:hover, .bar-icon-button.active { color: var(--primary-strong, #171717); background: var(--surface-hover, #f5f5f5); }
@media (max-width: 900px) {
  .audio-player { left: 80px; }
  .player-bar { grid-template-columns: minmax(120px, 1fr) auto; }
  .player-center { grid-column: 1 / -1; grid-row: 2; }
  .volume-control { display: none; }
}
</style>
