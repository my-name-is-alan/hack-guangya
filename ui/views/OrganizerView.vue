<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  ArrowLeftOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  DeleteOutlined,
  EditOutlined,
  EyeOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  PlayCircleOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  SettingOutlined,
  WarningOutlined,
} from '@antdv-next/icons';
import { bridge } from '../bridge.js';
import { errorText, fileId, formatSize, isFolder, unwrapData } from '../formatters.js';
import {
  organizerCandidates,
  organizerConflictLabel,
  organizerItemActionLabel,
  organizerItemKindLabel,
  organizerMatchedTitle,
  organizerMediaLabel,
  organizerPreviewItems,
  organizerPreviewTarget,
  organizerStatus,
  organizerTemplateExamples,
  organizerTransferLabel,
} from '../organizer.js';

const props = defineProps({ settingsOnly: { type: Boolean, default: false } });

const DEFAULT_SETTINGS = Object.freeze({
  configured: false,
  language: 'zh-CN',
  image_language: 'zh,null,en',
  include_adult: false,
  minimum_match_score: 0.72,
  movie_path_template: '{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}',
  tv_path_template: '{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}',
  movie_category: '电影',
  tv_category: '电视剧',
  default_scrape_types: ['movie_nfo', 'tvshow_nfo', 'poster', 'fanart'],
  scrape_type_options: [],
  path_presets: [],
  tmdb_api_base: 'https://api.themoviedb.org/3',
  tmdb_image_base: 'https://image.tmdb.org/t/p',
  category_rules: [],
  scrape_targets: [],
  template_examples: { movie: {}, tv: {} },
  tmdb_api_base_managed_by_environment: false,
  tmdb_image_base_managed_by_environment: false,
});

const TMDB_GENRE_OPTIONS = [
  ['28', '动作'], ['12', '冒险'], ['16', '动画'], ['35', '喜剧'], ['80', '犯罪'], ['99', '纪录'], ['18', '剧情'],
  ['10751', '家庭'], ['14', '奇幻'], ['36', '历史'], ['27', '恐怖'], ['10402', '音乐'], ['9648', '悬疑'], ['10749', '爱情'],
  ['878', '科幻'], ['53', '惊悚'], ['10752', '战争'], ['37', '西部'], ['10759', '动作冒险（剧集）'], ['10762', '儿童（剧集）'],
  ['10763', '新闻（剧集）'], ['10764', '真人秀（剧集）'], ['10765', '科幻奇幻（剧集）'], ['10766', '肥皂剧（剧集）'], ['10767', '脱口秀（剧集）'], ['10768', '战争政治（剧集）'],
].map(([value, label]) => ({ value, label: `${label} · TMDB ${value}` }));

