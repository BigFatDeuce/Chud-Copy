import { useEffect, useRef, useState } from "react";
import { AlertCircle, Archive, ArrowRight, CheckCircle2, CircleSlash, FileText, Home as HomeIcon, Pause, Play, Square, XCircle } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { isLive } from "../lib/machines";
import { Beam, Modal, tintStyle } from "../components/ui";
import { Blaze, heatOf } from "../effects/Blaze";
import { QueuePanel } from "./Queue";
import { PowerToggle } from "../components/power";
import type { LinkRecord, RunRecord, RunStatus } from "../lib/types";

/** `archive` : la liaison range ce qu'elle remplace ou retire dans le dossier d'archives de sa destination. */
export function LinkProgress({ link, live, archive }: { link: LinkRecord; live: boolean; archive?: boolean }) {
  const { toast } = useApp();
  const ratio = link.totalBytes > 0 ? link.bytes / link.totalBytes : link.phase === "done" ? 1 : 0;
  const verifying = link.phase === "verifying";
  const verifyRatio = link.verifyTotal > 0 ? link.verifyChecked / link.verifyTotal : 0;
  const scanning = link.phase === "copying" && link.totalBytes === 0 && link.totalTransfers === 0;
  const nothingYet = link.totalBytes === 0 && link.totalTransfers === 0 && link.checks === 0;
  const tone = link.phase === "failed" || link.status === "completedWithErrors" ? "bad" : link.phase === "done" ? "ok" : link.phase === "skipped" || link.phase === "cancelled" ? "muted" : undefined;
  return (
    <article className={`link-progress ${link.phase}`}>
      <div className="lp-head">
        <div className="lp-route" title={`${link.source} → ${link.target}`}>
          <span>{fmt.shortPath(link.source)}</span>
          <ArrowRight size={14} />
          <code>{fmt.ellipsize(link.target, 86)}</code>
        </div>
        {archive && (
          <button className="icon-btn labelled small" onClick={() => api.archiveReveal(link.target).catch((e) => toast(describe(String(e)), "bad"))}>
            <Archive size={14} /> {t("run.archives")}
          </button>
        )}
        <span className={`phase ${link.phase}`}>{t(`phase.${link.phase}`)}</span>
      </div>
      <Beam value={verifying ? verifyRatio : ratio} tone={tone} indeterminate={scanning && live} />
      <div className="lp-stats">
        {verifying ? (
          <span>{t("run.checked", { done: fmt.count(link.verifyChecked), total: fmt.count(link.verifyTotal) })}</span>
        ) : nothingYet ? (
          live && link.phase === "copying" && <span>{t("run.comparing")}</span>
        ) : (
          <>
            <span>
              {fmt.bytes(link.bytes)} / {fmt.bytes(link.totalBytes)}
            </span>
            <span>{t("run.files", { done: fmt.count(link.transfers), total: fmt.count(link.totalTransfers) })}</span>
          </>
        )}
        {link.checks > 0 && !verifying && <span>{t("run.compared", { n: fmt.count(link.checks) })}</span>}
        {link.deletes > 0 && <span className="danger-text">{t("run.deleted", { n: fmt.count(link.deletes) })}</span>}
        {link.verifyTotal > 0 && !verifying && <span className={link.verifyFailures ? "danger-text" : "ok-text"}>{link.verifyFailures ? t("run.gaps", { n: link.verifyFailures }) : t("run.verified", { n: fmt.count(link.verifyChecked) })}</span>}
        {link.errors > 0 && <span className="danger-text">{t("run.errors", { n: fmt.count(link.errors) })}</span>}
        {live && link.speed > 0 && <span className="lp-speed">{fmt.speed(link.speed)}</span>}
        {live && link.eta !== null && link.speed > 0 && <span>{fmt.duration(link.eta)}</span>}
      </div>
      {live && link.current.length > 0 && (
        <ul className="lp-current">
          {link.current.slice(0, 4).map((file) => (
            <li key={file.name}>
              <span title={file.name}>{file.name}</span>
              <em>{file.percentage} %</em>
            </li>
          ))}
        </ul>
      )}
      {link.message && <p className="lp-message">{describe(link.message)}</p>}
    </article>
  );
}

const ICON: Partial<Record<RunStatus, React.ReactNode>> = {
  completed: <CheckCircle2 size={30} />,
  completedWithErrors: <AlertCircle size={30} />,
  failed: <XCircle size={30} />,
  cancelled: <CircleSlash size={30} />,
  interrupted: <XCircle size={30} />,
};

