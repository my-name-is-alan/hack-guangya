<script setup>
import { computed, h, onBeforeUnmount, onMounted, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  ArrowLeftOutlined,
  CloudDownloadOutlined,
  CloudUploadOutlined,
  CopyOutlined,
  DeleteOutlined,
  DownOutlined,
  DownloadOutlined,
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
  FolderOpenOutlined,
  FolderOutlined,
  InboxOutlined,
  LinkOutlined,
  ReloadOutlined,
  ShareAltOutlined,
  SwapOutlined,
  UploadOutlined,
  VideoCameraOutlined,
} from '@antdv-next/icons';
import { bridge, isTauri } from '../bridge.js';
import {
  appState,
  currentFolderId,
  currentFolderName,
  currentPath,
  files,
  filesLoading,
  loadFiles,
  refreshState,
} from '../store.js';
import {
  errorText,
  fileId,
  formatSize,
  formatTime,
  isFolder,
  newDownloadId,
  pick,
  unwrapData,
} from '../formatters.js';

const emit = defineEmits(['share']);

const selectedKeys = ref([]);
const dragActive = ref(false);
const dragDepth = ref(0);
const uploading = ref(false);
const uploadProgress = ref(null);
const uploadProgressTimer = ref(null);
const uploadMenuItems = [{ key: 'folder', label: '上传文件夹' }];
const fileContextMenu = reactive({ open: false, x: 0, y: 0, record: null });
const fileContextMenuItems = computed(() => {
  const record = fileContextMenu.record;
  if (!record) return [];
  if (isFolder(record)) {
    return [
      { key: 'open', icon: () => h(FolderOpenOutlined), label: '打开文件夹' },
      { type: 'divider' },
      { key: 'rename', icon: () => h(EditOutlined), label: '重命名' },
      { type: 'divider' },
      { key: 'copy', icon: () => h(CopyOutlined), label: '复制到…' },
      { key: 'move', icon: () => h(SwapOutlined), label: '移动到…' },
      { type: 'divider' },
      { key: 'share', icon: () => h(ShareAltOutlined), label: '创建分享' },
      { key: 'delete', icon: () => h(DeleteOutlined), label: '删除', danger: true },
    ];
  }
  return [
    { key: 'download', icon: () => h(DownloadOutlined), label: '下载' },
    { type: 'divider' },
    { key: 'rename', icon: () => h(EditOutlined), label: '重命名' },
    { type: 'divider' },
    { key: 'copy', icon: () => h(CopyOutlined), label: '复制到…' },
    { key: 'move', icon: () => h(SwapOutlined), label: '移动到…' },
    { type: 'divider' },
    { key: 'share', icon: () => h(ShareAltOutlined), label: '创建分享' },
    { key: 'delete', icon: () => h(DeleteOutlined), label: '删除', danger: true },
  ];
});

const shareForm = reactive({ open: false, loading: false, url: '', password: '', name: '', remark: '' });
const gcidImport = reactive({ open: false, loading: false, json: '' });
const renameModal = reactive({ open: false, saving: false, records: [], mode: 'single', singleName: '', prefix: '', suffix: '', findText: '', replaceText: '', startNumber: 1, digits: 3, template: '' });
const folderPicker = reactive({ open: false, loading: false, title: '', action: 'copy', sourceIds: [], path: [{ id: '', name: '全部文件' }], options: [] });
const receivedShare = reactive({
  open: false, loading: false, restoring: false, downloading: false,
  url: '', password: '', info: null, files: [], selectedKeys: [], path: [], error: '',
});

const fileColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '大小', key: 'size', width: 110 },
  { title: '修改时间', key: 'time', width: 170 },
];
const folderPickerColumns = [
  { title: '文件夹', key: 'name', ellipsis: true },
  { title: '修改时间', key: 'time', width: 170 },
];

const rowSelection = computed(() => ({
  selectedRowKeys: selectedKeys.value,
  onChange: (keys) => { selectedKeys.value = keys; },
}));
const receivedShareRowSelection = computed(() => ({
  selectedRowKeys: receivedShare.selectedKeys,
  onChange: (keys) => { receivedShare.selectedKeys = keys; },
}));
const folderPickerRowSelection = computed(() => ({
  type: 'radio',
  selectedRowKeys: folderPicker.targetId ? [folderPicker.targetId] : [],
  onChange: (keys) => { folderPicker.targetId = keys[0] || ''; },
}));

