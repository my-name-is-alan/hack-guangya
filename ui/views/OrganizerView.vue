<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, toRaw } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  ArrowLeftOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  EyeOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  PlayCircleOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  ShareAltOutlined,
  SettingOutlined,
  WarningOutlined,
} from '@antdv-next/icons';
import PageHeader from '../components/layout/PageHeader.vue';
import { bridge } from '../bridge.js';
import { copyText, errorText, fileId, formatSize, isFolder, unwrapData } from '../formatters.js';
import {
  organizerCandidates,
  organizerConflictLabel,
  organizerItemActionLabel,
  organizerItemKindLabel,
  organizerMatchedTitle,
  organizerMediaLabel,
  normalizeOrganizerCategoryFormRules,
  organizerPreviewItems,
  organizerPreviewTarget,
  organizerStatus,
  organizerStatusMeta,
  organizerTemplateExamples,
  organizerTransferLabel,
  ORGANIZER_MOVIE_TEMPLATE_TOKENS,
  ORGANIZER_TV_TEMPLATE_TOKENS,
  validateOrganizerRuleBlock,
} from '../organizer.js';
import { COUNTRY_OPTIONS_ZH } from '../countries.js';

const props = defineProps({ settingsOnly: { type: Boolean, default: false } });

const DEFAULT_SETTINGS = Object.freeze({
  configured: false,
  language: 'zh-CN',
  image_language: 'zh,null,en',
  include_adult: false,
  minimum_match_score: 0.72,
  word_segment_search: true,
  similarity_match: true,
  recognition_words: '',
  release_groups: '',
  render_words: '',
  capture_groups: '',
  upgrade_criteria: ['resolution', 'dynamic_range', 'release_group', 'size'],
  upgrade_release_groups: '',
  upgrade_criteria_options: [],
  include_media_info: true,
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

const REFERENCE_CATEGORY_RULES = [
  { name: '电影/演唱会', media_type: 'movie', genres: ['10402'] },
  { name: '电影/动画电影', media_type: 'movie', genres: ['16'] },
  { name: '电影/华语电影', media_type: 'movie', original_languages: ['zh', 'cn', 'bo', 'za'] },
  { name: '电影/日韩电影', media_type: 'movie', original_languages: ['ja', 'ko', 'th'] },
  { name: '电影/欧美电影', media_type: 'movie', origin_countries: ['US', 'GB', 'CA', 'AU', 'NZ', 'IE', 'FR', 'DE', 'ES', 'IT', 'NL', 'BE', 'SE', 'NO', 'DK', 'FI', 'IS', 'AT', 'CH', 'PT', 'GR', 'PL', 'CZ', 'SK', 'HU', 'RO', 'BG', 'HR', 'SI', 'RS', 'ME', 'MK', 'AL', 'BA', 'EE', 'LV', 'LT', 'LU', 'MT', 'CY'] },
  { name: '电视剧/动漫/儿童', media_type: 'tv', genres: ['10762'] },
  { name: '电视剧/动漫/国漫', media_type: 'tv', genres: ['16'], origin_countries: ['CN', 'TW', 'HK'] },
  { name: '电视剧/动漫/日番', media_type: 'tv', genres: ['16'], origin_countries: ['JP'] },
  { name: '电视剧/动漫/欧美动漫', media_type: 'tv', genres: ['16'], origin_countries: ['US', 'GB', 'CA', 'AU', 'FR', 'DE', 'IT', 'ES', 'NL', 'BE', 'SE', 'NO', 'DK', 'FI'] },
  { name: '电视剧/动漫/其他', media_type: 'tv', genres: ['16', '10762'] },
  { name: '电视剧/纪录片', media_type: 'tv', genres: ['99'] },
  { name: '电视剧/综艺', media_type: 'tv', genres: ['10764', '10767'] },
  { name: '电视剧/亚洲剧/国产剧', media_type: 'tv', origin_countries: ['CN'] },
  { name: '电视剧/亚洲剧/港台剧集', media_type: 'tv', origin_countries: ['TW', 'HK'] },
  { name: '电视剧/亚洲剧/日韩剧', media_type: 'tv', origin_countries: ['JP', 'KR', 'KP', 'TH', 'IN', 'SG'] },
  { name: '电视剧/欧美剧', media_type: 'tv', origin_countries: ['US', 'GB', 'CA', 'AU', 'NZ', 'IE', 'FR', 'DE', 'ES', 'IT', 'NL', 'BE', 'SE', 'NO', 'DK', 'FI', 'IS', 'AT', 'CH', 'PT', 'GR', 'PL', 'CZ', 'SK', 'HU', 'RO', 'BG', 'HR', 'SI', 'RS', 'ME', 'MK', 'AL', 'BA', 'EE', 'LV', 'LT', 'LU', 'MT', 'CY'] },
].map((rule, index) => ({ id: `reference-category-${index + 1}`, genres: [], original_languages: [], origin_countries: [], enabled: true, ...rule }));

const loading = ref(true);
const refreshing = ref(false);
const settingsSaving = ref(false);
const settingsTesting = ref(false);
const mappingSubmitting = ref(false);
const settingsPanels = ref([]);
const movieTemplateInput = ref(null);
const tvTemplateInput = ref(null);
const recognitionSection = ref('recognition_words');
const settingsSection = ref('directories');
const targetModal = reactive({ open: false, editingId: '', name: '', dir_id: '', path: '/' });
const jobBusy = reactive({});
const deleteDialog = reactive({ open: false, job: null });
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
  mode: 'review',
  job: null,
  media_type: '',
  tmdb_id: '',
  title: '',
  year: '',
  season: '',
  episode: '',
  episode_end: '',
  episode_offset: '',
  recognition_words: '',
});
const selectedJobIds = ref([]);
const batchBusy = ref(false);
const batchProgress = reactive({ done: 0, total: 0 });
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
const scrapeTargetOptions = computed(() => (organizer.settings.scrape_targets || []).map((target) => ({
  label: `${target.name || '媒体库'} · ${target.path || '/'}`,
  value: String(target.dir_id || target.target_dir_id || ''),
})).filter((item) => item.value));
const mappingTargetConfigured = computed(() => scrapeTargetOptions.value.some((item) => item.value === String(mappingForm.target_dir_id || '')));
const globalRuleSummary = computed(() => {
  const settings = organizer.settings || DEFAULT_SETTINGS;
  const auxiliary = ['recognition_words', 'release_groups', 'render_words', 'capture_groups']
    .filter((key) => String(settings[key] || '').split(/\r?\n/).some((line) => line.trim() && !line.trim().startsWith('#'))).length;
  const categoryCount = (settings.category_rules || []).filter((rule) => rule.enabled !== false).length;
  const search = [
    settings.word_segment_search !== false ? '分词搜索' : '',
    settings.similarity_match !== false ? `相似度 ≥ ${Number(settings.minimum_match_score ?? 0.72).toFixed(2)}` : '仅精确匹配',
  ].filter(Boolean).join(' · ');
  return { auxiliary, categoryCount, search };
});
const templateExamples = computed(() => organizerTemplateExamples(
  settingsForm.movie_path_template,
  settingsForm.tv_path_template,
  settingsForm.movie_category,
  settingsForm.tv_category,
  settingsForm.include_media_info,
));

function cloneSerializable(value) {
  const raw = toRaw(value);
  // These settings are JSON data. Serializing after unwrapping the top-level
  // Vue proxy guarantees that no reactive proxy reaches Tauri's clone bridge.
  return JSON.parse(JSON.stringify(raw));
}

function organizerSettingsInput({ validate = false } = {}) {
  const input = {
    api_key: String(settingsForm.api_key || ''),
    language: String(settingsForm.language || ''),
    image_language: String(settingsForm.image_language || ''),
    include_adult: Boolean(settingsForm.include_adult),
    minimum_match_score: Number(settingsForm.minimum_match_score ?? DEFAULT_SETTINGS.minimum_match_score),
    word_segment_search: Boolean(settingsForm.word_segment_search),
    similarity_match: Boolean(settingsForm.similarity_match),
    recognition_words: String(settingsForm.recognition_words || ''),
    release_groups: String(settingsForm.release_groups || ''),
    render_words: String(settingsForm.render_words || ''),
    capture_groups: String(settingsForm.capture_groups || ''),
    upgrade_criteria: cloneSerializable(settingsForm.upgrade_criteria || []),
    upgrade_release_groups: String(settingsForm.upgrade_release_groups || ''),
    include_media_info: Boolean(settingsForm.include_media_info),
    movie_path_template: String(settingsForm.movie_path_template || ''),
    tv_path_template: String(settingsForm.tv_path_template || ''),
    movie_category: String(settingsForm.movie_category || ''),
    tv_category: String(settingsForm.tv_category || ''),
    tmdb_api_base: String(settingsForm.tmdb_api_base || ''),
    tmdb_image_base: String(settingsForm.tmdb_image_base || ''),
    category_rules: cloneSerializable(settingsForm.category_rules),
    scrape_targets: cloneSerializable(settingsForm.scrape_targets),
    default_scrape_types: cloneSerializable(settingsForm.default_scrape_types || []),
  };
  if (validate) {
    input.recognition_words = validateOrganizerRuleBlock(input.recognition_words, '自定义识别词');
    input.render_words = validateOrganizerRuleBlock(input.render_words, '自定义渲染词');
    input.capture_groups = validateOrganizerRuleBlock(input.capture_groups, '自定义捕获组', { replacement: false });
    input.category_rules = normalizeOrganizerCategoryFormRules(input.category_rules);
  }
  return input;
}