export function ErrorList({ run }: { run: RunRecord }) {
  if (run.errors.length === 0) return null;
  const groups = new Map<string, number>();
  for (const error of run.errors) {
    const title = t(`error.${error.category ?? "other"}`);
    groups.set(title, (groups.get(title) ?? 0) + 1);
  }
  return (
    <section className="panel errors">
      <h2>
        {t("errors.title")} <span className="count">{fmt.count(run.errorsTotal)}</span>
      </h2>
      <div className="error-groups">
        {[...groups.entries()].map(([title, n]) => (
          <span key={title}>
            {title} <b>{n}</b>
          </span>
        ))}
      </div>
      <ul className="error-list">
        {run.errors.slice(-120).reverse().map((error, i) => (
          <li key={i}>
            <b>{t(`error.${error.category ?? "other"}`)}</b>
            {error.path && <code>{error.path}</code>}
            <small>{describe(error.detail)}</small>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function RunView() {
  const { run, go, cosmos, colors } = useApp();
  const [confirmCancel, setConfirmCancel] = useState(false);
  const previous = useRef<RunStatus | null>(null);

  const live = isLive(run?.status);

  useEffect(() => {
    if (!run) return;
    // L'intensité suit le débit réel, sur une échelle logarithmique de 1 Mo/s à 1 Go/s.
    const megabytes = run.totals.speed / (1024 * 1024);
    const intensity = run.status === "running" && megabytes > 0 ? Math.min(1, 0.15 + Math.log10(Math.max(1, megabytes)) / 3.4) : 0;
    cosmos?.setFlow(intensity);
    if (previous.current && previous.current !== run.status && !live) cosmos?.burst(run.status === "completed" ? "ok" : "bad");
    previous.current = run.status;
  }, [run, cosmos, live]);

  useEffect(() => () => cosmos?.setFlow(0), [cosmos]);

  if (!run) {
    return (
      <div className="view run">
        <div className="empty">
          <p>{t("run.starting")}</p>
        </div>
      </div>
    );
  }

  const { totals } = run;
  const paused = run.status === "paused";
  const cancelling = run.status === "cancelling";
  const known = totals.totalKnown && (totals.totalBytes > 0 || !live);
  const ratio = totals.totalBytes > 0 ? Math.min(1, totals.bytes / totals.totalBytes) : !live && run.status === "completed" ? 1 : 0;
  const tone = fmt.statusTone(run.status);

  return (
    <div className="view run">
      <section className={`run-hero ${tone} ${colors[run.taskId] ? "tinted" : ""}`} style={tintStyle(colors[run.taskId])}>
        <Blaze heat={heatOf(run)} color={colors[run.taskId]} active={live} />
        <div className="run-title">
          <span className={`status-tag ${tone}`}>{fmt.statusLabel(run.status)}</span>
          <h1>{run.taskName}</h1>
        </div>

        {live ? (
          <div className="run-figure">
            {known ? (
              <span className="percent">
                {Math.floor(ratio * 100)}
                <small>%</small>
              </span>
            ) : totals.bytes > 0 ? (
              <span className="percent waiting">
                {fmt.bytes(totals.bytes)} <small>{t("run.noTotal")}</small>
              </span>
            ) : (
              <span className="percent waiting">{t("run.comparing")}</span>
            )}
          </div>
        ) : (
          <div className="run-result">
            {ICON[run.status]}
            <p>{t("run.result", { size: fmt.bytes(totals.bytes), time: fmt.duration(run.elapsed) })}</p>
          </div>
        )}

        <Beam value={ratio} indeterminate={live && !known && !paused} tone={!live ? (run.status === "completed" ? "ok" : "warn") : paused ? "warn" : undefined} />

        <dl className="metrics">
          <div>
            <dt>{t("metric.speed")}</dt>
            <dd>{live && !paused ? fmt.speed(totals.speed) : "—"}</dd>
          </div>
          <div>
            <dt>{t("metric.average")}</dt>
            <dd>{fmt.speed(totals.averageSpeed)}</dd>
          </div>
          <div>
            <dt>{t("metric.transferred")}</dt>
            <dd>
              {fmt.bytes(totals.bytes)} <small>/ {fmt.bytes(totals.totalBytes)}</small>
            </dd>
          </div>
          <div>
            <dt>{t("metric.files")}</dt>
            <dd>
              {fmt.count(totals.transfers)} <small>/ {fmt.count(totals.totalTransfers)}</small>
            </dd>
          </div>
          <div>
            <dt>{t("metric.elapsed")}</dt>
            <dd>{fmt.duration(run.elapsed)}</dd>
          </div>
          <div>
            <dt>{t("metric.remaining")}</dt>
            <dd>{live && !paused && totals.eta !== null ? fmt.duration(totals.eta) : "—"}</dd>
          </div>
        </dl>

        <div className="run-actions">
          {live ? (
            <>
              <PowerToggle />
              <button className="btn ghost" disabled={cancelling} onClick={() => (paused ? api.runResume() : api.runPause())}>
                {paused ? <Play size={15} /> : <Pause size={15} />} {paused ? t("run.resume") : t("run.pause")}
              </button>
              <button className="btn ghost danger-text" disabled={cancelling} onClick={() => setConfirmCancel(true)}>
                <Square size={14} /> {t("common.cancel")}
              </button>
            </>
          ) : (
            <>
              <button className="btn ghost" onClick={() => go({ name: "home" })}>
                <HomeIcon size={15} /> {t("home.title")}
              </button>
              <button className="btn primary" onClick={() => go({ name: "history", runId: run.id })}>
                <FileText size={15} /> {t("run.report")}
              </button>
            </>
          )}
        </div>
      </section>

      <QueuePanel />

      <section className="run-links">
        {run.links.map((link) => (
          <LinkProgress key={link.id} link={link} live={live} archive={run.mode === "mirrorArchive"} />
        ))}
      </section>

      <ErrorList run={run} />

      {confirmCancel && (
        <Modal
          title={t("cancel.title")}
          onClose={() => setConfirmCancel(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirmCancel(false)}>
                {t("cancel.continue")}
              </button>
              <button
                className="btn danger"
                onClick={() => {
                  setConfirmCancel(false);
                  api.runCancel();
                }}
              >
                {t("cancel.confirm")}
              </button>
            </>
          }
        >
          <p>{t("cancel.body")}</p>
        </Modal>
      )}
    </div>
  );
}
