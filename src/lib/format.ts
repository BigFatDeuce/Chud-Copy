import { locale, t } from "./i18n";
import type { Mode, RunStatus } from "./types";

export function count(value: number): string {
  return new Intl.NumberFormat(locale()).format(value);
}

export function bytes(value: number): string {
  const units = t("units").split(",");
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${new Intl.NumberFormat(locale(), { maximumFractionDigits: 1 }).format(amount)} ${units[unit]}`;
}

export function speed(value: number): string {
  return `${bytes(value)}/s`;
}

export function duration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return t("time.seconds", { s: total });
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (hours === 0) return t("time.minutes", { m: minutes, s: String(total % 60).padStart(2, "0") });
  return t("time.hours", { h: hours, m: String(minutes).padStart(2, "0") });
}

export function ago(timestamp: number): string {
  const seconds = (timestamp - Date.now()) / 1000;
  const steps: [number, Intl.RelativeTimeFormatUnit][] = [
    [60, "second"],
    [3600, "minute"],
    [86400, "hour"],
    [604800, "day"],
    [2629800, "week"],
    [31557600, "month"],
    [Infinity, "year"],
  ];
  const divisor: Record<string, number> = { second: 1, minute: 60, hour: 3600, day: 86400, week: 604800, month: 2629800, year: 31557600 };
  const format = new Intl.RelativeTimeFormat(locale(), { numeric: "auto" });
  for (const [limit, unit] of steps) {
    if (Math.abs(seconds) < limit) return format.format(Math.round(seconds / divisor[unit]), unit);
  }
  return "";
}

export function date(timestamp: number): string {
  return new Intl.DateTimeFormat(locale(), { dateStyle: "short", timeStyle: "short" }).format(new Date(timestamp));
}

export function shortPath(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** Raccourcit par le milieu : le disque et le dossier final sont ce qui identifie un chemin. */
export function ellipsize(path: string, max: number): string {
  if (path.length <= max) return path;
  const tail = Math.ceil(max * 0.62);
  return `${path.slice(0, max - tail - 1)}…${path.slice(-tail)}`;
}

export function isDestructive(mode: Mode): boolean {
  return mode === "mirror" || mode === "mirrorArchive";
}

export function modeLabel(mode: Mode): string {
  return t(`mode.${mode}.label`);
}

export function modeSummary(mode: Mode): string {
  return t(`mode.${mode}.summary`);
}

export function statusLabel(status: RunStatus): string {
  return t(`status.${status}`);
}

export function statusTone(status: RunStatus): "ok" | "warn" | "bad" | "live" | "muted" {
  const tones: Record<RunStatus, "ok" | "warn" | "bad" | "live" | "muted"> = {
    running: "live",
    paused: "warn",
    cancelling: "warn",
    completed: "ok",
    completedWithErrors: "warn",
    failed: "bad",
    cancelled: "muted",
    interrupted: "bad",
  };
  return tones[status];
}
