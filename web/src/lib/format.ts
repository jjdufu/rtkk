export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KiB", "MiB", "GiB", "TiB"] as const;
  let value = bytes;
  let idx = 0;
  while (value >= 1024 && idx < units.length - 1) {
    value /= 1024;
    idx += 1;
  }
  const unit = units[idx] ?? "B";
  return idx === 0 ? `${Math.round(value)} ${unit}` : `${value.toFixed(1)} ${unit}`;
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) {
    return "—";
  }
  if (ms < 1000) {
    return `${Math.round(ms)} ms`;
  }
  const secs = ms / 1000;
  if (secs < 60) {
    return `${secs.toFixed(1)} s`;
  }
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

export function formatUptimeSecs(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) {
    return "0s";
  }
  const days = Math.floor(secs / 86400);
  const hours = Math.floor((secs % 86400) / 3600);
  const mins = Math.floor((secs % 3600) / 60);
  const rem = Math.floor(secs % 60);
  if (days > 0) {
    return `${days}d ${hours}h`;
  }
  if (hours > 0) {
    return `${hours}h ${mins}m`;
  }
  if (mins > 0) {
    return `${mins}m ${rem}s`;
  }
  return `${rem}s`;
}

export function formatTimestampMs(tsMs: number | null | undefined): string {
  if (tsMs === null || tsMs === undefined || !Number.isFinite(tsMs) || tsMs <= 0) {
    return "—";
  }
  const date = new Date(tsMs);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export interface ExpandedHostEntry {
  hostname: string;
  ip: string;
}

/**
 * Expands an IPv4 range specification such as `192.168.26.61-66` with prefix `c23-p`
 * into deterministic `{ hostname: "c23-p61", ip: "192.168.26.61" }` entries.
 */
export function expandIpv4Range(prefix: string, rangeSpec: string): { items: ExpandedHostEntry[]; error: string | null } {
  const trimmedPrefix = prefix.trim();
  const trimmedSpec = rangeSpec.trim();
  if (!trimmedPrefix) {
    return { items: [], error: "主机名前缀不能为空" };
  }
  const match = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:-(\d{1,3}))?$/.exec(trimmedSpec);
  if (!match) {
    return { items: [], error: "IP 段格式示例：192.168.26.61-66" };
  }
  const o1 = Number.parseInt(match[1] ?? "", 10);
  const o2 = Number.parseInt(match[2] ?? "", 10);
  const o3 = Number.parseInt(match[3] ?? "", 10);
  const start = Number.parseInt(match[4] ?? "", 10);
  const end = match[5] !== undefined ? Number.parseInt(match[5], 10) : start;

  if ([o1, o2, o3, start, end].some((n) => Number.isNaN(n) || n < 0 || n > 255)) {
    return { items: [], error: "IPv4 地址段超出 0-255 范围" };
  }
  if (start === 0 || end === 255 || end < start) {
    return { items: [], error: "起止末位需满足 1 <= 起始 <= 结束 <= 254" };
  }
  if (end - start + 1 > 128) {
    return { items: [], error: "单次最多批量生成 128 台节点" };
  }

  const items: ExpandedHostEntry[] = [];
  for (let last = start; last <= end; last += 1) {
    items.push({
      hostname: `${trimmedPrefix}${last}`,
      ip: `${o1}.${o2}.${o3}.${last}`,
    });
  }
  return { items, error: null };
}
