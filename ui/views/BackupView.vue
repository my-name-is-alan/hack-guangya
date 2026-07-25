<script setup>
import { computed, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  CloudSyncOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderOpenOutlined,
  PlusOutlined,
  ReloadOutlined,
  SendOutlined,
} from '@antdv-next/icons';
import { bridge, isTauri } from '../bridge.js';
import { appState, refreshState } from '../store.js';
import {
  defaultSyncExtensions,
  errorText,
  extensionPresets,
  formatTime,
  mappingExtensions,
  normalizeExtensions,
  presetExtensions,
  receiptActionLabel,
  receiptAlertType,
  receiptColor,
  receiptDisplayMessage,
  receiptStatusLabel,
  sourcePolicyColor,
  sourcePolicyLabel,
  syncTypeSummary,
} from '../formatters.js';

const backupDrawerOpen = ref(false);
const backupSubmitting = ref(false);
const backupForm = reactive({ local: '', remote: '', policy: 'keep', monitor_mode: 'watch', auto_share: false, sync_types: [...defaultSyncExtensions] });
const syncTypesEditor = reactive({ open: false, saving: false, mapping: null, selected: [] });
const autoShareHistory = reactive({ open: false, mapping: null });
const hdhiveEditor = reactive({ open: false, saving: false, mapping: null });
const hdhiveForm = reactive({ base_url: '', secret: '' });

const autoShareHistoryEvents = computed(() => {
  const mappingId = autoShareHistory.mapping?.id;
  if (!mappingId) return [];
  return appState.auto_share_events.filter((event) => event.mapping_id === mappingId);
});

function openBackupDrawer() {
  backupForm.local = '';
  backupForm.remote = '';
  backupForm.policy = 'keep';
  backupForm.monitor_mode = 'watch';
  backupForm.auto_share = false;
  backupForm.sync_types = [...defaultSyncExtensions];
  backupDrawerOpen.value = true;
}
async function pickBackupFolder(kind) {
  const selected = await bridge.selectFolder();
  if (selected) backupForm[kind] = selected;
}
async function addBackup() {
  if (!backupForm.local || !backupForm.remote) {
    message.warning('请选择本地文件夹和云端目录');
    return;
  }
  backupSubmitting.value = true;
  try {
    await bridge.invoke('add_mapping', {
      local_path: backupForm.local,
      remote_path: backupForm.remote,
      source_policy: backupForm.policy,
      monitor_mode: backupForm.monitor_mode,
      auto_share: backupForm.auto_share,
      sync_types: normalizeExtensions(backupForm.sync_types),
    });
    backupDrawerOpen.value = false;
    await refreshState();
    message.success('备份任务已创建');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    backupSubmitting.value = false;
  }
}

