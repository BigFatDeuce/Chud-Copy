import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, FolderOpen, Search, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { liveRun } from "../lib/machines";
import { Modal, tintStyle } from "../components/ui";
import { ErrorList, LinkProgress } from "./Run";
import type { LogPage, RunRecord } from "../lib/types";

const LOG_PAGE = 200;

function Journal({ runId }: { runId: string }) {
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<LogPage | null>(null);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setQuery(search);
      setOffset(0);
    }, 220);
    return () => window.clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    api.logRead(runId, query, offset, LOG_PAGE).then(setPage).catch(() => {});
  }, [runId, query, offset]);

  return (
    <section className="panel journal">
      <div className="journal-head">
        <h2>{t("journal.title")}</h2>
        <div className="search">
          <Search size={15} />
          <input value={search} placeholder={t("journal.search")} aria-label={t("journal.search")} onChange={(e) => setSearch(e.target.value)} />
        </div>
        <button className="icon-btn labelled" onClick={() => api.logReveal(runId)}>
          <FolderOpen size={15} /> {t("journal.open")}
        </button>
      </div>
      <pre className="log">{page?.lines.join("\n") || t("journal.empty")}</pre>
      {page && page.total > LOG_PAGE && (
        <div className="pager">
          <button className="btn ghost small" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - LOG_PAGE))}>
            {t("journal.previous")}
          </button>
          <span>{t("journal.range", { a: fmt.count(offset + 1), b: fmt.count(Math.min(page.total, offset + LOG_PAGE)), c: fmt.count(page.total) })}</span>
          <button className="btn ghost small" disabled={offset + LOG_PAGE >= page.total} onClick={() => setOffset(offset + LOG_PAGE)}>
            {t("journal.next")}
          </button>
        </div>
      )}
    </section>
  );
}

function Report({ runId }: { runId: string }) {
  const { go, toast, colors } = useApp();
  const [run, setRun] = useState<RunRecord | null>(null);
  const [confirm, setConfirm] = useState(false);

  useEffect(() => {
    api.historyGet(runId).then(setRun).catch((e) => toast(describe(String(e)), "bad"));
  }, [runId, toast]);

  if (!run) return null;
  return (
    <div className="view report">
      <header className="view-head">
        <div>
          <button className="crumb" onClick={() => go({ name: "history" })}>
            <ArrowLeft size={15} /> {t("history.title")}
          </button>
          <h1>{run.taskName}</h1>
          <p>
            <span className={`status-tag ${fmt.statusTone(run.status)}`}>{fmt.statusLabel(run.status)}</span> {fmt.date(run.startedAt)} · {fmt.modeLabel(run.mode)}
          </p>
        </div>
        <button className="icon-btn labelled" onClick={() => setConfirm(true)}>
          <Trash2 size={15} /> {t("report.erase")}
        </button>
      </header>

      <dl className={colors[run.taskId] ? "metrics panel tinted" : "metrics panel"} style={tintStyle(colors[run.taskId])}>
        <div>
          <dt>{t("metric.transferred")}</dt>
          <dd>{fmt.bytes(run.totals.bytes)}</dd>
        </div>
        <div>
          <dt>{t("report.copied")}</dt>
          <dd>{fmt.count(run.totals.transfers)}</dd>
        </div>
        <div>
          <dt>{t("report.deleted")}</dt>
          <dd>{fmt.count(run.totals.deletes)}</dd>
        </div>
        <div>
          <dt>{t("report.errors")}</dt>
          <dd>{fmt.count(run.errorsTotal)}</dd>
        </div>
        <div>
          <dt>{t("report.duration")}</dt>
          <dd>{fmt.duration(run.elapsed)}</dd>
        </div>
        <div>
          <dt>{t("metric.average")}</dt>
          <dd>{fmt.speed(run.totals.averageSpeed)}</dd>
        </div>
      </dl>

      <section className="run-links">
        {run.links.map((link) => (
          <LinkProgress key={link.id} link={link} live={false} archive={run.mode === "mirrorArchive"} />
        ))}
      </section>

      <ErrorList run={run} />
      <Journal runId={run.id} />

      {confirm && (
        <Modal
          title={t("report.eraseTitle")}
          onClose={() => setConfirm(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirm(false)}>
                {t("common.keep")}
              </button>
              <button
                className="btn danger"
                onClick={async () => {
                  setConfirm(false);
                  try {
                    await api.historyDelete(run.id);
                    go({ name: "history" });
                  } catch (error) {
                    toast(describe(String(error)), "bad");
                  }
                }}
              >
                {t("report.eraseConfirm")}
              </button>
            </>
          }
        >
          <p>{t("report.eraseBody")}</p>
        </Modal>
      )}
    </div>
  );
}

export function History({ runId }: { runId?: string }) {
  const { go, toast, run, setRun, colors } = useApp();
  const [runs, setRuns] = useState<RunRecord[] | null>(null);
  const [clearing, setClearing] = useState(false);

  const load = useCallback(() => {
    api.historyList().then(setRuns).catch(() => {});
  }, []);

  useEffect(load, [load]);

  if (runId) return <Report runId={runId} />;

  // L'opération en cours garde son rapport : il n'est ni compté ni effacé.
  const live = liveRun(run);
  const erasable = runs?.filter((entry) => entry.id !== live?.id).length ?? 0;

  const clear = async () => {
    setClearing(false);
    try {
      const removed = await api.historyClear();
      // Le rapport de la dernière opération n'existe plus : l'écran d'exécution ne doit plus y mener.
      if (!live) setRun(null);
      toast(t("history.cleared", { n: fmt.count(removed) }));
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
    load();
  };

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t("history.title")}</h1>
          <p>{t("history.lead")}</p>
        </div>
        {erasable > 0 && (
          <button className="icon-btn labelled" onClick={() => setClearing(true)}>
            <Trash2 size={15} /> {t("history.clear")}
          </button>
        )}
      </header>
      {runs?.length === 0 && (
        <div className="empty">
          <p>{t("history.empty")}</p>
        </div>
      )}
      <ul className="history-list">
        {runs?.map((entry) => {
          return (
            <li key={entry.id} className={colors[entry.taskId] ? "tinted-row" : undefined} style={tintStyle(colors[entry.taskId])}>
              <button onClick={() => go({ name: "history", runId: entry.id })}>
                <i className={`dot ${fmt.statusTone(entry.status)}`} />
                <span className="history-name">
                  {entry.taskName}
                  <small>{fmt.modeLabel(entry.mode)}</small>
                </span>
                <span className="history-status">{fmt.statusLabel(entry.status)}</span>
                <span className="history-num">{fmt.bytes(entry.totals.bytes)}</span>
                <span className="history-num">{t("history.files", { n: fmt.count(entry.totals.transfers) })}</span>
                <span className="history-num">{fmt.duration(entry.elapsed)}</span>
                <span className="history-date">{fmt.date(entry.startedAt)}</span>
              </button>
            </li>
          );
        })}
      </ul>

      {clearing && (
        <Modal
          tone="danger"
          title={t("history.clearTitle")}
          onClose={() => setClearing(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setClearing(false)}>
                {t("common.keep")}
              </button>
              <button className="btn danger" onClick={clear}>
                <Trash2 size={15} /> {t("history.clearConfirm")}
              </button>
            </>
          }
        >
          <p>{t("history.clearBody", { n: fmt.count(erasable) })}</p>
        </Modal>
      )}
    </div>
  );
}
