export function normalizeUpdateMetadata(payload) {
  const value = payload?.data ?? payload
  if (!value || typeof value !== 'object') return null
  const version = String(value.version || '').trim()
  if (!version) return null
  return {
    ...value,
    version,
    current_version: String(value.current_version || '').trim(),
    notes: String(value.notes || ''),
  }
}
