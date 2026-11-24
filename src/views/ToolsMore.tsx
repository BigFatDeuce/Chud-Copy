import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, ArrowDownToLine, CheckCircle2, FolderOpen, HardDrive, Network, RefreshCw, Search, ShieldAlert, ShieldCheck, Sparkles, Trash2, Unplug, X } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { Modal, Select, Toggle } from "../components/ui";
import { Frame, JobBar, PathPicker, ResultHead, SpeedGraph, useJob } from "../components/tools";
import type { CleanupItem, CleanupKind, CleanupOptions, CleanupPlace, CleanupReport, DriveIdentity, DriveRow, NetworkReport } from "../lib/types";

// ---------------------------------------------------------------- Conseiller de nettoyage

const KINDS: CleanupKind[] = ["superseded", "installer", "partial", "temp", "download"];
const DAYS = { installerDays: [14, 30, 90, 180], tempDays: [1, 7, 30, 90], downloadDays: [90, 180, 365, 730] } as const;
const BIG = [512 * 1024 * 1024, 1024 ** 3, 5 * 1024 ** 3, 20 * 1024 ** 3];
const DEFAULT_OPTIONS: CleanupOptions = { installerDays: 30, tempDays: 7, downloadDays: 180, bigFolder: 1024 ** 3 };

function CleanupRow({ item, checked, onToggle }: { item: CleanupItem; checked: boolean; onToggle: () => void }) {
  return (
    <li className={checked ? "chosen" : ""}>
      <label>
        <input type="checkbox" checked={checked} onChange={onToggle} />
        <span className={`kind-badge ${item.kind}`}>{t(`cleanup.kind.${item.kind}`)}</span>
        <span className="file-path" title={item.path}>
          {fmt.ellipsize(item.path, 96)}
        </span>
      </label>
      <small>{fmt.ago(item.modified * 1000)}</small>
      <b>{fmt.bytes(item.size)}</b>
      <button className="icon-btn" aria-label={t("dup.reveal")} title={t("dup.reveal")} onClick={() => api.pathReveal(item.path).catch(() => {})}>
        <FolderOpen size={14} />
      </button>
    </li>
  );
}

