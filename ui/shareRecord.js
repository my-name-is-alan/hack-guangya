const EXPLICIT_NAME_KEYS = [
  'title',
  'shareTitle',
  'share_title',
  'shareName',
  'share_name',
];

const RESOURCE_NAME_KEYS = [
  'name',
  'fileName',
  'file_name',
  'resName',
  'res_name',
];

function firstText(record, keys) {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value !== 'string' && typeof value !== 'number') continue;
    const text = String(value).trim();
    if (text) return text;
  }
  return '';
}

/**
 * Resolve the human-readable name returned by different Guangya share-list APIs.
 * The native and web bridges intentionally pass the upstream record through, so
 * this is the single compatibility boundary for camelCase/snake_case variants.
 */
export function shareDisplayName(record) {
  const explicitName = firstText(record, EXPLICIT_NAME_KEYS);
  if (explicitName) return explicitName;

  const shareInfo = record?.shareInfo || record?.share_info;
  const nestedName = firstText(shareInfo, [...EXPLICIT_NAME_KEYS, ...RESOURCE_NAME_KEYS]);
  if (nestedName) return nestedName;

  const resourceName = firstText(record, RESOURCE_NAME_KEYS);
  if (resourceName) return resourceName;

  const resource = record?.resource || record?.file;
  const nestedResourceName = firstText(resource, RESOURCE_NAME_KEYS);
  if (nestedResourceName) return nestedResourceName;

  const shareId = firstText(record, ['shareId', 'shareID', 'share_id', 'id']);
  return shareId ? `分享 ${shareId}` : '未命名分享';
}