const uploadProgressPercent = computed(() => {
  const total = Number(uploadProgress.value?.total || 0);
  if (!total) return 0;
  return Math.min(100, Math.round((Number(uploadProgress.value?.uploaded || 0) / total) * 100));
});
const uploadProgressText = computed(() => {
  if (!uploadProgress.value) return '';
  if (uploadProgress.value.status === 'failed') return uploadProgress.value.message || '上传失败';
  if (uploadProgress.value.status === 'completed') return `已完成 ${uploadProgress.value.uploaded}/${uploadProgress.value.total}`;
  return `正在上传 ${uploadProgress.value.current || ''} · ${uploadProgress.value.uploaded}/${uploadProgress.value.total}`;
});

const receivedShareBreadcrumb = computed(() => [
  { key: 'root', label: '分享根目录' },
  ...receivedShare.path.map((folder) => ({ key: folder.id, label: folder.name })),
]);
const receivedShareCurrentFolderId = computed(() => receivedShare.path[receivedShare.path.length - 1]?.id || '');
const receivedShareSelectedCount = computed(() => receivedShare.selectedKeys.length);
const receivedShareFolderCount = computed(() => receivedShare.files.filter((item) => isFolder(item)).length);
const receivedShareFileCount = computed(() => receivedShare.files.length - receivedShareFolderCount.value);
const receivedShareTotalSize = computed(() => receivedShare.files.reduce((total, item) => total + Number(item.fileSize || 0), 0));

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

function fileRowProps(record) {
  return {
    onDblclick: () => { if (isFolder(record)) enterFolder(record); },
    onContextmenu: (event) => {
      event.preventDefault();
      openFileContextMenu(event, record);
    },
  };
}

function openFileContextMenu(event, record) {
  if (!appState.logged_in) return;
  const id = fileId(record);
  if (id && !selectedKeys.value.includes(id)) selectedKeys.value = [id];
  fileContextMenu.open = true;
  fileContextMenu.x = event.clientX;
  fileContextMenu.y = event.clientY;
  fileContextMenu.record = record;
}

function closeFileContextMenu() {
  fileContextMenu.open = false;
  fileContextMenu.record = null;
}

function selectedRecords() {
  const ids = new Set(selectedKeys.value);
  return files.value.filter((item) => ids.has(fileId(item)));
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
  if (!record) return;
  closeFileContextMenu();
  if (key === 'open') return enterFolder(record);
  if (key === 'download') return downloadCloudFiles([record]);
  if (key === 'rename') return openRenameModal(contextTargetRecords());
  if (key === 'copy') return openFolderPicker('copy', contextTargetRecords());
  if (key === 'move') return openFolderPicker('move', contextTargetRecords());
  if (key === 'share') return emit('share', [record]);
  if (key === 'delete') return deleteCloudFiles(contextTargetRecords());
}

async function loadCloudFiles() {
  try { await loadFiles(); } catch (error) { message.error(errorText(error)); }
}

function enterFolder(record) {
  currentPath.value = [...currentPath.value, { id: fileId(record), name: record.fileName }];
  selectedKeys.value = [];
  loadCloudFiles();
}
function goBack() {
  if (currentPath.value.length <= 1) return;
  currentPath.value = currentPath.value.slice(0, -1);
  selectedKeys.value = [];
  loadCloudFiles();
}