const jobColumns = [
  { title: '来源', key: 'source', width: 230 },
  { title: '识别与目标', key: 'result', width: 300 },
  { title: '状态', key: 'status', width: 108 },
  { title: '更新时间', key: 'time', width: 142 },
  { title: '操作', key: 'actions', width: 330, fixed: 'right' },
];

const jobFilters = reactive({ status: 'all', keyword: '' });
const jobStatusOptions = [
  { value: 'all', label: '全部状态' },
  ...Object.entries(organizerStatusMeta).map(([value, meta]) => ({ value, label: meta.label })),
];
const filteredJobs = computed(() => {
  const keyword = jobFilters.keyword.trim().toLowerCase();
  return (organizer.jobs || []).filter((job) => {
    if (jobFilters.status !== 'all' && job.status !== jobFilters.status) return false;
    if (!keyword) return true;
    return [job.source_path, job.query_title, job.message, organizerMatchedTitle(job)]
      .some((value) => String(value || '').toLowerCase().includes(keyword));
  });
});

const JOB_DELETE_ACTIONS = Object.freeze([
  { key: 'history', label: '仅删除历史记录', delete_source: false, delete_target: false },
  { key: 'source', label: '删除历史记录和源文件', delete_source: true, delete_target: false },
  { key: 'target', label: '删除历史记录和媒体库文件', delete_source: false, delete_target: true },
  { key: 'all', label: '删除历史记录、源文件和媒体库文件', delete_source: true, delete_target: true },
]);

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
    word_segment_search: settings.word_segment_search !== false,
    similarity_match: settings.similarity_match !== false,
    recognition_words: String(settings.recognition_words || ''),
    release_groups: String(settings.release_groups || ''),
    render_words: String(settings.render_words || ''),
    capture_groups: String(settings.capture_groups || ''),
    upgrade_criteria: Array.isArray(settings.upgrade_criteria) && settings.upgrade_criteria.length
      ? [...settings.upgrade_criteria]
      : [...DEFAULT_SETTINGS.upgrade_criteria],
    upgrade_release_groups: String(settings.upgrade_release_groups || ''),
    upgrade_criteria_options: Array.isArray(settings.upgrade_criteria_options) && settings.upgrade_criteria_options.length
      ? cloneSerializable(settings.upgrade_criteria_options)
      : [
        { value: 'resolution', label: '分辨率' },
        { value: 'dynamic_range', label: '动态范围' },
        { value: 'release_group', label: '制作组' },
        { value: 'size', label: '文件大小' },
      ],
    include_media_info: settings.include_media_info !== false,
    movie_path_template: settings.movie_path_template || DEFAULT_SETTINGS.movie_path_template,
    tv_path_template: settings.tv_path_template || DEFAULT_SETTINGS.tv_path_template,
    movie_category: settings.movie_category || DEFAULT_SETTINGS.movie_category,
    tv_category: settings.tv_category || DEFAULT_SETTINGS.tv_category,
    tmdb_api_base: settings.tmdb_api_base || DEFAULT_SETTINGS.tmdb_api_base,
    tmdb_image_base: settings.tmdb_image_base || DEFAULT_SETTINGS.tmdb_image_base,
    category_rules: Array.isArray(settings.category_rules) ? cloneSerializable(settings.category_rules).map((rule, index) => ({
      id: rule.id || `category-${index + 1}`,
      name: rule.name || '',
      media_type: rule.media_type || 'all',
      genres: Array.isArray(rule.genres) ? rule.genres.map(String) : [],
      original_languages: Array.isArray(rule.original_languages) ? rule.original_languages.map(String) : [],
      origin_countries: Array.isArray(rule.origin_countries) ? rule.origin_countries.map((value) => String(value).toUpperCase()) : [],
      enabled: rule.enabled !== false,
    })) : [],
    scrape_targets: Array.isArray(settings.scrape_targets) ? cloneSerializable(settings.scrape_targets) : [],
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
    const input = organizerSettingsInput({ validate: true });
    const saved = await bridge.invoke('update_organizer_settings', { input });
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
    const input = organizerSettingsInput();
    const result = await bridge.invoke('test_organizer_connection', { input });
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
  if (scrapeTargetOptions.value.length === 1) selectMappingOutputTarget(scrapeTargetOptions.value[0].value);
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

function selectMappingOutputTarget(value) {
  const target = (organizer.settings.scrape_targets || []).find((item) => String(item.dir_id || item.target_dir_id || '') === String(value || ''));
  mappingForm.target_dir_id = target ? String(target.dir_id || target.target_dir_id || '') : '';
  mappingForm.target_path = target ? String(target.path || target.target_path || '/') : '';
}

function configureScrapeTargets() {
  mappingDrawer.open = false;
  settingsSection.value = 'scrape';
}

function mappingOutputName(mapping) {
  const target = (organizer.settings.scrape_targets || []).find((item) => String(item.dir_id || item.target_dir_id || '') === String(mapping.target_dir_id || ''));
  return target?.name || '旧版自定义目标';
}

function applyPathPreset(value) {
  const preset = (organizer.settings.path_presets || []).find((item) => item.id === value);
  if (!preset) return;
  settingsForm.movie_path_template = preset.movie;
  settingsForm.tv_path_template = preset.tv;
}

function insertTemplateToken(kind, token) {
  const field = kind === 'tv' ? 'tv_path_template' : 'movie_path_template';
  const component = kind === 'tv' ? tvTemplateInput.value : movieTemplateInput.value;
  const textarea = component?.resizableTextArea?.textArea || component?.$el?.querySelector?.('textarea');
  const current = String(settingsForm[field] || '');
  const start = Number.isInteger(textarea?.selectionStart) ? textarea.selectionStart : current.length;
  const end = Number.isInteger(textarea?.selectionEnd) ? textarea.selectionEnd : start;
  settingsForm[field] = `${current.slice(0, start)}${token}${current.slice(end)}`;
  void nextTick(() => {
    textarea?.focus();
    textarea?.setSelectionRange(start + token.length, start + token.length);
  });
}

function addCategoryRule() {
  settingsForm.category_rules.push({
    id: `category-${Date.now()}-${settingsForm.category_rules.length + 1}`,
    name: '',
    media_type: 'all',
    genres: [],
    original_languages: [],
    origin_countries: [],
    enabled: true,
  });
}

function removeCategoryRule(rule) {
  settingsForm.category_rules = settingsForm.category_rules.filter((item) => item.id !== rule.id);
}

function moveCategoryRule(index, offset) {
  const target = index + offset;
  if (target < 0 || target >= settingsForm.category_rules.length) return;
  const [rule] = settingsForm.category_rules.splice(index, 1);
  settingsForm.category_rules.splice(target, 0, rule);
}

function applyReferenceCategoryRules() {
  const replace = async () => {
    settingsSaving.value = true;
    try {
      const categoryRules = normalizeOrganizerCategoryFormRules(cloneSerializable(REFERENCE_CATEGORY_RULES));
      const saved = await bridge.invoke('update_organizer_settings', { input: { category_rules: categoryRules } });
      organizer.settings = { ...DEFAULT_SETTINGS, ...(saved || {}) };
      hydrateSettings(organizer.settings, true);
      await loadState({ silent: true });
      message.success('默认二级分类已恢复并保存');
    } catch (error) {
      message.error(errorText(error));
      throw error;
    } finally {
      settingsSaving.value = false;
    }
  };
  if (!settingsForm.category_rules.length) {
    void replace().catch(() => {});
    return;
  }
  Modal.confirm({
    title: '恢复默认二级分类',
    content: '这会立即替换并保存当前分类规则；目录监控、命名配置和刮削目标不会变化。',
    okText: '恢复并保存',
    cancelText: '取消',
    onOk: replace,
  });
}

function openNewTarget() {
  Object.assign(targetModal, { open: true, editingId: '', name: '', dir_id: '', path: '/' });
}

function openEditTarget(target) {
  Object.assign(targetModal, { open: true, editingId: target.id || '', name: target.name || '', dir_id: target.dir_id || target.target_dir_id || '', path: target.path || target.target_path || '/' });
}

