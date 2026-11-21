// Display formatters shared by the pages (one definition per meaning).
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
export function whenSecs(secs: number): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString();
}
export function whenMs(ms: number): string {
  if (!ms) return "—";
  return new Date(ms).toLocaleTimeString();
}
export function uptime(secs: number): string {
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} 分钟`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 小时 ${m % 60} 分钟`;
  return `${Math.floor(h / 24)} 天 ${h % 24} 小时`;
}
