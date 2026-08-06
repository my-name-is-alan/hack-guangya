<script setup>
import { computed, reactive, ref, watchEffect } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  ArrowLeftOutlined,
  CloudSyncOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderOutlined,
  FolderOpenOutlined,
  PlusOutlined,
  ReloadOutlined,
  SendOutlined,
} from '@antdv-next/icons';
import { bridge, isTauri } from '../bridge.js';
import { needsTmdbReview } from '../receiptReview.js';
import { appState, refreshState } from '../store.js';
import {
  defaultSyncExtensions,
  errorText,
  extensionPresets,
  fileId,
  formatTime,
  isFolder,
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
  unwrapData,
} from '../formatters.js';

const backupDrawerOpen = ref(false);
const backupTab = ref('tasks');
const backupSubmitting = ref(false);
const backupForm = reactive({
  local: '',
  remote: '',
  remoteParentId: '',
  remoteLabel: '',
  remoteChosen: false,
  archive: '',
  policy: 'keep',
  monitor_mode: isTauri ? 'native' : 'polling',
  auto_share: false,
  organizer_mapping_id: '',
  sync_types: [...defaultSyncExtensions],
});
const organizerMappings = ref([]);
const autoOrganizeEditor = reactive({ open: false, saving: false, mapping: null, selected: '' });
const cloudFolderPicker = reactive({
  open: false,
  loading: false,
  items: [],
  path: [{ id: '', name: '全部文件' }],
  page: 0,
  total: 0,
});
const cloudFolderColumns = [
  { title: '文件夹', key: 'name', ellipsis: true },
  { title: '操作', key: 'actions', width: 84 },
];
const syncTypesEditor = reactive({ open: false, saving: false, mapping: null, selected: [] });
const autoShareHistory = reactive({ open: false, mapping: null });
const receiptReview = reactive({});
const autoShareBusy = reactive({});
const receiptMediaOptions = [
  { label: '电视剧', value: 'tv' },
  { label: '电影', value: 'movie' },
];
const matchingOrganizerOptions = computed(() => organizerMappings.value
  .filter((item) => item.enabled && item.source_dir_id === backupForm.remoteParentId)
  .map((item) => ({ label: `${item.source_path} → ${item.target_path}`, value: item.id })));
const editorOrganizerOptions = computed(() => organizerMappings.value
  .filter((item) => item.enabled && item.source_dir_id === String(autoOrganizeEditor.mapping?.remote_parent_id || ''))
  .map((item) => ({ label: `${item.source_path} → ${item.target_path}`, value: item.id })));

function organizerMappingLabel(id) {
  const mapping = organizerMappings.value.find((item) => item.id === id);
  return mapping ? `${mapping.source_path} → ${mapping.target_path}` : '已关联整理任务';
}

async function loadOrganizerMappings() {
  try {
    const data = await bridge.invoke('get_organizer_state');
    organizerMappings.value = Array.isArray(data?.mappings) ? data.mappings : [];
  } catch {
    organizerMappings.value = [];
  }
}

const allAutoShareEvents = computed(() => Array.isArray(appState.auto_share_events)
  ? appState.auto_share_events
  : []);

function receiptEventId(event) {
  return String(event?.event_id || '');
}

function receiptTargetLabel(event) {
  return String(event?.target_key || '未命名分享');
}

function ensureReceiptReview(event) {
  const eventId = receiptEventId(event);
  if (eventId && !receiptReview[eventId]) {
    receiptReview[eventId] = { tmdb_id: '', media_type: 'tv' };
  }
}

watchEffect(() => {
  allAutoShareEvents.value.forEach(ensureReceiptReview);
});

const autoShareHistoryEvents = computed(() => {
  const mappingId = autoShareHistory.mapping?.id;
  if (!mappingId) return [];
  return allAutoShareEvents.value.filter((event) => event.mapping_id === mappingId);
});
const recentActivity = computed(() => [
  ...allAutoShareEvents.value.map((event) => ({
    kind: 'event',
    id: `event:${event.event_id}`,
    timestamp: activityTimestamp(event.updated_at || event.created_at),
    value: event,
  })),
  ...appState.logs.map((log) => ({
    kind: 'log',
    id: `log:${log.id}`,
    timestamp: activityTimestamp(log.timestamp || log.created_at || String(log.id || '').split('-')[0] || log.time),
    value: log,
  })),
].sort((left, right) => right.timestamp - left.timestamp).slice(0, 80));

