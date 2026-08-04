<script setup>
import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { message, Modal } from 'antdv-next';
import {
  ArrowDownOutlined,
  ArrowLeftOutlined,
  ArrowUpOutlined,
  CheckOutlined,
  CopyOutlined,
  DeleteOutlined,
  DownloadOutlined,
  DragOutlined,
  EditOutlined,
  FileAddOutlined,
  FileExcelOutlined,
  FileGifOutlined,
  FileImageOutlined,
  FileJpgOutlined,
  FileMarkdownOutlined,
  FileOutlined,
  FilePdfOutlined,
  FilePptOutlined,
  FileTextOutlined,
  FileWordOutlined,
  FileZipOutlined,
  FolderAddOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  InfoCircleOutlined,
  ReloadOutlined,
  ScissorOutlined,
  ShareAltOutlined,
  SwapOutlined,
  UploadOutlined,
  VideoCameraOutlined,
} from '@antdv-next/icons';
import CompactFileBreadcrumb from '../components/files/CompactFileBreadcrumb.vue';
import FileDetailsDrawer from '../components/files/FileDetailsDrawer.vue';
import FileSelectionBar from '../components/files/FileSelectionBar.vue';
import GcidImportStatus from '../components/files/GcidImportStatus.vue';
import ShareResultDialog from '../components/shares/ShareResultDialog.vue';
import { bridge, isTauri } from '../bridge.js';
import { useFileKeyboardShortcuts } from '../composables/useFileKeyboardShortcuts.js';
import { FOLDER_OPEN_MODE, useFolderOpenPreference } from '../composables/useFolderOpenPreference.js';
import { gcidImportPercent, shouldConvertPasteToFile } from '../gcidImport.js';
import { buildRenamePreview } from '../renameRules.js';
import { parseGuangyaShareLink } from '../shareLink.js';
import { readJsonResponse } from '../httpResponse.js';
import { useTransfersStore } from '../stores/transfers.ts';
import {
  appState,
  currentFolderId,
  currentPath,
  files,
  filesLoading,
  filesPage,
  filesPageSize,
  filesTotal,
  loadFiles,
} from '../store.js';
import {
  errorText,
  fileId,
  formatSize,
  formatTime,
  isFolder,
  pick,
  receiptDisplayMessage,
  uploadFileName,
  unwrapData,
} from '../formatters.js';

const transfers = useTransfersStore();
const route = useRoute();
const router = useRouter();
const { folderOpenMode } = useFolderOpenPreference();

const selectedKeys = ref([]);
const dragActive = ref(false);
const dragDepth = ref(0);
const uploading = ref(false);
const operationBusy = ref(false);
const fileInput = ref(null);
const folderInput = ref(null);
const focusedRowId = ref('');
const fileClipboard = reactive({ mode: '', items: [] });
const fileContextMenu = reactive({ open: false, x: 0, y: 0, record: null, keyboard: false });
const newFolderInput = ref(null);
const newFolderModal = reactive({ open: false, saving: false, name: '', error: '' });
const detailsOpen = ref(false);
const detailsRecord = ref(null);
let selectionAnchorId = '';
let gcidImportPollTimer = null;
const uploadMenuItems = computed(() => [
  { key: 'files', label: '选择文件' },
  { key: 'folder', label: '选择文件夹' },
  ...(!isTauri ? [{ type: 'divider' }, { key: 'server', label: '选择服务器文件' }] : []),
]);
const fileContextMenuItems = computed(() => {
  const record = fileContextMenu.record;
  if (!record) return [
    { key: 'newFolder', icon: () => h(FolderAddOutlined), label: '新建文件夹' },
    { type: 'divider' },
    {
      key: 'paste',
      icon: () => h(CheckOutlined),
      label: fileClipboard.items.length
        ? `粘贴${fileClipboard.mode === 'move' ? '已剪切' : '已复制'}的 ${fileClipboard.items.length} 项`
        : '粘贴',
      disabled: !fileClipboard.items.length,
    },
    { type: 'divider' },
    { key: 'refresh', icon: () => h(ReloadOutlined), label: '刷新' },
  ];

  return [
    ...(isFolder(record)
      ? [{ key: 'open', icon: () => h(FolderOpenOutlined), label: '打开文件夹' }, { type: 'divider' }]
      : []),
    { key: 'copy', icon: () => h(CopyOutlined), label: '复制 (Ctrl+C)' },
    { key: 'cut', icon: () => h(ScissorOutlined), label: '剪切 (Ctrl+X)' },
    { key: 'copyTo', icon: () => h(CopyOutlined), label: '复制到…' },
    { key: 'moveTo', icon: () => h(SwapOutlined), label: '移动到…' },
    { type: 'divider' },
    { key: 'details', icon: () => h(InfoCircleOutlined), label: '查看详情' },
    { key: 'rename', icon: () => h(EditOutlined), label: '重命名 (F2)' },
    { key: 'download', icon: () => h(DownloadOutlined), label: '下载' },
    { key: 'share', icon: () => h(ShareAltOutlined), label: '创建分享' },
    { key: 'transferAccount', icon: () => h(SwapOutlined), label: '秒传到小号' },
    { type: 'divider' },
    { key: 'delete', icon: () => h(DeleteOutlined), label: '删除 (Del)', danger: true },
  ];
});

const gcidImport = reactive({
  open: false,
  detailsOpen: false,
  loading: false,
  sourcePath: '',
  sourceName: '',
  pastedJson: '',
  destinationName: '',
  concurrency: 4,
  status: null,
});
const developerTransfer = reactive({
  open: false,
  loading: false,
  submitting: false,
  records: [],
  targets: [],
  targetId: '',
});
const developerTerminalNotified = new Set();
const gcidImportRunning = computed(() => ['preparing', 'running'].includes(gcidImport.status?.status));
const gcidImportProgress = computed(() => gcidImportPercent(gcidImport.status));
const fileActionBarVisible = computed(() => selectedKeys.value.length > 0 || fileClipboard.items.length > 0);
const serverFilePicker = reactive({
  open: false,
  loading: false,
  submitting: false,
  roots: [],
  path: '',
  parent: '',
  displayPath: '/',
  items: [],
  selected: [],
});
const shareResult = reactive({
  open: false,
  creating: false,
  saving: false,
  label: '',
  url: '',
  code: '',
  reused: false,
  hdhiveEventId: '',
  hdhiveStatus: '',
  hdhiveMessage: '',
});
const shareAccess = reactive({
  open: false,
  records: [],
  mode: 'none',
  code: '',
});
const shareResultReceipt = computed(() => (appState.auto_share_receipts || [])
  .find((receipt) => receipt.event_id === shareResult.hdhiveEventId) || null);
const shareResultView = computed(() => ({
  label: shareResult.label,
  url: shareResult.url,
  code: shareResult.code,
  reused: shareResult.reused,
  hdhiveStatus: shareResultReceipt.value?.status || shareResult.hdhiveStatus,
  hdhiveMessage: receiptDisplayMessage(shareResultReceipt.value) || shareResult.hdhiveMessage,
  hdhiveResourceUrl: shareResultReceipt.value?.resource_url || '',
}));
const activeServerRoot = computed(() => [...serverFilePicker.roots]
  .sort((left, right) => right.length - left.length)
  .find((root) => serverFilePicker.path === root || serverFilePicker.path.startsWith(root.endsWith('/') || root.endsWith('\\') ? root : `${root}${root.includes('\\') ? '\\' : '/'}`))
  || serverFilePicker.roots[0]);
const renameModal = reactive({ open: false, saving: false, records: [], mode: 'single', singleName: '', preserveExtension: true, rules: [] });
let renameRuleId = 0;
const renameRuleOptions = [
  { label: '设置名称', value: 'set' },
  { label: '查找替换', value: 'replace' },
  { label: '正则替换', value: 'regex' },
  { label: '添加前缀', value: 'prefix' },
  { label: '添加后缀', value: 'suffix' },
  { label: '追加序号', value: 'sequence' },
  { label: '转为大写', value: 'upper' },
  { label: '转为小写', value: 'lower' },
];
const renameRuleValuePlaceholders = {
  set: '输入统一名称',
  prefix: '输入要添加的前缀',
  suffix: '输入要添加的后缀',
};
const folderPicker = reactive({ open: false, loading: false, title: '', action: 'copy', sourceIds: [], targetId: '', path: [{ id: '', name: '全部文件' }], options: [], page: 0, total: 0 });

const fileColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '类型', key: 'type', width: 96 },
  { title: '大小', key: 'size', width: 100 },
  { title: '修改时间', key: 'time', width: 160 },
];
const folderPickerColumns = [
  { title: '文件夹', key: 'name', ellipsis: true },
  { title: '修改时间', key: 'time', width: 170 },
];
const filePagination = computed(() => ({
  current: filesPage.value + 1,
  pageSize: filesPageSize,
  total: filesTotal.value,
  showSizeChanger: false,
  hideOnSinglePage: true,
  showQuickJumper: filesTotal.value > filesPageSize * 5,
}));
const folderPickerPagination = computed(() => ({
  current: folderPicker.page + 1,
  pageSize: 100,
  total: folderPicker.total,
  showSizeChanger: false,
  hideOnSinglePage: true,
}));