async function triggerUpload(kind) {
  if (!appState.logged_in) return;
  if (!isTauri) {
    message.warning('Web 控制台暂不支持直接上传文件，请使用桌面端上传');
    return;
  }
  uploading.value = true;
  try {
    const paths = kind === 'folder'
      ? [await bridge.selectUploadFolder()].filter(Boolean)
      : await bridge.selectUploadFiles();
    if (!paths.length) return;
    startUploadProgress(paths.length);
    await bridge.invoke('upload_files', { paths, parent_id: currentFolderId.value });
    await loadCloudFiles();
    message.success(`已提交 ${paths.length} 个上传任务`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    uploading.value = false;
  }
}
function handleUploadMenu({ key }) {
  if (key === 'folder') triggerUpload('folder');
}

function startUploadProgress(total) {
  stopUploadProgress();
  uploadProgress.value = { status: 'running', total, uploaded: 0, current: '', message: '' };
  uploadProgressTimer.value = setInterval(async () => {
    try {
      const state = unwrapData(await bridge.invoke('get_upload_progress'));
      if (!state || !state.total) return;
      uploadProgress.value = state;
      if (['completed', 'failed'].includes(state.status)) stopUploadProgress(false);
    } catch { /* 忽略轮询错误 */ }
  }, 800);
}
function stopUploadProgress(clear = true) {
  if (uploadProgressTimer.value) clearInterval(uploadProgressTimer.value);
  uploadProgressTimer.value = null;
  if (clear) uploadProgress.value = null;
}
function dismissUploadProgress() {
  stopUploadProgress();
}

async function downloadCloudFiles(records) {
  const targets = (Array.isArray(records) ? records : []).filter((record) => record && !isFolder(record));
  if (!targets.length) return;
  const hide = message.loading(`正在获取 ${targets.length} 个文件的下载地址…`, 0);
  try {
    const results = [];
    for (const record of targets) {
      const data = unwrapData(await bridge.invoke('get_cloud_download', { file_id: fileId(record) }));
      const url = pick(data, ['downloadUrl', 'download_url', 'url'], '');
      if (!url) throw new Error(`未获取到「${record.fileName}」的下载地址`);
      results.push({ id: newDownloadId(), name: record.fileName || '未命名文件', url, status: 'pending', progress: 0, error: '' });
    }
    window.dispatchEvent(new CustomEvent('guangya:add-downloads', { detail: results }));
    message.success(`已添加 ${results.length} 个下载任务`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    hide();
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
        message.success('已删除');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
}

function openRenameModal(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  renameModal.records = targets;
  renameModal.mode = targets.length > 1 ? 'batch' : 'single';
  renameModal.singleName = targets[0]?.fileName || '';
  renameModal.prefix = '';
  renameModal.suffix = '';
  renameModal.findText = '';
  renameModal.replaceText = '';
  renameModal.startNumber = 1;
  renameModal.digits = 3;
  renameModal.template = '';
  renameModal.open = true;
}
function splitFileName(name) {
  const value = String(name || '');
  const index = value.lastIndexOf('.');
  if (index <= 0) return { stem: value, ext: '' };
  return { stem: value.slice(0, index), ext: value.slice(index) };
}
function previewRenameName(record, index) {
  const { stem, ext } = splitFileName(record.fileName);
  if (renameModal.mode === 'single') return renameModal.singleName || record.fileName;
  if (renameModal.mode === 'affix') return `${renameModal.prefix}${stem}${renameModal.suffix}${ext}`;
  if (renameModal.mode === 'replace') return `${stem.split(renameModal.findText).join(renameModal.replaceText)}${ext}`;
  if (renameModal.mode === 'number') {
    const number = String(Number(renameModal.startNumber || 1) + index).padStart(Number(renameModal.digits || 3), '0');
    return `${renameModal.prefix}${number}${renameModal.suffix}${ext}`;
  }
  if (renameModal.mode === 'template') {
    const number = String(Number(renameModal.startNumber || 1) + index).padStart(Number(renameModal.digits || 3), '0');
    const rendered = String(renameModal.template || '')
      .replaceAll('{name}', stem)
      .replaceAll('{num}', number)
      .replaceAll('{ext}', ext.replace(/^\./, ''));
    return rendered || record.fileName;
  }
  return record.fileName;
}
const renamePreviewRows = computed(() => renameModal.records.map((record, index) => ({
  key: fileId(record) || index,
  before: record.fileName,
  after: previewRenameName(record, index),
})));
async function submitRename() {
  if (!renameModal.records.length) return;
  const renames = renameModal.records.map((record, index) => ({
    file_id: fileId(record),
    new_name: previewRenameName(record, index),
  })).filter((item) => item.file_id && item.new_name && item.new_name !== renameModal.records.find((record) => fileId(record) === item.file_id)?.fileName);
  if (!renames.length) {
    message.info('文件名没有变化');
    return;
  }
  renameModal.saving = true;
  try {
    await bridge.invoke('batch_rename_files', { renames });
    renameModal.open = false;
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
  await loadFolderPickerOptions('');
}
async function loadFolderPickerOptions(parentId) {
  folderPicker.loading = true;
  try {
    const data = unwrapData(await bridge.invoke('list_files', { page: 0, parent_id: parentId }));
    folderPicker.options = (data.list || []).filter((item) => isFolder(item) && !folderPicker.sourceIds.includes(fileId(item)));
  } catch (error) {
    message.error(errorText(error));
  } finally {
    folderPicker.loading = false;
  }
}
function folderPickerRowProps(record) {
  return { onDblclick: () => enterFolderPicker(record) };
}
function enterFolderPicker(record) {
  folderPicker.path = [...folderPicker.path, { id: fileId(record), name: record.fileName }];
  folderPicker.targetId = '';
  loadFolderPickerOptions(fileId(record));
}
function folderPickerBack() {
  if (folderPicker.path.length <= 1) return;
  folderPicker.path = folderPicker.path.slice(0, -1);
  folderPicker.targetId = '';
  loadFolderPickerOptions(folderPicker.path[folderPicker.path.length - 1]?.id || '');
}
function folderPickerJump(index) {
  folderPicker.path = folderPicker.path.slice(0, index + 1);
  folderPicker.targetId = '';
  loadFolderPickerOptions(folderPicker.path[index]?.id || '');
}
async function submitFolderPicker() {
  const command = folderPicker.action === 'copy' ? 'copy_files' : 'move_files';
  try {
    await bridge.invoke(command, { file_ids: folderPicker.sourceIds, target_folder_id: folderPicker.targetId || '' });
    folderPicker.open = false;
    selectedKeys.value = [];
    await loadCloudFiles();
    message.success(folderPicker.action === 'copy' ? '已复制' : '已移动');
  } catch (error) {
    message.error(errorText(error));
  }
}

function openShareForm() {
  shareForm.url = '';
  shareForm.password = '';
  shareForm.name = '';
  shareForm.remark = '';
  shareForm.open = true;
}
async function submitShareForm() {
  if (!shareForm.url.trim()) {
    message.warning('请输入分享链接');
    return;
  }
  shareForm.loading = true;
  try {
    await bridge.invoke('save_share_link', {
      url: shareForm.url.trim(),
      password: shareForm.password.trim(),
      name: shareForm.name.trim(),
      remark: shareForm.remark.trim(),
    });
    shareForm.open = false;
    await refreshState();
    message.success('分享链接已收藏');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    shareForm.loading = false;
  }
}

function openGcidImport() {
  gcidImport.json = '';
  gcidImport.open = true;
}
async function submitGcidImport() {
  if (!gcidImport.json.trim()) {
    message.warning('请粘贴 JSON 内容');
    return;
  }
  gcidImport.loading = true;
  try {
    const result = unwrapData(await bridge.invoke('import_gcid_json', {
      json_text: gcidImport.json,
      parent_id: currentFolderId.value,
    }));
    gcidImport.open = false;
    await loadCloudFiles();
    const parts = [`成功 ${result.success || 0} 项`];
    if (result.skipped) parts.push(`跳过 ${result.skipped} 项`);
    if (result.failed) parts.push(`失败 ${result.failed} 项`);
    message.success(`JSON 秒传完成：${parts.join('，')}`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    gcidImport.loading = false;
  }
}

function openReceivedShare() {
  receivedShare.url = '';
  receivedShare.password = '';
  receivedShare.info = null;
  receivedShare.files = [];
  receivedShare.selectedKeys = [];
  receivedShare.path = [];
  receivedShare.error = '';
  receivedShare.open = true;
}
async function loadReceivedShareFiles() {
  if (!receivedShare.info) return;
  receivedShare.loading = true;
  receivedShare.error = '';
  try {
    const data = unwrapData(await bridge.invoke('list_received_share_files', {
      share_url: receivedShare.url.trim(),
      password: receivedShare.password.trim(),
      share_id: receivedShare.info.share_id || '',
      parent_id: receivedShareCurrentFolderId.value,
    }));
    receivedShare.files = data.list || [];
    receivedShare.selectedKeys = [];
  } catch (error) {
    receivedShare.error = errorText(error);
  } finally {
    receivedShare.loading = false;
  }
}
async function openReceivedShareLink() {
  if (!receivedShare.url.trim()) {
    message.warning('请输入分享链接');
    return;
  }
  receivedShare.loading = true;
  receivedShare.error = '';
  try {
    const data = unwrapData(await bridge.invoke('open_received_share', {
      share_url: receivedShare.url.trim(),
      password: receivedShare.password.trim(),
    }));
    receivedShare.info = data;
    receivedShare.path = [];
    await loadReceivedShareFiles();
  } catch (error) {
    receivedShare.error = errorText(error);
    receivedShare.info = null;
  } finally {
    receivedShare.loading = false;
  }
}
function enterReceivedShareFolder(record) {
  if (!isFolder(record)) return;
  receivedShare.path = [...receivedShare.path, { id: fileId(record), name: record.fileName }];
  loadReceivedShareFiles();
}
function receivedShareBack() {
  if (!receivedShare.path.length) return;
  receivedShare.path = receivedShare.path.slice(0, -1);
  loadReceivedShareFiles();
}
function receivedShareJump(index) {
  receivedShare.path = index < 0 ? [] : receivedShare.path.slice(0, index + 1);
  loadReceivedShareFiles();
}
function receivedShareRowProps(record) {
  return { onDblclick: () => enterReceivedShareFolder(record) };
}
function receivedShareTargets() {
  const ids = new Set(receivedShare.selectedKeys);
  return receivedShare.files.filter((item) => ids.has(fileId(item)));
}
async function restoreReceivedShare() {
  if (!receivedShare.info) return;
  const targets = receivedShareTargets();
  if (!targets.length) {
    message.warning('请先选择要转存的文件或文件夹');
    return;
  }
  receivedShare.restoring = true;
  try {
    await bridge.invoke('restore_received_share', {
      share_url: receivedShare.url.trim(),
      password: receivedShare.password.trim(),
      share_id: receivedShare.info.share_id || '',
      file_ids: targets.map((item) => fileId(item)),
      parent_id: currentFolderId.value,
    });
    message.success(`已转存 ${targets.length} 项到「${currentFolderName.value}」`);
    receivedShare.open = false;
    await loadCloudFiles();
  } catch (error) {
    message.error(errorText(error));
  } finally {
    receivedShare.restoring = false;
  }
}
async function downloadReceivedShare() {
  if (!receivedShare.info) return;
  const targets = receivedShareTargets().filter((item) => !isFolder(item));
  if (!targets.length) {
    message.warning('请先选择要下载的文件');
    return;
  }
  receivedShare.downloading = true;
  try {
    const results = [];
    for (const record of targets) {
      const data = unwrapData(await bridge.invoke('get_received_share_download', {
        share_url: receivedShare.url.trim(),
        password: receivedShare.password.trim(),
        share_id: receivedShare.info.share_id || '',
        file_id: fileId(record),
      }));
      const url = pick(data, ['downloadUrl', 'download_url', 'url'], '');
      if (!url) throw new Error(`未获取到「${record.fileName}」的下载地址`);
      results.push({ id: newDownloadId(), name: record.fileName || '未命名文件', url, status: 'pending', progress: 0, error: '' });
    }
    window.dispatchEvent(new CustomEvent('guangya:add-downloads', { detail: results }));
    message.success(`已添加 ${results.length} 个下载任务`);
  } catch (error) {
    message.error(errorText(error));
  } finally {
    receivedShare.downloading = false;
  }
}

function handleWindowDragOver(event) {
  if (!isTauri || !appState.logged_in) return;
  event.preventDefault();
}
function handleWindowDrop(event) {
  event.preventDefault();
}
function handleWindowClick(event) {
  if (!fileContextMenu.open) return;
  if (event.target?.closest?.('.file-context-menu')) return;
  closeFileContextMenu();
}

let unlistenDrag = null;
onMounted(async () => {
  window.addEventListener('dragover', handleWindowDragOver);
  window.addEventListener('drop', handleWindowDrop);
  window.addEventListener('click', handleWindowClick);
  if (isTauri) {
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
          startUploadProgress(paths.length);
          await bridge.invoke('upload_files', { paths, parent_id: currentFolderId.value });
          await loadCloudFiles();
          message.success(`已提交 ${paths.length} 个上传任务`);
        } catch (error) {
          message.error(errorText(error));
        }
      }
    });
  }
});
onBeforeUnmount(() => {
  window.removeEventListener('dragover', handleWindowDragOver);
  window.removeEventListener('drop', handleWindowDrop);
  window.removeEventListener('click', handleWindowClick);
  stopUploadProgress();
  unlistenDrag?.();
});
</script>

<template>
  <div class="cloud-view">
    <a-alert v-if="!isTauri" class="web-notice" type="info" show-icon message="Docker Web 控制台：可浏览、转存与下载云盘文件；上传与本地备份请使用桌面端。" />

    <a-card class="content-card file-card file-drop-surface" :class="{ 'drag-active': dragActive }" :bordered="false">
      <div class="file-toolbar">
        <a-flex align="center" gap="small" wrap="wrap">
          <a-button :disabled="currentPath.length <= 1" @click="goBack"><template #icon><ArrowLeftOutlined /></template>返回</a-button>
          <a-breadcrumb>
            <a-breadcrumb-item v-for="item in currentPath" :key="item.id || 'root'">{{ item.name }}</a-breadcrumb-item>
          </a-breadcrumb>
        </a-flex>
        <a-flex align="center" gap="small" wrap="wrap">
          <a-button :disabled="!appState.logged_in" @click="openReceivedShare"><template #icon><InboxOutlined /></template>接收分享</a-button>
          <a-button :disabled="!appState.logged_in" @click="openShareForm"><template #icon><LinkOutlined /></template>收藏链接</a-button>
          <a-button v-if="isTauri" :disabled="!appState.logged_in" @click="openGcidImport"><template #icon><FileAddOutlined /></template>JSON 秒传</a-button>
          <div class="upload-split">
            <a-button type="primary" :disabled="!appState.logged_in" @click="triggerUpload('files')"><template #icon><UploadOutlined /></template>上传文件</a-button>
            <a-dropdown :disabled="!appState.logged_in" :trigger="['click']" :menu="{ items: uploadMenuItems, onClick: handleUploadMenu }">
              <a-button type="primary" class="upload-split-menu" aria-label="选择上传文件夹"><DownOutlined /></a-button>
            </a-dropdown>
          </div>
          <a-button :loading="filesLoading" :disabled="!appState.logged_in" @click="loadCloudFiles"><template #icon><ReloadOutlined /></template>刷新</a-button>
        </a-flex>
      </div>

      <div v-if="uploadProgress" class="upload-progress" :class="uploadProgress.status">
        <a-flex align="center" gap="small">
          <CloudUploadOutlined :spin="uploadProgress.status === 'running'" />
          <div class="upload-progress-body">
            <strong>{{ uploadProgressText }}</strong>
            <a-progress :percent="uploadProgressPercent" :show-info="false" size="small" :status="uploadProgress.status === 'failed' ? 'exception' : uploadProgress.status === 'completed' ? 'success' : 'active'" />
          </div>
          <a-button v-if="uploadProgress.status !== 'running'" type="text" size="small" @click="dismissUploadProgress">关闭</a-button>
        </a-flex>
      </div>

      <a-table :columns="fileColumns" :data-source="files" :loading="filesLoading" :row-key="fileId" :row-selection="rowSelection" :custom-row="fileRowProps" :pagination="false" :scroll="{ y: 'clamp(240px, calc(100vh - 330px), 640px)' }" size="small">
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
                <a v-if="isFolder(record)" class="file-name" @click.prevent="enterFolder(record)">{{ record.fileName }}</a>
                <span v-else class="file-name">{{ record.fileName }}</span>
                <a-tag v-if="!isFolder(record) && record.fileSuffix" class="ext-tag">{{ record.fileSuffix }}</a-tag>
              </div>
            </a-flex>
          </template>
          <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
          <template v-else-if="column.key === 'time'">{{ formatTime(record.lastUpdateTime) }}</template>
        </template>
      </a-table>

      <div class="file-footer">
        <span>{{ files.length }} 个项目{{ selectedKeys.length ? ` · 已选 ${selectedKeys.length} 项` : '' }}</span>
        <span v-if="isTauri && appState.logged_in">拖拽文件到此处可直接上传</span>
      </div>
      <div v-if="isTauri && appState.logged_in" class="drop-hint">
        <div class="drop-hint-inner">
          <CloudUploadOutlined />
          <strong>松开鼠标上传到当前目录</strong>
          <span>{{ currentFolderName }}</span>
        </div>
      </div>
    </a-card>

    <teleport to="body">
      <a-dropdown v-model:open="fileContextMenu.open" :trigger="['contextmenu']" :menu="{ items: fileContextMenuItems, onClick: handleFileContextMenuClick }">
        <span class="file-context-menu" :style="{ left: `${fileContextMenu.x}px`, top: `${fileContextMenu.y}px` }" />
      </a-dropdown>
    </teleport>

    <a-modal v-model:open="renameModal.open" title="重命名" :confirm-loading="renameModal.saving" ok-text="应用" cancel-text="取消" width="560px" @ok="submitRename">
      <template v-if="renameModal.mode === 'single'">
        <a-input v-model:value="renameModal.singleName" placeholder="输入新的文件名" @press-enter="submitRename" />
      </template>
      <template v-else>
        <a-tabs v-model:active-key="renameModal.mode" size="small">
          <a-tab-pane key="affix" tab="前后缀">
            <a-space direction="vertical" style="width:100%">
              <a-input v-model:value="renameModal.prefix" addon-before="前缀" />
              <a-input v-model:value="renameModal.suffix" addon-before="后缀" />
            </a-space>
          </a-tab-pane>
          <a-tab-pane key="replace" tab="查找替换">
            <a-space direction="vertical" style="width:100%">
              <a-input v-model:value="renameModal.findText" addon-before="查找" />
              <a-input v-model:value="renameModal.replaceText" addon-before="替换为" />
            </a-space>
          </a-tab-pane>
          <a-tab-pane key="number" tab="序号重命名">
            <a-space direction="vertical" style="width:100%">
              <a-input v-model:value="renameModal.prefix" addon-before="前缀" />
              <a-input v-model:value="renameModal.suffix" addon-before="后缀" />
              <a-space>
                <a-input-number v-model:value="renameModal.startNumber" :min="0" addon-before="起始" />
                <a-input-number v-model:value="renameModal.digits" :min="1" :max="8" addon-before="位数" />
              </a-space>
            </a-space>
          </a-tab-pane>
          <a-tab-pane key="template" tab="模板">
            <a-space direction="vertical" style="width:100%">
              <a-input v-model:value="renameModal.template" placeholder="例如 {name}-副本-{num}" />
              <small class="modal-hint">可用变量：{name} 原名、{num} 序号、{ext} 扩展名</small>
              <a-space>
                <a-input-number v-model:value="renameModal.startNumber" :min="0" addon-before="起始" />
                <a-input-number v-model:value="renameModal.digits" :min="1" :max="8" addon-before="位数" />
              </a-space>
            </a-space>
          </a-tab-pane>
        </a-tabs>
        <a-table class="rename-preview" :columns="[{ title: '原文件名', dataIndex: 'before', ellipsis: true }, { title: '新文件名', dataIndex: 'after', ellipsis: true }]" :data-source="renamePreviewRows" :pagination="false" size="small" :scroll="{ y: 220 }" />
      </template>
    </a-modal>

    <a-modal v-model:open="folderPicker.open" :title="folderPicker.title" ok-text="确定" cancel-text="取消" width="520px" @ok="submitFolderPicker">
      <div class="folder-picker-toolbar">
        <a-button size="small" :disabled="folderPicker.path.length <= 1" @click="folderPickerBack"><template #icon><ArrowLeftOutlined /></template></a-button>
        <a-breadcrumb>
          <a-breadcrumb-item v-for="(item, index) in folderPicker.path" :key="item.id || 'root'">
            <a @click.prevent="folderPickerJump(index)">{{ item.name }}</a>
          </a-breadcrumb-item>
        </a-breadcrumb>
      </div>
      <a-table :columns="folderPickerColumns" :data-source="folderPicker.options" :loading="folderPicker.loading" :row-key="fileId" :row-selection="folderPickerRowSelection" :custom-row="folderPickerRowProps" :pagination="false" size="small" :scroll="{ y: 280 }">
        <template #emptyText><a-empty description="此目录下没有可选文件夹" /></template>
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'name'">
            <a-flex align="center" gap="small">
              <div class="file-icon folder"><FolderOutlined /></div>
              <a class="file-name" @click.prevent="enterFolderPicker(record)">{{ record.fileName }}</a>
            </a-flex>
          </template>
          <template v-else-if="column.key === 'time'">{{ formatTime(record.lastUpdateTime) }}</template>
        </template>
      </a-table>
      <small class="modal-hint">不选择文件夹则{{ folderPicker.action === 'copy' ? '复制' : '移动' }}到当前目录：{{ folderPicker.path[folderPicker.path.length - 1]?.name }}</small>
    </a-modal>

    <a-modal v-model:open="shareForm.open" title="收藏分享链接" :confirm-loading="shareForm.loading" ok-text="收藏" cancel-text="取消" @ok="submitShareForm">
      <a-form layout="vertical">
        <a-form-item label="分享链接" required><a-input v-model:value="shareForm.url" placeholder="https://yun.139.com/shareweb#/w/i/…" /></a-form-item>
        <a-form-item label="提取码"><a-input v-model:value="shareForm.password" placeholder="如有提取码请填写" /></a-form-item>
        <a-form-item label="名称"><a-input v-model:value="shareForm.name" placeholder="便于识别的名称（可选）" /></a-form-item>
        <a-form-item label="备注"><a-textarea v-model:value="shareForm.remark" :rows="2" placeholder="备注信息（可选）" /></a-form-item>
      </a-form>
    </a-modal>

    <a-modal v-model:open="gcidImport.open" title="JSON 秒传导入" :confirm-loading="gcidImport.loading" ok-text="导入" cancel-text="取消" width="560px" @ok="submitGcidImport">
      <a-alert type="info" show-icon message="粘贴包含 gcid 信息的 JSON，将文件秒传到当前目录。" style="margin-bottom: 12px" />
      <a-textarea v-model:value="gcidImport.json" :rows="10" placeholder='[{"file_name":"…","gcid":"…","size":123}]' />
    </a-modal>

    <a-modal v-model:open="receivedShare.open" title="接收分享" :footer="null" width="720px">
      <a-space direction="vertical" style="width:100%" :size="12">
        <a-flex gap="small" wrap="wrap">
          <a-input v-model:value="receivedShare.url" style="flex:1;min-width:240px" placeholder="分享链接 https://yun.139.com/shareweb#/w/i/…" @press-enter="openReceivedShareLink" />
          <a-input v-model:value="receivedShare.password" style="width:140px" placeholder="提取码" @press-enter="openReceivedShareLink" />
          <a-button type="primary" :loading="receivedShare.loading" @click="openReceivedShareLink">打开</a-button>
        </a-flex>
        <a-alert v-if="receivedShare.error" type="error" show-icon :message="receivedShare.error" />
        <template v-if="receivedShare.info">
          <div class="received-share-meta">
            <strong>{{ receivedShare.info.share_name || '分享内容' }}</strong>
            <span>{{ receivedShareFileCount }} 个文件 · {{ receivedShareFolderCount }} 个文件夹 · {{ formatSize(receivedShareTotalSize) }}</span>
          </div>
          <div class="folder-picker-toolbar">
            <a-button size="small" :disabled="!receivedShare.path.length" @click="receivedShareBack"><template #icon><ArrowLeftOutlined /></template></a-button>
            <a-breadcrumb>
              <a-breadcrumb-item v-for="(item, index) in receivedShareBreadcrumb" :key="item.key">
                <a @click.prevent="receivedShareJump(index - 1)">{{ item.label }}</a>
              </a-breadcrumb-item>
            </a-breadcrumb>
          </div>
          <a-table :columns="fileColumns" :data-source="receivedShare.files" :loading="receivedShare.loading" :row-key="fileId" :row-selection="receivedShareRowSelection" :custom-row="receivedShareRowProps" :pagination="false" size="small" :scroll="{ y: 300 }">
            <template #emptyText><a-empty description="此目录为空" /></template>
            <template #bodyCell="{ column, record }">
              <template v-if="column.key === 'name'">
                <a-flex align="center" gap="small">
                  <div class="file-icon" :class="fileIcon(record).cls"><component :is="fileIcon(record).icon" /></div>
                  <a v-if="isFolder(record)" class="file-name" @click.prevent="enterReceivedShareFolder(record)">{{ record.fileName }}</a>
                  <span v-else class="file-name">{{ record.fileName }}</span>
                </a-flex>
              </template>
              <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
              <template v-else-if="column.key === 'time'">{{ formatTime(record.lastUpdateTime) }}</template>
            </template>
          </a-table>
          <a-flex justify="space-between" align="center" wrap="wrap" gap="small">
            <span class="modal-hint">已选 {{ receivedShareSelectedCount }} 项 · 转存到「{{ currentFolderName }}」</span>
            <a-space>
              <a-button :loading="receivedShare.downloading" :disabled="!receivedShareSelectedCount" @click="downloadReceivedShare"><template #icon><CloudDownloadOutlined /></template>下载所选</a-button>
              <a-button type="primary" :loading="receivedShare.restoring" :disabled="!receivedShareSelectedCount" @click="restoreReceivedShare"><template #icon><InboxOutlined /></template>转存所选</a-button>
            </a-space>
          </a-flex>
        </template>
      </a-space>
    </a-modal>
  </div>
</template>