async function toggleMapping(item) {
  try {
    await bridge.invoke('toggle_mapping', { id: item.id, enabled: !item.enabled });
    await refreshState();
  } catch (error) {
    message.error(errorText(error));
  }
}
async function setMonitorMode(item, mode) {
  try {
    await bridge.invoke('update_mapping_monitor_mode', { id: item.id, monitor_mode: mode });
    await refreshState();
    message.success(mode === 'watch' ? '已开启实时监控' : '已切换为手动扫描');
  } catch (error) {
    message.error(errorText(error));
  }
}
async function toggleAutoShare(item) {
  try {
    await bridge.invoke('update_mapping_auto_share', { id: item.id, auto_share: !item.auto_share });
    await refreshState();
    message.success(item.auto_share ? '已关闭自动分享' : '已开启自动分享');
  } catch (error) {
    message.error(errorText(error));
  }
}
async function removeMapping(item) {
  Modal.confirm({
    title: '删除备份任务',
    content: `确定删除「${item.local_path}」的备份任务吗？`,
    okText: '删除',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      try {
        await bridge.invoke('remove_mapping', { id: item.id });
        await refreshState();
        message.success('已删除');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
}

function openSyncTypesEditor(item) {
  syncTypesEditor.mapping = item;
  syncTypesEditor.selected = mappingExtensions(item);
  syncTypesEditor.open = true;
}
function toggleExtension(ext) {
  const index = syncTypesEditor.selected.indexOf(ext);
  if (index >= 0) syncTypesEditor.selected.splice(index, 1);
  else syncTypesEditor.selected.push(ext);
}
function applyPreset(key) {
  syncTypesEditor.selected = presetExtensions(key);
}
async function saveSyncTypes() {
  if (!syncTypesEditor.mapping) return;
  syncTypesEditor.saving = true;
  try {
    await bridge.invoke('update_mapping_sync_types', {
      id: syncTypesEditor.mapping.id,
      sync_types: normalizeExtensions(syncTypesEditor.selected),
    });
    syncTypesEditor.open = false;
    await refreshState();
    message.success('同步格式已更新');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    syncTypesEditor.saving = false;
  }
}

function openAutoShareHistory(item) {
  autoShareHistory.mapping = item;
  autoShareHistory.open = true;
}
function openHdhiveEditor(item) {
  hdhiveEditor.mapping = item;
  hdhiveForm.base_url = item.hdhive_base_url || '';
  hdhiveForm.secret = item.hdhive_secret || '';
  hdhiveEditor.open = true;
}
async function saveHdhiveConfig() {
  if (!hdhiveEditor.mapping) return;
  hdhiveEditor.saving = true;
  try {
    await bridge.invoke('update_hdhive_config', {
      id: hdhiveEditor.mapping.id,
      hdhive_base_url: hdhiveForm.base_url.trim(),
      hdhive_secret: hdhiveForm.secret.trim(),
    });
    hdhiveEditor.open = false;
    await refreshState();
    message.success('Hdhive 配置已保存');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    hdhiveEditor.saving = false;
  }
}
async function backfillAutoShares(item) {
  try {
    await bridge.invoke('backfill_auto_shares', { id: item.id });
    await refreshState();
    message.success('已补发历史文件的自动分享');
  } catch (error) {
    message.error(errorText(error));
  }
}
async function retryAutoShareEvent(event) {
  try {
    await bridge.invoke('retry_auto_share_event', { event_id: event.id, tmdb_id: event.tmdb_id, media_type: event.media_type });
    await refreshState();
    message.success('已重新投递');
  } catch (error) {
    message.error(errorText(error));
  }
}
</script>

<template>
  <div class="view-section">
    <a-alert v-if="!isTauri" class="web-notice" type="warning" show-icon message="本地文件夹备份仅在桌面端可用。" />
    <div class="section-toolbar">
      <div class="section-title">
        <div class="section-icon"><CloudSyncOutlined /></div>
        <div><h2>本地文件夹备份</h2><p>自动将本地文件夹同步到云盘，支持实时监控与手动扫描</p></div>
      </div>
      <a-space>
        <a-button :loading="false" @click="refreshState"><template #icon><ReloadOutlined /></template>刷新</a-button>
        <a-button type="primary" :disabled="!isTauri" @click="openBackupDrawer"><template #icon><PlusOutlined /></template>新建备份</a-button>
      </a-space>
    </div>

    <a-empty v-if="!appState.mappings.length" class="section-empty" description="还没有备份任务">
      <a-button type="primary" :disabled="!isTauri" @click="openBackupDrawer"><template #icon><PlusOutlined /></template>创建第一个备份任务</a-button>
    </a-empty>

    <div v-else class="task-list">
      <a-card v-for="item in appState.mappings" :key="item.id" class="task-card" :bordered="false">
        <a-flex align="center" gap="middle">
          <div class="task-icon"><CloudSyncOutlined /></div>
          <div class="task-body">
            <div class="task-title">
              <strong :title="item.local_path">{{ item.local_path }}</strong>
              <a-tag :color="item.enabled ? 'success' : 'default'">{{ item.enabled ? '运行中' : '已停用' }}</a-tag>
              <a-tag :color="item.monitor_mode === 'watch' ? 'processing' : 'default'">{{ item.monitor_mode === 'watch' ? '实时监控' : '手动扫描' }}</a-tag>
              <a-tag v-if="item.auto_share" color="purple">自动分享</a-tag>
            </div>
            <div class="task-meta">
              <span>云端：{{ item.remote_path }}</span>
              <span>格式：{{ syncTypeSummary(item) }}</span>
              <a-tag :color="sourcePolicyColor(item.source_policy)">{{ sourcePolicyLabel(item.source_policy) }}</a-tag>
            </div>
          </div>
          <a-flex class="task-actions" align="center" gap="small" wrap="wrap">
            <a-switch :checked="item.enabled" size="small" @change="toggleMapping(item)" />
            <a-button size="small" @click="setMonitorMode(item, item.monitor_mode === 'watch' ? 'manual' : 'watch')">{{ item.monitor_mode === 'watch' ? '改手动扫描' : '改实时监控' }}</a-button>
            <a-button size="small" @click="openSyncTypesEditor(item)"><template #icon><EditOutlined /></template>格式</a-button>
            <a-button size="small" @click="toggleAutoShare(item)">{{ item.auto_share ? '关自动分享' : '开自动分享' }}</a-button>
            <a-button v-if="item.auto_share" size="small" @click="openAutoShareHistory(item)">分享记录</a-button>
            <a-button v-if="item.auto_share" size="small" @click="openHdhiveEditor(item)">Hdhive</a-button>
            <a-button v-if="item.auto_share" size="small" @click="backfillAutoShares(item)">补发</a-button>
            <a-button size="small" danger type="text" @click="removeMapping(item)"><template #icon><DeleteOutlined /></template></a-button>
          </a-flex>
        </a-flex>
      </a-card>
    </div>

    <a-drawer v-model:open="backupDrawerOpen" title="新建备份任务" width="420">
      <a-form layout="vertical">
        <a-form-item label="本地文件夹" required>
          <a-input v-model:value="backupForm.local" placeholder="选择要备份的本地文件夹" readonly @click="pickBackupFolder('local')">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
        </a-form-item>
        <a-form-item label="云端目录" required>
          <a-input v-model:value="backupForm.remote" placeholder="例如 /备份/照片" />
        </a-form-item>
        <a-form-item label="监控方式">
          <a-radio-group v-model:value="backupForm.monitor_mode">
            <a-radio-button value="watch">实时监控</a-radio-button>
            <a-radio-button value="manual">手动扫描</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="源文件处理">
          <a-radio-group v-model:value="backupForm.policy">
            <a-radio-button value="keep">保留</a-radio-button>
            <a-radio-button value="archive">归档</a-radio-button>
            <a-radio-button value="delete">删除</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="同步格式">
          <div class="preset-row">
            <a-button v-for="preset in extensionPresets" :key="preset.key" size="small" @click="backupForm.sync_types = presetExtensions(preset.key)">{{ preset.label }}</a-button>
          </div>
          <a-select v-model:value="backupForm.sync_types" mode="tags" :options="backupForm.sync_types.map((ext) => ({ value: ext, label: ext }))" placeholder="输入扩展名后回车" />
        </a-form-item>
        <a-form-item label="自动分享">
          <a-switch v-model:checked="backupForm.auto_share" />
          <small class="form-hint">上传完成后自动创建分享并通知 Hdhive</small>
        </a-form-item>
        <a-button type="primary" block :loading="backupSubmitting" @click="addBackup"><template #icon><PlusOutlined /></template>创建备份任务</a-button>
      </a-form>
    </a-drawer>

    <a-modal v-model:open="syncTypesEditor.open" title="同步格式" :confirm-loading="syncTypesEditor.saving" ok-text="保存" cancel-text="取消" width="520px" @ok="saveSyncTypes">
      <div class="preset-row">
        <a-button v-for="preset in extensionPresets" :key="preset.key" size="small" @click="applyPreset(preset.key)">{{ preset.label }}</a-button>
      </div>
      <div class="ext-grid">
        <template v-for="preset in extensionPresets" :key="preset.key">
          <a-checkable-tag v-for="ext in preset.extensions" :key="ext" :checked="syncTypesEditor.selected.includes(ext)" @change="toggleExtension(ext)">{{ ext }}</a-checkable-tag>
        </template>
      </div>
    </a-modal>

    <a-drawer v-model:open="autoShareHistory.open" :title="`自动分享记录 · ${autoShareHistory.mapping?.local_path || ''}`" width="560">
      <a-empty v-if="!autoShareHistoryEvents.length" description="暂无自动分享记录" />
      <div v-else class="receipt-list">
        <a-card v-for="event in autoShareHistoryEvents" :key="event.id" class="receipt-card" :bordered="false" size="small">
          <a-flex align="center" gap="small">
            <a-tag :color="receiptColor(event.status)">{{ receiptStatusLabel(event) }}</a-tag>
            <strong class="receipt-name" :title="event.file_name">{{ event.file_name }}</strong>
            <span class="receipt-time">{{ formatTime(event.updated_at || event.created_at) }}</span>
          </a-flex>
          <a-alert :type="receiptAlertType(event.status)" :message="receiptDisplayMessage(event)" show-icon style="margin-top:8px" />
          <a-flex gap="small" wrap="wrap" style="margin-top:8px">
            <a-tag v-if="event.action">{{ receiptActionLabel(event.action) }}</a-tag>
            <a v-if="event.share_url" :href="event.share_url" target="_blank">查看分享</a>
            <a-button v-if="['failed', 'delivery_failed'].includes(event.status)" size="small" @click="retryAutoShareEvent(event)"><template #icon><SendOutlined /></template>重试</a-button>
          </a-flex>
        </a-card>
      </div>
    </a-drawer>

    <a-modal v-model:open="hdhiveEditor.open" title="Hdhive 配置" :confirm-loading="hdhiveEditor.saving" ok-text="保存" cancel-text="取消" @ok="saveHdhiveConfig">
      <a-form layout="vertical">
        <a-form-item label="Hdhive 地址"><a-input v-model:value="hdhiveForm.base_url" placeholder="https://hdhive.example.com" /></a-form-item>
        <a-form-item label="密钥"><a-input-password v-model:value="hdhiveForm.secret" placeholder="Hdhive Secret" /></a-form-item>
      </a-form>
    </a-modal>
  </div>
</template>
