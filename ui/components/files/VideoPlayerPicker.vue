<script setup>
import { computed, ref, shallowRef, watch } from 'vue';
import { message } from 'antdv-next';
import { CopyOutlined, PlayCircleOutlined } from '@antdv-next/icons';
import { bridge, isTauri } from '../../bridge.js';
import { copyText, errorText, pick } from '../../formatters.js';
import { externalPlayerOptions } from '../../fileOpen.js';
import {
  getAbsolutePlayUrl,
  launchVideo,
  rememberPlayer,
  useFileOpener,
} from '../../composables/useFileOpener.js';

const CUSTOM_PLAYER_STORAGE_KEY = 'guangya.open.custom-player-path';

const { videoPicker } = useFileOpener();
const localPlayers = shallowRef([]);
const detecting = shallowRef(false);
const launching = shallowRef('');
const rememberChoice = ref(false);
const customPath = ref(readCustomPath());
const schemePlayers = externalPlayerOptions();

const fileName = computed(() => String(pick(videoPicker.record || {}, ['fileName', 'name'], '')));

function readCustomPath() {
  try {
    return window.localStorage?.getItem(CUSTOM_PLAYER_STORAGE_KEY) || '';
  } catch {
    return '';
  }
}

function saveCustomPath(value) {
  try {
    if (value) window.localStorage?.setItem(CUSTOM_PLAYER_STORAGE_KEY, value);
    else window.localStorage?.removeItem(CUSTOM_PLAYER_STORAGE_KEY);
  } catch {
    // localStorage 不可用时仅保留会话内输入。
  }
}

watch(() => videoPicker.open, (open) => {
  if (!open) return;
  rememberChoice.value = false;
  customPath.value = readCustomPath();
  if (isTauri) void detectLocalPlayers();
});

async function detectLocalPlayers() {
  detecting.value = true;
  try {
    const data = await bridge.invoke('list_local_players');
    const list = Array.isArray(data?.players) ? data.players : Array.isArray(data) ? data : [];
    localPlayers.value = list
      .map((item) => ({ id: String(item.id || item.path || ''), name: String(item.name || ''), path: String(item.path || '') }))
      .filter((item) => item.path);
  } catch (error) {
    localPlayers.value = [];
    message.warning(`检测本地播放器失败：${errorText(error)}`);
  } finally {
    detecting.value = false;
  }
}

async function playWith(player, key) {
  if (!videoPicker.record || launching.value) return;
  launching.value = key;
  try {
    await launchVideo(videoPicker.record, player);
    if (rememberChoice.value) rememberPlayer(player);
    if (player.type === 'local' && player.custom) saveCustomPath(player.path);
    message.success(`已调用 ${player.name || '播放器'} 播放`);
    videoPicker.open = false;
  } catch (error) {
    message.error(errorText(error));
  } finally {
    launching.value = '';
  }
}

function playWithLocal(item) {
  void playWith({ type: 'local', name: item.name, path: item.path }, `local:${item.path}`);
}

function playWithCustom() {
  const path = customPath.value.trim();
  if (!path) {
    message.warning('请输入播放器可执行文件的完整路径');
    return;
  }
  void playWith({ type: 'local', name: '自定义播放器', path, custom: true }, 'custom');
}

function playWithScheme(option) {
  void playWith({ type: 'scheme', id: option.id, name: option.name }, `scheme:${option.id}`);
}

async function copyPlayLink() {
  if (!videoPicker.record) return;
  try {
    await copyText(await getAbsolutePlayUrl(videoPicker.record), message);
  } catch (error) {
    message.error(errorText(error));
  }
}

function handleClose() {
  videoPicker.open = false;
  videoPicker.record = null;
}
</script>

<template>
  <a-modal
    :open="videoPicker.open"
    title="用播放器打开"
    :width="460"
    :footer="null"
    @cancel="handleClose"
  >
    <div class="player-picker">
      <div class="picker-file" :title="fileName">{{ fileName }}</div>

      <template v-if="isTauri">
        <div class="picker-section-title">本机播放器</div>
        <a-spin :spinning="detecting">
          <div v-if="localPlayers.length" class="picker-options">
            <button
              v-for="item in localPlayers"
              :key="item.path"
              type="button"
              class="picker-option"
              :disabled="Boolean(launching)"
              @click="playWithLocal(item)"
            >
              <PlayCircleOutlined />
              <span class="option-name">{{ item.name }}</span>
              <span class="option-hint" :title="item.path">{{ item.path }}</span>
            </button>
          </div>
          <div v-else-if="!detecting" class="picker-empty">未检测到常见播放器（PotPlayer / VLC / MPC-HC / mpv），可在下方填写路径</div>
        </a-spin>
        <div class="picker-custom">
          <a-input
            v-model:value="customPath"
            placeholder="自定义播放器路径，例如 D:\Tools\mpv\mpv.exe"
            @press-enter="playWithCustom"
          />
          <a-button :loading="launching === 'custom'" @click="playWithCustom">播放</a-button>
        </div>
      </template>

      <template v-else>
        <div class="picker-section-title">通过播放器协议唤起</div>
        <div class="picker-options">
          <button
            v-for="option in schemePlayers"
            :key="option.id"
            type="button"
            class="picker-option"
            :disabled="Boolean(launching)"
            @click="playWithScheme(option)"
          >
            <PlayCircleOutlined />
            <span class="option-name">{{ option.name }}</span>
            <span class="option-hint">{{ option.hint }}</span>
          </button>
        </div>
        <div class="picker-empty">需要本机已安装对应播放器并注册协议；没反应时可复制链接后在播放器里打开网络串流。</div>
      </template>

      <div class="picker-footer">
        <a-checkbox v-model:checked="rememberChoice">记住选择，下次直接播放</a-checkbox>
        <a-button @click="copyPlayLink"><template #icon><CopyOutlined /></template>复制播放链接</a-button>
      </div>
    </div>
  </a-modal>
</template>

<style scoped>
.player-picker { display: flex; flex-direction: column; gap: 12px; }
.picker-file { overflow: hidden; color: var(--text-2, #525252); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.picker-section-title { color: var(--text-3, #737373); font-size: 12px; font-weight: 600; }
.picker-options { display: flex; flex-direction: column; gap: 8px; }
.picker-option { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface, #fff); text-align: left; cursor: pointer; }
.picker-option:hover { border-color: var(--primary, #262626); background: var(--surface-hover, #f5f5f5); }
.picker-option:disabled { opacity: .6; cursor: not-allowed; }
.picker-option :deep(.anticon) { font-size: 18px; color: var(--text-2, #525252); }
.option-name { flex: 0 0 auto; font-weight: 600; }
.option-hint { flex: 1; overflow: hidden; color: var(--text-3, #737373); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.picker-custom { display: flex; gap: 8px; }
.picker-empty { color: var(--text-3, #737373); font-size: 12px; }
.picker-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-top: 4px; border-top: 1px solid var(--line-soft, #f5f5f5); }
</style>
