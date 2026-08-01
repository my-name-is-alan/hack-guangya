<script setup>
import { ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { ClockCircleOutlined, DeleteOutlined, FolderOpenOutlined } from '@antdv-next/icons';
import RecentFilesPanel from '../components/files/RecentFilesPanel.vue';
import RecycleBinPanel from '../components/files/RecycleBinPanel.vue';
import CloudView from './CloudView.vue';

const route = useRoute();
const router = useRouter();
const validTabs = new Set(['cloud', 'recent', 'recycle']);
const activeTab = ref(validTabs.has(String(route.query.tab)) ? String(route.query.tab) : 'cloud');

watch(() => route.query.tab, (value) => {
  const next = validTabs.has(String(value)) ? String(value) : 'cloud';
  if (activeTab.value !== next) activeTab.value = next;
});

watch(activeTab, (value) => {
  const current = validTabs.has(String(route.query.tab)) ? String(route.query.tab) : 'cloud';
  if (current === value) return;
  const query = { ...route.query };
  if (value === 'cloud') delete query.tab;
  else query.tab = value;
  void router.replace({ query });
});
</script>

<template>
  <div class="view-section files-workspace">
    <a-tabs v-model:active-key="activeTab" class="page-tabs" :animated="false">
      <a-tab-pane key="cloud">
        <template #tab><span class="workspace-tab"><FolderOpenOutlined />文件</span></template>
        <CloudView />
      </a-tab-pane>
      <a-tab-pane key="recent">
        <template #tab><span class="workspace-tab"><ClockCircleOutlined />云端最近</span></template>
        <RecentFilesPanel />
      </a-tab-pane>
      <a-tab-pane key="recycle">
        <template #tab><span class="workspace-tab"><DeleteOutlined />回收站</span></template>
        <RecycleBinPanel />
      </a-tab-pane>
    </a-tabs>
  </div>
</template>

<style scoped>
.files-workspace { min-width: 0; }
.workspace-tab { display: inline-flex; align-items: center; gap: 6px; }
</style>
