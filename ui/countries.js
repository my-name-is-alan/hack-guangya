import COUNTRY_NAMES_ZH from '../shared/countries-zh.json' with { type: 'json' };

const COMMON_COUNTRY_CODES = ['CN', 'HK', 'TW', 'MO', 'JP', 'KR', 'KP', 'US', 'GB', 'CA', 'AU', 'FR', 'DE', 'IT', 'ES', 'IN', 'TH', 'SG'];

export function countryNameZh(value) {
  const code = String(value || '').trim().toUpperCase();
  return COUNTRY_NAMES_ZH[code] || code;
}

export const COUNTRY_OPTIONS_ZH = Object.freeze(Object.keys(COUNTRY_NAMES_ZH)
  .sort((left, right) => {
    const leftRank = COMMON_COUNTRY_CODES.indexOf(left);
    const rightRank = COMMON_COUNTRY_CODES.indexOf(right);
    if (leftRank >= 0 || rightRank >= 0) return (leftRank < 0 ? 999 : leftRank) - (rightRank < 0 ? 999 : rightRank);
    return countryNameZh(left).localeCompare(countryNameZh(right), 'zh-CN');
  })
  .map((code) => ({ value: code, label: `${countryNameZh(code)} · ${code}` })));

export { COUNTRY_NAMES_ZH };
