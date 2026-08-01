export function classifyVipStatus(value) {
  const status = Number(value);
  if (status === 2) return Object.freeze({ status, active: true, expired: false, label: 'VIP 有效', color: 'gold' });
  if (status === 3) return Object.freeze({ status, active: false, expired: true, label: 'VIP 已过期', color: 'warning' });
  return Object.freeze({ status: 1, active: false, expired: false, label: '非 VIP', color: 'default' });
}