const loading = ref(true);
const refreshing = ref(false);
const settingsSaving = ref(false);
const settingsTesting = ref(false);
const mappingSubmitting = ref(false);
const settingsPanels = ref([]);
const settingsSection = ref('directories');
const targetModal = reactive({ open: false, editingId: '', name: '', dir_id: '', path: '/' });
const jobBusy = reactive({});
const organizer = reactive({ settings: { ...DEFAULT_SETTINGS }, mappings: [], jobs: [], counts: {} });
const settingsForm = reactive({ ...DEFAULT_SETTINGS, api_key: '' });
const mappingDrawer = reactive({ open: false, editingId: '' });
const mappingForm = reactive({
  source_path: '',
  target_path: '',
  source_dir_id: '',
  target_dir_id: '',
  enabled: true,
  scan_existing: true,
  monitor_mode: 'cloud_polling',
  transfer_type: 'copy',
  media_type: '',
  scrape: false,
  scrape_types: [],
  sync_extras: true,
  conflict_policy: 'skip',
  auto_execute: false,
  share_after_organize: false,
  share_risk_acknowledged: false,
  settle_seconds: 30,
});
const cloudFolderPicker = reactive({
  open: false,
  kind: 'source',
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
const review = reactive({
  open: false,
  job: null,
  media_type: '',
  tmdb_id: '',
  title: '',
  year: '',
  season: '',
  episode: '',
  episode_end: '',
});
const preview = reactive({ open: false, job: null });
let settingsHydrated = false;
let refreshTimer = null;
let unsubscribe = null;

const activeJobs = computed(() => organizer.jobs.filter((job) => ['recognizing', 'ready', 'running', 'needs_review'].includes(job.status)).length);
const completedJobs = computed(() => Number(organizer.counts.completed || 0) + Number(organizer.counts.completed_warning || 0));
const mappingLocked = (mapping) => organizer.jobs.some((job) => job.mapping_id === mapping.id && ['recognizing', 'running'].includes(job.status));
const reviewJobs = computed(() => Number(organizer.counts.needs_review || 0) + Number(organizer.counts.failed || 0));
const previewItems = computed(() => organizerPreviewItems(preview.job));
const previewSummary = computed(() => preview.job?.preview?.data?.summary || {});
const reviewCandidates = computed(() => organizerCandidates(review.job));
const mappingTitle = computed(() => mappingDrawer.editingId ? '编辑整理监控' : '新增整理监控');
const pathPresetOptions = computed(() => (organizer.settings.path_presets || []).map((item) => ({ label: item.name, value: item.id })));
const templateExamples = computed(() => organizerTemplateExamples(
  settingsForm.movie_path_template,
  settingsForm.tv_path_template,
  settingsForm.movie_category,
  settingsForm.tv_category,
));

const jobColumns = [
  { title: '来源', key: 'source', width: 270 },
  { title: '识别与目标', key: 'result' },
  { title: '状态', key: 'status', width: 118 },
  { title: '更新时间', key: 'time', width: 150 },
  { title: '操作', key: 'actions', width: 252, fixed: 'right' },
];

function timeText(value) {
  const timestamp = Number(value || 0);
  if (!timestamp) return '—';
  return new Date(timestamp < 1e12 ? timestamp * 1000 : timestamp).toLocaleString('zh-CN', { hour12: false });
}

function fileName(value) {
  return String(value || '').split(/[\/]/).filter(Boolean).at(-1) || '未知项目';
}

function hydrateSettings(settings, force = false) {
  if (settingsHydrated && !force) return;
  Object.assign(settingsForm, {
    api_key: '',
    language: settings.language || DEFAULT_SETTINGS.language,
    image_language: settings.image_language || DEFAULT_SETTINGS.image_language,
    include_adult: Boolean(settings.include_adult),
    minimum_match_score: Number(settings.minimum_match_score ?? DEFAULT_SETTINGS.minimum_match_score),
    movie_path_template: settings.movie_path_template || DEFAULT_SETTINGS.movie_path_template,
    tv_path_template: settings.tv_path_template || DEFAULT_SETTINGS.tv_path_template,
    movie_category: settings.movie_category || DEFAULT_SETTINGS.movie_category,
    tv_category: settings.tv_category || DEFAULT_SETTINGS.tv_category,
    tmdb_api_base: settings.tmdb_api_base || DEFAULT_SETTINGS.tmdb_api_base,
    tmdb_image_base: settings.tmdb_image_base || DEFAULT_SETTINGS.tmdb_image_base,
    category_rules: Array.isArray(settings.category_rules) ? structuredClone(settings.category_rules) : [],
    scrape_targets: Array.isArray(settings.scrape_targets) ? structuredClone(settings.scrape_targets) : [],
    default_scrape_types: Array.isArray(settings.default_scrape_types) ? [...settings.default_scrape_types] : [...DEFAULT_SETTINGS.default_scrape_types],
  });
  settingsHydrated = true;
}

function updateOpenJobs() {
  if (review.job) review.job = organizer.jobs.find((job) => job.id === review.job.id) || review.job;
  if (preview.job) preview.job = organizer.jobs.find((job) => job.id === preview.job.id) || preview.job;
}

async function loadState({ silent = false } = {}) {
  if (silent) refreshing.value = true;
  else loading.value = true;
  try {
    const data = await bridge.invoke('get_organizer_state');
    organizer.settings = { ...DEFAULT_SETTINGS, ...(data.settings || {}) };
    organizer.mappings = Array.isArray(data.mappings) ? data.mappings : [];
    organizer.jobs = Array.isArray(data.jobs) ? data.jobs : [];
    organizer.counts = data.counts || {};
    hydrateSettings(organizer.settings);
    updateOpenJobs();
    return data;
  } catch (error) {
    if (!silent) message.error(errorText(error));
    return null;
  } finally {
    loading.value = false;
    refreshing.value = false;
  }
}

async function saveSettings() {
  settingsSaving.value = true;
  try {
    const saved = await bridge.invoke('update_organizer_settings', { input: { ...settingsForm } });
    organizer.settings = { ...DEFAULT_SETTINGS, ...(saved || {}) };
    hydrateSettings(organizer.settings, true);
    await loadState({ silent: true });
    message.success('原生整理设置已保存');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    settingsSaving.value = false;
  }
}

async function testSettings() {
  settingsTesting.value = true;
  try {
    const result = await bridge.invoke('test_organizer_connection', { input: { ...settingsForm } });
    message.success(result?.message || 'TMDB 连接成功');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    settingsTesting.value = false;
  }
}

function resetMappingForm() {
  Object.assign(mappingForm, {
    source_path: '',
    target_path: '',
    source_dir_id: '',
    target_dir_id: '',
    enabled: true,
    scan_existing: true,
    monitor_mode: 'cloud_polling',
    transfer_type: 'copy',
    media_type: '',
    scrape: false,
    scrape_types: [],
    sync_extras: true,
    conflict_policy: 'skip',
    auto_execute: false,
    share_after_organize: false,
    share_risk_acknowledged: false,
    settle_seconds: 30,
  });
}

function openNewMapping() {
  mappingDrawer.editingId = '';
  resetMappingForm();
  mappingDrawer.open = true;
}

function openEditMapping(mapping) {
  mappingDrawer.editingId = mapping.id;
  Object.assign(mappingForm, {
    source_path: mapping.source_path,
    target_path: mapping.target_path || '',
    source_dir_id: mapping.source_dir_id || '',
    target_dir_id: mapping.target_dir_id || '',
    enabled: Boolean(mapping.enabled),
    scan_existing: Boolean(mapping.scan_existing),
    monitor_mode: 'cloud_polling',
    transfer_type: mapping.transfer_type || 'copy',
    media_type: mapping.media_type || '',
    scrape: Boolean(mapping.scrape),
    scrape_types: Array.isArray(mapping.scrape_types) ? [...mapping.scrape_types] : [],
    sync_extras: mapping.sync_extras !== false,
    conflict_policy: mapping.conflict_policy || 'skip',
    auto_execute: Boolean(mapping.auto_execute),
    share_after_organize: Boolean(mapping.share_after_organize),
    share_risk_acknowledged: Boolean(mapping.share_risk_acknowledged),
    settle_seconds: Number(mapping.settle_seconds || 30),
  });
  mappingDrawer.open = true;
}

function applyPathPreset(value) {
  const preset = (organizer.settings.path_presets || []).find((item) => item.id === value);
  if (!preset) return;
  settingsForm.movie_path_template = preset.movie;
  settingsForm.tv_path_template = preset.tv;
}

function openNewTarget() {
  Object.assign(targetModal, { open: true, editingId: '', name: '', dir_id: '', path: '/' });
}

function openEditTarget(target) {
  Object.assign(targetModal, { open: true, editingId: target.id || '', name: target.name || '', dir_id: target.dir_id || target.target_dir_id || '', path: target.path || target.target_path || '/' });
}

function removeTarget(target) {
  settingsForm.scrape_targets = settingsForm.scrape_targets.filter((item) => item.id !== target.id);
}

function chooseScrapeTargetFolder() {
  void openCloudFolderPicker('scrape-target');
}

function saveTarget() {
  if (!targetModal.name.trim() || !targetModal.dir_id) {
    message.warning('请填写名称并选择云盘目录');
    return;
  }
  const next = { id: targetModal.editingId || `target-${Date.now()}`, name: targetModal.name.trim(), dir_id: targetModal.dir_id, path: targetModal.path || '/' };
  const index = settingsForm.scrape_targets.findIndex((item) => item.id === next.id);
  if (index >= 0) settingsForm.scrape_targets.splice(index, 1, next);
  else settingsForm.scrape_targets.push(next);
  targetModal.open = false;
}

function toggleScrape(enabled) {
  mappingForm.scrape = enabled;
  if (enabled && !mappingForm.scrape_types.length) mappingForm.scrape_types = [...(organizer.settings.default_scrape_types || DEFAULT_SETTINGS.default_scrape_types)];
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

async function openCloudFolderPicker(kind) {
  cloudFolderPicker.kind = kind;
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
  if (!current?.id) {
    message.warning('整理目录不能选择云盘根目录');
    return;
  }
  const value = `/${cloudFolderPicker.path.slice(1).map((item) => item.name).join('/')}`;
  if (cloudFolderPicker.kind === 'source') {
    mappingForm.source_dir_id = current.id;
    mappingForm.source_path = value;
  } else if (cloudFolderPicker.kind === 'scrape-target') {
    targetModal.dir_id = current.id;
    targetModal.path = value;
  } else {
    mappingForm.target_dir_id = current.id;
    mappingForm.target_path = value;
  }
  cloudFolderPicker.open = false;
}

function handleCloudFolderTableChange(pagination) {
  const page = Math.max(0, Number(pagination?.current || 1) - 1);
  if (page !== cloudFolderPicker.page) void loadCloudFolders(page);
}

async function submitMapping() {
  if (!mappingForm.source_dir_id) {
    message.warning('请选择光鸭云盘来源 A 目录');
    return;
  }
  if (!mappingForm.target_dir_id) {
    message.warning('请选择光鸭云盘目标 B 目录');
    return;
  }
  if (!organizer.settings.configured) {
    message.warning('请先保存 TMDB API Key');
    return;
  }
  if (mappingForm.scrape && !mappingForm.scrape_types.length) {
    message.warning('开启刮削后请至少选择一种元数据类型');
    return;
  }
  if ((mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite') && !mappingForm.share_risk_acknowledged) {
    message.warning('请先确认移动/覆盖导致旧分享失效的风险');
    return;
  }
  mappingSubmitting.value = true;
  try {
    const input = { ...mappingForm, scrape_types: [...mappingForm.scrape_types], settle_seconds: Number(mappingForm.settle_seconds) };
    if (mappingDrawer.editingId) {
      await bridge.invoke('update_organizer_mapping', { id: mappingDrawer.editingId, input });
      message.success('整理监控已更新');
    } else {
      await bridge.invoke('add_organizer_mapping', { input });
      message.success('整理监控已创建');
    }
    mappingDrawer.open = false;
    await loadState({ silent: true });
  } catch (error) {
    message.error(errorText(error));
  } finally {
    mappingSubmitting.value = false;
  }
}

async function toggleMapping(mapping, enabled) {
  try {
    await bridge.invoke('update_organizer_mapping', { id: mapping.id, input: { enabled } });
    await loadState({ silent: true });
    message.success(enabled ? '目录监控已启用' : '目录监控已暂停');
  } catch (error) {
    message.error(errorText(error));
  }
}

async function scanMapping(mapping) {
  jobBusy[`mapping:${mapping.id}`] = true;
  try {
    const result = await bridge.invoke('scan_organizer_mapping', { id: mapping.id });
    message.success(`已提交 ${Number(result?.queued || 0)} 个候选项进行识别`);
    await loadState({ silent: true });
  } catch (error) {
    message.error(errorText(error));
  } finally {
    jobBusy[`mapping:${mapping.id}`] = false;
  }
}

function removeMapping(mapping) {
  Modal.confirm({
    title: '删除整理监控',
    content: `确定删除「${mapping.source_path}」及其整理历史吗？已整理的媒体文件不会被删除。`,
    okText: '删除',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      try {
        await bridge.invoke('remove_organizer_mapping', { id: mapping.id });
        await loadState({ silent: true });
        message.success('整理监控已删除');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
}

function removeJob(job) {
  Modal.confirm({
    title: '删除整理记录',
    content: `确定删除「${fileName(job.source_path)}」的任务记录吗？源文件和媒体库文件都不会被删除。`,
    okText: '删除记录',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      try {
        await bridge.invoke('remove_organizer_job', { id: job.id });
        if (review.job?.id === job.id) review.open = false;
        if (preview.job?.id === job.id) preview.open = false;
        await loadState({ silent: true });
        message.success('整理记录已删除');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
}

async function runJob(job) {
  jobBusy[job.id] = true;
  try {
    const result = await bridge.invoke('run_organizer_job', { id: job.id, input: {} });
    await loadState({ silent: true });
    if (result?.status === 'failed') message.error(result.message || '整理失败');
    else if (result?.status === 'completed_warning') message.warning(result.message || '整理完成，但有非阻断提示');
    else message.success(result?.status === 'completed' ? '整理完成' : '任务已更新');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    jobBusy[job.id] = false;
  }
}

function openReview(job) {
  review.job = job;
  review.media_type = job.media_type || job.preview?.query?.media_type || '';
  review.tmdb_id = job.tmdb_id ?? '';
  review.title = job.query_title || job.preview?.query?.title || '';
  review.year = job.query_year ?? job.preview?.query?.year ?? '';
  review.season = job.season ?? '';
  review.episode = job.episode ?? '';
  review.episode_end = job.episode_end ?? '';
  review.open = true;
}

function chooseCandidate(candidate) {
  review.tmdb_id = candidate.tmdb_id;
  review.media_type = candidate.media_type;
  if (candidate.title) review.title = candidate.title;
  if (candidate.year) review.year = candidate.year;
}

function optionalNumber(value) {
  return value === '' || value === null || value === undefined ? undefined : Number(value);
}

async function submitReview(execute) {
  const job = review.job;
  if (!job) return;
  jobBusy[job.id] = true;
  try {
    const input = {
      media_type: review.media_type || '',
      tmdb_id: optionalNumber(review.tmdb_id),
      title: String(review.title || '').trim() || undefined,
      year: optionalNumber(review.year),
      season: optionalNumber(review.season),
      episode: optionalNumber(review.episode),
      episode_end: optionalNumber(review.episode_end),
      clear_tmdb_id: optionalNumber(review.tmdb_id) === undefined,
      clear_title: !String(review.title || '').trim(),
      clear_year: optionalNumber(review.year) === undefined,
      clear_season: optionalNumber(review.season) === undefined,
      clear_episode: optionalNumber(review.episode) === undefined,
      clear_episode_end: optionalNumber(review.episode_end) === undefined,
    };
    const result = await bridge.invoke(execute ? 'run_organizer_job' : 'retry_organizer_job', { id: job.id, input });
    review.open = false;
    await loadState({ silent: true });
    if (result?.status === 'failed' || result?.status === 'needs_review') message.warning(result.message || '仍需人工确认');
    else if (result?.status === 'completed_warning') message.warning(result.message || '整理完成，但有非阻断提示');
    else message.success(execute && result?.status === 'completed' ? '整理完成' : '重新识别完成');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    jobBusy[job.id] = false;
  }
}

function openPreview(job) {
  preview.job = job;
  preview.open = true;
}

function candidateScore(candidate) {
  return `${Math.round(Number(candidate.score || 0) * 100)}%`;
}

onMounted(async () => {
  await loadState();
  unsubscribe = await bridge.subscribe((event) => {
    if (event?.type === 'organizer') void loadState({ silent: true });
  });
  refreshTimer = window.setInterval(() => void loadState({ silent: true }), 5000);
});

onBeforeUnmount(() => {
  if (refreshTimer) window.clearInterval(refreshTimer);
  if (typeof unsubscribe === 'function') unsubscribe();
});
</script>

<template>
  <a-spin :spinning="loading">
    <section class="organizer-page">
      <header v-if="!props.settingsOnly" class="page-heading">
        <div>
          <span class="eyebrow">GUANGYA NATIVE ORGANIZER</span>
          <h1>媒体识别与整理</h1>
          <p>光鸭直接在云盘内完成 A → B 识别整理；不经过 MoviePilot API，也不依赖本地挂载盘搬运。</p>
        </div>
        <div class="heading-actions">
          <a-button :loading="refreshing" @click="loadState({ silent: true })"><ReloadOutlined />刷新任务</a-button>
        </div>
      </header>

      <a-tabs v-if="props.settingsOnly" v-model:active-key="settingsSection" class="organizer-settings-tabs">
        <a-tab-pane key="directories">
          <template #tab><span class="inner-tab"><FolderOpenOutlined />目录设置</span></template>
        </a-tab-pane>
        <a-tab-pane key="tmdb">
          <template #tab><span class="inner-tab"><SettingOutlined />TMDB 配置</span></template>
        </a-tab-pane>
        <a-tab-pane key="scrape">
          <template #tab><span class="inner-tab"><EyeOutlined />刮削偏好</span></template>
        </a-tab-pane>
      </a-tabs>

      <a-card v-if="props.settingsOnly && settingsSection === 'tmdb'" class="settings-card" :bordered="false">
        <template #title><span class="card-title"><SettingOutlined />原生识别设置</span></template>
        <template #extra>
          <a-space>
            <a-tag color="blue">内置引擎</a-tag>
            <a-tag :color="organizer.settings.configured ? 'success' : 'warning'">{{ organizer.settings.configured ? 'TMDB 已配置' : '需要 TMDB Key' }}</a-tag>
          </a-space>
        </template>
        <a-alert
          v-if="organizer.settings.api_key_managed_by_environment || organizer.settings.language_managed_by_environment || organizer.settings.image_language_managed_by_environment || organizer.settings.tmdb_api_base_managed_by_environment || organizer.settings.tmdb_image_base_managed_by_environment"
          type="info"
          show-icon
          message="部分参数由 TMDB_API_KEY / TMDB_LANGUAGE / TMDB_IMAGE_LANGUAGE / TMDB_API_BASE / TMDB_IMAGE_BASE 环境变量托管，环境变量优先。"
          class="settings-alert"
        />
        <div class="settings-primary">
          <a-form-item label="TMDB API Key / Read Token">
            <a-input-password
              v-model:value="settingsForm.api_key"
              :placeholder="organizer.settings.configured ? '已保存；留空保持不变' : '填写 TMDB v3 Key 或 v4 Read Token'"
              :disabled="organizer.settings.api_key_managed_by_environment"
            />
          </a-form-item>
          <a-form-item label="元数据语言">
            <a-select v-model:value="settingsForm.language" :disabled="organizer.settings.language_managed_by_environment" :options="[{ label: '简体中文', value: 'zh-CN' }, { label: '繁體中文', value: 'zh-TW' }, { label: 'English', value: 'en-US' }, { label: '日本語', value: 'ja-JP' }]" />
          </a-form-item>
          <a-form-item label="图片语言优先级">
            <a-input v-model:value="settingsForm.image_language" :disabled="organizer.settings.image_language_managed_by_environment" placeholder="zh,null,en" />
          </a-form-item>
          <a-form-item label="TMDB API 镜像">
            <a-input v-model:value="settingsForm.tmdb_api_base" :disabled="organizer.settings.tmdb_api_base_managed_by_environment" placeholder="https://api.themoviedb.org/3" />
          </a-form-item>
          <a-form-item label="TMDB 图片镜像">
            <a-input v-model:value="settingsForm.tmdb_image_base" :disabled="organizer.settings.tmdb_image_base_managed_by_environment" placeholder="https://image.tmdb.org/t/p" />
          </a-form-item>
          <a-form-item label="自动匹配阈值">
            <a-input-number v-model:value="settingsForm.minimum_match_score" :min="0.4" :max="0.98" :step="0.01" :precision="2" style="width: 100%" />
          </a-form-item>
          <div class="settings-actions">
            <a-button :loading="settingsTesting" @click="testSettings">测试 TMDB</a-button>
            <a-button type="primary" :loading="settingsSaving" @click="saveSettings">保存设置</a-button>
          </div>
        </div>
        <a-collapse v-model:activeKey="settingsPanels" ghost class="naming-collapse">
          <a-collapse-panel key="naming" header="命名模板与匹配选项">
            <div class="template-grid">
              <a-form-item label="套用模板预设" class="template-wide"><a-select placeholder="选择后仍可自由修改" :options="pathPresetOptions" @change="applyPathPreset" /></a-form-item>
              <a-form-item label="电影分类值"><a-input v-model:value="settingsForm.movie_category" /></a-form-item>
              <a-form-item label="电视剧分类值"><a-input v-model:value="settingsForm.tv_category" /></a-form-item>
              <a-form-item label="电影完整相对路径" class="template-wide"><a-textarea v-model:value="settingsForm.movie_path_template" :auto-size="{ minRows: 2, maxRows: 4 }" /></a-form-item>
              <a-form-item label="电视剧完整相对路径" class="template-wide"><a-textarea v-model:value="settingsForm.tv_path_template" :auto-size="{ minRows: 2, maxRows: 4 }" /></a-form-item>
              <label class="adult-setting template-wide"><span><strong>允许成人内容候选</strong><small>仅影响 TMDB 搜索结果，不改变云盘文件扫描。</small></span><a-switch v-model:checked="settingsForm.include_adult" /></label>
            </div>
              <p class="template-help">可用字段：{category}（兼容 {catgroy}）、{country}、{year}、{title}、{original_title}、{tmdb_id}（兼容 {tmdbid}）、{season:02}、{episode:02}、{season_tag}、{episode_tag}、{Season x}、{Expose n}、{episode_end}、{episode_title}、{edition}、{quality}、{part}、{ext}。{ext} 可选；不使用时可直接写固定后缀（例如 .mkv）。模板必须同时包含目录与文件名。</p>
             <div class="template-preview-grid">
                <article><small>电影标准文件名预览</small><code>{{ templateExamples.movie.filename || '示例电影 (2024) [tmdb-12345].mkv' }}</code><span>{{ templateExamples.movie.path || '电影/US/2024/示例电影 (2024) [tmdb-12345]/示例电影 (2024).mkv' }}</span></article>
                <article><small>电视剧标准文件名预览</small><code>{{ templateExamples.tv.filename || '示例剧集.S01E02.mkv' }}</code><span>{{ templateExamples.tv.path || '电视剧/CN/2024/示例剧集 (2024) [tmdb-67890]/Season 01/示例剧集.S01E02.mkv' }}</span></article>
             </div>
           </a-collapse-panel>
        </a-collapse>
      </a-card>

      <section v-if="props.settingsOnly && settingsSection === 'directories'" class="section-block">
        <div class="section-heading">
          <div><h2>云盘目录监控</h2><p>光鸭每 15 秒直接读取 A 目录；每个一级文件夹或视频视为一个候选项目。</p></div>
          <a-button type="text" :disabled="!organizer.settings.configured" @click="openNewMapping"><PlusOutlined />新增</a-button>
        </div>
        <a-empty v-if="!organizer.mappings.length" description="配置 TMDB 后，选择光鸭云盘 A 来源目录与 B 媒体库目录" />
        <div v-else class="mapping-list">
          <article v-for="mapping in organizer.mappings" :key="mapping.id" class="mapping-card">
            <div class="mapping-icon"><FolderOpenOutlined /></div>
            <div class="mapping-copy">
              <div class="mapping-title-row">
                <strong>{{ mapping.source_path }}</strong>
                <a-tag :color="mapping.enabled && !mapping.watch_error ? 'success' : mapping.watch_error ? 'error' : 'default'">
                  {{ mapping.watch_error ? '监控异常' : mapping.enabled ? '监控中' : '已暂停' }}
                </a-tag>
              </div>
              <span class="path-flow">A {{ mapping.source_path }} → B {{ mapping.target_path }}</span>
              <div class="mapping-meta">
                <span>云端每 15 秒轮询</span>
                <span>{{ organizerMediaLabel(mapping.media_type) }}</span>
                <span>{{ organizerTransferLabel(mapping.transfer_type) }}</span>
                <span>{{ organizerConflictLabel(mapping.conflict_policy) }}</span>
                <span>{{ mapping.scrape ? `刮削 ${mapping.scrape_types?.length || 0} 类元数据` : '不刮削（默认）' }}</span>
                <span>{{ mapping.sync_extras ? '同步字幕/音轨' : '仅主视频' }}</span>
                <span>{{ mapping.auto_execute ? '自动执行' : '预览后确认' }}</span>
                <span>{{ mapping.share_after_organize ? '整理后从 B 重新分享' : '整理后不分享' }}</span>
                <span>静默 {{ mapping.settle_seconds }} 秒</span>
              </div>
              <a-alert v-if="mapping.watch_error" type="error" :message="mapping.watch_error" show-icon />
            </div>
            <div class="mapping-actions">
              <a-switch :checked="mapping.enabled" :disabled="mappingLocked(mapping)" checked-children="开" un-checked-children="停" @change="(value) => toggleMapping(mapping, value)" />
              <a-tooltip :title="mappingLocked(mapping) ? '目录正在识别或整理' : '立即扫描'"><a-button shape="circle" :disabled="!mapping.enabled || mappingLocked(mapping)" :loading="jobBusy[`mapping:${mapping.id}`]" @click="scanMapping(mapping)"><ReloadOutlined /></a-button></a-tooltip>
              <a-tooltip :title="mappingLocked(mapping) ? '完成当前任务后才能编辑' : '编辑'"><a-button shape="circle" :disabled="mappingLocked(mapping)" @click="openEditMapping(mapping)"><EditOutlined /></a-button></a-tooltip>
              <a-tooltip :title="mappingLocked(mapping) ? '完成当前任务后才能删除' : '删除'"><a-button danger shape="circle" :disabled="mappingLocked(mapping)" @click="removeMapping(mapping)"><DeleteOutlined /></a-button></a-tooltip>
            </div>
          </article>
        </div>
      </section>

      <section v-if="props.settingsOnly && settingsSection === 'scrape'" class="section-block scrape-settings-block">
        <div class="section-heading">
          <div><h2>刮削偏好</h2><p>默认只刮削常用元数据；可按 TMDB 类型把媒体归入自定义分类，并配置多个媒体库目标。</p></div>
          <a-button type="primary" @click="saveSettings" :loading="settingsSaving">保存刮削偏好</a-button>
        </div>
        <div class="scrape-preference-grid">
          <a-form-item label="默认刮削类型" class="template-wide">
            <a-select v-model:value="settingsForm.default_scrape_types" mode="multiple" :options="organizer.settings.scrape_type_options || []" placeholder="选择默认生成的 NFO/图片类型" />
          </a-form-item>
          <div class="target-panel template-wide">
            <div class="target-panel-head"><div><strong>已配置媒体库目标</strong><small>右键/勾选文件刮削时从这里选择；支持多个。</small></div><a-button size="small" @click="openNewTarget"><PlusOutlined />添加目标</a-button></div>
            <a-empty v-if="!settingsForm.scrape_targets.length" description="尚未配置刮削目标" />
            <div v-else class="scrape-target-list">
              <article v-for="target in settingsForm.scrape_targets" :key="target.id" class="scrape-target-card">
                <FolderOpenOutlined /><div><strong>{{ target.name }}</strong><small>{{ target.path }} · {{ target.dir_id }}</small></div><a-space><a-button type="text" size="small" @click="openEditTarget(target)"><EditOutlined /></a-button><a-button type="text" danger size="small" @click="removeTarget(target)"><DeleteOutlined /></a-button></a-space>
              </article>
            </div>
          </div>
          <div class="category-rule-panel template-wide">
            <div class="target-panel-head"><div><strong>媒体分类规则</strong><small>按 TMDB 类型名称或 genre id 匹配，第一条命中优先；未命中使用电影/电视剧默认分类。</small></div><a-button size="small" @click="settingsForm.category_rules.push({ id: `category-${Date.now()}`, name: '', media_type: 'all', genres: [], enabled: true })"><PlusOutlined />添加规则</a-button></div>
            <a-empty v-if="!settingsForm.category_rules.length" description="未配置自定义分类规则" />
            <div v-else class="category-rule-list">
              <div v-for="rule in settingsForm.category_rules" :key="rule.id" class="category-rule-row">
                <a-switch v-model:checked="rule.enabled" />
                <a-input v-model:value="rule.name" placeholder="分类名称，例如：儿童" />
                <a-select v-model:value="rule.media_type" :options="[{ label: '全部', value: 'all' }, { label: '电影', value: 'movie' }, { label: '电视剧', value: 'tv' }]" />
                <a-select v-model:value="rule.genres" mode="tags" :options="TMDB_GENRE_OPTIONS" placeholder="选择或输入 TMDB 类型名 / ID" />
                <a-button type="text" danger @click="settingsForm.category_rules = settingsForm.category_rules.filter((item) => item.id !== rule.id)"><DeleteOutlined /></a-button>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section v-if="!props.settingsOnly" class="section-block jobs-block">
        <div class="section-heading">
          <div><h2>整理任务</h2><p>每次执行前都会校验源文件是否变化；有多个 TMDB 候选时会停下来等待选择。</p></div>
        </div>
        <a-table :columns="jobColumns" :data-source="organizer.jobs" row-key="id" :pagination="{ pageSize: 12, showSizeChanger: false }" :scroll="{ x: 1080 }" size="middle">
          <template #emptyText><a-empty description="等待目录中出现媒体文件" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'source'">
              <div class="job-source"><strong>{{ fileName(record.source_path) }}</strong><span>{{ record.source_path }}</span><small>{{ formatSize(record.source_size) }} · {{ record.source_file_count }} 个文件</small></div>
            </template>
            <template v-else-if="column.key === 'result'">
              <div class="job-result">
                <strong>{{ organizerMatchedTitle(record) || record.query_title || record.message || '等待识别' }}</strong>
                <span v-if="organizerPreviewTarget(record)">{{ organizerPreviewTarget(record) }}</span>
                <small>{{ record.message }}</small>
              </div>
            </template>
            <template v-else-if="column.key === 'status'">
              <a-tag :color="organizerStatus(record.status).color">{{ organizerStatus(record.status).label }}</a-tag>
            </template>
            <template v-else-if="column.key === 'time'">{{ timeText(record.updated_at) }}</template>
            <template v-else-if="column.key === 'actions'">
              <a-space :size="2">
                <a-button v-if="record.preview" type="text" size="small" @click="openPreview(record)"><EyeOutlined />预览</a-button>
                <a-button v-if="record.status === 'ready'" type="primary" size="small" :loading="jobBusy[record.id]" @click="runJob(record)"><PlayCircleOutlined />执行</a-button>
                <a-button v-else-if="['needs_review', 'failed'].includes(record.status)" type="link" size="small" :loading="jobBusy[record.id]" @click="openReview(record)"><SearchOutlined />人工确认</a-button>
                <a-tooltip title="仅删除任务记录"><a-button v-if="record.status !== 'running'" type="text" danger size="small" @click="removeJob(record)"><DeleteOutlined /></a-button></a-tooltip>
              </a-space>
            </template>
          </template>
        </a-table>
      </section>
    </section>
  </a-spin>

  <a-drawer v-model:open="mappingDrawer.open" :title="mappingTitle" width="min(660px, 94vw)" :destroy-on-close="false">
    <a-alert
      type="info"
      show-icon
      message="整理直接调用光鸭云盘文件 ID：从 A 目录复制或移动到 B 目录，不经过本地挂载盘，也不存在跨盘整理。"
      class="drawer-alert"
    />
    <a-form layout="vertical">
      <a-form-item label="来源 A 目录" required>
        <a-input-group compact>
          <a-input v-model:value="mappingForm.source_path" readonly placeholder="选择网盘内等待整理的目录" style="width: calc(100% - 92px)" />
          <a-button style="width: 92px" @click="openCloudFolderPicker('source')"><FolderOpenOutlined />选择</a-button>
        </a-input-group>
      </a-form-item>
      <a-form-item label="目标 B 目录" required extra="路径模板生成的相对目录和文件名会写入这个云盘目录。">
        <a-input-group compact>
          <a-input v-model:value="mappingForm.target_path" readonly placeholder="选择网盘内媒体库根目录" style="width: calc(100% - 92px)" />
          <a-button style="width: 92px" @click="openCloudFolderPicker('target')"><FolderOpenOutlined />选择</a-button>
        </a-input-group>
      </a-form-item>
      <div class="form-grid">
        <a-form-item label="云端静默等待">
          <a-input-number v-model:value="mappingForm.settle_seconds" :min="5" :max="3600" :step="5" style="width: 100%" addon-after="秒" />
        </a-form-item>
        <a-form-item label="媒体类型">
          <a-select v-model:value="mappingForm.media_type" :options="[{ label: '自动识别', value: '' }, { label: '电影', value: 'movie' }, { label: '电视剧', value: 'tv' }]" />
        </a-form-item>
        <a-form-item label="整理方式">
          <a-select v-model:value="mappingForm.transfer_type" :options="[{ label: '云盘内复制（推荐）', value: 'copy' }, { label: '云盘内移动', value: 'move' }]" />
        </a-form-item>
        <a-form-item label="目标冲突">
          <a-select v-model:value="mappingForm.conflict_policy" :options="[{ label: '跳过已有文件', value: 'skip' }, { label: '覆盖已有文件', value: 'overwrite' }, { label: '追加短标识保留两份', value: 'rename' }]" />
        </a-form-item>
      </div>
      <a-alert v-if="mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite'" type="warning" show-icon message="光鸭分享不是稳定快照：移动、删除或覆盖云端资源可能让 A 目录或旧目标的已有分享失效。整理后分享会从 B 目录重新创建新链接，不复用旧链接。" class="transfer-alert" />
      <a-checkbox v-if="mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite'" v-model:checked="mappingForm.share_risk_acknowledged" class="risk-check">我已了解移动/覆盖会使旧分享失效</a-checkbox>
      <div class="switch-list">
        <label><span><strong>扫描已有内容</strong><small>创建任务后立即检查云盘 A 目录中的一级项目</small></span><a-switch v-model:checked="mappingForm.scan_existing" /></label>
        <label><span><strong>刮削元数据（默认关闭）</strong><small>开启后仅执行下方选中的类型，不会全量刮削</small></span><a-switch :checked="mappingForm.scrape" @change="toggleScrape" /></label>
        <a-form-item v-if="mappingForm.scrape" label="刮削类型" class="scrape-types">
          <a-select v-model:value="mappingForm.scrape_types" mode="multiple" :options="organizer.settings.scrape_type_options || []" placeholder="至少选择一种元数据" />
        </a-form-item>
        <label><span><strong>同步字幕与外置音轨</strong><small>同名或同季集的字幕、音轨会跟随主视频命名</small></span><a-switch v-model:checked="mappingForm.sync_extras" /></label>
        <label><span><strong>识别成功后自动执行</strong><small>关闭时任务停在“待执行”，确认目标路径后再整理</small></span><a-switch v-model:checked="mappingForm.auto_execute" /></label>
        <label><span><strong>整理后重新分享并投稿 HDHive</strong><small>先完成 B 目录落库，再从 B 目录创建新分享；不会先分享 A 目录</small></span><a-switch v-model:checked="mappingForm.share_after_organize" /></label>
        <label><span><strong>启用云盘目录监控</strong><small>关闭时仅保存配置，不轮询 A 目录</small></span><a-switch v-model:checked="mappingForm.enabled" /></label>
      </div>
    </a-form>
    <template #footer>
      <div class="drawer-footer"><a-button @click="mappingDrawer.open = false">取消</a-button><a-button type="primary" :loading="mappingSubmitting" @click="submitMapping">保存监控</a-button></div>
    </template>
  </a-drawer>

  <a-modal v-model:open="targetModal.open" :title="targetModal.editingId ? '编辑刮削目标' : '添加刮削目标'" ok-text="保存" cancel-text="取消" @ok="saveTarget">
    <a-form layout="vertical">
      <a-form-item label="目标名称" required><a-input v-model:value="targetModal.name" placeholder="例如：电影媒体库" /></a-form-item>
      <a-form-item label="云盘目录" required>
        <a-input-group compact><a-input v-model:value="targetModal.path" readonly style="width: calc(100% - 92px)" /><a-button style="width: 92px" @click="chooseScrapeTargetFolder"><FolderOpenOutlined />选择</a-button></a-input-group>
        <small class="field-help">目录 ID：{{ targetModal.dir_id || '未选择' }}</small>
      </a-form-item>
    </a-form>
  </a-modal>

  <a-modal v-model:open="cloudFolderPicker.open" :title="cloudFolderPicker.kind === 'source' ? '选择来源 A 目录' : cloudFolderPicker.kind === 'scrape-target' ? '选择刮削目标目录' : '选择目标 B 目录'" width="min(720px, 94vw)" :confirm-loading="cloudFolderPicker.loading" @ok="chooseCloudFolder">
    <a-alert type="info" show-icon message="只能选择光鸭云盘内的文件夹，不能选择根目录；A 与 B 不能相同或互相包含。" class="drawer-alert" />
    <a-flex align="center" gap="small" class="cloud-picker-nav">
      <a-button :disabled="cloudFolderPicker.path.length <= 1" @click="leaveCloudFolder"><ArrowLeftOutlined /></a-button>
      <a-breadcrumb>
        <a-breadcrumb-item v-for="(part, index) in cloudFolderPicker.path" :key="part.id || 'root'">
          <a-button type="link" size="small" :disabled="index === cloudFolderPicker.path.length - 1" @click="jumpToCloudFolder(index)">{{ part.name }}</a-button>
        </a-breadcrumb-item>
      </a-breadcrumb>
    </a-flex>
    <a-table :columns="cloudFolderColumns" :data-source="cloudFolderPicker.items" :loading="cloudFolderPicker.loading" :row-key="(record) => String(fileId(record))" :pagination="{ current: cloudFolderPicker.page + 1, pageSize: 100, total: cloudFolderPicker.total, showSizeChanger: false }" size="small" @change="handleCloudFolderTableChange">
      <template #emptyText><a-empty description="当前目录没有子文件夹；可以直接选择当前目录" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'"><a-space><FolderOutlined />{{ record.fileName || record.name }}</a-space></template>
        <template v-else-if="column.key === 'actions'"><a-button type="link" size="small" @click="enterCloudFolder(record)">进入</a-button></template>
      </template>
    </a-table>
    <p class="picker-current">当前选择：/{{ cloudFolderPicker.path.slice(1).map((item) => item.name).join('/') }}</p>
  </a-modal>

  <a-modal v-model:open="review.open" title="人工确认媒体信息" width="min(820px, 94vw)" :closable="!jobBusy[review.job?.id]" :mask-closable="false">
    <a-alert v-if="review.job?.message" type="warning" show-icon :message="review.job.message" class="review-alert" />
    <section v-if="reviewCandidates.length" class="candidate-section">
      <div class="candidate-heading"><strong>TMDB 候选</strong><span>选择后仍可修改下方季集信息</span></div>
      <div class="candidate-grid">
        <button
          v-for="candidate in reviewCandidates"
          :key="`${candidate.media_type}:${candidate.tmdb_id}`"
          type="button"
          class="candidate-card"
          :class="{ selected: Number(review.tmdb_id) === Number(candidate.tmdb_id) }"
          :aria-pressed="Number(review.tmdb_id) === Number(candidate.tmdb_id)"
          @click="chooseCandidate(candidate)"
        >
          <img v-if="candidate.poster_url" :src="candidate.poster_url" :alt="`${candidate.title} 海报`" loading="lazy" referrerpolicy="no-referrer" />
          <div v-else class="poster-placeholder"><FolderOpenOutlined /></div>
          <span><strong>{{ candidate.title }}</strong><small>{{ candidate.year || '年份未知' }} · TMDB {{ candidate.tmdb_id }}</small><small>匹配度 {{ candidateScore(candidate) }}</small></span>
          <CheckCircleOutlined v-if="Number(review.tmdb_id) === Number(candidate.tmdb_id)" />
        </button>
      </div>
    </section>
    <a-form layout="vertical">
      <div class="review-grid">
        <a-form-item label="媒体名称" class="review-title"><a-input v-model:value="review.title" placeholder="用于重新搜索 TMDB" /></a-form-item>
        <a-form-item label="年份"><a-input-number v-model:value="review.year" :min="1800" :max="2200" :precision="0" style="width: 100%" /></a-form-item>
        <a-form-item label="媒体类型"><a-select v-model:value="review.media_type" :options="[{ label: '自动识别', value: '' }, { label: '电影', value: 'movie' }, { label: '电视剧', value: 'tv' }]" /></a-form-item>
        <a-form-item label="TMDB ID"><a-input-number v-model:value="review.tmdb_id" :min="1" :precision="0" placeholder="可直接指定" style="width: 100%" /></a-form-item>
        <a-form-item label="季号"><a-input-number v-model:value="review.season" :min="0" :precision="0" placeholder="电视剧可填写" style="width: 100%" /></a-form-item>
        <a-form-item label="集号"><a-input-number v-model:value="review.episode" :min="0" :precision="0" placeholder="单文件可填写" style="width: 100%" /></a-form-item>
        <a-form-item label="结束集号"><a-input-number v-model:value="review.episode_end" :min="0" :precision="0" placeholder="多集文件可填写" style="width: 100%" /></a-form-item>
      </div>
    </a-form>
    <template #footer>
      <a-button @click="review.open = false">取消</a-button>
      <a-button :loading="jobBusy[review.job?.id]" @click="submitReview(false)">仅重新识别</a-button>
      <a-button type="primary" :loading="jobBusy[review.job?.id]" @click="submitReview(true)">识别并整理</a-button>
    </template>
  </a-modal>

  <a-modal v-model:open="preview.open" title="光鸭原生整理预览" width="min(960px, 96vw)" :footer="null">
    <div v-if="preview.job?.preview?.metadata" class="matched-media">
      <img v-if="preview.job.preview.metadata.poster_url" :src="preview.job.preview.metadata.poster_url" :alt="`${preview.job.preview.metadata.title} 海报`" referrerpolicy="no-referrer" />
      <div><span>{{ organizerMediaLabel(preview.job.preview.metadata.media_type) }} · TMDB {{ preview.job.preview.metadata.tmdb_id }}</span><strong>{{ organizerMatchedTitle(preview.job) }}</strong><p>{{ preview.job.preview.metadata.overview || 'TMDB 暂无简介' }}</p></div>
    </div>
    <div v-if="previewItems.length" class="preview-summary">
      <span><small>计划项目</small><strong>{{ previewSummary.total || previewItems.length }}</strong></span>
      <span><small>可执行</small><strong>{{ previewSummary.success || 0 }}</strong></span>
      <span><small>跳过</small><strong>{{ previewSummary.skipped || 0 }}</strong></span>
      <span><small>提示</small><strong>{{ previewSummary.warnings || 0 }}</strong></span>
    </div>
    <a-alert v-if="preview.job?.preview?.message" :type="preview.job.preview.success ? 'info' : 'warning'" show-icon :message="preview.job.preview.message" class="preview-alert" />
    <a-empty v-if="!previewItems.length" description="没有文件目标；请进入人工确认选择 TMDB 候选" />
    <div v-else class="preview-list">
      <article v-for="(item, index) in previewItems" :key="`${item.target || item.source || ''}:${index}`" :class="{ failed: !item.success, skipped: item.action === 'skip' }">
        <CheckCircleOutlined v-if="item.success && item.action !== 'skip'" />
        <WarningOutlined v-else-if="item.success" />
        <CloseCircleOutlined v-else />
        <div>
          <strong><a-tag>{{ organizerItemKindLabel(item.kind) }}</a-tag>{{ item.message || `项目 ${index + 1}` }}</strong>
          <small v-if="item.source">来源：{{ item.source }}</small>
          <small>目标：{{ item.target || '未生成目标路径' }}</small>
        </div>
        <a-tag :color="!item.success ? 'error' : item.action === 'skip' ? 'warning' : 'success'">{{ organizerItemActionLabel(item) }}</a-tag>
      </article>
    </div>
  </a-modal>
</template>

<style scoped>
.organizer-page { display: grid; gap: 14px; max-width: 1500px; margin: 0 auto; }
.organizer-settings-tabs { margin: 2px 0 -4px; padding: 0 4px; }
.inner-tab { display: inline-flex; align-items: center; gap: 7px; }
.template-preview-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-top: 12px; }
.template-preview-grid article { display: grid; min-width: 0; gap: 5px; padding: 10px 12px; border: 1px solid var(--line, #e5e7eb); border-radius: 9px; background: var(--surface-muted, #f8f9fa); }
.template-preview-grid small, .template-preview-grid span { color: var(--text-3, #667085); font-size: 11px; line-height: 1.45; }
.template-preview-grid code { overflow-wrap: anywhere; color: var(--text-1, #20242c); font-size: 12px; }
.scrape-preference-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
.template-wide { grid-column: 1 / -1; }
.target-panel, .category-rule-panel { padding: 14px; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; }
.target-panel-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
.target-panel-head strong, .target-panel-head small { display: block; }
.target-panel-head small { margin-top: 4px; color: var(--text-3, #98a2b3); font-size: 12px; }
.scrape-target-list, .category-rule-list { display: grid; gap: 8px; }
.scrape-target-card { display: flex; min-width: 0; align-items: center; gap: 9px; padding: 9px 10px; border: 1px solid var(--line, #eef0f3); border-radius: 8px; }
.scrape-target-card > div { min-width: 0; flex: 1; }
.scrape-target-card strong, .scrape-target-card small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.scrape-target-card small { margin-top: 3px; color: var(--text-3, #98a2b3); font-size: 11px; }
.category-rule-row { display: grid; grid-template-columns: auto minmax(110px, .8fr) 110px minmax(180px, 1.5fr) auto; align-items: center; gap: 8px; }
.field-help { color: var(--text-3, #98a2b3); font-size: 11px; }
.page-heading { display: flex; align-items: flex-end; justify-content: space-between; gap: 24px; padding: 12px 4px 6px; }
.eyebrow { color: var(--text-3, #8b8f98); font-size: 10px; font-weight: 700; letter-spacing: .16em; }
.page-heading h1 { margin: 3px 0 2px; font-size: 27px; line-height: 1.2; letter-spacing: -.035em; }
.page-heading p, .section-heading p { margin: 0; color: var(--text-2, #667085); font-size: 12px; }
.heading-actions, .settings-actions, .drawer-footer { display: flex; justify-content: flex-end; gap: 8px; }
.metric-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; }
.metric-grid article { display: grid; min-height: 92px; align-content: center; padding: 14px 16px; border: 1px solid var(--line, #e4e7ec); border-radius: 12px; background: var(--surface, #fff); }
.metric-grid small, .metric-grid span { color: var(--text-3, #8b8f98); font-size: 10px; }
.metric-grid strong { margin: 2px 0; font-size: 25px; line-height: 1; font-variant-numeric: tabular-nums; }
.metric-grid .attention { border-color: #ffd591; background: color-mix(in srgb, #faad14 8%, var(--surface, #fff)); }
.settings-card, .section-block { border: 1px solid var(--line, #e4e7ec); border-radius: 12px; background: var(--surface, #fff); }
.settings-card :deep(.ant-card-head) { min-height: 48px; }
.card-title { display: inline-flex; align-items: center; gap: 8px; }
.settings-alert, .drawer-alert, .review-alert, .preview-alert { margin-bottom: 14px; }
.settings-primary { display: grid; grid-template-columns: minmax(260px, 1.35fr) minmax(150px, .7fr) minmax(170px, .8fr) 150px auto; align-items: end; gap: 12px; }
.settings-primary :deep(.ant-form-item), .template-grid :deep(.ant-form-item) { margin: 0; }
.naming-collapse { margin-top: 10px; border-top: 1px solid var(--line-soft, #edf0f3); }
.naming-collapse :deep(.ant-collapse-header) { padding-inline: 0 !important; color: var(--text-2, #667085) !important; font-size: 12px; }
.naming-collapse :deep(.ant-collapse-content-box) { padding: 2px 0 0 !important; }
.template-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.template-wide { grid-column: 1 / -1; }
.template-help { margin: 10px 0 0; color: var(--text-3, #8b8f98); font-size: 10px; word-break: break-all; }
.adult-setting { display: flex; align-items: center; justify-content: space-between; gap: 20px; min-height: 54px; padding: 0 2px; }
.adult-setting span { display: grid; gap: 2px; }
.adult-setting small { color: var(--text-3, #8b8f98); font-size: 10px; }
.section-block { padding: 16px; }
.section-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin-bottom: 14px; }
.section-heading h2 { margin: 0 0 2px; font-size: 16px; }
.mapping-list { display: grid; gap: 8px; }
.mapping-card { display: grid; grid-template-columns: 42px minmax(0, 1fr) auto; align-items: center; gap: 12px; padding: 12px; border: 1px solid var(--line-soft, #edf0f3); border-radius: 10px; background: var(--surface-muted, #fafbfc); }
.mapping-icon { display: grid; width: 38px; height: 38px; place-items: center; border-radius: 9px; color: var(--text-1, #344054); background: var(--surface-strong, #eceef1); font-size: 18px; }
.mapping-copy { display: grid; min-width: 0; gap: 3px; }
.mapping-title-row { display: flex; min-width: 0; align-items: center; gap: 8px; }
.mapping-title-row strong, .path-flow { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.path-flow { color: var(--text-2, #667085); font-size: 11px; }
.mapping-meta { display: flex; flex-wrap: wrap; gap: 5px 12px; margin-top: 4px; color: var(--text-3, #8b8f98); font-size: 10px; }
.mapping-actions { display: flex; align-items: center; gap: 7px; }
.jobs-block { padding-bottom: 8px; }
.jobs-block :deep(.ant-table-wrapper) { margin-inline: -8px; }
.job-source, .job-result { display: grid; min-width: 0; gap: 2px; }
.job-source strong, .job-result strong, .job-source span, .job-result span, .job-result small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.job-source span, .job-result span, .job-source small, .job-result small { color: var(--text-3, #8b8f98); font-size: 10px; }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.transfer-alert { margin: 0 0 10px; }
.risk-check { margin: 0 0 12px; color: var(--text-2, #667085); }
.switch-list { display: grid; border-top: 1px solid var(--line, #e4e7ec); }
.switch-list label { display: flex; align-items: center; justify-content: space-between; gap: 20px; padding: 12px 0; border-bottom: 1px solid var(--line-soft, #edf0f3); }
.switch-list label span { display: grid; gap: 2px; }
.switch-list small { color: var(--text-3, #8b8f98); font-size: 11px; }
.scrape-types { margin: 0; padding: 10px 0 12px; border-bottom: 1px solid var(--line-soft, #edf0f3); }
.cloud-picker-nav { min-height: 34px; margin-bottom: 10px; overflow: auto; }
.cloud-picker-nav :deep(.ant-breadcrumb) { min-width: max-content; }
.picker-current { margin: 10px 0 0; color: var(--text-2, #667085); font-size: 11px; word-break: break-all; }
.candidate-section { margin-bottom: 16px; }
.candidate-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
.candidate-heading span { color: var(--text-3, #8b8f98); font-size: 11px; }
.candidate-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; max-height: 280px; overflow: auto; }
.candidate-card { position: relative; display: grid; grid-template-columns: 52px minmax(0, 1fr) 20px; align-items: center; gap: 10px; min-width: 0; padding: 7px; border: 1px solid var(--line, #e4e7ec); border-radius: 10px; color: inherit; background: var(--surface, #fff); text-align: left; cursor: pointer; }
.candidate-card:hover { border-color: #91caff; }
.candidate-card:focus-visible { outline: 2px solid #1677ff; outline-offset: 2px; }
.candidate-card.selected { border-color: #1677ff; background: color-mix(in srgb, #1677ff 7%, var(--surface, #fff)); }
.candidate-card img, .poster-placeholder { width: 52px; height: 70px; border-radius: 6px; object-fit: cover; background: var(--surface-muted, #f2f4f7); }
.poster-placeholder { display: grid; place-items: center; color: var(--text-3, #98a2b3); font-size: 18px; }
.candidate-card > span { display: grid; min-width: 0; gap: 2px; }
.candidate-card strong, .candidate-card small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.candidate-card small { color: var(--text-3, #8b8f98); font-size: 10px; }
.candidate-card > :deep(.anticon) { color: #1677ff; }
.review-grid { display: grid; grid-template-columns: 1.4fr repeat(3, minmax(110px, .7fr)); gap: 12px; }
.review-title { grid-column: span 2; }
.matched-media { display: grid; grid-template-columns: 74px minmax(0, 1fr); gap: 12px; margin-bottom: 14px; padding: 10px; border: 1px solid var(--line-soft, #edf0f3); border-radius: 10px; background: var(--surface-muted, #fafbfc); }
.matched-media img { width: 74px; height: 106px; border-radius: 7px; object-fit: cover; }
.matched-media div { display: grid; min-width: 0; align-content: center; gap: 3px; }
.matched-media span { color: var(--text-3, #8b8f98); font-size: 10px; }
.matched-media strong { font-size: 17px; }
.matched-media p { display: -webkit-box; margin: 2px 0 0; overflow: hidden; color: var(--text-2, #667085); font-size: 11px; -webkit-box-orient: vertical; -webkit-line-clamp: 3; }
.preview-summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-bottom: 12px; }
.preview-summary span { display: grid; padding: 8px 10px; border: 1px solid var(--line-soft, #edf0f3); border-radius: 8px; background: var(--surface-muted, #fafbfc); }
.preview-summary small { color: var(--text-3, #8b8f98); font-size: 9px; }
.preview-summary strong { font-size: 17px; font-variant-numeric: tabular-nums; }
.preview-list { display: grid; gap: 8px; max-height: 54vh; overflow: auto; }
.preview-list article { display: grid; grid-template-columns: 28px minmax(0, 1fr) auto; align-items: center; gap: 10px; padding: 11px; border: 1px solid #b7eb8f; border-radius: 9px; background: color-mix(in srgb, #52c41a 7%, var(--surface, #fff)); }
.preview-list article.failed { border-color: #ffccc7; background: color-mix(in srgb, #ff4d4f 7%, var(--surface, #fff)); }
.preview-list article.skipped { border-color: #ffe58f; background: color-mix(in srgb, #faad14 7%, var(--surface, #fff)); }
.preview-list article > :deep(.anticon) { color: #389e0d; font-size: 18px; }
.preview-list article.failed > :deep(.anticon) { color: #cf1322; }
.preview-list article.skipped > :deep(.anticon) { color: #d48806; }
.preview-list article div { display: grid; min-width: 0; gap: 2px; }
.preview-list article div strong { display: flex; align-items: center; gap: 6px; }
.preview-list small { overflow: hidden; color: var(--text-2, #667085); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
@media (max-width: 1180px) {
  .settings-primary { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .settings-actions { grid-column: 1 / -1; }
}
@media (max-width: 980px) {
  .metric-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .mapping-card { grid-template-columns: 38px minmax(0, 1fr); }
  .mapping-actions { grid-column: 2; justify-content: flex-end; }
  .review-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .review-title { grid-column: span 2; }
  .scrape-preference-grid, .template-preview-grid { grid-template-columns: 1fr; }
  .category-rule-row { grid-template-columns: auto minmax(0, 1fr) auto; }
  .category-rule-row .ant-select, .category-rule-row .ant-input { grid-column: span 2; }
}
@media (max-width: 640px) {
  .page-heading { align-items: flex-start; flex-direction: column; }
  .heading-actions { width: 100%; }
  .heading-actions .ant-btn { flex: 1; }
  .metric-grid, .settings-primary, .template-grid, .form-grid, .candidate-grid, .review-grid { grid-template-columns: 1fr; }
  .template-wide, .review-title { grid-column: 1; }
  .mapping-card { grid-template-columns: 1fr; }
  .mapping-icon { display: none; }
  .mapping-actions { grid-column: 1; justify-content: flex-start; }
  .preview-summary { grid-template-columns: repeat(2, 1fr); }
  .matched-media { grid-template-columns: 58px minmax(0, 1fr); }
  .matched-media img { width: 58px; height: 84px; }
}
</style>