function removeTarget(target) {
  const dirId = String(target.dir_id || target.target_dir_id || '');
  const usedBy = organizer.mappings.find((mapping) => String(mapping.target_dir_id || '') === dirId);
  if (usedBy) {
    message.warning(`该输出仍被监控 ${usedBy.source_path} 使用，请先修改监控的输出媒体库`);
    return;
  }
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
    message.warning('请从刮削输出中选择媒体库目标');
    return;
  }
  if (!mappingTargetConfigured.value) {
    message.warning('当前 B 目录不在“刮削输出”目标中，请重新选择');
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
  if ((mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite' || mappingForm.conflict_policy === 'upgrade') && !mappingForm.share_risk_acknowledged) {
    message.warning('请先确认移动/覆盖/洗版导致旧分享失效的风险');
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

function removeJob(job, actionKey = 'history') {
  const action = JOB_DELETE_ACTIONS.find((item) => item.key === actionKey) || JOB_DELETE_ACTIONS[0];
  const affected = action.delete_source && action.delete_target
    ? '源文件和本次整理生成的媒体库文件都会永久删除，已有分享可能失效。'
    : action.delete_source
      ? '源文件会永久删除，引用该源文件的已有分享可能失效；媒体库文件不受影响。'
      : action.delete_target
        ? '本次整理生成的媒体库文件会永久删除，媒体库分享可能失效；源文件不受影响。'
        : '源文件和媒体库文件都不会被删除。';
  Modal.confirm({
    title: action.label,
    content: `确定处理「${fileName(job.source_path)}」吗？${affected}`,
    okText: action.label,
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      jobBusy[job.id] = true;
      try {
        const result = await bridge.invoke('remove_organizer_job', {
          id: job.id,
          input: { delete_source: action.delete_source, delete_target: action.delete_target },
        });
        if (review.job?.id === job.id) review.open = false;
        if (preview.job?.id === job.id) preview.open = false;
        await loadState({ silent: true });
        const deletedSource = Number(result?.deleted_source || 0);
        const deletedTarget = Number(result?.deleted_target || 0);
        const summary = [deletedSource ? `源文件 ${deletedSource} 项` : '', deletedTarget ? `媒体库文件 ${deletedTarget} 项` : ''].filter(Boolean).join('、');
        if (Array.isArray(result?.warnings) && result.warnings.length) message.warning(result.warnings.join('；'));
        else message.success(summary ? `整理记录及${summary}已删除` : '整理记录已删除');
      } catch (error) {
        message.error(errorText(error));
        throw error;
      } finally {
        jobBusy[job.id] = false;
      }
    },
  });
}

function openDeleteActions(job) {
  deleteDialog.job = job;
  deleteDialog.open = true;
}

function chooseDeleteAction(actionKey) {
  const job = deleteDialog.job;
  deleteDialog.open = false;
  if (job) removeJob(job, actionKey);
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

function jobShareUrl(job) {
  const share = job?.result?.share || {};
  return String(share.share_url || share.shareUrl || share.shareURL || share.url || '').trim();
}

async function copyJobShare(job) {
  const url = jobShareUrl(job);
  if (!url) {
    message.warning('当前任务还没有分享链接');
    return;
  }
  await copyText(url, message);
}

function createJobShare(job) {
  const existing = jobShareUrl(job);
  Modal.confirm({
    title: existing ? '重新创建最终媒体目录分享' : '创建最终媒体目录分享',
    content: `将直接分享“${job.preview?.share_title || fileName(job.source_path)}”整理后的最终媒体文件夹，不需要逐层进入分类目录。${existing ? '这会生成一个新链接，旧链接不会复用。' : ''}`,
    okText: existing ? '创建新链接' : '创建分享',
    cancelText: '取消',
    async onOk() {
      jobBusy[`share:${job.id}`] = true;
      try {
        const share = await bridge.invoke('share_organizer_job', { id: job.id });
        await loadState({ silent: true });
        const url = String(share?.share_url || share?.shareUrl || share?.shareURL || share?.url || '').trim();
        if (url) await copyText(url, message);
        else message.success('最终媒体目录分享已创建');
      } catch (error) {
        message.error(errorText(error));
        throw error;
      } finally {
        jobBusy[`share:${job.id}`] = false;
      }
    },
  });
}

function openReview(job, mode = 'review') {
  review.mode = mode;
  review.job = job;
  review.media_type = job.media_type || job.preview?.query?.media_type || '';
  review.tmdb_id = job.tmdb_id ?? '';
  review.title = job.query_title || job.preview?.query?.title || '';
  review.year = job.query_year ?? job.preview?.query?.year ?? '';
  review.season = job.season ?? '';
  review.episode = job.episode ?? '';
  review.episode_end = job.episode_end ?? '';
  review.episode_offset = job.episode_offset ?? '';
  review.recognition_words = job.recognition_words || '';
  review.open = true;
}

function openReorganize(job) {
  openReview(job, 'rearchive');
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
      episode_offset: optionalNumber(review.episode_offset),
      recognition_words: String(review.recognition_words || '').trim() || undefined,
      clear_tmdb_id: optionalNumber(review.tmdb_id) === undefined,
      clear_title: !String(review.title || '').trim(),
      clear_year: optionalNumber(review.year) === undefined,
      clear_season: optionalNumber(review.season) === undefined,
      clear_episode: optionalNumber(review.episode) === undefined,
      clear_episode_end: optionalNumber(review.episode_end) === undefined,
      clear_episode_offset: optionalNumber(review.episode_offset) === undefined,
      clear_recognition_words: !String(review.recognition_words || '').trim(),
    };
    const command = execute
      ? (review.mode === 'rearchive' ? 'rearchive_organizer_job' : 'run_organizer_job')
      : 'retry_organizer_job';
    const result = await bridge.invoke(command, { id: job.id, input });
    review.open = false;
    await loadState({ silent: true });
    if (review.mode === 'rearchive' && result?.status === 'recognizing') message.success('重新归档已提交，正在后台处理');
    else if (result?.status === 'failed' || result?.status === 'needs_review') message.warning(result.message || '仍需人工确认');
    else if (result?.status === 'completed_warning') message.warning(result.message || '整理完成，但有非阻断提示');
    else message.success(execute && result?.status === 'completed' ? (review.mode === 'rearchive' ? '重新归档完成' : '整理完成') : '重新识别完成');
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

const jobRowSelection = computed(() => ({
  selectedRowKeys: selectedJobIds.value,
  onChange: (keys) => { selectedJobIds.value = keys; },
  getCheckboxProps: (record) => ({ disabled: ['recognizing', 'running'].includes(record.status) }),
}));

/**
 * 批量重新识别并整理：走“重新归档”链路（后台识别 → 自动执行；
 * 已落位过的任务先清理旧产物），沿用每个任务已保存的人工修正。
 * 归档提交立即返回，整体进度看顶栏“整理中”指示。
 */
async function batchRetryJobs() {
  const ids = selectedJobIds.value.filter((id) => {
    const job = organizer.jobs.find((item) => item.id === id);
    return job && !['recognizing', 'running'].includes(job.status);
  });
  if (!ids.length) return;
  batchBusy.value = true;
  batchProgress.total = ids.length;
  batchProgress.done = 0;
  const failures = [];
  for (const id of ids) {
    try { await bridge.invoke('rearchive_organizer_job', { id, input: {} }); }
    catch (error) { failures.push(errorText(error)); }
    batchProgress.done += 1;
  }
  batchBusy.value = false;
  selectedJobIds.value = [];
  await loadState({ silent: true });
  if (failures.length) message.warning(`已提交 ${ids.length - failures.length} 个任务重新归档，${failures.length} 个提交失败：${failures[0]}`);
  else message.success(`已提交 ${ids.length} 个任务重新归档，正在后台识别并整理`);
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
  <a-spin class="organizer-spin" :spinning="loading">
    <section class="organizer-page">
      <PageHeader
        v-if="!props.settingsOnly"
        title="媒体识别与整理"
        description="光鸭直接在云盘内完成 A → B 识别整理；不经过 MoviePilot API，也不依赖本地挂载盘搬运。"
      >
        <template #actions>
          <a-button :loading="refreshing" @click="loadState({ silent: true })"><ReloadOutlined />刷新任务</a-button>
        </template>
      </PageHeader>

      <a-tabs v-if="props.settingsOnly" v-model:active-key="settingsSection" class="organizer-settings-tabs">
        <a-tab-pane key="general">
          <template #tab><span class="inner-tab"><SettingOutlined />通用配置</span></template>
        </a-tab-pane>
        <a-tab-pane key="recognition">
          <template #tab><span class="inner-tab"><SearchOutlined />辅助识别</span></template>
        </a-tab-pane>
        <a-tab-pane key="categories">
          <template #tab><span class="inner-tab"><FolderOutlined />二级分类</span></template>
        </a-tab-pane>
        <a-tab-pane key="search">
          <template #tab><span class="inner-tab"><EyeOutlined />搜索设置</span></template>
        </a-tab-pane>
        <a-tab-pane key="directories">
          <template #tab><span class="inner-tab"><FolderOpenOutlined />归档规则</span></template>
        </a-tab-pane>
        <a-tab-pane key="scrape">
          <template #tab><span class="inner-tab"><PlayCircleOutlined />刮削输出</span></template>
        </a-tab-pane>
      </a-tabs>

      <a-card v-if="props.settingsOnly && settingsSection === 'general'" class="settings-card" :bordered="false">
        <template #title><span class="card-title"><SettingOutlined />通用配置</span></template>
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
          <div class="settings-actions">
            <a-button :loading="settingsTesting" @click="testSettings">测试 TMDB</a-button>
            <a-button type="primary" :loading="settingsSaving" @click="saveSettings">保存设置</a-button>
          </div>
        </div>
        <a-collapse v-model:activeKey="settingsPanels" ghost class="naming-collapse">
          <a-collapse-panel key="naming" header="全局电影 / 电视剧命名规则">
            <div class="template-grid">
              <a-form-item label="套用模板预设" class="template-wide"><a-select placeholder="选择后仍可自由修改" :options="pathPresetOptions" @change="applyPathPreset" /></a-form-item>
              <a-form-item label="电影分类值"><a-input v-model:value="settingsForm.movie_category" /></a-form-item>
              <a-form-item label="电视剧分类值"><a-input v-model:value="settingsForm.tv_category" /></a-form-item>
              <a-form-item label="电影完整相对路径" class="template-wide">
                <a-textarea ref="movieTemplateInput" v-model:value="settingsForm.movie_path_template" :auto-size="{ minRows: 2, maxRows: 4 }" />
                <div class="template-inline-preview" aria-live="polite">
                  <small>实时文件名</small><code>{{ templateExamples.movie.filename || '模板为空' }}</code>
                  <small>完整路径</small><span>{{ templateExamples.movie.path || '模板为空' }}</span>
                </div>
              </a-form-item>
              <a-form-item label="电视剧完整相对路径" class="template-wide">
                <a-textarea ref="tvTemplateInput" v-model:value="settingsForm.tv_path_template" :auto-size="{ minRows: 2, maxRows: 4 }" />
                <div class="template-inline-preview" aria-live="polite">
                  <small>实时文件名</small><code>{{ templateExamples.tv.filename || '模板为空' }}</code>
                  <small>完整路径</small><span>{{ templateExamples.tv.path || '模板为空' }}</span>
                </div>
              </a-form-item>
              <label class="media-info-setting template-wide"><span><strong>携带媒体信息（FFprobe）</strong><small>识别时读取真实分辨率、编解码、声道、帧率、色深、HDR / 杜比视界；模板未写技术标签时，自动拼到扩展名前。</small></span><a-switch v-model:checked="settingsForm.include_media_info" aria-label="使用 FFprobe 携带媒体信息" /></label>
            </div>
              <p class="template-help">支持单/双花括号和条件片段，例如 <code v-pre>{{@if@}}-{{releaseGroup}}{{@endif@}}</code> 仅在发布组存在时输出；也可使用 <code>{ } [ ] ( ) - .</code>。国家默认输出中文，需 ISO 代码时使用 <code>{country_code}</code>；<code>{category}</code> 可输出多级目录。</p>
              <div class="template-token-groups">
                <details open><summary>电影可用标签（{{ ORGANIZER_MOVIE_TEMPLATE_TOKENS.length }}，点击插入）</summary><div class="template-token-list"><button v-for="item in ORGANIZER_MOVIE_TEMPLATE_TOKENS" :key="`movie-${item.key}`" type="button" class="template-token-button" title="插入标签并实时预览" @click="insertTemplateToken('movie', item.token)"><code>{{ item.token }}</code>{{ item.label }}</button></div></details>
                <details><summary>电视剧可用标签（{{ ORGANIZER_TV_TEMPLATE_TOKENS.length }}，点击插入）</summary><div class="template-token-list"><button v-for="item in ORGANIZER_TV_TEMPLATE_TOKENS" :key="`tv-${item.key}`" type="button" class="template-token-button" title="插入标签并实时预览" @click="insertTemplateToken('tv', item.token)"><code>{{ item.token }}</code>{{ item.label }}</button></div></details>
              </div>
           </a-collapse-panel>
        </a-collapse>
      </a-card>

      <section v-if="props.settingsOnly && settingsSection === 'recognition'" class="section-block recognition-settings-block">
        <div class="section-heading">
          <div><h2>辅助识别</h2><p>在解析片名、季集和技术参数前按顺序处理规则；注释行不会执行。</p></div>
          <a-button type="primary" :loading="settingsSaving" @click="saveSettings">保存识别规则</a-button>
        </div>
        <a-alert type="info" show-icon class="settings-alert">
          <template #message>每行一条规则：<code>正则 =&gt; 替换</code>；只有正则表示删除。支持 <code>\1</code> 捕获、<code>\1@-12</code> 集数运算，以及 <code>{[tmdbid=123;type=tv;s=1;e=2]}</code> 强制识别。规则使用原生 Rust regex 语法，不支持前后查找、正则内反向引用、命名捕获和 <code>@?</code> 条件行。</template>
        </a-alert>
        <a-card :bordered="false" class="rule-editor-card">
          <a-tabs v-model:active-key="recognitionSection" class="recognition-tabs">
            <a-tab-pane key="recognition_words" tab="自定义识别词">
              <a-textarea v-model:value="settingsForm.recognition_words" aria-label="自定义识别词规则" :auto-size="{ minRows: 14, maxRows: 28 }" :spellcheck="false" placeholder="# 示例：(?i)^Some\.Show => Some Show{[tmdbid=123;type=tv]}" />
              <p class="field-help">用于改名、去除干扰词、校正季集或固定 TMDB 条目，按从上到下的顺序执行。</p>
            </a-tab-pane>
            <a-tab-pane key="release_groups" tab="自定义制作组">
              <a-textarea v-model:value="settingsForm.release_groups" aria-label="自定义制作组列表" :auto-size="{ minRows: 14, maxRows: 28 }" :spellcheck="false" placeholder="# 每行一个制作组，例如&#10;WiKi&#10;MTeam&#10;ADE" />
              <p class="field-help">已知制作组优先于文件名末尾推断，用于稳定生成 {releaseGroup}。</p>
            </a-tab-pane>
            <a-tab-pane key="render_words" tab="自定义渲染词">
              <a-textarea v-model:value="settingsForm.render_words" aria-label="自定义渲染词规则" :auto-size="{ minRows: 14, maxRows: 28 }" :spellcheck="false" placeholder="(?i)H\.264 => AVC&#10;(?i)H\.265 => HEVC&#10;(?i)4K => 2160p" />
              <p class="field-help">在识别词之后统一画质、编码、来源等写法，再提取命名变量。</p>
            </a-tab-pane>
            <a-tab-pane key="capture_groups" tab="自定义捕获组">
              <a-textarea v-model:value="settingsForm.capture_groups" aria-label="自定义制作组捕获正则" :auto-size="{ minRows: 14, maxRows: 28 }" :spellcheck="false" placeholder="# 每行一个含捕获组的正则，例如&#10;-([A-Za-z0-9@._-]+)$" />
              <p class="field-help">第一个非空捕获组会作为制作组；未命中时仍会尝试文件名末尾的 <code>-Group</code>。</p>
            </a-tab-pane>
            <a-tab-pane key="upgrade_policy" tab="洗版策略">
              <a-form-item label="比较维度与优先级">
                <a-select
                  v-model:value="settingsForm.upgrade_criteria"
                  mode="multiple"
                  :options="settingsForm.upgrade_criteria_options"
                  placeholder="按点选顺序决定优先级"
                  aria-label="洗版比较维度与优先级"
                />
                <p class="field-help">目标冲突选择“洗版”的监控生效：按选中顺序逐项比较（先分出胜负的维度定结果），全部持平视为同一版本并跳过。动态范围排序为 DV &gt; HDR10+ &gt; HDR10 &gt; HLG &gt; SDR。</p>
              </a-form-item>
              <a-form-item label="制作组优先级（从高到低）">
                <a-textarea v-model:value="settingsForm.upgrade_release_groups" aria-label="洗版制作组优先级" :auto-size="{ minRows: 6, maxRows: 16 }" :spellcheck="false" placeholder="# 每行一个制作组，靠前优先，例如&#10;FRDS&#10;WiKi&#10;CHD" />
                <p class="field-help">仅“制作组”维度使用；留空时跳过该维度。名单外的制作组视为最低且彼此持平。</p>
              </a-form-item>
            </a-tab-pane>
          </a-tabs>
        </a-card>
      </section>

      <section v-if="props.settingsOnly && settingsSection === 'categories'" class="section-block category-settings-block">
        <div class="section-heading">
          <div><h2>二级分类</h2><p>类型、原始语言、来源地区按组同时满足；同组内任一值命中即可，第一条命中的规则优先。</p></div>
          <a-space><a-button :loading="settingsSaving" @click="applyReferenceCategoryRules">恢复默认分类</a-button><a-button type="primary" :loading="settingsSaving" @click="saveSettings">保存分类</a-button></a-space>
        </div>
        <div class="category-toolbar">
          <div><strong>规则顺序</strong><span>名称允许使用 <code>/</code> 输出多级目录，例如“电视剧/动漫/国漫”。</span></div>
          <a-button size="small" @click="addCategoryRule"><PlusOutlined />添加规则</a-button>
        </div>
        <a-empty v-if="!settingsForm.category_rules.length" description="未配置分类规则；未命中时使用通用配置中的默认分类" />
        <div v-else class="category-rule-list category-rule-list--expanded">
          <article v-for="(rule, index) in settingsForm.category_rules" :key="rule.id" class="category-rule-card">
            <header>
              <span class="rule-index">{{ String(index + 1).padStart(2, '0') }}</span>
              <a-input v-model:value="rule.name" aria-label="分类目录" placeholder="分类目录，例如：电视剧/动漫/国漫" />
              <a-select v-model:value="rule.media_type" aria-label="媒体类型" :options="[{ label: '全部', value: 'all' }, { label: '电影', value: 'movie' }, { label: '电视剧', value: 'tv' }]" />
              <a-switch v-model:checked="rule.enabled" :aria-label="`启用分类规则 ${index + 1}`" checked-children="开" un-checked-children="停" />
            </header>
            <div class="category-condition-grid">
              <a-form-item label="TMDB 类型 / ID"><a-select v-model:value="rule.genres" :aria-label="`分类规则 ${index + 1} 的 TMDB 类型或 ID`" mode="tags" :options="TMDB_GENRE_OPTIONS" placeholder="例如 16、动画" /></a-form-item>
              <a-form-item label="原始语言"><a-select v-model:value="rule.original_languages" :aria-label="`分类规则 ${index + 1} 的原始语言`" mode="tags" placeholder="例如 zh、ja、ko" /></a-form-item>
              <a-form-item label="来源地区"><a-select v-model:value="rule.origin_countries" :aria-label="`分类规则 ${index + 1} 的来源地区`" mode="multiple" show-search :options="COUNTRY_OPTIONS_ZH" :filter-option="(input, option) => String(option.label).toLowerCase().includes(String(input).toLowerCase())" placeholder="选择国家或地区（中文显示）" /></a-form-item>
            </div>
            <footer>
              <span>三个条件组均为空时不能保存</span>
              <a-space :size="4">
                <a-button size="small" :disabled="index === 0" :aria-label="`上移规则 ${index + 1}`" @click="moveCategoryRule(index, -1)">上移</a-button>
                <a-button size="small" :disabled="index === settingsForm.category_rules.length - 1" :aria-label="`下移规则 ${index + 1}`" @click="moveCategoryRule(index, 1)">下移</a-button>
                <a-button type="text" danger size="small" :aria-label="`删除规则 ${index + 1}`" @click="removeCategoryRule(rule)"><DeleteOutlined />删除</a-button>
              </a-space>
            </footer>
          </article>
        </div>
      </section>

      <section v-if="props.settingsOnly && settingsSection === 'search'" class="section-block search-settings-block">
        <div class="section-heading">
          <div><h2>搜索设置</h2><p>控制 TMDB 无结果时的回退与自动选择边界；关闭相似度匹配后只会自动接受同名、同年份结果。</p></div>
          <a-button type="primary" :loading="settingsSaving" @click="saveSettings">保存搜索设置</a-button>
        </div>
        <div class="search-setting-grid">
          <label class="search-setting-card"><span><strong>分词搜索</strong><small>首次查询无结果时，尝试括号外标题、别名分段等最多三个安全变体。</small></span><a-switch v-model:checked="settingsForm.word_segment_search" aria-label="启用分词搜索" /></label>
          <label class="search-setting-card"><span><strong>相似度匹配</strong><small>按标题、年份和候选差距自动选择；关闭后所有非精确结果进入人工确认。</small></span><a-switch v-model:checked="settingsForm.similarity_match" aria-label="启用相似度匹配" /></label>
          <label class="search-setting-card"><span><strong>允许成人内容候选</strong><small>只影响 TMDB 返回候选，不改变云盘扫描或分类。</small></span><a-switch v-model:checked="settingsForm.include_adult" aria-label="允许成人内容候选" /></label>
          <a-form-item label="自动匹配阈值" class="search-score-card" :extra="settingsForm.similarity_match ? '候选分数低于阈值时转人工确认。' : '相似度匹配关闭时，此阈值不参与自动选择。'">
            <a-input-number v-model:value="settingsForm.minimum_match_score" :disabled="!settingsForm.similarity_match" :min="0.4" :max="0.98" :step="0.01" :precision="2" style="width: 100%" />
          </a-form-item>
        </div>
      </section>

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
              <span class="path-flow">A {{ mapping.source_path }} → B {{ mappingOutputName(mapping) }} · {{ mapping.target_path }}</span>
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
              <a-switch :checked="mapping.enabled" :aria-label="`启用归档规则 ${mapping.source_path}`" :disabled="mappingLocked(mapping)" checked-children="开" un-checked-children="停" @change="(value) => toggleMapping(mapping, value)" />
              <a-tooltip :title="mappingLocked(mapping) ? '目录正在识别或整理' : '立即扫描'"><a-button shape="circle" :aria-label="`立即扫描 ${mapping.source_path}`" :disabled="!mapping.enabled || mappingLocked(mapping)" :loading="jobBusy[`mapping:${mapping.id}`]" @click="scanMapping(mapping)"><ReloadOutlined /></a-button></a-tooltip>
              <a-tooltip :title="mappingLocked(mapping) ? '完成当前任务后才能编辑' : '编辑'"><a-button shape="circle" :aria-label="`编辑归档规则 ${mapping.source_path}`" :disabled="mappingLocked(mapping)" @click="openEditMapping(mapping)"><EditOutlined /></a-button></a-tooltip>
              <a-tooltip :title="mappingLocked(mapping) ? '完成当前任务后才能删除' : '删除'"><a-button danger shape="circle" :aria-label="`删除归档规则 ${mapping.source_path}`" :disabled="mappingLocked(mapping)" @click="removeMapping(mapping)"><DeleteOutlined /></a-button></a-tooltip>
            </div>
          </article>
        </div>
      </section>

      <section v-if="props.settingsOnly && settingsSection === 'scrape'" class="section-block scrape-settings-block">
        <div class="section-heading">
          <div><h2>刮削输出</h2><p>配置默认生成的 NFO 与图片类型，以及手动刮削时可选的云盘媒体库目标。</p></div>
          <a-button type="primary" @click="saveSettings" :loading="settingsSaving">保存刮削输出</a-button>
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
                <FolderOpenOutlined /><div><strong>{{ target.name }}</strong><small>{{ target.path }} · {{ target.dir_id }}</small></div><a-space><a-button type="text" size="small" :aria-label="`编辑刮削目标 ${target.name}`" @click="openEditTarget(target)"><EditOutlined /></a-button><a-button type="text" danger size="small" :aria-label="`删除刮削目标 ${target.name}`" @click="removeTarget(target)"><DeleteOutlined /></a-button></a-space>
              </article>
            </div>
          </div>
        </div>
      </section>

      <section v-if="!props.settingsOnly" class="section-block jobs-block">
        <div class="section-heading">
          <div><h2>整理任务</h2><p>每次执行前都会校验源文件是否变化；有多个 TMDB 候选时会停下来等待选择。</p></div>
        </div>
        <div class="table-filter-bar">
          <a-select v-model:value="jobFilters.status" :options="jobStatusOptions" class="filter-status" aria-label="按状态筛选整理任务" />
          <a-input v-model:value="jobFilters.keyword" allow-clear placeholder="搜索来源 / 识别结果 / 消息" class="filter-keyword" aria-label="搜索整理任务">
            <template #prefix><SearchOutlined /></template>
          </a-input>
          <span v-if="jobFilters.status !== 'all' || jobFilters.keyword" class="filter-count">{{ filteredJobs.length }} / {{ organizer.jobs.length }} 条</span>
          <template v-if="selectedJobIds.length">
            <span class="filter-count">已选 {{ selectedJobIds.length }} 条</span>
            <a-tooltip title="后台重新识别并自动整理；已落位过的任务会先清理上次的文件与元数据">
              <a-button size="small" type="primary" :loading="batchBusy" @click="batchRetryJobs">
                <ReloadOutlined />批量重新识别并整理{{ batchBusy ? `（${batchProgress.done}/${batchProgress.total}）` : '' }}
              </a-button>
            </a-tooltip>
            <a-button size="small" :disabled="batchBusy" @click="selectedJobIds = []">取消选择</a-button>
          </template>
        </div>
        <a-table :columns="jobColumns" :data-source="filteredJobs" row-key="id" :row-selection="jobRowSelection" :pagination="{ pageSize: 12, showSizeChanger: false }" :scroll="{ x: 990 }" size="middle">
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
                <a-button v-if="['failed', 'completed', 'completed_warning'].includes(record.status)" type="link" size="small" :loading="jobBusy[record.id]" @click="openReorganize(record)"><ReloadOutlined />重新归档</a-button>
                <a-button v-if="['completed', 'completed_warning'].includes(record.status) && record.preview?.share_relative_path" type="link" size="small" :loading="jobBusy[`share:${record.id}`]" @click="createJobShare(record)"><ShareAltOutlined />{{ jobShareUrl(record) ? '重新分享' : '创建分享' }}</a-button>
                <a-button v-if="jobShareUrl(record)" type="text" size="small" aria-label="复制整理分享链接" @click="copyJobShare(record)"><CopyOutlined /></a-button>
                <a-button v-if="!['recognizing', 'running'].includes(record.status)" type="text" danger size="small" :loading="jobBusy[record.id]" :aria-label="`删除整理记录 ${fileName(record.source_path)}`" @click="openDeleteActions(record)"><DeleteOutlined />删除</a-button>
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
      <a-form-item label="输出媒体库（刮削输出）" required extra="监控只能写入“刮削输出”中统一配置的媒体库目标；路径模板会在该目录下创建分类结构。">
        <a-select :value="mappingForm.target_dir_id || undefined" :options="scrapeTargetOptions" placeholder="选择已配置的刮削输出目标" @change="selectMappingOutputTarget" />
        <div class="output-target-help">
          <small>{{ mappingForm.target_path || '尚未选择输出目录' }}</small>
          <a-button type="link" size="small" @click="configureScrapeTargets">管理刮削输出</a-button>
        </div>
        <a-alert v-if="mappingForm.target_dir_id && !mappingTargetConfigured" type="warning" show-icon message="这是旧版自定义 B 目录；保存前需要改为刮削输出中配置的目标。" />
      </a-form-item>
      <div class="global-rule-source" aria-label="全局整理规则继承说明">
        <div><strong>统一沿用全局整理规则</strong><a-tag color="blue">自动联动</a-tag></div>
        <p>监控不再维护重复规则；每次识别都会读取最新的全局二级分类、辅助识别、搜索设置和命名模板。</p>
        <div class="global-rule-metrics">
          <span><small>二级分类</small><strong>{{ globalRuleSummary.categoryCount }} 条启用</strong></span>
          <span><small>辅助识别</small><strong>{{ globalRuleSummary.auxiliary }} 组已配置</strong></span>
          <span><small>搜索策略</small><strong>{{ globalRuleSummary.search }}</strong></span>
        </div>
      </div>
      <div class="mapping-template-preview" aria-label="当前命名规则预览">
        <span><strong>电影命名</strong><code>{{ templateExamples.movie.path || '尚无预览' }}</code></span>
        <span><strong>电视剧命名</strong><code>{{ templateExamples.tv.path || '尚无预览' }}</code></span>
        <small>全局规则保存后会自动作用于所有监控；已有预览会要求重新识别，避免按旧规则执行。</small>
      </div>
      <div class="form-grid">
        <a-form-item label="云端静默等待">
          <a-input-number v-model:value="mappingForm.settle_seconds" :min="5" :max="3600" :step="5" style="width: 100%" addon-after="秒" />
        </a-form-item>
        <a-form-item label="媒体类型">
          <a-radio-group v-model:value="mappingForm.media_type" class="mapping-choice" aria-label="媒体类型">
            <a-radio-button value="">自动识别</a-radio-button>
            <a-radio-button value="movie">电影</a-radio-button>
            <a-radio-button value="tv">电视剧</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="整理方式">
          <a-radio-group v-model:value="mappingForm.transfer_type" class="mapping-choice" aria-label="整理方式">
            <a-radio-button value="copy">云盘内复制（推荐）</a-radio-button>
            <a-radio-button value="move">云盘内移动</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="目标冲突">
          <a-radio-group v-model:value="mappingForm.conflict_policy" class="mapping-choice" aria-label="目标冲突">
            <a-radio-button value="skip">跳过已有文件</a-radio-button>
            <a-radio-button value="overwrite">覆盖已有文件</a-radio-button>
            <a-radio-button value="rename">追加短标识保留两份</a-radio-button>
            <a-radio-button value="upgrade">洗版（按优先级替换旧版本）</a-radio-button>
          </a-radio-group>
        </a-form-item>
      </div>
      <a-alert v-if="mappingForm.conflict_policy === 'upgrade'" type="info" show-icon message="洗版：目标目录中同一部电影/同一集的旧版本会按“设置 → 整理与刮削 → 识别规则 → 洗版策略”的优先级比较；新版本更优时旧文件移入回收站再落位，否则跳过新文件。" class="transfer-alert" />
      <a-alert v-if="mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite' || mappingForm.conflict_policy === 'upgrade'" type="warning" show-icon message="光鸭分享不是稳定快照：移动、删除或覆盖云端资源可能让 A 目录或旧目标的已有分享失效。整理后分享会从 B 目录重新创建新链接，不复用旧链接。" class="transfer-alert" />
      <a-checkbox v-if="mappingForm.transfer_type === 'move' || mappingForm.conflict_policy === 'overwrite' || mappingForm.conflict_policy === 'upgrade'" v-model:checked="mappingForm.share_risk_acknowledged" class="risk-check">我已了解移动/覆盖/洗版会使旧分享失效</a-checkbox>
      <div class="switch-list">
        <label><span><strong>扫描已有内容</strong><small>创建任务后立即检查云盘 A 目录中的一级项目</small></span><a-switch v-model:checked="mappingForm.scan_existing" /></label>
        <label><span><strong>刮削元数据（默认关闭）</strong><small>开启后仅执行下方选中的类型，不会全量刮削</small></span><a-switch :checked="mappingForm.scrape" @change="toggleScrape" /></label>
        <a-form-item v-if="mappingForm.scrape" label="刮削类型" class="scrape-types">
          <a-select v-model:value="mappingForm.scrape_types" mode="multiple" :options="organizer.settings.scrape_type_options || []" placeholder="至少选择一种元数据" />
        </a-form-item>
        <label><span><strong>同步字幕与外置音轨</strong><small>同名或同季集的字幕、音轨会跟随主视频命名</small></span><a-switch v-model:checked="mappingForm.sync_extras" /></label>
        <label><span><strong>识别成功后自动执行</strong><small>关闭时任务停在“待执行”，确认目标路径后再整理</small></span><a-switch v-model:checked="mappingForm.auto_execute" /></label>
        <label><span><strong>自动分享最终媒体目录并投稿 HDHive</strong><small>不分享冗长的分类根目录；每个项目完成后直接分享其最终电影/剧集文件夹。关闭后仍可在任务列表一键创建。</small></span><a-switch v-model:checked="mappingForm.share_after_organize" /></label>
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

  <a-modal v-model:open="review.open" :title="review.mode === 'rearchive' ? '重新归档' : '人工确认媒体信息'" width="min(820px, 94vw)" :closable="!jobBusy[review.job?.id]" :mask-closable="false">
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
      <a-collapse ghost class="review-advanced">
        <a-collapse-panel key="advanced" :header="`高级选项${review.episode_offset !== '' || review.recognition_words ? '（已设置）' : ''}`">
          <div class="review-advanced-grid">
            <a-form-item label="集偏移">
              <a-input-number v-model:value="review.episode_offset" :precision="0" placeholder="例如 -12 或 24" style="width: 100%" />
              <div class="field-help">识别出的每集集号统一加此偏移（可为负），用于源命名与 TMDB 集数错位的剧集。</div>
            </a-form-item>
            <a-form-item label="临时识别词" class="review-words">
              <a-textarea v-model:value="review.recognition_words" :auto-size="{ minRows: 3, maxRows: 8 }" :spellcheck="false" placeholder="仅对本任务生效，优先于全局识别词：&#10;屏蔽词&#10;被替换词 => 替换词&#10;(?i)^Alias\.(\d+) => Show.S01E\1" />
              <div class="field-help">格式与“识别设置 → 自定义识别词”一致；可先在顶部“识别测试工具”里调试。</div>
            </a-form-item>
          </div>
        </a-collapse-panel>
      </a-collapse>
    </a-form>
    <template #footer>
      <a-button @click="review.open = false">取消</a-button>
      <a-button v-if="review.mode !== 'rearchive'" :loading="jobBusy[review.job?.id]" @click="submitReview(false)">仅重新识别</a-button>
      <a-button type="primary" :loading="jobBusy[review.job?.id]" @click="submitReview(true)">{{ review.mode === 'rearchive' ? '重新归档' : '识别并整理' }}</a-button>
    </template>
  </a-modal>

  <a-modal v-model:open="deleteDialog.open" title="操作选项" width="min(520px, 92vw)" :footer="null" :destroy-on-close="true">
    <p class="delete-dialog-title">{{ fileName(deleteDialog.job?.source_path) }}</p>
    <div class="delete-option-list">
      <a-button v-for="action in JOB_DELETE_ACTIONS" :key="action.key" block danger @click="chooseDeleteAction(action.key)">
        <DeleteOutlined />{{ action.label }}
      </a-button>
    </div>
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
    <a-alert v-if="preview.job?.preview?.media_probe_warnings?.length" type="warning" show-icon message="部分文件未取得 FFprobe 媒体信息" :description="preview.job.preview.media_probe_warnings.join('\n')" class="preview-alert probe-warning-alert" />
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
.organizer-spin, .organizer-page { width: 100%; min-width: 0; max-width: 100%; }
.organizer-spin { display: block; }
.organizer-spin :deep(.ant-spin-nested-loading), .organizer-spin :deep(.ant-spin-container) { min-width: 0; max-width: 100%; }
.organizer-page { display: grid; gap: 14px; max-width: var(--page-max, 1440px); margin: 0 auto; }
.organizer-settings-tabs { margin: 2px 0 -4px; padding: 0 4px; }
.inner-tab { display: inline-flex; align-items: center; gap: 7px; }
.template-inline-preview { display: grid; grid-template-columns: 72px minmax(0, 1fr); gap: 5px 10px; margin-top: 8px; padding: 9px 10px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 8px; background: var(--surface-muted, #fafafa); }
.template-inline-preview small { color: var(--text-3, #737373); font-size: 10px; }
.template-inline-preview code, .template-inline-preview span { min-width: 0; overflow-wrap: anywhere; color: var(--text-1, #262626); font-size: 11px; }
.scrape-preference-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
.template-wide { grid-column: 1 / -1; }
.target-panel, .category-rule-panel { padding: 14px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; }
.target-panel-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
.target-panel-head strong, .target-panel-head small { display: block; }
.target-panel-head small { margin-top: 4px; color: var(--text-3, #737373); font-size: 12px; }
.scrape-target-list, .category-rule-list { display: grid; gap: 8px; }
.scrape-target-card { display: flex; min-width: 0; align-items: center; gap: 9px; padding: 9px 10px; border: 1px solid var(--line, #e5e5e5); border-radius: 8px; }
.scrape-target-card > div { min-width: 0; flex: 1; }
.scrape-target-card strong, .scrape-target-card small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.scrape-target-card small { margin-top: 3px; color: var(--text-3, #737373); font-size: 11px; }
.category-rule-row { display: grid; grid-template-columns: auto minmax(110px, .8fr) 110px minmax(180px, 1.5fr) auto; align-items: center; gap: 8px; }
.field-help { color: var(--text-3, #737373); font-size: 11px; }
.rule-editor-card { overflow: hidden; background: var(--surface-muted, #fafafa); }
.rule-editor-card :deep(.ant-card-body) { padding: 0 16px 14px; }
.recognition-tabs :deep(.ant-tabs-nav) { margin-bottom: 12px; }
.recognition-tabs :deep(textarea) { font-family: "Cascadia Code", "SFMono-Regular", Consolas, monospace; font-size: 12px; line-height: 1.65; }
.recognition-tabs .field-help { display: block; margin: 8px 2px 0; }
.category-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 12px; padding: 11px 12px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 9px; background: var(--surface-muted, #fafafa); }
.category-toolbar > div { display: grid; gap: 2px; }
.category-toolbar span { color: var(--text-3, #737373); font-size: 11px; }
.category-rule-list--expanded { gap: 10px; }
.category-rule-card { display: grid; gap: 12px; padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface-muted, #fafafa); }
.category-rule-card header { display: grid; grid-template-columns: 34px minmax(220px, 1fr) 118px auto; align-items: center; gap: 9px; }
.rule-index { color: var(--text-3, #737373); font: 700 11px/1 "Cascadia Code", monospace; letter-spacing: .08em; }
.category-condition-grid { display: grid; grid-template-columns: 1.35fr 1fr 1fr; gap: 10px; }
.category-condition-grid :deep(.ant-form-item) { margin: 0; }
.category-rule-card footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-top: 9px; border-top: 1px solid var(--line-soft, #f5f5f5); }
.category-rule-card footer > span { color: var(--text-3, #737373); font-size: 10px; }
.search-setting-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
.search-setting-card, .search-score-card { min-height: 88px; margin: 0 !important; padding: 14px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 10px; background: var(--surface-muted, #fafafa); }
.search-setting-card { display: flex; align-items: center; justify-content: space-between; gap: 22px; cursor: pointer; }
.search-setting-card span { display: grid; gap: 4px; }
.search-setting-card small { color: var(--text-3, #737373); font-size: 11px; line-height: 1.45; }
.mapping-template-preview { display: grid; gap: 7px; margin: -2px 0 14px; padding: 11px 12px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 9px; background: var(--surface-muted, #fafafa); }
.mapping-template-preview > span { display: grid; grid-template-columns: 78px minmax(0, 1fr); align-items: baseline; gap: 8px; }
.mapping-template-preview code { overflow-wrap: anywhere; color: var(--text-2, #525252); font-size: 10px; }
.mapping-template-preview small { color: var(--text-3, #737373); font-size: 10px; }
.output-target-help { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 30px; }
.output-target-help small { min-width: 0; overflow: hidden; color: var(--text-3, #737373); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.global-rule-source { display: grid; gap: 8px; margin: 0 0 14px; padding: 12px; border: 1px solid color-mix(in srgb, var(--primary) 24%, var(--line-soft, #f5f5f5)); border-radius: 9px; background: color-mix(in srgb, var(--primary) 5%, var(--surface, #fff)); }
.global-rule-source > div:first-child { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.global-rule-source p { margin: 0; color: var(--text-2, #525252); font-size: 11px; line-height: 1.5; }
.global-rule-metrics { display: grid; grid-template-columns: .8fr .8fr 1.4fr; gap: 7px; }
.global-rule-metrics span { display: grid; gap: 2px; min-width: 0; padding: 8px 9px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 7px; background: var(--surface, #fff); }
.global-rule-metrics small { color: var(--text-3, #737373); font-size: 9px; }
.global-rule-metrics strong { overflow: hidden; font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.section-heading p { margin: 0; color: var(--text-2, #525252); font-size: 12px; }
.settings-actions, .drawer-footer { display: flex; justify-content: flex-end; gap: 8px; }
.metric-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; }
.metric-grid article { display: grid; min-height: 92px; align-content: center; padding: 14px 16px; border: 1px solid var(--line, #e5e5e5); border-radius: 12px; background: var(--surface, #fff); }
.metric-grid small, .metric-grid span { color: var(--text-3, #737373); font-size: 10px; }
.metric-grid strong { margin: 2px 0; font-size: var(--fs-2xl, 24px); line-height: 1; font-variant-numeric: tabular-nums; }
.metric-grid .attention { border-color: #fdba74; background: color-mix(in srgb, var(--warning) 8%, var(--surface, #fff)); }
.settings-card, .section-block { min-width: 0; border: 1px solid var(--line, #e5e5e5); border-radius: 12px; background: var(--surface, #fff); }
.settings-card :deep(.ant-card-head) { min-height: 48px; }
.card-title { display: inline-flex; align-items: center; gap: 8px; }
.settings-alert, .drawer-alert, .review-alert, .preview-alert { margin-bottom: 14px; }
.delete-dialog-title { margin: 0 0 12px; overflow: hidden; color: var(--text-2, #525252); text-overflow: ellipsis; white-space: nowrap; }
.delete-option-list { display: grid; gap: 10px; }
.delete-option-list :deep(.ant-btn) { height: 42px; justify-content: flex-start; }
.probe-warning-alert :deep(.ant-alert-description) { white-space: pre-line; }
.settings-primary { display: grid; grid-template-columns: minmax(260px, 1.35fr) minmax(150px, .7fr) minmax(170px, .8fr) 150px auto; align-items: end; gap: 12px; }
.settings-primary :deep(.ant-form-item), .template-grid :deep(.ant-form-item) { margin: 0; }
.naming-collapse { margin-top: 10px; border-top: 1px solid var(--line-soft, #f5f5f5); }
.naming-collapse :deep(.ant-collapse-header) { padding-inline: 0 !important; color: var(--text-2, #525252) !important; font-size: 12px; }
.naming-collapse :deep(.ant-collapse-content-box) { padding: 2px 0 0 !important; }
.template-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.template-wide { grid-column: 1 / -1; }
.media-info-setting { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 62px; padding: 11px 12px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 9px; background: var(--surface-muted, #fafafa); }
.media-info-setting span { display: grid; gap: 3px; }
.media-info-setting small { color: var(--text-3, #737373); font-size: 10px; line-height: 1.5; }
.template-help { margin: 10px 0 0; color: var(--text-3, #737373); font-size: 10px; word-break: break-all; }
.template-token-groups { display: grid; gap: 6px; margin-top: 10px; }
.template-token-groups details { padding: 8px 10px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 8px; }
.template-token-groups summary { color: var(--text-2, #525252); font-size: 11px; cursor: pointer; }
.template-token-list { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 8px; }
.template-token-button { display: inline-flex; align-items: center; gap: 5px; margin: 0; padding: 2px 7px; border: 1px solid var(--line, #e5e5e5); border-radius: 5px; background: var(--surface, #fff); color: var(--text-2, #525252); font: inherit; font-size: 10px; cursor: pointer; }
.template-token-button:hover, .template-token-button:focus-visible { border-color: var(--primary); color: var(--primary); outline: none; }
.template-token-button code { color: var(--primary); }
.adult-setting { display: flex; align-items: center; justify-content: space-between; gap: 20px; min-height: 54px; padding: 0 2px; }
.adult-setting span { display: grid; gap: 2px; }
.adult-setting small { color: var(--text-3, #737373); font-size: 10px; }
.section-block { padding: 16px; }
.section-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin-bottom: 14px; }
.section-heading h2 { margin: 0 0 2px; font-size: 16px; }
.mapping-list { display: grid; gap: 8px; }
.mapping-card { display: grid; grid-template-columns: 42px minmax(0, 1fr) auto; align-items: center; gap: 12px; padding: 12px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 10px; background: var(--surface-muted, #fafafa); }
.mapping-icon { display: grid; width: 38px; height: 38px; place-items: center; border-radius: 9px; color: var(--text-1, #262626); background: var(--surface-strong, #ececec); font-size: 18px; }
.mapping-copy { display: grid; min-width: 0; gap: 3px; }
.mapping-title-row { display: flex; min-width: 0; align-items: center; gap: 8px; }
.mapping-title-row strong, .path-flow { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.path-flow { color: var(--text-2, #525252); font-size: 11px; }
.mapping-meta { display: flex; flex-wrap: wrap; gap: 5px 12px; margin-top: 4px; color: var(--text-3, #737373); font-size: 10px; }
.mapping-actions { display: flex; align-items: center; gap: 7px; }
.jobs-block { overflow: hidden; padding-bottom: 8px; }
.jobs-block :deep(.ant-table-wrapper) { width: 100%; min-width: 0; max-width: 100%; margin-inline: 0; }
.jobs-block :deep(.ant-table), .jobs-block :deep(.ant-table-container) { max-width: 100%; }
.job-source, .job-result { display: grid; min-width: 0; gap: 2px; }
.job-source strong, .job-result strong, .job-source span, .job-result span, .job-result small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.job-source span, .job-result span, .job-source small, .job-result small { color: var(--text-3, #737373); font-size: 10px; }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; overflow: visible; }
.form-grid > :nth-child(n + 3) { grid-column: 1 / -1; }
.mapping-choice { display: flex; flex-wrap: wrap; width: 100%; }
.mapping-choice :deep(.ant-radio-button-wrapper) { flex: 1 1 auto; text-align: center; }
.transfer-alert { margin: 0 0 10px; }
.risk-check { margin: 0 0 12px; color: var(--text-2, #525252); }
.switch-list { display: grid; border-top: 1px solid var(--line, #e5e5e5); }
.switch-list label { display: flex; align-items: center; justify-content: space-between; gap: 20px; padding: 12px 0; border-bottom: 1px solid var(--line-soft, #f5f5f5); }
.switch-list label span { display: grid; gap: 2px; }
.switch-list small { color: var(--text-3, #737373); font-size: 11px; }
.scrape-types { margin: 0; padding: 10px 0 12px; border-bottom: 1px solid var(--line-soft, #f5f5f5); }
.cloud-picker-nav { min-height: 34px; margin-bottom: 10px; overflow: auto; }
.cloud-picker-nav :deep(.ant-breadcrumb) { min-width: max-content; }
.picker-current { margin: 10px 0 0; color: var(--text-2, #525252); font-size: 11px; word-break: break-all; }
.candidate-section { margin-bottom: 16px; }
.candidate-heading { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
.candidate-heading span { color: var(--text-3, #737373); font-size: 11px; }
.candidate-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; max-height: 280px; overflow: auto; }
.candidate-card { position: relative; display: grid; grid-template-columns: 52px minmax(0, 1fr) 20px; align-items: center; gap: 10px; min-width: 0; padding: 7px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; color: inherit; background: var(--surface, #fff); text-align: left; cursor: pointer; }
.candidate-card:hover { border-color: var(--line-strong); }
.candidate-card:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.candidate-card.selected { border-color: var(--primary); background: color-mix(in srgb, var(--primary) 7%, var(--surface, #fff)); }
.candidate-card img, .poster-placeholder { width: 52px; height: 70px; border-radius: 6px; object-fit: cover; background: var(--surface-muted, #fafafa); }
.poster-placeholder { display: grid; place-items: center; color: var(--text-3, #737373); font-size: 18px; }
.candidate-card > span { display: grid; min-width: 0; gap: 2px; }
.candidate-card strong, .candidate-card small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.candidate-card small { color: var(--text-3, #737373); font-size: 10px; }
.candidate-card > :deep(.anticon) { color: var(--primary); }
.review-grid { display: grid; grid-template-columns: 1.4fr repeat(3, minmax(110px, .7fr)); gap: 12px; }
.review-title { grid-column: span 2; }
.review-advanced :deep(.ant-collapse-header) { padding-inline-start: 0 !important; color: var(--text-2, #525252); font-size: 13px; }
.review-advanced-grid { display: grid; grid-template-columns: minmax(140px, .6fr) 1.4fr; gap: 12px; }
.review-advanced-grid :deep(textarea) { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; }
@media (max-width: 900px) { .review-advanced-grid { grid-template-columns: 1fr; } }
.matched-media { display: grid; grid-template-columns: 74px minmax(0, 1fr); gap: 12px; margin-bottom: 14px; padding: 10px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 10px; background: var(--surface-muted, #fafafa); }
.matched-media img { width: 74px; height: 106px; border-radius: 7px; object-fit: cover; }
.matched-media div { display: grid; min-width: 0; align-content: center; gap: 3px; }
.matched-media span { color: var(--text-3, #737373); font-size: 10px; }
.matched-media strong { font-size: 17px; }
.matched-media p { display: -webkit-box; margin: 2px 0 0; overflow: hidden; color: var(--text-2, #525252); font-size: 11px; -webkit-box-orient: vertical; -webkit-line-clamp: 3; }
.preview-summary { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-bottom: 12px; }
.preview-summary span { display: grid; padding: 8px 10px; border: 1px solid var(--line-soft, #f5f5f5); border-radius: 8px; background: var(--surface-muted, #fafafa); }
.preview-summary small { color: var(--text-3, #737373); font-size: 9px; }
.preview-summary strong { font-size: 17px; font-variant-numeric: tabular-nums; }
.preview-list { display: grid; gap: 8px; max-height: 54vh; overflow: auto; }
.preview-list article { display: grid; grid-template-columns: 28px minmax(0, 1fr) auto; align-items: center; gap: 10px; padding: 11px; border: 1px solid #bbf7d0; border-radius: 9px; background: color-mix(in srgb, var(--success) 7%, var(--surface, #fff)); }
.preview-list article.failed { border-color: #fca5a5; background: color-mix(in srgb, var(--danger) 7%, var(--surface, #fff)); }
.preview-list article.skipped { border-color: #fdba74; background: color-mix(in srgb, var(--warning) 7%, var(--surface, #fff)); }
.preview-list article > :deep(.anticon) { color: #15803d; font-size: 18px; }
.preview-list article.failed > :deep(.anticon) { color: #b91c1c; }
.preview-list article.skipped > :deep(.anticon) { color: #ea580c; }
.preview-list article div { display: grid; min-width: 0; gap: 2px; }
.preview-list article div strong { display: flex; align-items: center; gap: 6px; }
.preview-list small { overflow: hidden; color: var(--text-2, #525252); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
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
  .scrape-preference-grid { grid-template-columns: 1fr; }
  .category-rule-row { grid-template-columns: auto minmax(0, 1fr) auto; }
  .category-rule-row .ant-select, .category-rule-row .ant-input { grid-column: span 2; }
  .category-rule-card header { grid-template-columns: 30px minmax(0, 1fr) 110px auto; }
  .category-condition-grid { grid-template-columns: 1fr; }
}
@media (max-width: 640px) {
  .settings-actions { width: 100%; }
  .settings-actions .ant-btn { flex: 1; }
  .metric-grid, .settings-primary, .template-grid, .form-grid, .candidate-grid, .review-grid { grid-template-columns: 1fr; }
  .template-wide, .review-title { grid-column: 1; }
  .mapping-card { grid-template-columns: 1fr; }
  .mapping-icon { display: none; }
  .mapping-actions { grid-column: 1; justify-content: flex-start; }
  .preview-summary { grid-template-columns: repeat(2, 1fr); }
  .matched-media { grid-template-columns: 58px minmax(0, 1fr); }
  .matched-media img { width: 58px; height: 84px; }
  .section-heading, .category-toolbar, .category-rule-card footer { align-items: stretch; flex-direction: column; }
  .search-setting-grid { grid-template-columns: 1fr; }
  .category-rule-card header { grid-template-columns: 28px minmax(0, 1fr) auto; }
  .category-rule-card header > :deep(.ant-select) { grid-column: 2 / -1; }
  .mapping-template-preview > span { grid-template-columns: 1fr; }
  .global-rule-metrics { grid-template-columns: 1fr; }
}
</style>