const rowSelection = computed(() => ({
  selectedRowKeys: selectedKeys.value,
  onChange: (keys) => {
    selectedKeys.value = keys;
    selectionAnchorId = String(keys.at(-1) || '');
  },
}));
const folderPickerRowSelection = computed(() => ({
  type: 'radio',
  selectedRowKeys: folderPicker.targetId ? [folderPicker.targetId] : [],
  onChange: (keys) => { folderPicker.targetId = keys[0] || ''; },
}));


function fileIcon(record) {
  if (isFolder(record)) return { icon: FolderOutlined, cls: 'folder' };
  const ext = pick(record, ['fileSuffix'], '').toLowerCase();
  if (['mp4', 'mov', 'mkv', 'avi', 'wmv', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts', '3gp'].includes(ext)) return { icon: VideoCameraOutlined, cls: 'video' };
  if (['jpg', 'jpeg'].includes(ext)) return { icon: FileJpgOutlined, cls: 'image' };
  if (['png', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff'].includes(ext)) return { icon: FileImageOutlined, cls: 'image' };
  if (ext === 'gif') return { icon: FileGifOutlined, cls: 'image' };
  if (ext === 'pdf') return { icon: FilePdfOutlined, cls: 'pdf' };
  if (['doc', 'docx'].includes(ext)) return { icon: FileWordOutlined, cls: 'word' };
  if (['xls', 'xlsx', 'csv'].includes(ext)) return { icon: FileExcelOutlined, cls: 'excel' };
  if (['ppt', 'pptx'].includes(ext)) return { icon: FilePptOutlined, cls: 'ppt' };
  if (['zip', 'rar', '7z', 'tar', 'gz'].includes(ext)) return { icon: FileZipOutlined, cls: 'zip' };
  if (['md', 'markdown'].includes(ext)) return { icon: FileMarkdownOutlined, cls: 'text' };
  if (['txt', 'log', 'json', 'xml', 'yml', 'yaml'].includes(ext)) return { icon: FileTextOutlined, cls: 'text' };
  return { icon: FileOutlined, cls: 'other' };
}

function fileTypeLabel(record) {
  if (isFolder(record)) return '文件夹';
  const extension = String(pick(record, ['fileSuffix', 'ext', 'extension'], '') || '').replace(/^\./, '');
  return extension ? extension.toUpperCase() : '文件';
}

function fileModifiedTime(record) {
  return formatTime(pick(record, ['lastUpdateTime', 'updateTime', 'utime', 'ctime', 'modifiedAt'], 0));
}

function selectRangeTo(record) {
  const targetId = fileId(record);
  const anchorIndex = files.value.findIndex((item) => String(fileId(item)) === String(selectionAnchorId));
  const targetIndex = files.value.findIndex((item) => String(fileId(item)) === String(targetId));
  if (anchorIndex < 0 || targetIndex < 0) {
    selectedKeys.value = [targetId];
    selectionAnchorId = String(targetId);
    return;
  }
  const [start, end] = anchorIndex <= targetIndex ? [anchorIndex, targetIndex] : [targetIndex, anchorIndex];
  selectedKeys.value = files.value.slice(start, end + 1).map(fileId).filter(Boolean);
}

function toggleRecordSelection(record) {
  const id = fileId(record);
  if (!id) return;
  selectedKeys.value = selectedKeys.value.includes(id)
    ? selectedKeys.value.filter((key) => key !== id)
    : [...selectedKeys.value, id];
  selectionAnchorId = String(id);
}

function handleFileRowClick(event, record) {
  if (event.target?.closest?.('input, button, a, .ant-checkbox-wrapper, [role="button"]')) return;
  const id = fileId(record);
  if (!id) return;
  event.currentTarget?.focus?.({ preventScroll: true });
  focusedRowId.value = String(id);
  if (
    isFolder(record)
    && folderOpenMode.value === FOLDER_OPEN_MODE.SINGLE_CLICK
    && !event.shiftKey
    && !event.ctrlKey
    && !event.metaKey
  ) {
    enterFolder(record);
    return;
  }
  if (event.shiftKey) selectRangeTo(record);
  else if (event.ctrlKey || event.metaKey) toggleRecordSelection(record);
  else {
    selectedKeys.value = [id];
    selectionAnchorId = String(id);
  }
}

async function focusFileRow(index, extendSelection = false) {
  if (!files.value.length) return;
  const nextIndex = Math.max(0, Math.min(files.value.length - 1, index));
  const record = files.value[nextIndex];
  const id = String(fileId(record));
  if (!id) return;
  focusedRowId.value = id;
  if (extendSelection) selectRangeTo(record);
  else {
    selectedKeys.value = [fileId(record)];
    selectionAnchorId = id;
  }
  await nextTick();
  const rows = document.querySelectorAll('.file-card .ant-table-tbody > tr[data-row-key]');
  [...rows].find((row) => String(row.getAttribute('data-row-key')) === id)?.focus();
}

function fileRowProps(record, rowIndex) {
  const id = String(fileId(record));
  return {
    tabindex: focusedRowId.value ? (focusedRowId.value === id ? 0 : -1) : (rowIndex === 0 ? 0 : -1),
    'aria-selected': selectedKeys.value.includes(fileId(record)),
    onClick: (event) => handleFileRowClick(event, record),
    onFocus: () => { focusedRowId.value = id; },
    onDblclick: () => {
      if (isFolder(record) && folderOpenMode.value === FOLDER_OPEN_MODE.DOUBLE_CLICK) enterFolder(record);
    },
    onKeydown: (event) => {
      if (fileContextMenu.open) {
        if (handleFileContextMenuKeydown(event)) return;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      const index = files.value.findIndex((item) => String(fileId(item)) === id);
      if (event.key === 'Enter' && isFolder(record)) {
        event.preventDefault();
        enterFolder(record);
      } else if (event.key === 'Enter') {
        event.preventDefault();
        openFileDetails(record);
      }
      if (event.key === ' ') {
        event.preventDefault();
        toggleRecordSelection(record);
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        void focusFileRow(index + 1, event.shiftKey);
      }
      if (event.key === 'ArrowUp' && !event.altKey) {
        event.preventDefault();
        void focusFileRow(index - 1, event.shiftKey);
      }
      if (event.key === 'Home') {
        event.preventDefault();
        void focusFileRow(0, event.shiftKey);
      }
      if (event.key === 'End') {
        event.preventDefault();
        void focusFileRow(files.value.length - 1, event.shiftKey);
      }
      if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
        event.preventDefault();
        const bounds = event.currentTarget.getBoundingClientRect();
        openFileContextMenu({ preventDefault() {}, clientX: bounds.left + 24, clientY: bounds.top + 24 }, record, true);
      }
    },
    onContextmenu: (event) => {
      event.preventDefault();
      event.stopPropagation();
      openFileContextMenu(event, record);
    },
  };
}

function openFileContextMenu(event, record, keyboard = false) {
  if (!appState.logged_in) return;
  const id = fileId(record);
  if (id && !selectedKeys.value.includes(id)) selectedKeys.value = [id];
  if (id) {
    focusedRowId.value = String(id);
    selectionAnchorId = String(id);
  }
  fileContextMenu.x = event.clientX;
  fileContextMenu.y = event.clientY;
  fileContextMenu.record = record;
  fileContextMenu.keyboard = keyboard;
  fileContextMenu.open = true;
  if (keyboard) void focusKeyboardContextMenu();
}

async function focusKeyboardContextMenu() {
  await nextTick();
  requestAnimationFrame(() => {
    requestAnimationFrame(() => visibleFileContextMenuItems()[0]?.focus?.({ preventScroll: true }));
  });
}

function visibleFileContextMenuItems() {
  const menus = [...document.querySelectorAll('.ant-dropdown:not(.ant-dropdown-hidden)')]
    .filter((element) => getComputedStyle(element).display !== 'none');
  const menu = menus.at(-1);
  return menu
    ? [...menu.querySelectorAll('.ant-dropdown-menu-item:not(.ant-dropdown-menu-item-disabled), [role="menuitem"]:not([aria-disabled="true"])')]
    : [];
}

function handleFileContextMenuKeydown(event) {
  if (!fileContextMenu.open) return false;
  if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    closeFileContextMenu(true);
    return true;
  }
  const items = visibleFileContextMenuItems();
  if (!items.length) return false;
  if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    event.preventDefault();
    event.stopPropagation();
    const currentIndex = items.indexOf(document.activeElement);
    const nextIndex = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? items.length - 1
        : currentIndex < 0
          ? (event.key === 'ArrowUp' ? items.length - 1 : 0)
          : (currentIndex + (event.key === 'ArrowUp' ? -1 : 1) + items.length) % items.length;
    items[nextIndex]?.focus?.({ preventScroll: true });
    return true;
  }
  if (['Enter', ' '].includes(event.key) && items.includes(document.activeElement)) {
    event.preventDefault();
    event.stopPropagation();
    document.activeElement?.click?.();
    return true;
  }
  return false;
}

async function restoreFocusedFileRow(id) {
  if (!id) return;
  await nextTick();
  const rows = document.querySelectorAll('.file-card .ant-table-tbody > tr[data-row-key]');
  [...rows].find((row) => String(row.getAttribute('data-row-key')) === String(id))?.focus();
}

function closeFileContextMenu(restoreFocus = false) {
  const shouldRestore = restoreFocus && fileContextMenu.keyboard;
  const focusedId = focusedRowId.value;
  fileContextMenu.open = false;
  fileContextMenu.record = null;
  fileContextMenu.keyboard = false;
  if (shouldRestore) void restoreFocusedFileRow(focusedId);
}

function openBackgroundContextMenu(event) {
  if (!appState.logged_in || event.target?.closest?.('.ant-table-tbody > tr')) return;
  event.preventDefault();
  fileContextMenu.open = true;
  fileContextMenu.x = event.clientX;
  fileContextMenu.y = event.clientY;
  fileContextMenu.record = null;
  fileContextMenu.keyboard = false;
}

function handleFileContextOpenChange(open) {
  if (!open) closeFileContextMenu(true);
}

function selectedRecords() {
  const ids = new Set(selectedKeys.value);
  return files.value.filter((item) => ids.has(fileId(item)));
}

async function openDeveloperTransfer(records = selectedRecords()) {
  const targets = (Array.isArray(records) ? records : []).filter((item) => fileId(item));
  if (!targets.length) {
    message.warning('请先选择要传给小号的文件或文件夹');
    return;
  }
  if (targets.length > 20) {
    message.warning('开发者接口一次最多互传 20 项，请减少选择后重试');
    return;
  }
  developerTransfer.loading = true;
  try {
    const settings = unwrapData(await bridge.invoke('get_developer_settings'));
    if (!settings.enabled || !Array.isArray(settings.targets) || !settings.targets.length) {
      Modal.confirm({
        title: settings.enabled ? '先添加小号接收 TOKEN' : '先为当前账号开启开发者模式',
        content: settings.enabled
          ? '在“设置 → 账号 → 开发者模式”中添加小号生成的接收 TOKEN。'
          : '在“设置 → 账号”中填写并验证当前账号自己的 client_id / client_secret，开启开发者模式后添加小号接收 TOKEN。',
        okText: '去设置',
        cancelText: '取消',
        onOk: () => router.push({ name: 'settings' }),
      });
      return;
    }
    developerTransfer.records = targets;
    developerTransfer.targets = settings.targets;
    developerTransfer.targetId = settings.targets.some((item) => item.id === developerTransfer.targetId)
      ? developerTransfer.targetId
      : settings.targets[0].id;
    developerTransfer.open = true;
  } catch (error) {
    message.error(errorText(error));
  } finally {
    developerTransfer.loading = false;
  }
}

async function submitDeveloperTransfer() {
  if (!developerTransfer.targetId) {
    message.warning('请选择接收小号');
    return;
  }
  developerTransfer.submitting = true;
  try {
    const job = unwrapData(await bridge.invoke('start_developer_transfer', {
      target_id: developerTransfer.targetId,
      file_ids: developerTransfer.records.map(fileId),
      file_names: developerTransfer.records.map((item) => String(item.fileName || '未命名文件')),
    }));
    developerTransfer.open = false;
    clearSelection();
    message.success(job.reused ? '相同的互传任务已在处理中' : '已开始小号秒传；需要预审时会自动续传');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    developerTransfer.submitting = false;
  }
}

function contextTargetRecords() {
  const record = fileContextMenu.record;
  if (!record) return [];
  const id = fileId(record);
  if (id && selectedKeys.value.includes(id)) {
    const selected = selectedRecords();
    if (selected.length) return selected;
  }
  return [record];
}

async function handleFileContextMenuClick({ key }) {
  const record = fileContextMenu.record;
  const targets = record ? contextTargetRecords() : [];
  closeFileContextMenu(false);
  if (key === 'newFolder') return openNewFolderModal();
  if (key === 'paste') return pasteFileClipboard();
  if (key === 'refresh') return loadCloudFiles();
  if (!record) return;
  if (key === 'open') return enterFolder(record);
  if (key === 'copy') return setFileClipboard('copy', targets);
  if (key === 'cut') return setFileClipboard('move', targets);
  if (key === 'download') return downloadCloudFiles(targets);
  if (key === 'details') return openFileDetails(record);
  if (key === 'rename') return openRenameModal(targets);
  if (key === 'copyTo') return openFolderPicker('copy', targets);
  if (key === 'moveTo') return openFolderPicker('move', targets);
  if (key === 'share') return createCloudShare(targets);
  if (key === 'transferAccount') return openDeveloperTransfer(targets);
  if (key === 'delete') return deleteCloudFiles(targets);
}

function clearSelection() {
  selectedKeys.value = [];
  selectionAnchorId = '';
  closeFileContextMenu(false);
}

function selectAllFiles() {
  selectedKeys.value = files.value.map(fileId).filter(Boolean);
  selectionAnchorId = String(selectedKeys.value[0] || '');
}

function setFileClipboard(mode, records = selectedRecords()) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  fileClipboard.mode = mode;
  fileClipboard.items = targets
    .map((item) => ({ id: fileId(item), fileName: item.fileName, resType: item.resType }))
    .filter((item) => item.id);
  message.success(`已${mode === 'move' ? '剪切' : '复制'} ${fileClipboard.items.length} 项，可进入目标目录粘贴`);
}

function clearFileClipboard() {
  fileClipboard.mode = '';
  fileClipboard.items = [];
}

async function pasteFileClipboard() {
  if (!fileClipboard.items.length || operationBusy.value) return;
  operationBusy.value = true;
  try {
    const command = fileClipboard.mode === 'move' ? 'move_files' : 'copy_files';
    await bridge.invoke(command, {
      file_ids: [...new Set(fileClipboard.items.map((item) => item.id).filter(Boolean))],
      parent_id: currentFolderId.value,
    });
    if (fileClipboard.mode === 'move') clearFileClipboard();
    clearSelection();
    await loadCloudFiles();
    message.success(command === 'move_files' ? '已移动到当前目录' : '已复制到当前目录');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    operationBusy.value = false;
  }
}

function reconcileVisibleFileState() {
  const visibleIds = new Set(files.value.map((item) => String(fileId(item))).filter(Boolean));
  selectedKeys.value = selectedKeys.value.filter((id) => visibleIds.has(String(id)));
  if (focusedRowId.value && !visibleIds.has(String(focusedRowId.value))) focusedRowId.value = '';
  if (selectionAnchorId && !visibleIds.has(String(selectionAnchorId))) selectionAnchorId = String(selectedKeys.value.at(-1) || '');
}

async function loadCloudFiles(page = filesPage.value) {
  try {
    await loadFiles(page);
    reconcileVisibleFileState();
  } catch (error) {
    message.error(errorText(error));
  }
}

watch(() => route.query.focus, async (focusValue) => {
  const focusId = String(focusValue || '');
  if (!focusId) return;
  const parentId = String(route.query.parent || '');
  const parentName = String(route.query.parentName || '搜索结果目录');
  currentPath.value = parentId
    ? [{ id: '', name: '全部文件' }, { id: parentId, name: parentName }]
    : [{ id: '', name: '全部文件' }];
  clearSelection();
  focusedRowId.value = '';
  await loadCloudFiles(0);
  if (files.value.some((item) => fileId(item) === focusId)) selectedKeys.value = [focusId];
}, { immediate: true });

function enterFolder(record) {
  currentPath.value = [...currentPath.value, { id: fileId(record), name: record.fileName }];
  clearSelection();
  focusedRowId.value = '';
  loadCloudFiles(0);
}
function goBack() {
  if (currentPath.value.length <= 1) return;
  currentPath.value = currentPath.value.slice(0, -1);
  clearSelection();
  focusedRowId.value = '';
  loadCloudFiles(0);
}

function jumpToPath(index) {
  if (index < 0 || index >= currentPath.value.length - 1) return;
  currentPath.value = currentPath.value.slice(0, index + 1);
  clearSelection();
  focusedRowId.value = '';
  loadCloudFiles(0);
}

function handleFileTableChange(pagination) {
  const nextPage = Math.max(0, Number(pagination?.current || 1) - 1);
  if (nextPage === filesPage.value) return;
  clearSelection();
  focusedRowId.value = '';
  void loadCloudFiles(nextPage);
}

async function triggerUpload(kind = 'files') {
  if (!appState.logged_in) return;
  if (!isTauri) {
    (kind === 'folder' ? folderInput.value : fileInput.value)?.click();
    return;
  }
  uploading.value = true;
  try {
    const selection = kind === 'folder'
      ? await bridge.selectUploadFolder()
      : await bridge.selectUploadFiles();
    const paths = Array.isArray(selection) ? selection : selection ? [selection] : [];
    if (!paths.length) return;
    const count = await bridge.invoke('queue_upload_paths', { paths, parent_id: currentFolderId.value });
    message.success(`已加入上传队列：${Number(count || paths.length)} 个文件`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    uploading.value = false;
  }
}

async function downloadCloudFiles(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  try {
    const queued = await transfers.downloadRecords(targets);
    if (isTauri && queued) message.success('已加入下载队列');
  } catch (error) {
    message.error(errorText(error));
  }
}

function handleUploadMenuClick({ key }) {
  if (key === 'server') {
    void chooseServerUpload();
    return;
  }
  void triggerUpload(key);
}

async function loadServerDirectory(relativePath = '') {
  serverFilePicker.loading = true;
  try {
    const query = new URLSearchParams({ path: relativePath });
    const response = await fetch(`/api/server-files?${query}`);
    const payload = await readJsonResponse(response, '读取服务器目录失败');
    Object.assign(serverFilePicker, {
      roots: payload.roots || [],
      path: payload.path || '',
      parent: payload.parent || '',
      displayPath: payload.display_path || '/',
      items: payload.items || [],
    });
  } catch (error) {
    message.error(errorText(error));
  } finally {
    serverFilePicker.loading = false;
  }
}

async function chooseServerUpload() {
  if (isTauri) return;
  serverFilePicker.open = true;
  serverFilePicker.selected = [];
  await loadServerDirectory('');
}

function toggleServerSelection(item, checked) {
  const selected = new Set(serverFilePicker.selected);
  if (checked) selected.add(item.path);
  else selected.delete(item.path);
  serverFilePicker.selected = [...selected];
}

async function confirmServerUpload() {
  if (!serverFilePicker.selected.length) {
    message.warning('请至少选择一个服务器文件或文件夹');
    return;
  }
  serverFilePicker.submitting = true;
  try {
    const response = await fetch('/api/server-upload', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ paths: serverFilePicker.selected, parent_id: currentFolderId.value }),
    });
    const payload = await readJsonResponse(response, '加入服务器上传队列失败');
    serverFilePicker.open = false;
    if (payload.queued) message.success(`已加入上传队列：${payload.queued} 个文件${payload.skipped ? `，跳过已上传 ${payload.skipped} 个` : ''}`);
    else message.info(`没有需要上传的文件，已跳过 ${payload.skipped || 0} 个已上传文件`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    serverFilePicker.submitting = false;
  }
}