function activityTimestamp(value) {
  const numeric = Number(value);
  if (Number.isFinite(numeric) && numeric > 0) return numeric < 1e12 ? numeric * 1000 : numeric;
  const parsed = Date.parse(String(value || ''));
  return Number.isFinite(parsed) ? parsed : 0;
}

async function openBackupDrawer() {
  backupForm.local = '';
  backupForm.remote = '';
  backupForm.remoteParentId = '';
  backupForm.remoteLabel = '';
  backupForm.remoteChosen = false;
  backupForm.archive = '';
  backupForm.policy = 'keep';
  backupForm.monitor_mode = isTauri ? 'native' : 'polling';
  backupForm.auto_share = false;
  backupForm.organizer_mapping_id = '';
  backupForm.sync_types = [...defaultSyncExtensions];
  await loadOrganizerMappings();
  backupDrawerOpen.value = true;
}
async function pickBackupFolder(kind) {
  const selected = await bridge.selectFolder();
  if (selected) backupForm[kind] = selected;
}
async function loadCloudFolders(page = 0) {
  cloudFolderPicker.loading = true;
  try {
    const parentId = cloudFolderPicker.path.at(-1)?.id || '';
    const data = unwrapData(await bridge.invoke('list_files', { parent_id: parentId, page }));
    cloudFolderPicker.items = (data.list || []).filter(isFolder);
    cloudFolderPicker.page = page;
    cloudFolderPicker.total = Math.max(Number(data.total || 0), cloudFolderPicker.items.length);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    cloudFolderPicker.loading = false;
  }
}
async function openCloudFolderPicker() {
  cloudFolderPicker.path = [{ id: '', name: '全部文件' }];
  cloudFolderPicker.open = true;
  await loadCloudFolders(0);
}
async function enterCloudFolder(record) {
  cloudFolderPicker.path.push({ id: String(fileId(record)), name: String(record.fileName || record.name || '未命名文件夹') });
  await loadCloudFolders(0);
}
async function leaveCloudFolder() {
  if (cloudFolderPicker.path.length <= 1) return;
  cloudFolderPicker.path.pop();
  await loadCloudFolders(0);
}
async function jumpToCloudFolder(index) {
  if (index < 0 || index >= cloudFolderPicker.path.length - 1) return;
  cloudFolderPicker.path = cloudFolderPicker.path.slice(0, index + 1);
  await loadCloudFolders(0);
}
function chooseCloudFolder() {
  const current = cloudFolderPicker.path.at(-1);
  const names = cloudFolderPicker.path.slice(1).map((item) => item.name);
  backupForm.remoteParentId = current?.id || '';
  backupForm.remote = names.length ? `/${names.join('/')}` : '';
  backupForm.remoteLabel = names.length ? `全部文件 / ${names.join(' / ')}` : '全部文件';
  backupForm.remoteChosen = true;
  if (!organizerMappings.value.some((item) => item.id === backupForm.organizer_mapping_id && item.source_dir_id === backupForm.remoteParentId)) backupForm.organizer_mapping_id = '';
  cloudFolderPicker.open = false;
}
function handleCloudFolderTableChange(pagination) {
  const page = Math.max(0, Number(pagination?.current || 1) - 1);
  if (page !== cloudFolderPicker.page) void loadCloudFolders(page);
}
async function addBackup() {
  if (!backupForm.local || !backupForm.remoteChosen) {
    message.warning('请选择本地文件夹和云端目录');
    return;
  }
  if (backupForm.policy === 'archive' && !backupForm.archive) {
    message.warning('请选择归档目录');
    return;
  }
  backupSubmitting.value = true;
  try {
    await bridge.invoke('add_mapping', {
      local_path: backupForm.local,
      remote_path: backupForm.remote,
      remote_parent_id: backupForm.remoteParentId,
      source_policy: backupForm.policy,
      archive_path: backupForm.policy === 'archive' ? backupForm.archive : undefined,
      scan_existing: true,
      monitor_mode: backupForm.monitor_mode,
      auto_share: backupForm.auto_share,
      organizer_mapping_id: backupForm.organizer_mapping_id,
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
    message.success(mode === 'native' ? '已切换为系统监听' : '已切换为轮询扫描');
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
  const eventId = receiptEventId(event);
  if (!eventId) {
    message.error('当前回执缺少事件 ID，请刷新后重试');
    return;
  }
  if (autoShareBusy[eventId]) return;
  ensureReceiptReview(event);
  const review = receiptReview[eventId];
  const tmdbId = String(review.tmdb_id || '').trim();
  if (needsTmdbReview(event) && !tmdbId) {
    message.warning('请输入 TMDB ID 后再重试');
    return;
  }
  autoShareBusy[eventId] = true;
  try {
    await bridge.invoke('retry_auto_share_event', {
      event_id: eventId,
      tmdb_id: tmdbId || null,
      media_type: tmdbId ? review.media_type : null,
    });
    await refreshState();
    message.success('已重新投递');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    autoShareBusy[eventId] = false;
  }
}

async function openAutoOrganizeEditor(item) {
  await loadOrganizerMappings();
  autoOrganizeEditor.mapping = item;
  autoOrganizeEditor.selected = String(item.organizer_mapping_id || '');
  autoOrganizeEditor.open = true;
}

async function saveAutoOrganizeEditor() {
  const item = autoOrganizeEditor.mapping;
  if (!item) return;
  autoOrganizeEditor.saving = true;
  try {
    await bridge.invoke('update_mapping_organizer', { id: item.id, organizer_mapping_id: autoOrganizeEditor.selected });
    autoOrganizeEditor.open = false;
    await refreshState();
    message.success(autoOrganizeEditor.selected ? '已启用“上传 → 整理 B → 分享”流程' : '已恢复原上传后分享流程');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    autoOrganizeEditor.saving = false;
  }
}
</script>

<template>
  <div class="view-section">
    <a-tabs v-model:active-key="backupTab" class="page-tabs">
      <template #rightExtra>
        <a-space>
          <a-button @click="refreshState"><template #icon><ReloadOutlined /></template>刷新</a-button>
          <a-button type="primary" @click="openBackupDrawer"><template #icon><PlusOutlined /></template>新建备份</a-button>
        </a-space>
      </template>

      <a-tab-pane key="tasks" tab="备份任务">
        <a-empty v-if="!appState.mappings.length" class="section-empty" description="还没有备份任务">
          <a-button type="primary" @click="openBackupDrawer"><template #icon><PlusOutlined /></template>创建第一个备份任务</a-button>
        </a-empty>

        <div v-else class="task-list">
          <a-card v-for="item in appState.mappings" :key="item.id" class="task-card" :bordered="false">
        <a-flex align="center" gap="middle">
          <div class="task-icon"><CloudSyncOutlined /></div>
          <div class="task-body">
            <div class="task-title">
              <strong :title="item.local_path">{{ item.local_path }}</strong>
              <a-tag :color="item.enabled ? 'success' : 'default'">{{ item.enabled ? '运行中' : '已停用' }}</a-tag>
              <a-tag :color="item.monitor_mode === 'native' ? 'processing' : 'default'">{{ item.monitor_mode === 'native' ? '系统监听' : '轮询扫描' }}</a-tag>
              <a-tag v-if="item.auto_share" color="purple">自动分享</a-tag>
              <a-tag v-if="item.organizer_mapping_id" color="blue" :title="organizerMappingLabel(item.organizer_mapping_id)">上传后先整理</a-tag>
            </div>
            <div class="task-meta">
              <span>云端：{{ item.remote_path || '全部文件' }}</span>
              <span>格式：{{ syncTypeSummary(item) }}</span>
              <a-tag :color="sourcePolicyColor(item.source_policy)">{{ sourcePolicyLabel(item.source_policy) }}</a-tag>
            </div>
          </div>
          <a-flex class="task-actions" align="center" gap="small" wrap="wrap">
            <a-switch :checked="item.enabled" size="small" :aria-label="item.enabled ? '停用备份任务' : '启用备份任务'" @change="toggleMapping(item)" />
            <a-button size="small" @click="setMonitorMode(item, item.monitor_mode === 'native' ? 'polling' : 'native')">{{ item.monitor_mode === 'native' ? '改轮询扫描' : '改系统监听' }}</a-button>
            <a-button size="small" @click="openSyncTypesEditor(item)"><template #icon><EditOutlined /></template>格式</a-button>
            <a-button size="small" @click="toggleAutoShare(item)">{{ item.auto_share ? '关自动分享' : '开自动分享' }}</a-button>
            <a-button size="small" @click="openAutoOrganizeEditor(item)">{{ item.organizer_mapping_id ? '改整理流程' : '上传后整理' }}</a-button>
            <a-button v-if="item.auto_share" size="small" @click="openAutoShareHistory(item)">分享记录</a-button>
            <a-button v-if="item.auto_share && !item.organizer_mapping_id" size="small" @click="backfillAutoShares(item)">补发</a-button>
            <a-button size="small" danger type="text" aria-label="删除备份任务" @click="removeMapping(item)"><template #icon><DeleteOutlined /></template></a-button>
          </a-flex>
        </a-flex>
          </a-card>
        </div>
      </a-tab-pane>

      <a-tab-pane key="activity" tab="最近活动">
        <a-empty v-if="!recentActivity.length" class="section-empty" description="暂无最近活动" />
        <div v-else class="activity-list">
          <div v-for="item in recentActivity" :key="item.id" class="activity-row">
            <template v-if="item.kind === 'event'">
              <a-tag :color="receiptColor(item.value.status)">{{ receiptStatusLabel(item.value) }}</a-tag>
              <strong>{{ item.value.file_name || item.value.target_key || '自动分享任务' }}</strong>
              <span>{{ receiptDisplayMessage(item.value) }}</span>
              <time>{{ formatTime(item.value.updated_at || item.value.created_at) }}</time>
            </template>
            <template v-else>
              <a-tag :color="item.value.level === 'error' ? 'error' : item.value.level === 'success' ? 'success' : 'default'">{{ item.value.level }}</a-tag>
              <strong>{{ item.value.text }}</strong>
              <span />
              <time>{{ item.value.time }}</time>
            </template>
          </div>
        </div>
      </a-tab-pane>

      <a-tab-pane key="receipts" :tab="`分享回执${allAutoShareEvents.length ? ` (${allAutoShareEvents.length})` : ''}`">
        <a-empty v-if="!allAutoShareEvents.length" class="section-empty" description="暂无分享与 HDHive 回执" />
        <div v-else class="receipt-list global-receipt-list">
          <a-card v-for="event in allAutoShareEvents" :key="event.event_id" class="receipt-card" :bordered="false" size="small">
            <a-flex class="receipt-heading" align="center" gap="small" wrap="wrap">
              <a-tag :color="receiptColor(event.status)">{{ receiptStatusLabel(event) }}</a-tag>
              <a-tag>{{ event.mapping_id === '__manual__' ? '手动分享' : '备份自动分享' }}</a-tag>
              <strong class="receipt-name" :title="receiptTargetLabel(event)">{{ receiptTargetLabel(event) }}</strong>
              <span class="receipt-time">{{ formatTime(event.updated_at || event.created_at) }}</span>
            </a-flex>
            <a-alert :type="receiptAlertType(event.status)" :message="receiptDisplayMessage(event)" show-icon class="receipt-alert" />
            <a-flex class="receipt-actions" gap="small" align="center" wrap="wrap">
              <a-tag v-if="event.action">{{ receiptActionLabel(event.action) }}</a-tag>
              <a-tag v-if="event.notification_status">通知：{{ event.notification_status }}</a-tag>
              <a v-if="event.share_url" :href="event.share_url" target="_blank" rel="noopener noreferrer">查看分享</a>
              <a v-if="event.resource_url" :href="event.resource_url" target="_blank" rel="noopener noreferrer">查看 HDHive 资源</a>
              <template v-if="needsTmdbReview(event)">
                <a-input v-model:value="receiptReview[receiptEventId(event)].tmdb_id" size="small" placeholder="TMDB ID" class="receipt-tmdb" />
                <a-select v-model:value="receiptReview[receiptEventId(event)].media_type" size="small" :options="receiptMediaOptions" class="receipt-media" />
              </template>
              <a-button
                v-if="['needs_review', 'failed', 'delivery_failed'].includes(event.status)"
                size="small"
                :loading="autoShareBusy[receiptEventId(event)]"
                @click="retryAutoShareEvent(event)"
              ><template #icon><SendOutlined /></template>重试</a-button>
            </a-flex>
          </a-card>
        </div>
      </a-tab-pane>
    </a-tabs>

    <a-drawer v-model:open="backupDrawerOpen" title="新建备份任务" width="420">
      <a-form layout="vertical">
        <a-form-item label="本地文件夹" required>
          <a-input v-model:value="backupForm.local" :placeholder="isTauri ? '选择要备份的本地文件夹' : '输入容器内可访问的绝对路径'" :readonly="isTauri" @click="isTauri && pickBackupFolder('local')">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
          <small v-if="!isTauri" class="form-hint">Web 端路径以服务器或容器内的挂载目录为准</small>
        </a-form-item>
        <a-form-item label="云端目录" required>
          <a-input :value="backupForm.remoteLabel" readonly placeholder="选择光鸭云盘目录" @click="openCloudFolderPicker">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
        </a-form-item>
        <a-form-item label="监控方式">
          <a-radio-group v-model:value="backupForm.monitor_mode">
            <a-radio-button value="native">系统监听</a-radio-button>
            <a-radio-button value="polling">轮询扫描</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="源文件处理">
          <a-radio-group v-model:value="backupForm.policy">
            <a-radio-button value="keep">保留</a-radio-button>
            <a-radio-button value="archive">归档</a-radio-button>
            <a-radio-button value="delete">删除</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item v-if="backupForm.policy === 'archive'" label="归档目录" required>
          <a-input v-model:value="backupForm.archive" :placeholder="isTauri ? '选择归档目录' : '输入容器内的归档路径'" :readonly="isTauri" @click="isTauri && pickBackupFolder('archive')">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
        </a-form-item>
        <a-form-item label="同步格式">
          <div class="preset-row">
            <a-button v-for="preset in extensionPresets" :key="preset.key" size="small" @click="backupForm.sync_types = presetExtensions(preset.key)">{{ preset.label }}</a-button>
          </div>
          <a-select v-model:value="backupForm.sync_types" mode="tags" :options="backupForm.sync_types.map((ext) => ({ value: ext, label: ext }))" placeholder="输入扩展名后回车" />
        </a-form-item>
        <a-form-item label="自动分享">
          <a-switch v-model:checked="backupForm.auto_share" aria-label="上传完成后自动分享并通知 HDHive" />
          <small class="form-hint">未启用上传后整理时走原分享逻辑；启用后会等待 B 目录整理完成，再创建新分享并通知 HDHive</small>
        </a-form-item>
        <a-form-item label="上传后自动整理">
          <a-select v-model:value="backupForm.organizer_mapping_id" allow-clear placeholder="不启用，保持原上传逻辑" :options="matchingOrganizerOptions" />
          <small class="form-hint">只显示 A 目录与当前上传目标完全一致的整理任务。请先在“媒体整理”中创建 A → B 规则。</small>
        </a-form-item>
        <a-button type="primary" block :loading="backupSubmitting" @click="addBackup"><template #icon><PlusOutlined /></template>创建备份任务</a-button>
      </a-form>
    </a-drawer>

    <a-modal v-model:open="cloudFolderPicker.open" title="选择云端目录" width="620px" ok-text="选择当前目录" cancel-text="取消" @ok="chooseCloudFolder">
      <a-flex class="cloud-folder-toolbar" align="center" gap="small">
        <a-button type="text" :disabled="cloudFolderPicker.path.length <= 1" aria-label="返回上级目录" @click="leaveCloudFolder">
          <template #icon><ArrowLeftOutlined /></template>
        </a-button>
        <a-breadcrumb>
          <a-breadcrumb-item v-for="(segment, index) in cloudFolderPicker.path" :key="segment.id || 'root'">
            <a v-if="index < cloudFolderPicker.path.length - 1" href="#" @click.prevent="jumpToCloudFolder(index)">{{ segment.name }}</a>
            <span v-else>{{ segment.name }}</span>
          </a-breadcrumb-item>
        </a-breadcrumb>
      </a-flex>
      <a-table
        :columns="cloudFolderColumns"
        :data-source="cloudFolderPicker.items"
        :loading="cloudFolderPicker.loading"
        :row-key="(item) => fileId(item)"
        :pagination="{ current: cloudFolderPicker.page + 1, pageSize: 100, total: cloudFolderPicker.total, showSizeChanger: false }"
        size="small"
        @change="handleCloudFolderTableChange"
      >
        <template #emptyText><a-empty description="当前目录下没有文件夹" /></template>
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'name'">
            <a-space><FolderOutlined /><span>{{ record.fileName || record.name }}</span></a-space>
          </template>
          <template v-else-if="column.key === 'actions'">
            <a-button type="link" size="small" @click="enterCloudFolder(record)">进入</a-button>
          </template>
        </template>
      </a-table>
    </a-modal>

    <a-modal v-model:open="autoOrganizeEditor.open" title="上传后自动整理" :confirm-loading="autoOrganizeEditor.saving" ok-text="保存" cancel-text="取消" width="560px" @ok="saveAutoOrganizeEditor">
      <a-alert type="warning" show-icon message="启用后，上传完成不会立即分享 A 目录；光鸭会先整理到 B 目录，再从 B 目录创建新分享。这样可避免移动或删除导致刚创建的分享失效。" class="receipt-alert" />
      <a-select v-model:value="autoOrganizeEditor.selected" allow-clear placeholder="关闭上传后自动整理" :options="editorOrganizerOptions" style="width: 100%" />
      <p class="form-hint">只有来源 A 目录与本备份任务云端目标完全一致的整理任务可选。</p>
    </a-modal>

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
        <a-card v-for="event in autoShareHistoryEvents" :key="event.event_id" class="receipt-card" :bordered="false" size="small">
          <a-flex class="receipt-heading" align="center" gap="small" wrap="wrap">
            <a-tag :color="receiptColor(event.status)">{{ receiptStatusLabel(event) }}</a-tag>
            <strong class="receipt-name" :title="receiptTargetLabel(event)">{{ receiptTargetLabel(event) }}</strong>
            <span class="receipt-time">{{ formatTime(event.updated_at || event.created_at) }}</span>
          </a-flex>
          <a-alert :type="receiptAlertType(event.status)" :message="receiptDisplayMessage(event)" show-icon class="receipt-alert" />
          <a-flex class="receipt-actions" gap="small" align="center" wrap="wrap">
            <a-tag v-if="event.action">{{ receiptActionLabel(event.action) }}</a-tag>
            <a v-if="event.share_url" :href="event.share_url" target="_blank" rel="noopener noreferrer">查看分享</a>
            <a v-if="event.resource_url" :href="event.resource_url" target="_blank" rel="noopener noreferrer">查看 HDHive 资源</a>
            <template v-if="needsTmdbReview(event)">
              <a-input v-model:value="receiptReview[receiptEventId(event)].tmdb_id" size="small" placeholder="TMDB ID" class="receipt-tmdb" />
              <a-select v-model:value="receiptReview[receiptEventId(event)].media_type" size="small" :options="receiptMediaOptions" class="receipt-media" />
            </template>
            <a-button
              v-if="['needs_review', 'failed', 'delivery_failed'].includes(event.status)"
              size="small"
              :loading="autoShareBusy[receiptEventId(event)]"
              @click="retryAutoShareEvent(event)"
            ><template #icon><SendOutlined /></template>重试</a-button>
          </a-flex>
        </a-card>
      </div>
    </a-drawer>

  </div>
</template>

<style scoped>
.activity-list { display: grid; }
.activity-row { display: grid; grid-template-columns:100px minmax(180px,.8fr) minmax(240px,1.2fr) 160px; align-items: center; gap: 12px; min-height: 48px; border-bottom: 1px solid var(--line, #e7e8eb); }
.activity-row strong, .activity-row span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.activity-row span, .activity-row time { color: var(--text-3, #98a2b3); font-size: 12px; }
.receipt-list { display: grid; gap: 10px; }
.global-receipt-list { max-width: 960px; }
.receipt-card { background: var(--surface, #fff); }
.receipt-heading { min-width: 0; }
.receipt-name { min-width: 160px; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.receipt-time { margin-left: auto; color: var(--text-3, #98a2b3); font-size: 12px; }
.receipt-alert { margin-top: 8px; }
.receipt-actions { margin-top: 8px; }
.receipt-tmdb { width: 120px; }
.receipt-media { width: 100px; }
</style>