export function CleanupTool() {
  const { keep, toast } = useApp();
  const [form, setForm] = keep("form:cleanup", { roots: [] as string[], options: DEFAULT_OPTIONS });
  const [chosen, setChosen] = keep<string[]>("form:cleanup:chosen", []);
  const [places, setPlaces] = useState<CleanupPlace[]>([]);
  const [adding, setAdding] = useState<string | null>(null);
  const [filter, setFilter] = useState<CleanupKind | null>(null);
  const [shown, setShown] = useState(200);
  const [confirm, setConfirm] = useState(false);
  const [trashing, setTrashing] = useState(false);
  const job = useJob<CleanupReport>("cleanup");
  const report = job.result;

  useEffect(() => {
    api.cleanupPlaces().then(setPlaces).catch(() => {});
  }, []);

  const add = (path: string) => {
    setAdding(null);
    if (!form.roots.includes(path)) setForm({ ...form, roots: [...form.roots, path] });
  };
  const setOption = (key: keyof CleanupOptions, value: number) => setForm({ ...form, options: { ...form.options, [key]: value } });

  const byKind = useMemo(() => {
    const totals = Object.fromEntries(KINDS.map((kind) => [kind, { n: 0, size: 0 }])) as Record<CleanupKind, { n: number; size: number }>;
    for (const item of report?.items ?? []) {
      totals[item.kind].n += 1;
      totals[item.kind].size += item.size;
    }
    return totals;
  }, [report]);
  const visible = (report?.items ?? []).filter((item) => !filter || item.kind === filter);
  const chosenSet = new Set(chosen);
  const selected = (report?.items ?? []).filter((item) => chosenSet.has(item.path));
  const selectedBytes = selected.reduce((sum, item) => sum + item.size, 0);
  const toggle = (path: string) => setChosen((list) => (list.includes(path) ? list.filter((p) => p !== path) : [...list, path]));

  const start = () => {
    setChosen([]);
    setFilter(null);
    job.run(() => api.toolCleanup("cleanup", form.roots, form.options));
  };

  const trash = async () => {
    setConfirm(false);
    setTrashing(true);
    try {
      const outcome = await api.cleanupTrash(selected);
      toast(t("cleanup.trashed", { n: fmt.count(outcome.trashed.length), size: fmt.bytes(outcome.bytes) }));
      for (const path of outcome.holding) toast(t("trash.held", { path }));
      if (outcome.skipped.length) toast(t("cleanup.skipped", { n: fmt.count(outcome.skipped.length) }), "bad");
      if (report) {
        const gone = new Set(outcome.trashed);
        job.replace({ ...report, items: report.items.filter((item) => !gone.has(item.path)) });
      }
      setChosen([]);
    } catch (error) {
      toast(describe(String(error)), "bad");
    } finally {
      setTrashing(false);
    }
  };

  return (
    <Frame id="cleanup">
      <section className="panel">
        <h2>{t("dup.where")}</h2>
        {places.length > 0 && (
          <div className="target-choices">
            {places.map((place) => (
              <button key={place.path} className={form.roots.includes(place.path) ? "chip-choice active" : "chip-choice"} disabled={job.running} onClick={() => (form.roots.includes(place.path) ? setForm({ ...form, roots: form.roots.filter((r) => r !== place.path) }) : add(place.path))} title={place.path}>
                {place.kind === "downloads" ? <ArrowDownToLine size={14} /> : <Sparkles size={14} />} {t(`cleanup.place.${place.kind}`)}
              </button>
            ))}
          </div>
        )}
        <div className="root-list">
          {form.roots
            .filter((root) => !places.some((p) => p.path === root))
            .map((root) => (
              <span key={root} className="chip-choice active">
                <span className="file-path" title={root}>
                  {fmt.ellipsize(root, 60)}
                </span>
                <button aria-label={t("tools.dismiss")} onClick={() => setForm({ ...form, roots: form.roots.filter((r) => r !== root) })} disabled={job.running}>
                  <X size={12} />
                </button>
              </span>
            ))}
        </div>
        <PathPicker value={adding} onPick={add} disabled={job.running} />
        <div className="cleanup-options">
          {(Object.keys(DAYS) as (keyof typeof DAYS)[]).map((key) => (
            <label key={key} className="field inline">
              <span>{t(`cleanup.${key}`)}</span>
              <Select<number> value={form.options[key]} onChange={(days) => setOption(key, days)} disabled={job.running} choices={DAYS[key].map((days) => ({ value: days, label: t("cleanup.days", { n: days }) }))} />
            </label>
          ))}
          <label className="field inline">
            <span>{t("cleanup.bigFolder")}</span>
            <Select<number> value={form.options.bigFolder} onChange={(size) => setOption("bigFolder", size)} disabled={job.running} choices={BIG.map((size) => ({ value: size, label: fmt.bytes(size) }))} />
          </label>
        </div>
        <div className="tool-actions">
          <button className="btn primary" disabled={form.roots.length === 0 || job.running} onClick={start}>
            <Search size={15} /> {t("cleanup.start")}
          </button>
        </div>
        <p className="muted small">{t("cleanup.safe")}</p>
      </section>

      {job.running && <JobBar title={t("cleanup.running")} progress={job.progress} onStop={job.stop} />}

      {report && (
        <section className="panel">
          <ResultHead title={report.cancelled ? t("integrity.stopped") : report.items.length ? t("cleanup.found", { n: fmt.count(report.items.length) }) : t("cleanup.none")} onDismiss={job.dismiss} />
          <p className="muted small">{t("cleanup.summary", { n: fmt.count(report.scanned), size: fmt.bytes(report.scannedBytes), time: fmt.duration(report.elapsed) })}</p>
          {report.items.length > 0 && (
            <>
              <div className="big-figure">
                <span>{fmt.bytes(report.items.reduce((sum, item) => sum + item.size, 0))}</span>
                <small>{t("cleanup.reclaimable")}</small>
              </div>
              <div className="tiles">
                {KINDS.map((kind) => (
                  <button key={kind} className={`tile ${filter === kind ? "active" : ""}`} disabled={byKind[kind].n === 0} onClick={() => setFilter(filter === kind ? null : kind)} title={t(`cleanup.kind.${kind}.hint`)}>
                    <span className="tile-value">{fmt.count(byKind[kind].n)}</span>
                    <span className="tile-label">{t(`cleanup.kind.${kind}`)}</span>
                    <span className="tile-sub">{fmt.bytes(byKind[kind].size)}</span>
                  </button>
                ))}
              </div>
              {filter && <p className="muted small">{t(`cleanup.kind.${filter}.hint`)}</p>}
              <div className="restore-bar">
                <button className="btn ghost small" onClick={() => setChosen([...new Set([...chosen, ...visible.map((item) => item.path)])])}>
                  {t("restore.all")}
                </button>
                <button className="btn ghost small" disabled={chosen.length === 0} onClick={() => setChosen([])}>
                  {t("cleanup.none.select")}
                </button>
                <span className="muted small">{t("cleanup.chosen", { n: fmt.count(selected.length), size: fmt.bytes(selectedBytes) })}</span>
                <button className="btn danger small" disabled={selected.length === 0 || trashing} onClick={() => setConfirm(true)}>
                  {trashing ? <RefreshCw size={14} className="spin" /> : <Trash2 size={14} />} {trashing ? t("trash.working") : t("cleanup.trash")}
                </button>
              </div>
              <ul className="cleanup-list">
                {visible.slice(0, shown).map((item) => (
                  <CleanupRow key={item.path} item={item} checked={chosenSet.has(item.path)} onToggle={() => toggle(item.path)} />
                ))}
              </ul>
              {visible.length > shown && (
                <button className="btn ghost" onClick={() => setShown(shown + 200)}>
                  {t("dup.more", { n: visible.length - shown })}
                </button>
              )}
            </>
          )}
          {report.folders.length > 0 && (
            <>
              <h3 className="sub">{t("cleanup.folders")}</h3>
              <p className="muted small">{t("cleanup.foldersHint")}</p>
              <ul className="space-list big-folders">
                {report.folders.map((folder) => (
                  <li key={folder.path}>
                    <i />
                    <span className="file-path" title={folder.path}>
                      {fmt.ellipsize(folder.path, 80)}
                      <small className="muted">{t("cleanup.folderInfo", { n: fmt.count(folder.files), when: folder.newest ? fmt.ago(folder.newest * 1000) : "—" })}</small>
                    </span>
                    <span className="space-bar">
                      <span style={{ width: `${(folder.size / report.folders[0].size) * 100}%` }} />
                    </span>
                    <b>{fmt.bytes(folder.size)}</b>
                    <button className="icon-btn" aria-label={t("dup.reveal")} title={t("dup.reveal")} onClick={() => api.pathReveal(folder.path).catch(() => {})}>
                      <FolderOpen size={14} />
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}

      {confirm && (
        <Modal
          tone="danger"
          title={t("cleanup.confirmTitle")}
          onClose={() => setConfirm(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirm(false)}>
                {t("common.cancel")}
              </button>
              <button className="btn danger" onClick={trash}>
                <Trash2 size={15} /> {t("cleanup.trashConfirm", { n: fmt.count(selected.length), size: fmt.bytes(selectedBytes) })}
              </button>
            </>
          }
        >
          <p>{t("cleanup.confirmBody")}</p>
          <p className="muted small">{t("trash.holdingNote")}</p>
          <ul className="confirm-list">
            {selected.slice(0, 60).map((item) => (
              <li key={item.path}>
                <code>{item.path}</code>
              </li>
            ))}
          </ul>
          {selected.length > 60 && <p className="muted small">{t("editor.more", { n: selected.length - 60 })}</p>}
        </Modal>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Testeur réseau et NAS

const NET_SIZES = [64 * 1024 ** 2, 256 * 1024 ** 2, 1024 ** 3, 4 * 1024 ** 3];

function megabits(bytesPerSecond: number) {
  return `${fmt.count(Math.round((bytesPerSecond * 8) / 1_000_000))} Mbit/s`;
}

/** Ce que les mesures veulent dire, en clair, avec un conseil quand il y en a un. */
function advice(report: NetworkReport): { tone: "ok" | "warn" | "bad"; text: string }[] {
  const out: { tone: "ok" | "warn" | "bad"; text: string }[] = [];
  if (!report.readable) return [{ tone: "bad", text: t("net.advice.unreadable") }];
  if (report.verified === false) out.push({ tone: "bad", text: t("net.advice.corrupt") });
  if (report.writable === false) out.push({ tone: "bad", text: t("net.advice.readOnly") });
  if (report.cleaned === false) out.push({ tone: "warn", text: t("net.advice.leftover", { path: report.path }) });
  if (report.host && !report.reachable) out.push({ tone: "warn", text: t("net.advice.port", { host: report.host, port: String(report.port ?? "") }) });
  const best = Math.max(report.read?.speed ?? 0, report.write?.speed ?? 0);
  if (best > 0 && report.kind !== "local") {
    const mb = best / 1_000_000;
    const key = mb < 13 ? "net.advice.fast100" : mb < 45 ? "net.advice.wifi" : mb < 95 ? "net.advice.partial" : mb < 130 ? "net.advice.gigabit" : "net.advice.multigig";
    out.push({ tone: mb < 45 ? "warn" : "ok", text: t(key, { speed: megabits(best) }) });
  }
  const latency = report.connect?.average ?? report.metadata.average;
  if (latency > 10) out.push({ tone: "warn", text: t("net.advice.latency", { ms: latency.toFixed(1) }) });
  if ((report.connect?.jitter ?? 0) > 5) out.push({ tone: "warn", text: t("net.advice.jitter") });
  if ((report.connect?.failures ?? 0) > 0) out.push({ tone: "bad", text: t("net.advice.drops", { n: String(report.connect!.failures) }) });
  if (report.smallFilesPerSecond !== null && report.smallFilesPerSecond < 25) out.push({ tone: "warn", text: t("net.advice.small", { n: report.smallFilesPerSecond.toFixed(1) }) });
  if (report.readFromTest) out.push({ tone: "warn", text: t("net.advice.cache") });
  if (report.write && report.write.speed > 0) out.push({ tone: "ok", text: t("net.advice.estimate", { time: fmt.duration((100 * 1024 ** 3) / report.write.speed) }) });
  if (out.every((item) => item.tone === "ok")) out.unshift({ tone: "ok", text: t("net.advice.healthy") });
  return out;
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="net-stat">
      <span className="net-label">{label}</span>
      <span className="net-value">{value}</span>
      {sub && <span className="net-sub">{sub}</span>}
    </div>
  );
}

export function NetworkTool() {
  const { keep } = useApp();
  const [form, setForm] = keep("form:network", { path: null as string | null, writeTest: false, size: 256 * 1024 ** 2 });
  const job = useJob<NetworkReport>("network");
  const report = job.result;
  const phase = job.progress?.phase;
  const ms = (value: number) => `${value < 10 ? value.toFixed(2) : value.toFixed(0)} ms`;
  return (
    <Frame id="network">
      <section className="panel">
        <PathPicker value={form.path} onPick={(path) => setForm({ ...form, path })} disabled={job.running} />
        <Toggle checked={form.writeTest} onChange={(writeTest) => setForm({ ...form, writeTest })} label={t("net.write")} hint={t("net.writeHint")} disabled={job.running} />
        <label className="field inline">
          <span>{t("net.size")}</span>
          <Select<number> value={form.size} onChange={(size) => setForm({ ...form, size })} disabled={job.running} choices={NET_SIZES.map((size) => ({ value: size, label: fmt.bytes(size) }))} />
        </label>
        <div className="tool-actions">
          <button className="btn primary" disabled={!form.path || job.running} onClick={() => job.run(() => api.toolNetwork("network", form.path!, { writeTest: form.writeTest, size: form.size }))}>
            <Network size={15} /> {t("net.start")}
          </button>
        </div>
        <p className="muted small">{form.writeTest ? t("net.noteWrite") : t("net.noteRead")}</p>
      </section>

      {job.running && (
        <JobBar
          title={t(`net.phase.${phase ?? "connect"}`)}
          progress={job.progress}
          onStop={job.stop}
          detail={!job.progress?.totalBytes ? "" : phase === "small" ? `${job.progress.bytes} / ${job.progress.totalBytes}` : `${fmt.bytes(job.progress.bytes)} / ${fmt.bytes(job.progress.totalBytes)}`}
        />
      )}

      {report && (
        <section className={`panel verdict ${report.readable && report.verified !== false ? "ok" : "bad"}`}>
          <ResultHead title={report.cancelled ? t("integrity.stopped") : report.readable ? t("net.reachable") : t("net.unreachable")} onDismiss={job.dismiss} />
          <p className="muted small">
            {t(`net.kind.${report.kind}`)}
            {report.host ? ` · ${report.host}${report.address && report.address !== report.host ? ` (${report.address})` : ""}${report.port ? ` · port ${report.port}` : ""}` : ""}
          </p>
          {report.error && <p className="issue error">{report.error}</p>}
          <div className="net-grid">
            {report.connect && <Stat label={t("net.server")} value={report.connect.samples.length ? ms(report.connect.average) : "—"} sub={report.connect.samples.length ? t("net.range", { min: ms(report.connect.min), max: ms(report.connect.max), jitter: ms(report.connect.jitter) }) : t("net.noAnswer")} />}
            <Stat label={t("net.share")} value={report.metadata.samples.length ? ms(report.metadata.average) : "—"} sub={t("net.listed", { n: fmt.count(report.entries), ms: ms(report.listMs) })} />
            <Stat label={t("net.read")} value={report.read ? fmt.speed(report.read.speed) : "—"} sub={report.read ? `${megabits(report.read.speed)} · ${fmt.bytes(report.read.bytes)}` : t("net.noFiles")} />
            {report.writable !== null && <Stat label={t("net.writeSpeed")} value={report.write ? fmt.speed(report.write.speed) : "—"} sub={report.write ? `${megabits(report.write.speed)} · ${fmt.bytes(report.write.bytes)}` : t("net.denied")} />}
            {report.smallFilesPerSecond !== null && <Stat label={t("net.small")} value={t("net.perSecond", { n: report.smallFilesPerSecond.toFixed(1) })} />}
            {report.verified !== null && <Stat label={t("net.verified")} value={report.verified ? t("net.yes") : t("net.no")} />}
          </div>
          {(report.read?.samples.length ?? 0) > 1 && (
            <>
              <h3 className="sub">{t("net.read")}</h3>
              <SpeedGraph samples={report.read!.samples} />
            </>
          )}
          {(report.write?.samples.length ?? 0) > 1 && (
            <>
              <h3 className="sub">{t("net.writeSpeed")}</h3>
              <SpeedGraph samples={report.write!.samples} />
            </>
          )}
          <h3 className="sub">{t("net.meaning")}</h3>
          <ul className="advice-list">
            {advice(report).map((item, i) => (
              <li key={i} className={item.tone}>
                {item.tone === "ok" ? <CheckCircle2 size={15} /> : <AlertTriangle size={15} />}
                <span>{item.text}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Garde d'identité des disques

function driveName(identity: DriveIdentity) {
  const parts = [identity.label || identity.mount, identity.fileSystem, identity.total ? fmt.bytes(identity.total) : ""].filter(Boolean);
  return parts.join(" · ");
}

export function DrivesTool() {
  const { toast } = useApp();
  const [rows, setRows] = useState<DriveRow[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [confirm, setConfirm] = useState<DriveRow | null>(null);

  const refresh = useCallback(() => {
    setLoading(true);
    api
      .drivesStatus()
      .then(setRows)
      .catch((error) => toast(describe(String(error)), "bad"))
      .finally(() => setLoading(false));
  }, [toast]);

  useEffect(refresh, [refresh]);

  const accept = async (row: DriveRow) => {
    setConfirm(null);
    try {
      await api.driveAccept(row.taskId, row.endpointId);
      toast(t("drives.accepted"));
      refresh();
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const tasks = useMemo(() => {
    const groups = new Map<string, DriveRow[]>();
    for (const row of rows ?? []) groups.set(row.taskId, [...(groups.get(row.taskId) ?? []), row]);
    return [...groups.values()];
  }, [rows]);
  const different = (rows ?? []).filter((row) => row.check.state === "different").length;
  const protectedCount = (rows ?? []).filter((row) => row.identity).length;

  return (
    <Frame id="drives">
      <section className={`panel verdict ${different ? "bad" : "ok"}`}>
        <div className="result-head">
          <h2>{different ? t("drives.alert", { n: fmt.count(different) }) : t("drives.allGood", { n: fmt.count(protectedCount) })}</h2>
          <div className="result-actions">
            <button className="btn ghost small" onClick={refresh} disabled={loading}>
              <RefreshCw size={14} className={loading ? "spin" : ""} /> {t("drives.refresh")}
            </button>
          </div>
        </div>
        <p className="muted">{t("drives.explain")}</p>
      </section>

      {rows && rows.length === 0 && <p className="issue info">{t("drives.empty")}</p>}

      {tasks.map((group) => (
        <section key={group[0].taskId} className="panel drive-task">
          <h2>{group[0].taskName}</h2>
          <ul className="drive-list">
            {group.map((row) => {
              const state = row.check.state;
              const present = state === "same" || state === "different" || (state === "unknown" && !!row.path);
              return (
                <li key={row.endpointId} className={state}>
                  <span className="drive-role">{t(`drives.role.${row.role}`)}</span>
                  <span className="drive-main">
                    <code className="file-path" title={row.path}>
                      {fmt.ellipsize(row.path, 80)}
                    </code>
                    <small className="muted">{row.identity ? t("drives.remembered", { drive: driveName(row.identity) }) : t("drives.notYet")}</small>
                    {row.check.state === "different" && <small className="drive-found">{t("drives.found", { drive: driveName(row.check.found) })}</small>}
                  </span>
                  <span className={`drive-state ${state}`}>
                    {state === "same" && <ShieldCheck size={15} />}
                    {state === "different" && <ShieldAlert size={15} />}
                    {state === "absent" && <Unplug size={15} />}
                    {state === "unknown" && <HardDrive size={15} />}
                    {t(`drives.state.${state}`)}
                  </span>
                  {state === "different" && (
                    <button className="btn ghost small danger-text" onClick={() => setConfirm(row)}>
                      {t("drives.accept")}
                    </button>
                  )}
                  {state === "unknown" && present && (
                    <button className="btn ghost small" onClick={() => accept(row)}>
                      {t("drives.learn")}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </section>
      ))}

      {confirm && confirm.check.state === "different" && (
        <Modal
          tone="danger"
          title={t("drives.confirmTitle")}
          onClose={() => setConfirm(null)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirm(null)}>
                {t("common.cancel")}
              </button>
              <button className="btn danger" onClick={() => accept(confirm)}>
                {t("drives.confirm")}
              </button>
            </>
          }
        >
          <p>{t("drives.confirmBody", { path: confirm.path })}</p>
          <ul className="confirm-list">
            <li>{t("drives.expected", { drive: confirm.identity ? driveName(confirm.identity) : "—" })}</li>
            <li>{t("drives.found", { drive: driveName(confirm.check.found) })}</li>
          </ul>
          <p className="muted small">{t("drives.confirmNote")}</p>
        </Modal>
      )}
    </Frame>
  );
}
