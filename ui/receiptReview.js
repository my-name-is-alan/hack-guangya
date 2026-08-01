const TMDB_REQUIRED_ERROR_CODE = 'tmdb_required';

export function needsTmdbReview(receipt = {}) {
  if (receipt.status !== 'needs_review') return false;

  const errorCode = String(receipt.error_code || '').trim().toLowerCase();
  if (errorCode) return errorCode === TMDB_REQUIRED_ERROR_CODE;

  const message = String(receipt.message || '');
  return /TMDB/i.test(message) || message.includes('无法从分享根目录识别影视标题');
}