async function createCloudShare(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length || shareResult.creating) return;
  shareAccess.records = targets;
  shareAccess.mode = 'none';
  shareAccess.code = '';
  shareAccess.open = true;
}

function validateFolderName(value) {
  const name = String(value || '').trim();
  if (!name) return '请输入文件夹名称';
  if (name === '.' || name === '..') return '文件夹名称不能是 . 或 ..';
  if (name.length > 255) return '文件夹名称不能超过 255 个字符';
  if (/[\\/:*?"<>|\u0000-\u001f]/.test(name)) return '名称不能包含 \\ / : * ? " < > | 或控制字符';
  return '';
}

function openNewFolderModal() {
  if (!appState.logged_in) return;
  newFolderModal.name = '';
  newFolderModal.error = '';
  newFolderModal.open = true;
}

function focusNewFolderInput(open) {
  if (!open) return;
  void nextTick(() => newFolderInput.value?.focus?.({ cursor: 'all' }));
}

async function submitNewFolder() {
  if (newFolderModal.saving) return;
  newFolderModal.error = validateFolderName(newFolderModal.name);
  if (newFolderModal.error) {
    void nextTick(() => newFolderInput.value?.focus?.());
    return;
  }
  const name = newFolderModal.name.trim();
  newFolderModal.saving = true;
  try {
    const data = unwrapData(await bridge.invoke('create_folder', {
      parent_id: currentFolderId.value,
      dir_name: name,
      fail_if_name_exist: true,
    }));
    newFolderModal.open = false;
    await loadCloudFiles(0);
    const createdId = pick(data, ['fileId', 'file_id', 'id'], '')
      || fileId(files.value.find((item) => isFolder(item) && item.fileName === name));
    if (createdId) {
      selectedKeys.value = [createdId];
      focusedRowId.value = String(createdId);
      await restoreFocusedFileRow(createdId);
    }
    message.success(`文件夹「${name}」已创建`);
  } catch (error) {
    newFolderModal.error = errorText(error);
    message.error(newFolderModal.error);
    void nextTick(() => newFolderInput.value?.focus?.());
  } finally {
    newFolderModal.saving = false;
  }
}

function openFileDetails(record) {
  if (!record || !fileId(record)) return;
  detailsRecord.value = record;
  focusedRowId.value = String(fileId(record));
  detailsOpen.value = true;
}

function handleDetailsClosed() {
  void restoreFocusedFileRow(focusedRowId.value);
}

async function confirmCloudShare() {
  const targets = shareAccess.records;
  if (!targets.length || shareResult.creating) return;
  const code = shareAccess.code.trim();
  if (shareAccess.mode === 'fixed' && !/^[A-Za-z0-9]{4}$/.test(code)) {
    message.warning('固定访问码必须是 4 位英文或数字');
    return;
  }
  const names = targets
    .map((item) => String(pick(item, ['fileName', 'name'], '')).trim())
    .filter(Boolean);
  const title = names.length > 1 ? `${names[0]} 等 ${names.length} 项` : names[0] || '云盘分享';
  const targetType = targets.length === 1 && isFolder(targets[0]) ? 'folder' : 'file';
  const shareType = ({ none: 0, random: 1, fixed: 2 })[shareAccess.mode] ?? 0;
  shareResult.creating = true;
  const closeProgress = message.loading('正在创建分享，请稍候…', 0);
  try {
    const data = unwrapData(await bridge.invoke('create_share', {
      file_ids: targets.map(fileId).filter(Boolean),
      title,
      target_type: targetType,
      share_type: shareType,
      code: shareType === 2 ? code : '',
      auto_fill_code: false,
    }));
    const url = pick(data, ['shareUrl', 'share_url', 'url'], '');
    if (!url) throw new Error('光鸭没有返回分享链接');
    let code = String(pick(data, ['code', 'extractCode'], '') || '').trim();
    if (!code) {
      try { code = parseGuangyaShareLink(url).code; } catch { /* 部分历史链接不带标准域名。 */ }
    }
    Object.assign(shareResult, {
      open: true,
      label: title,
      url,
      code,
      reused: data.reused_existing === true,
      hdhiveEventId: String(pick(data, ['hdhive_event_id'], '') || ''),
      hdhiveStatus: String(pick(data, ['hdhive_status'], 'disabled') || 'disabled'),
      hdhiveMessage: String(pick(data, ['hdhive_message'], '光鸭分享已创建') || ''),
    });
    shareAccess.open = false;
    if (['accepted', 'processing', 'completed'].includes(shareResult.hdhiveStatus)) {
      message.success(shareResult.reused ? '已复用已有分享，Hdhive 将更新现有内容' : '分享已创建，Hdhive 正在处理');
    } else if (shareResult.hdhiveStatus === 'disabled') {
      message.success(shareResult.reused ? '已复用已有分享' : '分享已创建');
    } else {
      message.warning(shareResult.hdhiveMessage);
    }
  } catch (error) {
    message.error(errorText(error));
  } finally {
    closeProgress?.();
    shareResult.creating = false;
  }
}

function shareUrlForSave(url, code) {
  if (!code) return url;
  try {
    const parsed = new URL(url);
    if (!parsed.searchParams.get('code')) parsed.searchParams.set('code', code);
    return parsed.toString();
  } catch {
    return url;
  }
}

async function saveCreatedShare() {
  if (!shareResult.url || shareResult.saving) return;
  shareResult.saving = true;
  try {
    await bridge.invoke('save_share_link', {
      label: shareResult.label || '分享链接',
      url: shareUrlForSave(shareResult.url, shareResult.code),
    });
    shareResult.open = false;
    message.success('分享链接已加入收藏');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    shareResult.saving = false;
  }
}

async function deleteCloudFiles(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  const ids = targets.map((item) => fileId(item)).filter(Boolean);
  if (!ids.length) return;
  const label = targets.length === 1 ? `「${targets[0].fileName}」` : `选中的 ${targets.length} 项`;
  Modal.confirm({
    title: '删除确认',
    content: `确定删除 ${label} 吗？删除后可在云盘回收站找回。`,
    okText: '删除',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      try {
        await bridge.invoke('delete_files', { file_ids: ids });
        selectedKeys.value = [];
        await loadCloudFiles();
        message.success('已移入回收站');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
}

function createRenameRule(type = 'replace', seed = '') {
  return {
    id: ++renameRuleId,
    type,
    value: seed,
    search: '',
    replacement: '',
    ignoreCase: false,
    start: 1,
    padding: 2,
  };
}

function openRenameModal(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  renameModal.records = targets;
  renameModal.mode = targets.length > 1 ? 'rules' : 'single';
  renameModal.singleName = targets[0]?.fileName || '';
  renameModal.preserveExtension = true;
  renameModal.rules = targets.length > 1 ? [createRenameRule('replace')] : [];
  renameModal.open = true;
}

const renamePreview = computed(() => {
  if (!renameModal.records.length) return { rows: [], error: '' };
  if (renameModal.mode === 'single') {
    return buildRenamePreview(
      renameModal.records,
      [{ type: 'set', value: renameModal.singleName }],
      false,
    );
  }
  return buildRenamePreview(
    renameModal.records,
    renameModal.rules,
    renameModal.preserveExtension,
  );
});

const renameChangedCount = computed(() => renamePreview.value.rows
  .filter((row) => row.currentName !== row.newName).length);

function addRenameRule() {
  renameModal.rules.push(createRenameRule('replace'));
}

function removeRenameRule(index) {
  if (renameModal.rules.length <= 1) return;
  renameModal.rules.splice(index, 1);
}

function moveRenameRule(index, direction) {
  const target = index + direction;
  if (target < 0 || target >= renameModal.rules.length) return;
  const [rule] = renameModal.rules.splice(index, 1);
  renameModal.rules.splice(target, 0, rule);
}

async function submitRename() {
  if (!renameModal.records.length) return;
  if (renamePreview.value.error) {
    message.error(renamePreview.value.error);
    return;
  }
  const renames = renamePreview.value.rows
    .filter((item) => item.fileId && item.newName !== item.currentName);
  if (!renames.length) {
    message.info('文件名没有变化');
    return;
  }
  renameModal.saving = true;
  try {
    await bridge.invoke('batch_rename_files', { renames });
    renameModal.open = false;
    clearSelection();
    await loadCloudFiles();
    message.success(`已重命名 ${renames.length} 项`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    renameModal.saving = false;
  }
}

async function openFolderPicker(action, records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  const ids = targets.map((item) => fileId(item)).filter(Boolean);
  if (!ids.length) return;
  folderPicker.action = action;
  folderPicker.title = action === 'copy' ? '复制到…' : '移动到…';
  folderPicker.sourceIds = ids;
  folderPicker.path = [{ id: '', name: '全部文件' }];
  folderPicker.targetId = '';
  folderPicker.open = true;
  await loadFolderPickerOptions('', 0);
}
async function loadFolderPickerOptions(parentId, page = 0) {
  folderPicker.loading = true;
  try {
    const normalizedPage = Math.max(0, Math.floor(Number(page) || 0));
    const data = unwrapData(await bridge.invoke('list_files', { page: normalizedPage, parent_id: parentId }));
    folderPicker.options = (data.list || []).filter((item) => isFolder(item) && !folderPicker.sourceIds.includes(fileId(item)));
    folderPicker.page = normalizedPage;
    folderPicker.total = Math.max(folderPicker.options.length, Number(data.total ?? folderPicker.options.length) || 0);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    folderPicker.loading = false;
  }
}
function folderPickerRowProps(record) {
  return {
    tabindex: 0,
    onDblclick: () => enterFolderPicker(record),
    onKeydown: (event) => {
      if (event.key === 'Enter') enterFolderPicker(record);
    },
  };
}
function enterFolderPicker(record) {
  folderPicker.path = [...folderPicker.path, { id: fileId(record), name: record.fileName }];
  folderPicker.targetId = '';
  loadFolderPickerOptions(fileId(record), 0);
}
function folderPickerBack() {
  if (folderPicker.path.length <= 1) return;
  folderPicker.path = folderPicker.path.slice(0, -1);
  folderPicker.targetId = '';
  loadFolderPickerOptions(folderPicker.path[folderPicker.path.length - 1]?.id || '', 0);
}
function folderPickerJump(index) {
  folderPicker.path = folderPicker.path.slice(0, index + 1);
  folderPicker.targetId = '';
  loadFolderPickerOptions(folderPicker.path[index]?.id || '', 0);
}
function handleFolderPickerTableChange(pagination) {
  const nextPage = Math.max(0, Number(pagination?.current || 1) - 1);
  if (nextPage === folderPicker.page) return;
  folderPicker.targetId = '';
  void loadFolderPickerOptions(folderPicker.path.at(-1)?.id || '', nextPage);
}
async function submitFolderPicker() {
  const command = folderPicker.action === 'copy' ? 'copy_files' : 'move_files';
  try {
    const targetId = folderPicker.targetId || folderPicker.path.at(-1)?.id || '';
    await bridge.invoke(command, { file_ids: [...new Set(folderPicker.sourceIds)], parent_id: targetId });
    folderPicker.open = false;
    selectedKeys.value = [];
    await loadCloudFiles();
    message.success(folderPicker.action === 'copy' ? '已复制' : '已移动');
  } catch (error) {
    message.error(errorText(error));
  }
}

function stopGcidImportPolling() {
  clearInterval(gcidImportPollTimer);
  gcidImportPollTimer = null;
}

function resetGcidImportDraft() {
  stopGcidImportPolling();
  Object.assign(gcidImport, {
    detailsOpen: false,
    loading: false,
    sourcePath: '',
    sourceName: '',
    pastedJson: '',
    destinationName: '',
    concurrency: 4,
    status: null,
  });
}

function applyGcidImportStatus(status) {
  if (!status) return;
  gcidImport.status = status;
  if (status.source_path) {
    gcidImport.sourcePath = status.source_path;
    gcidImport.sourceName = status.source_name || uploadFileName(status.source_path);
  }
  if (status.destination_name) gcidImport.destinationName = status.destination_name;
  if (!['preparing', 'running'].includes(status.status)) {
    gcidImport.detailsOpen = false;
    stopGcidImportPolling();
  }
}

async function refreshGcidImportStatus(jobId = gcidImport.status?.job_id) {
  const status = unwrapData(await bridge.invoke('get_gcid_import_status', { job_id: jobId || null }));
  if (status) applyGcidImportStatus(status);
  return status;
}

function startGcidImportPolling() {
  stopGcidImportPolling();
  gcidImportPollTimer = setInterval(async () => {
    try {
      const previousStatus = gcidImport.status?.status;
      const status = await refreshGcidImportStatus();
      if (['preparing', 'running'].includes(previousStatus) && status && !['preparing', 'running'].includes(status.status)) {
        await loadCloudFiles();
        if (status.status === 'completed') message.success('GCID JSON 秒传导入完成');
        else message.warning(status.error || 'GCID 导入结束，部分记录需要处理');
      }
    } catch {
      // 后台任务继续运行，短暂轮询失败不打断导入。
    }
  }, 1000);
}

async function openGcidImport() {
  try {
    const latest = await refreshGcidImportStatus();
    if (latest && ['preparing', 'running'].includes(latest.status)) {
      startGcidImportPolling();
      gcidImport.detailsOpen = true;
      return;
    }
  } catch {
    // 没有历史任务时直接创建新的导入。
  }
  resetGcidImportDraft();
  gcidImport.destinationName = 'GCID 导入';
  gcidImport.open = true;
}

async function resumeGcidImport() {
  try {
    const latest = await refreshGcidImportStatus();
    if (latest && ['preparing', 'running'].includes(latest.status)) startGcidImportPolling();
  } catch {
    // 当前没有可恢复的导入任务。
  }
}

async function selectGcidImportFile() {
  try {
    const path = await bridge.invoke('select_gcid_import_file');
    if (!path) return;
    gcidImport.sourcePath = path;
    gcidImport.sourceName = uploadFileName(path);
    gcidImport.pastedJson = '';
    if (!gcidImport.destinationName || gcidImport.destinationName === 'GCID 导入') {
      gcidImport.destinationName = gcidImport.sourceName.replace(/\.json$/i, '') || 'GCID 导入';
    }
  } catch (error) {
    message.error(errorText(error));
  }
}

async function submitGcidImport() {
  if (!gcidImport.sourcePath && !gcidImport.pastedJson.trim()) {
    message.warning('请选择 GCID JSON 文件或粘贴 JSON 内容');
    return;
  }
  if (!gcidImport.destinationName.trim()) {
    message.warning('请输入云端目标文件夹名称');
    return;
  }
  gcidImport.loading = true;
  try {
    let sourcePath = gcidImport.sourcePath;
    if (!sourcePath) {
      const staged = unwrapData(await bridge.invoke('stage_gcid_import_text', { content: gcidImport.pastedJson }));
      sourcePath = staged.path;
      gcidImport.sourcePath = staged.path;
      gcidImport.sourceName = staged.name;
    }
    let status = gcidImport.status;
    if (!status || status.source_path !== sourcePath || status.destination_parent_id !== currentFolderId.value || status.destination_name !== gcidImport.destinationName.trim()) {
      status = unwrapData(await bridge.invoke('prepare_gcid_import', {
        source_path: sourcePath,
        destination_parent_id: currentFolderId.value,
        destination_name: gcidImport.destinationName.trim(),
      }));
      applyGcidImportStatus(status);
    }
    status = unwrapData(await bridge.invoke('start_gcid_import', {
      job_id: status.job_id,
      concurrency: Math.min(16, Math.max(1, Math.round(Number(gcidImport.concurrency) || 4))),
    }));
    applyGcidImportStatus(status);
    startGcidImportPolling();
    gcidImport.open = false;
    message.success('GCID 导入已启动，可在后台继续运行');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    gcidImport.loading = false;
  }
}

async function readDirectoryEntry(entry, prefix, result) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    result.push({ file, relativePath: `${prefix}${file.name}` });
    return;
  }
  if (!entry.isDirectory) return;
  const reader = entry.createReader();
  const entries = [];
  while (true) {
    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    if (!batch.length) break;
    entries.push(...batch);
  }
  for (const child of entries) await readDirectoryEntry(child, `${prefix}${entry.name}/`, result);
}

async function filesFromTransfer(dataTransfer) {
  const entries = [...(dataTransfer.items || [])]
    .map((item) => item.webkitGetAsEntry?.())
    .filter(Boolean);
  if (entries.length) {
    const result = [];
    for (const entry of entries) await readDirectoryEntry(entry, '', result);
    return result;
  }
  return [...(dataTransfer.files || [])].map((file) => ({
    file,
    relativePath: file.webkitRelativePath || file.name,
  }));
}

async function uploadWebFiles(entries) {
  if (!entries.length) return;
  uploading.value = true;
  let cursor = 0;
  let queued = 0;
  let skipped = 0;
  try {
    const worker = async () => {
      while (cursor < entries.length) {
        const entry = entries[cursor++];
        const query = new URLSearchParams({
          parentId: currentFolderId.value,
          fileName: entry.file.name,
          relativePath: entry.relativePath || entry.file.name,
          lastModified: String(entry.file.lastModified || 0),
        });
        const eventPath = `[浏览器]/${entry.relativePath || entry.file.name}`;
        transfers.handleSyncEvent({
          type: 'file', state: 'uploading', file_path: eventPath,
          uploaded_bytes: 0, total_bytes: entry.file.size, stage: '正在传到服务器',
        });
        const payload = await new Promise((resolve, reject) => {
          const request = new XMLHttpRequest();
          const startedAt = performance.now();
          request.open('POST', `/api/upload?${query}`);
          request.setRequestHeader('content-type', entry.file.type || 'application/octet-stream');
          request.upload.onprogress = (event) => {
            const total = event.lengthComputable ? event.total : entry.file.size;
            const elapsed = Math.max((performance.now() - startedAt) / 1000, .001);
            transfers.handleSyncEvent({
              type: 'progress', file_path: eventPath,
              percent: total ? Math.round(event.loaded / total * 100) : 0,
              uploaded_bytes: event.loaded,
              total_bytes: total,
              bytes_per_second: event.loaded / elapsed,
              stage: '正在传到服务器',
            });
          };
          request.onerror = () => {
            const error = new Error(`上传接口网络错误：${entry.file.name}`);
            transfers.handleSyncEvent({ type: 'file', state: 'error', file_path: eventPath, total_bytes: entry.file.size, error: error.message });
            reject(error);
          };
          request.onload = async () => {
            try {
              const response = new Response(request.responseText || '', {
                status: request.status || 500,
                headers: { 'content-type': request.getResponseHeader('content-type') || 'text/plain' },
              });
              const result = await readJsonResponse(response, `上传接口失败：${entry.file.name}`);
              if (result.skipped) transfers.handleSyncEvent({ type: 'file', state: 'done', file_path: eventPath, uploaded_bytes: entry.file.size, total_bytes: entry.file.size, stage: '文件未变化，已跳过' });
              resolve(result);
            } catch (error) {
              transfers.handleSyncEvent({ type: 'file', state: 'error', file_path: eventPath, total_bytes: entry.file.size, error: error.message });
              reject(error);
            }
          };
          request.send(entry.file);
        });
        queued += Number(payload.queued || 0);
        skipped += Number(payload.skipped || 0);
        if (payload.skipped) transfers.handleSyncEvent({ type: 'file', state: 'done', file_path: eventPath, stage: '文件未变化，已跳过' });
      }
    };
    await Promise.all([worker(), worker()]);
    if (queued) message.success(`已加入上传队列：${queued} 个文件${skipped ? `，跳过 ${skipped} 个` : ''}`);
    else message.info(`没有需要上传的文件，已跳过 ${skipped} 个`);
    await loadCloudFiles();
  } catch (error) {
    message.error(errorText(error));
  } finally {
    uploading.value = false;
  }
}

async function handleWebInput(event) {
  const entries = [...event.target.files].map((file) => ({ file, relativePath: file.webkitRelativePath || file.name }));
  event.target.value = '';
  await uploadWebFiles(entries);
}

function handleWindowDragOver(event) {
  if (!appState.logged_in) return;
  if (!event.dataTransfer?.types?.includes('Files')) return;
  event.preventDefault();
  if (!isTauri) dragActive.value = true;
}
function handleWindowDragLeave(event) {
  if (!isTauri && event.relatedTarget == null) dragActive.value = false;
}
function handleWindowDragEnd() {
  if (!isTauri) dragActive.value = false;
}
async function handleWindowDrop(event) {
  event.preventDefault();
  if (isTauri || !appState.logged_in) return;
  dragActive.value = false;
  await uploadWebFiles(await filesFromTransfer(event.dataTransfer));
}
function handleWindowClick(event) {
  if (!fileContextMenu.open) return;
  if (event.target?.closest?.('.ant-dropdown, .ant-dropdown-menu')) return;
  closeFileContextMenu(true);
}

useFileKeyboardShortcuts({
  getState: () => ({
    blocked: fileContextMenu.open || shareAccess.open || shareResult.open || shareResult.creating || renameModal.open || folderPicker.open || gcidImport.open || gcidImport.detailsOpen || developerTransfer.open || serverFilePicker.open || operationBusy.value,
    fileCount: files.value.length,
    selectedCount: selectedKeys.value.length,
    clipboardCount: fileClipboard.items.length,
    canGoBack: currentPath.value.length > 1,
    contextMenuOpen: fileContextMenu.open,
  }),
  actions: {
    selectAll: selectAllFiles,
    copy: () => setFileClipboard('copy'),
    cut: () => setFileClipboard('move'),
    paste: pasteFileClipboard,
    rename: () => openRenameModal(selectedRecords()),
    delete: () => deleteCloudFiles(selectedRecords()),
    goBack,
    refresh: () => loadCloudFiles(),
    clearSelection,
  },
});

let unlistenDrag = null;
let unlistenDeveloperTransfer = null;
onMounted(async () => {
  window.addEventListener('keydown', handleFileContextMenuKeydown, true);
  window.addEventListener('dragover', handleWindowDragOver);
  window.addEventListener('dragleave', handleWindowDragLeave);
  window.addEventListener('dragend', handleWindowDragEnd);
  window.addEventListener('drop', handleWindowDrop);
  window.addEventListener('click', handleWindowClick);
  unlistenDeveloperTransfer = await bridge.subscribe((payload) => {
    const job = payload?.type === 'developer-transfer' ? payload.job : null;
    if (!job?.id || !['success', 'failed'].includes(job.status) || developerTerminalNotified.has(job.id)) return;
    developerTerminalNotified.add(job.id);
    if (job.status === 'success') message.success(job.message || `已完成到 ${job.target_name} 的小号秒传`);
    else message.error(job.message || `传到 ${job.target_name} 失败`);
  });
  if (isTauri) {
    void resumeGcidImport();
    unlistenDrag = await bridge.subscribeDrag(async (phase, payload) => {
      if (phase === 'enter') {
        dragDepth.value += 1;
        dragActive.value = true;
      }
      if (phase === 'leave') {
        dragDepth.value = Math.max(0, dragDepth.value - 1);
        if (!dragDepth.value) dragActive.value = false;
      }
      if (phase === 'drop') {
        dragDepth.value = 0;
        dragActive.value = false;
        const paths = payload?.paths || [];
        if (!paths.length || !appState.logged_in) return;
        try {
          const count = await bridge.invoke('queue_upload_paths', { paths, parent_id: currentFolderId.value });
          message.success(`已加入上传队列：${Number(count || paths.length)} 个文件`);
        } catch (error) {
          message.error(errorText(error));
        }
      }
    });
  }
});
onBeforeUnmount(() => {
  stopGcidImportPolling();
  window.removeEventListener('keydown', handleFileContextMenuKeydown, true);
  window.removeEventListener('dragover', handleWindowDragOver);
  window.removeEventListener('dragleave', handleWindowDragLeave);
  window.removeEventListener('dragend', handleWindowDragEnd);
  window.removeEventListener('drop', handleWindowDrop);
  window.removeEventListener('click', handleWindowClick);
  unlistenDrag?.();
  unlistenDeveloperTransfer?.();
});
</script>

<template>
  <div class="cloud-view">
    <a-card class="content-card file-card file-drop-surface" :class="{ 'drag-active': dragActive }" :bordered="false">
      <input ref="fileInput" class="hidden-file-input" type="file" multiple @change="handleWebInput" />
      <input ref="folderInput" class="hidden-file-input" type="file" multiple webkitdirectory directory @change="handleWebInput" />
      <div class="file-toolbar" :class="{ 'selection-mode': fileActionBarVisible }">
        <FileSelectionBar
          v-if="fileActionBarVisible"
          :selected-count="selectedKeys.length"
          :clipboard-count="fileClipboard.items.length"
          :clipboard-mode="fileClipboard.mode"
          @copy="setFileClipboard('copy')"
          @cut="setFileClipboard('move')"
          @move="openFolderPicker('move', selectedRecords())"
          @rename="openRenameModal(selectedRecords())"
          @download="downloadCloudFiles(selectedRecords())"
          @share="createCloudShare(selectedRecords())"
          @transfer-account="openDeveloperTransfer(selectedRecords())"
          @delete="deleteCloudFiles(selectedRecords())"
          @paste="pasteFileClipboard"
          @clear-selection="clearSelection"
          @clear-clipboard="clearFileClipboard"
        >
          <template #status>
            <GcidImportStatus v-if="gcidImportRunning" v-model:open="gcidImport.detailsOpen" :status="gcidImport.status" :percent="gcidImportProgress" />
          </template>
        </FileSelectionBar>

        <template v-else>
          <a-flex align="center" gap="small" class="file-path-actions">
            <a-button :disabled="currentPath.length <= 1" @click="goBack"><template #icon><ArrowLeftOutlined /></template>返回</a-button>
            <CompactFileBreadcrumb :segments="currentPath" @navigate="jumpToPath($event.index)" />
          </a-flex>
          <a-flex align="center" gap="small" class="file-primary-actions">
            <GcidImportStatus v-if="gcidImportRunning" v-model:open="gcidImport.detailsOpen" :status="gcidImport.status" :percent="gcidImportProgress" />
            <a-button v-if="isTauri && !gcidImportRunning" :disabled="!appState.logged_in" @click="openGcidImport"><template #icon><FileAddOutlined /></template>JSON 秒传</a-button>
            <a-button :disabled="!appState.logged_in" @click="openNewFolderModal"><template #icon><FolderAddOutlined /></template>新建文件夹</a-button>
            <a-dropdown :menu="{ items: uploadMenuItems, onClick: handleUploadMenuClick }" :trigger="['click']">
              <a-button type="primary" :loading="uploading" :disabled="!appState.logged_in"><template #icon><UploadOutlined /></template>上传</a-button>
            </a-dropdown>
            <a-button :loading="filesLoading" :disabled="!appState.logged_in" @click="loadCloudFiles()"><template #icon><ReloadOutlined /></template>刷新</a-button>
          </a-flex>
        </template>
      </div>

      <div class="file-list-region" @contextmenu="openBackgroundContextMenu">
        <a-table :columns="fileColumns" :data-source="files" :loading="filesLoading" :row-key="fileId" :row-selection="rowSelection" :on-row="fileRowProps" :pagination="filePagination" :scroll="{ y: 'clamp(240px, calc(100vh - 330px), 640px)' }" size="small" @change="handleFileTableChange">
          <template #emptyText>
            <a-empty :description="appState.logged_in ? '此文件夹为空' : '登录后查看云盘文件'">
              <a-button v-if="!appState.logged_in" type="primary" @click="$emit('login')">去登录</a-button>
            </a-empty>
          </template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon" :class="fileIcon(record).cls"><component :is="fileIcon(record).icon" /></div>
                <div class="file-name-wrap">
                  <span v-if="isFolder(record)" class="file-name clickable">{{ record.fileName }}</span>
                  <span v-else class="file-name">{{ record.fileName }}</span>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'type'"><span class="file-type">{{ fileTypeLabel(record) }}</span></template>
            <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
            <template v-else-if="column.key === 'time'">{{ fileModifiedTime(record) }}</template>
          </template>
        </a-table>

        <div class="file-footer">
          <span>{{ filesTotal }} 个项目{{ selectedKeys.length ? ` · 已选 ${selectedKeys.length} 项` : '' }}</span>
        </div>
      </div>
    </a-card>

    <teleport to="body">
      <a-dropdown v-model:open="fileContextMenu.open" :trigger="['contextmenu']" :auto-focus="fileContextMenu.keyboard" :menu="{ items: fileContextMenuItems, onClick: handleFileContextMenuClick }" @open-change="handleFileContextOpenChange">
        <span class="file-context-anchor" :style="{ left: `${fileContextMenu.x}px`, top: `${fileContextMenu.y}px` }" />
      </a-dropdown>
    </teleport>

    <FileDetailsDrawer v-model:open="detailsOpen" :record="detailsRecord" @closed="handleDetailsClosed" />

    <a-modal
      v-model:open="newFolderModal.open"
      title="新建文件夹"
      ok-text="创建"
      cancel-text="取消"
      :confirm-loading="newFolderModal.saving"
      :ok-button-props="{ disabled: Boolean(newFolderModal.error) && !newFolderModal.name.trim() }"
      @after-open-change="focusNewFolderInput"
      @ok="submitNewFolder"
    >
      <a-form layout="vertical" @submit.prevent="submitNewFolder">
        <a-form-item label="文件夹名称" :validate-status="newFolderModal.error ? 'error' : undefined" :help="newFolderModal.error || undefined">
          <a-input
            ref="newFolderInput"
            v-model:value="newFolderModal.name"
            aria-label="文件夹名称"
            :maxlength="255"
            show-count
            placeholder="请输入文件夹名称"
            @input="newFolderModal.error = validateFolderName(newFolderModal.name)"
            @press-enter="submitNewFolder"
          />
        </a-form-item>
      </a-form>
    </a-modal>

    <a-modal
      v-model:open="shareAccess.open"
      title="创建分享"
      ok-text="创建分享"
      cancel-text="取消"
      :confirm-loading="shareResult.creating"
      @ok="confirmCloudShare"
    >
      <a-form layout="vertical">
        <a-form-item label="访问码">
          <a-radio-group v-model:value="shareAccess.mode">
            <a-radio-button value="none">不设置</a-radio-button>
            <a-radio-button value="random">随机</a-radio-button>
            <a-radio-button value="fixed">固定</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item v-if="shareAccess.mode === 'fixed'" label="固定访问码" extra="仅支持 4 位英文字母或数字">
          <a-input v-model:value="shareAccess.code" :maxlength="4" placeholder="4 位英文或数字" @press-enter="confirmCloudShare" />
        </a-form-item>
      </a-form>
    </a-modal>

    <ShareResultDialog v-model:open="shareResult.open" :result="shareResultView" :saving="shareResult.saving" @save="saveCreatedShare" />

    <a-modal
      v-model:open="renameModal.open"
      title="重命名"
      :confirm-loading="renameModal.saving"
      :ok-button-props="{ disabled: Boolean(renamePreview.error) || renameChangedCount === 0 }"
      ok-text="应用"
      cancel-text="取消"
      :width="renameModal.mode === 'single' ? 520 : 860"
      @ok="submitRename"
    >
      <template v-if="renameModal.mode === 'single'">
        <a-input v-model:value="renameModal.singleName" aria-label="新的文件名" placeholder="输入新的文件名" @press-enter="submitRename" />
        <a-alert v-if="renamePreview.error" class="rename-preview-error" type="error" show-icon :message="renamePreview.error" />
      </template>
      <template v-else>
        <div class="rename-summary">
          <div>
            <strong>{{ renameModal.records.length }} 个项目</strong>
            <span>规则将从上到下依次执行</span>
          </div>
          <a-checkbox v-model:checked="renameModal.preserveExtension">保留文件扩展名</a-checkbox>
        </div>

        <div class="rename-rules">
          <div
            v-for="(rule, index) in renameModal.rules"
            :key="rule.id"
            class="rename-rule"
            :class="{ compact: !['replace', 'regex', 'sequence'].includes(rule.type) }"
          >
            <div class="rule-order" :title="`第 ${index + 1} 条规则`">
              <DragOutlined aria-hidden="true" />
              <span>{{ index + 1 }}</span>
            </div>
            <a-select v-model:value="rule.type" class="rule-type" :options="renameRuleOptions" :aria-label="`第 ${index + 1} 条规则类型`" />

            <template v-if="['replace', 'regex'].includes(rule.type)">
              <a-input v-model:value="rule.search" :aria-label="`第 ${index + 1} 条规则查找内容`" :placeholder="rule.type === 'regex' ? '正则表达式' : '查找内容'" />
              <a-input v-model:value="rule.replacement" :aria-label="`第 ${index + 1} 条规则替换内容`" placeholder="替换为（可留空）" />
              <a-checkbox v-model:checked="rule.ignoreCase">忽略大小写</a-checkbox>
            </template>
            <template v-else-if="rule.type === 'sequence'">
              <a-input v-model:value="rule.value" :aria-label="`第 ${index + 1} 条规则序号格式`" placeholder="例如 -{n}" />
              <a-input-number v-model:value="rule.start" :min="0" :precision="0" addon-before="起始" :aria-label="`第 ${index + 1} 条规则起始序号`" />
              <a-input-number v-model:value="rule.padding" :min="1" :max="12" :precision="0" addon-before="位数" :aria-label="`第 ${index + 1} 条规则序号位数`" />
            </template>
            <template v-else-if="['set', 'prefix', 'suffix'].includes(rule.type)">
              <a-input v-model:value="rule.value" :aria-label="`第 ${index + 1} 条规则内容`" :placeholder="renameRuleValuePlaceholders[rule.type]" />
            </template>
            <span v-else class="rule-description">{{ rule.type === 'upper' ? '将当前名称转为大写' : '将当前名称转为小写' }}</span>

            <div class="rule-actions">
              <a-button type="text" size="small" :disabled="index === 0" :aria-label="`上移第 ${index + 1} 条规则`" @click="moveRenameRule(index, -1)"><template #icon><ArrowUpOutlined /></template></a-button>
              <a-button type="text" size="small" :disabled="index === renameModal.rules.length - 1" :aria-label="`下移第 ${index + 1} 条规则`" @click="moveRenameRule(index, 1)"><template #icon><ArrowDownOutlined /></template></a-button>
              <a-button type="text" size="small" danger :disabled="renameModal.rules.length <= 1" :aria-label="`删除第 ${index + 1} 条规则`" @click="removeRenameRule(index)"><template #icon><DeleteOutlined /></template></a-button>
            </div>
          </div>
        </div>

        <a-button block @click="addRenameRule"><template #icon><FileAddOutlined /></template>添加规则</a-button>
        <a-alert v-if="renamePreview.error" class="rename-preview-error" type="error" show-icon :message="renamePreview.error" />
        <div v-else class="rename-preview">
          <div class="preview-head">
            <span>结果预览</span>
            <span>{{ renameChangedCount }} / {{ renamePreview.rows.length }} 项将被修改</span>
          </div>
          <div class="preview-list">
            <div v-for="row in renamePreview.rows" :key="row.fileId">
              <span :title="row.currentName">{{ row.currentName }}</span>
              <SwapOutlined aria-hidden="true" />
              <strong :class="{ unchanged: row.currentName === row.newName }" :title="row.newName">{{ row.newName }}</strong>
            </div>
          </div>
        </div>
      </template>
    </a-modal>

    <a-modal v-model:open="folderPicker.open" :title="folderPicker.title" ok-text="确定" cancel-text="取消" width="520px" @ok="submitFolderPicker">
      <div class="folder-picker-toolbar">
        <a-button size="small" aria-label="返回上一级目录" :disabled="folderPicker.path.length <= 1" @click="folderPickerBack"><template #icon><ArrowLeftOutlined /></template></a-button>
        <a-breadcrumb>
          <a-breadcrumb-item v-for="(item, index) in folderPicker.path" :key="item.id || 'root'">
            <button type="button" class="folder-crumb-button" @click="folderPickerJump(index)">{{ item.name }}</button>
          </a-breadcrumb-item>
        </a-breadcrumb>
      </div>
      <a-table :columns="folderPickerColumns" :data-source="folderPicker.options" :loading="folderPicker.loading" :row-key="fileId" :row-selection="folderPickerRowSelection" :on-row="folderPickerRowProps" :pagination="folderPickerPagination" size="small" :scroll="{ y: 280 }" @change="handleFolderPickerTableChange">
        <template #emptyText><a-empty description="此目录下没有可选文件夹" /></template>
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'name'">
            <a-flex align="center" gap="small">
              <div class="file-icon folder"><FolderOutlined /></div>
              <span class="file-name clickable">{{ record.fileName }}</span>
            </a-flex>
          </template>
          <template v-else-if="column.key === 'time'">{{ formatTime(record.lastUpdateTime) }}</template>
        </template>
      </a-table>
      <small class="modal-hint">不选择文件夹则{{ folderPicker.action === 'copy' ? '复制' : '移动' }}到当前目录：{{ folderPicker.path[folderPicker.path.length - 1]?.name }}</small>
    </a-modal>

    <a-modal
      v-model:open="developerTransfer.open"
      title="秒传到小号"
      ok-text="开始秒传"
      cancel-text="取消"
      :width="520"
      :confirm-loading="developerTransfer.submitting"
      :mask-closable="!developerTransfer.submitting"
      @ok="submitDeveloperTransfer"
    >
      <a-alert type="info" show-icon class="developer-transfer-note">
        <template #message>服务端直接复制，不下载文件</template>
        <template #description>若文件还未通过开发者预审，应用会自动提交预审，并在通过后继续传输。</template>
      </a-alert>
      <a-form layout="vertical">
        <a-form-item label="接收小号" required>
          <a-select
            v-model:value="developerTransfer.targetId"
            :options="developerTransfer.targets.map((item) => ({ value: item.id, label: `${item.name} · ${item.token_masked}` }))"
            placeholder="请选择小号 TOKEN"
          />
        </a-form-item>
        <a-form-item :label="`本次传输（${developerTransfer.records.length} 项）`">
          <div class="developer-transfer-files">
            <div v-for="record in developerTransfer.records" :key="fileId(record)">
              <FolderOutlined v-if="isFolder(record)" />
              <FileOutlined v-else />
              <span :title="record.fileName">{{ record.fileName }}</span>
            </div>
          </div>
        </a-form-item>
      </a-form>
      <div class="modal-hint">提交前会再次确认文件属于当前开发者账号；一个接收 TOKEN 不能反向传回。</div>
    </a-modal>

    <a-modal
      v-if="isTauri"
      v-model:open="gcidImport.open"
      title="导入 GCID JSON"
      :width="520"
      ok-text="开始导入"
      cancel-text="取消"
      :confirm-loading="gcidImport.loading"
      :ok-button-props="{ disabled: gcidImportRunning }"
      :mask-closable="!gcidImport.loading"
      @ok="submitGcidImport"
    >
      <div class="gcid-import-note">
        <FileAddOutlined aria-hidden="true" />
        <span>导入到「{{ currentPath.at(-1)?.name || '全部文件' }}」下的新文件夹，启动后可关闭窗口继续处理。</span>
      </div>

      <a-form layout="vertical" class="gcid-import-form">
        <a-form-item label="JSON 来源">
          <a-flex gap="small">
            <a-input :value="gcidImport.sourceName || gcidImport.sourcePath" readonly placeholder="请选择 JSON 文件，或在下方粘贴内容" />
            <a-button :disabled="gcidImportRunning" @click="selectGcidImportFile"><template #icon><FileAddOutlined /></template>选择文件</a-button>
          </a-flex>
        </a-form-item>
        <a-form-item v-if="!gcidImport.sourcePath" label="或粘贴 JSON 内容">
          <a-textarea v-model:value="gcidImport.pastedJson" :rows="4" :disabled="gcidImportRunning" placeholder="粘贴完整的光鸭 GCID 导出 JSON" />
          <div v-if="shouldConvertPasteToFile(gcidImport.pastedJson)" class="form-help">内容较大，提交时会先写入本机暂存文件，避免一次性跨进程传输。</div>
        </a-form-item>
        <a-row :gutter="12">
          <a-col :span="15">
            <a-form-item label="云端目标文件夹" required>
              <a-input v-model:value="gcidImport.destinationName" :disabled="gcidImportRunning" placeholder="例如：影视资源导入" />
            </a-form-item>
          </a-col>
          <a-col :span="9">
            <a-form-item label="并发数">
              <a-input-number v-model:value="gcidImport.concurrency" :min="1" :max="16" :precision="0" :disabled="gcidImportRunning" style="width:100%" />
            </a-form-item>
          </a-col>
        </a-row>
      </a-form>
    </a-modal>

    <a-modal
      v-if="!isTauri"
      v-model:open="serverFilePicker.open"
      title="选择服务器文件或文件夹"
      ok-text="加入上传队列"
      cancel-text="取消"
      :width="720"
      :confirm-loading="serverFilePicker.submitting"
      :ok-button-props="{ disabled: !serverFilePicker.selected.length }"
      @ok="confirmServerUpload"
    >
      <div class="server-picker-toolbar">
        <a-button size="small" :disabled="!serverFilePicker.parent" @click="loadServerDirectory(serverFilePicker.parent)"><ArrowUpOutlined />上一级</a-button>
        <a-select v-if="serverFilePicker.roots.length > 1" :value="activeServerRoot" size="small" style="min-width:160px" :options="serverFilePicker.roots.map((root) => ({ label: root, value: root }))" @change="loadServerDirectory" />
        <span>{{ serverFilePicker.displayPath }}</span>
        <a-tag>已选 {{ serverFilePicker.selected.length }} 项</a-tag>
      </div>
      <a-spin :spinning="serverFilePicker.loading">
        <div v-if="serverFilePicker.items.length" class="server-file-list">
          <div v-for="item in serverFilePicker.items" :key="item.path" class="server-file-row">
            <a-checkbox :checked="serverFilePicker.selected.includes(item.path)" @change="(event) => toggleServerSelection(item, event.target.checked)" />
            <span class="file-icon" :class="item.type === 'directory' ? 'folder' : 'file'"><FolderOutlined v-if="item.type === 'directory'" /><FileOutlined v-else /></span>
            <button type="button" class="server-file-name" @dblclick="item.type === 'directory' && loadServerDirectory(item.path)">{{ item.name }}</button>
            <span class="server-file-size">{{ item.type === 'directory' ? '文件夹' : formatSize(item.size) }}</span>
            <a-button v-if="item.type === 'directory'" type="link" size="small" @click="loadServerDirectory(item.path)">打开</a-button>
          </div>
        </div>
        <a-empty v-else description="这个服务器目录为空" />
      </a-spin>
      <a-alert class="server-picker-tip" type="info" show-icon message="文件夹会递归上传并保留目录结构；未修改且已上传的文件会自动跳过。" />
    </a-modal>

  </div>
</template>

<style scoped>
.file-context-anchor {
  position: fixed;
  width: 1px;
  height: 1px;
  pointer-events: none;
}

.folder-crumb-button {
  padding: 0;
  border: 0;
  color: inherit;
  background: transparent;
  cursor: pointer;
  font: inherit;
}

.folder-crumb-button:not(.current):hover {
  color: var(--primary, #262626);
  text-decoration: underline;
  text-underline-offset: 3px;
}

.folder-crumb-button.current {
  color: var(--text-2, #525252);
  cursor: default;
  opacity: 1;
}

.folder-crumb-button:focus-visible {
  border-radius: 3px;
  outline: 2px solid var(--primary, #52c41a);
  outline-offset: 2px;
}

.file-path-actions {
  flex: 1 1 auto !important;
  min-width: 0;
  overflow: hidden;
}

.file-path-actions :deep(.ant-breadcrumb) {
  min-width: 0;
  overflow: hidden;
}

.file-primary-actions {
  flex: 0 0 auto;
}

.gcid-import-note {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 9px 11px;
  border-radius: 8px;
  color: var(--text-2, #525252);
  background: var(--bg-toolbar, #fafafa);
  font-size: 12px;
  line-height: 1.6;
}

.gcid-import-note > :deep(.anticon) {
  margin-top: 3px;
  color: var(--primary, #262626);
}

.gcid-import-form {
  margin-top: 14px;
}

.gcid-import-form :deep(.ant-form-item) {
  margin-bottom: 14px;
}

.developer-transfer-note {
  margin-bottom: 16px;
}

.developer-transfer-files {
  max-height: 220px;
  overflow-y: auto;
  border: 1px solid var(--line, #e5e7eb);
  border-radius: 8px;
  background: var(--bg-toolbar, #fafafa);
}

.developer-transfer-files > div {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--line, #e5e7eb);
}

.developer-transfer-files > div:last-child {
  border-bottom: 0;
}

.developer-transfer-files span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-list-region {
  min-width: 0;
}

.file-type {
  color: var(--text-2, #525252);
  font-size: 11px;
}
</style>
