export const organizerStatusMeta = Object.freeze({
  recognizing: { label: '识别中', color: 'processing' },
  ready: { label: '待执行', color: 'blue' },
  running: { label: '整理中', color: 'processing' },
  completed: { label: '已完成', color: 'success' },
  completed_warning: { label: '完成有提示', color: 'warning' },
  needs_review: { label: '需人工确认', color: 'warning' },
  failed: { label: '失败', color: 'error' },
});

export function organizerStatus(status) {
  return organizerStatusMeta[status] || { label: String(status || '未知'), color: 'default' };
}

export function organizerPreviewItems(job) {
  const items = job?.preview?.data?.items;
  return Array.isArray(items) ? items : [];
}

export function organizerPreviewTarget(job) {
  const items = organizerPreviewItems(job).filter((item) => item?.success && item?.target);
  const targets = items.filter((item) => item.kind === 'video').map((item) => String(item.target));
  const fallback = items.map((item) => String(item.target));
  const selected = targets.length ? targets : fallback;
  if (!selected.length) return '';
  return selected.length === 1 ? selected[0] : `${selected[0]} 等 ${selected.length} 项`;
}

export function organizerCandidates(job) {
  return Array.isArray(job?.preview?.candidates) ? job.preview.candidates : [];
}

export function organizerMatchedTitle(job) {
  const metadata = job?.preview?.metadata;
  if (!metadata?.title) return '';
  return `${metadata.title}${metadata.year ? ` (${metadata.year})` : ''}`;
}

export function organizerMediaLabel(value) {
  if (value === 'movie') return '电影';
  if (value === 'tv') return '电视剧';
  return '自动识别';
}

export function organizerTransferLabel(value) {
  return {
    copy: '云盘内复制',
    move: '云盘内移动',
  }[value] || '云盘内复制';
}

export function organizerConflictLabel(value) {
  return {
    skip: '跳过已有文件',
    overwrite: '覆盖已有文件',
    rename: '保留两份',
  }[value] || '跳过已有文件';
}

export function organizerItemKindLabel(value) {
  return {
    video: '视频',
    subtitle: '字幕',
    audio: '外置音轨',
    trailer: '预告片',
    extra: '附加视频',
    nfo: 'NFO',
    image: '图片',
  }[value] || String(value || '文件');
}

export function organizerItemActionLabel(item) {
  if (!item?.success) return '不可执行';
  if (item.action === 'skip') return '跳过';
  if (item.action === 'overwrite') return '覆盖';
  if (item.operation === 'generate') return '生成';
  if (item.operation === 'download') return '下载';
  return organizerTransferLabel(item.operation);
}

function renderTemplateExample(template, context) {
  const aliases = String(template || '')
    .replace(/\{catgroy\}/gi, '{category}')
    .replace(/\{tmdbid\}/gi, '{tmdb_id}')
    .replace(/\{Season\s+x\}/gi, '{season_tag}')
    .replace(/\{(?:Episode|Expose)\s+n\}/gi, '{episode_tag}');
  const rendered = aliases.replace(/\{([a-z_]+)(?::(\d+))?\}/gi, (_, key, width) => {
    const value = context[String(key).toLowerCase()] ?? '';
    return width && value !== '' ? String(value).padStart(Number(width), '0') : String(value);
  });
  return rendered.replaceAll('\\', '/').split('/').map((part) => part.trim()).filter(Boolean).join('/');
}

function splitExamplePath(value) {
  const parts = String(value || '').split('/').filter(Boolean);
  return {
    path: value,
    directory: parts.slice(0, -1).join('/'),
    filename: parts.at(-1) || '',
  };
}

export function organizerTemplateExamples(movieTemplate, tvTemplate, movieCategory = '电影', tvCategory = '电视剧') {
  const movie = splitExamplePath(renderTemplateExample(movieTemplate, {
    category: movieCategory, country: 'US', year: 2024, title: '示例电影', original_title: 'Example Movie', tmdb_id: 12345,
    edition: '', quality: ' - 1080p', part: '', ext: 'mkv', season: '', episode: '', season_tag: '', episode_tag: '', episode_end: '', episode_title: '',
  }));
  movie.input = '示例电影.2024.1080p.WEB-DL.mkv';
  const tv = splitExamplePath(renderTemplateExample(tvTemplate, {
    category: tvCategory, country: 'CN', year: 2024, title: '示例剧集', original_title: 'Example Series', tmdb_id: 67890,
    edition: '', quality: ' - 1080p', part: '', ext: 'mkv', season: 1, episode: 2, season_tag: 'S01', episode_tag: 'E02', episode_end: '', episode_title: '第二集',
  }));
  tv.input = '示例剧集.S01E02.1080p.WEB-DL.mkv';
  return { movie, tv };
}
