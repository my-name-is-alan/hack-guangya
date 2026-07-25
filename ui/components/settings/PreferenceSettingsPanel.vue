<script setup lang="ts">
import { computed } from 'vue'
import {
  FOLDER_OPEN_MODE,
  useFolderOpenPreference,
} from '../../composables/useFolderOpenPreference.js'

type FolderOpenMode = 'single-click' | 'double-click'

const { folderOpenMode, setFolderOpenMode } = useFolderOpenPreference()
const selectedFolderOpenMode = computed({
  get: () => folderOpenMode.value as FolderOpenMode,
  set: (value: FolderOpenMode) => setFolderOpenMode(value),
})

const modeDescription = computed(() => (
  selectedFolderOpenMode.value === FOLDER_OPEN_MODE.SINGLE_CLICK
    ? '单击文件夹名称或空白区域即可进入，适合快速浏览。'
    : '单击只选中文件夹，双击后进入，可减少误操作。'
))
</script>

<template>
  <section class="setting-section">
    <div class="section-lead">
      <strong>偏好设置</strong>
      <span>调整文件列表的交互方式，修改后立即生效。</span>
    </div>

    <div class="setting-row">
      <div class="setting-copy">
        <strong>文件夹打开方式</strong>
        <span>单击打开模式下，可通过行首勾选框选择文件夹。</span>
      </div>
      <a-radio-group
        v-model:value="selectedFolderOpenMode"
        button-style="solid"
        aria-label="文件夹打开方式"
      >
        <a-radio-button :value="FOLDER_OPEN_MODE.SINGLE_CLICK">单击打开</a-radio-button>
        <a-radio-button :value="FOLDER_OPEN_MODE.DOUBLE_CLICK">双击打开</a-radio-button>
      </a-radio-group>
    </div>

    <div class="mode-description" role="status">
      {{ modeDescription }}
    </div>
  </section>
</template>

<style scoped>
.setting-section {
  max-width: 760px;
  padding: 8px 18px 36px 24px;
}

.section-lead {
  margin-bottom: 28px;
}

.section-lead strong,
.section-lead span,
.setting-copy strong,
.setting-copy span {
  display: block;
}

.section-lead strong {
  font-size: 18px;
}

.section-lead span,
.setting-copy span,
.mode-description {
  margin-top: 5px;
  color: var(--text-3, #98a2b3);
  font-size: 12px;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  min-height: 54px;
}

.setting-copy {
  min-width: 0;
}

.mode-description {
  padding-top: 12px;
  border-top: 1px solid var(--line, #e7e8eb);
}

@media (max-width: 720px) {
  .setting-row {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
