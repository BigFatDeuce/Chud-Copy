import { Activity, Bot, HardDrive, Info, ShieldAlert, TrendingUp } from "lucide-react";
import { api } from "../lib/api";
import { copyText, healthPrompt } from "../lib/ai";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import type { DiskHealth, HealthReport } from "../lib/types";

// Notes d'information : un fait passé ou bénin, qui ne dit pas que le disque va mal.
const INFO_NOTES = ["link-errors", "nvme-log-benign", "errors-old"];

function Metric({ label, value, change }: { label: string; value: string | null; change?: { before: number; after: number } }) {
  if (value === null) return null;
  const rising = change && change.after > change.before;
  return (
    <div className={rising ? "rising" : undefined}>
      <dt>{label}</dt>
      <dd>
        {value}
        {change && (
          <small className={rising ? "delta up" : "delta"}>
            {rising ? "+" : ""}
            {fmt.count(change.after - change.before)}
          </small>
        )}
      </dd>
    </div>
  );
}

function DiskCard({ disk }: { disk: DiskHealth }) {
  const verdict = disk.verdict ?? "unknown";
  const n = (value: number | null | undefined) => (value === null || value === undefined ? null : fmt.count(value));
  const change = (field: string) => disk.changes.find((c) => c.field === field);
  const surface = [disk.reallocated, disk.pending, disk.uncorrectable, disk.mediaErrors].some((v) => v !== null && v !== undefined);
  const link = [disk.crcErrors, disk.commandTimeouts].some((v) => v !== null && v !== undefined);
  return (
    <article className={`disk-health ${verdict}`}>
      <header>
        <HardDrive size={18} />
        <div>
          <h3>{disk.model || disk.name}</h3>
          <small dir="ltr">{[disk.kind, disk.size ? fmt.bytes(disk.size) : "", disk.name].filter(Boolean).join(" · ")}</small>
        </div>
        <span className={`status-tag ${verdict === "healthy" ? "ok" : verdict === "unknown" ? "muted" : "bad"}`}>{t(`verdict.${verdict}`)}</span>
      </header>

      <dl className="health-metrics">
        <Metric label={t("health.temperature")} value={disk.temperature ? `${Math.round(disk.temperature)} °C` : null} />
        <Metric label={t("health.hours")} value={n(disk.powerOnHours)} />
        <Metric label={t("health.wear")} value={disk.wearUsed === null || disk.wearUsed === undefined ? null : `${disk.wearUsed} %`} />
        <Metric label={t("health.cycles")} value={n(disk.powerCycles)} />
      </dl>

      {surface && (
        <div className="metric-group">
          <h4>{t("health.surface")}</h4>
          <p className="muted small">{t("health.surfaceHint")}</p>
          <dl className="health-metrics">
            <Metric label={t("health.reallocated")} value={n(disk.reallocated)} change={change("reallocated")} />
            <Metric label={t("health.pending")} value={n(disk.pending)} change={change("pending")} />
            <Metric label={t("health.uncorrectable")} value={n(disk.uncorrectable)} change={change("uncorrectable")} />
            <Metric label={t("health.media")} value={n(disk.mediaErrors)} change={change("mediaErrors")} />
          </dl>
        </div>
      )}

      {link && (
        <div className="metric-group">
          <h4>{t("health.link")}</h4>
          <p className="muted small">{t("health.linkHint")}</p>
          <dl className="health-metrics">
            <Metric label={t("health.crc")} value={n(disk.crcErrors)} change={change("crcErrors")} />
            <Metric label={t("health.timeouts")} value={n(disk.commandTimeouts)} change={change("commandTimeouts")} />
          </dl>
        </div>
      )}

      {disk.errorLog !== null && disk.errorLog !== undefined && disk.errorLog > 0 && (
        <div className="metric-group">
          <h4>{t("health.log")}</h4>
          <dl className="health-metrics">
            <Metric label={t("health.logEntries")} value={n(disk.errorLog)} change={change("errorLog")} />
            <Metric label={t("health.logRecent")} value={n(disk.errorsRecent)} />
          </dl>
        </div>
      )}

      {disk.notes.map((note) => (
        <p key={note} className={INFO_NOTES.includes(note) ? "issue info" : "issue warning"}>
          {INFO_NOTES.includes(note) ? <Info size={14} /> : note.startsWith("rising") ? <TrendingUp size={14} /> : <ShieldAlert size={14} />}
          <span>{t(`health.note.${note}`)}</span>
        </p>
      ))}

      <p className="muted small">{disk.previousAt ? t("health.compared", { when: fmt.date(disk.previousAt) }) : t("health.firstReading")}</p>
    </article>
  );
}

export function HealthView() {
  const { toast, keep } = useApp();
  const [state, setState] = keep<{ report: HealthReport | null; running: boolean }>("health", { report: null, running: false });
  const { report, running } = state;

  const scan = async () => {
    setState((current) => ({ ...current, running: true }));
    try {
      const result = await api.healthScan();
      setState({ report: result, running: false });
    } catch (error) {
      setState((current) => ({ ...current, running: false }));
      toast(describe(String(error)), "bad");
    }
  };

  const askAi = async () => {
    if (!report) return;
    try {
      await copyText(healthPrompt(report));
      toast(t("ai.copied"));
    } catch {
      toast(t("ai.copyFailed"), "bad");
    }
  };

  return (
    <div className="view tool">
      <header className="view-head">
        <div>
          <h1>{t("nav.health")}</h1>
          <p>{t("health.lead")}</p>
        </div>
        <div className="head-actions">
          {report && (
            <button className="btn ghost" onClick={askAi}>
              <Bot size={15} /> {t("ai.button")}
            </button>
          )}
          <button className={running ? "btn primary analyzing" : "btn primary"} disabled={running} onClick={scan}>
            <Activity size={15} /> {running ? t("health.scanning") : report ? t("health.rescan") : t("health.scan")}
          </button>
        </div>
      </header>

      {!report && !running && (
        <section className="panel">
          <p>{t("health.explain")}</p>
          <p className="muted small">{t("health.admin")}</p>
        </section>
      )}

      {report && (
        <>
          {report.missingTool && <p className="issue warning">{t("health.noTool")}</p>}
          {report.limited && <p className="issue warning">{t("health.limited")}</p>}
          <div className="health-grid">
            {report.disks.map((disk, i) => (
              <DiskCard key={`${disk.serial || disk.name}-${i}`} disk={disk} />
            ))}
          </div>
          <p className="issue info">
            <Info size={14} /> <span>{t("health.disclaimer")}</span>
          </p>
        </>
      )}
    </div>
  );
}
