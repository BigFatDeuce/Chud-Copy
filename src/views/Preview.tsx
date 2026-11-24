import { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, AlertTriangle, ArrowLeft, Info, ListPlus, Pencil, Play, RefreshCw, Search, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { analysisProgress, launchChoice, planRunnable, planVerdict } from "../lib/machines";
import { Beam, Modal, tintStyle } from "../components/ui";
import { Clouds } from "../effects/Clouds";
import type { Counts, EntryKind, Plan, PlanRow, Task } from "../lib/types";

const ROW_HEIGHT = 40;
const PAGE = 200;

function Tile({ label, value, sub, tone, active, onClick }: { label: string; value: number; sub?: string; tone?: string; active?: boolean; onClick?: () => void }) {
  return (
    <button className={`tile ${tone ?? ""} ${active ? "active" : ""}`} onClick={onClick} disabled={value === 0} aria-pressed={active}>
      <span className="tile-value">{fmt.count(value)}</span>
      <span className="tile-label">{label}</span>
      {sub && <span className="tile-sub">{sub}</span>}
    </button>
  );
}

function FileList({ taskId, linkId, kinds, search }: { taskId: string; linkId: string | null; kinds: EntryKind[]; search: string }) {
  const [total, setTotal] = useState(0);
  const [rows, setRows] = useState<Map<number, PlanRow>>(new Map());
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const box = useRef<HTMLDivElement>(null);
  const loaded = useRef(new Set<number>());
  const generation = useRef(0);

  const loadPage = useCallback(
    async (page: number, reset = false) => {
      if (!reset && loaded.current.has(page)) return;
      loaded.current.add(page);
      const stamp = generation.current;
      let result;
      try {
        result = await api.planQuery(taskId, { linkId, kinds, search, offset: page * PAGE, limit: PAGE });
      } catch {
        // La page sera redemandée au prochain défilement plutôt que de rester vide.
        if (stamp === generation.current) loaded.current.delete(page);
        return;
      }
      if (stamp !== generation.current) return;
      setTotal(result.total);
      setRows((current) => {
        const next = reset ? new Map<number, PlanRow>() : new Map(current);
        result.rows.forEach((row, i) => next.set(page * PAGE + i, row));
        return next;
      });
    },
    [taskId, linkId, kinds, search],
  );

  useEffect(() => {
    generation.current += 1;
    loaded.current = new Set([0]);
    box.current?.scrollTo({ top: 0 });
    setScrollTop(0);
    loadPage(0, true);
  }, [loadPage]);

  useEffect(() => {
    if (!box.current) return;
    const observer = new ResizeObserver(([entry]) => setHeight(entry.contentRect.height));
    observer.observe(box.current);
    return () => observer.disconnect();
  }, []);

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 6);
  const last = Math.min(total, Math.ceil((scrollTop + height) / ROW_HEIGHT) + 6);

  useEffect(() => {
    for (let page = Math.floor(first / PAGE); page <= Math.floor(last / PAGE); page += 1) loadPage(page);
  }, [first, last, loadPage]);

  const visible = [];
  for (let index = first; index < last; index += 1) {
    const row = rows.get(index);
    visible.push(
      <div key={index} className={`file-row ${row?.kind ?? ""}`} style={{ transform: `translateY(${index * ROW_HEIGHT}px)` }}>
        {row && (
          <>
            <span className={`kind ${row.kind}`}>{t(`kind.${row.kind}`)}</span>
            <span className="file-path" title={row.note ? `${row.rel}\n${describe(row.note)}` : row.rel}>
              {row.rel}
              {row.note && <small>{describe(row.note)}</small>}
            </span>
            <span className="file-size">{row.size !== null ? fmt.bytes(row.size) : row.source ? fmt.bytes(row.source.size) : row.target ? fmt.bytes(row.target.size) : ""}</span>
            <span className="file-side">{row.source?.modified ? fmt.date(row.source.modified) : "—"}</span>
            <span className="file-side">{row.target?.modified ? fmt.date(row.target.modified) : "—"}</span>
          </>
        )}
      </div>,
    );
  }

  return (
    <div className="file-table">
      <div className="file-head">
        <span>{t("table.state")}</span>
        <span>{t("table.file")}</span>
        <span>{t("table.size")}</span>
        <span>{t("table.sourceDate")}</span>
        <span>{t("table.targetDate")}</span>
      </div>
      <div className="file-scroll" ref={box} onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}>
        <div className="file-canvas" style={{ height: total * ROW_HEIGHT }}>
          {visible}
        </div>
        {total === 0 && <p className="muted file-none">{t("table.none")}</p>}
      </div>
    </div>
  );
}

function ConfirmDestruction({ plan, task, onClose, onConfirm }: { plan: Plan; task: Task; onClose: () => void; onConfirm: () => void }) {
  const [accepted, setAccepted] = useState(false);
  const [word, setWord] = useState("");
  const severe = plan.links.some((l) => l.massiveDelete || l.sourceLooksEmpty);
  const archive = task.mode === "mirrorArchive";
  const ready = accepted && (!severe || word.trim().toUpperCase() === t("confirm.word").toUpperCase());
  return (
    <Modal
      tone="danger"
      title={archive ? t("confirm.titleArchive") : t("confirm.titleDelete")}
      onClose={onClose}
      actions={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn danger" disabled={!ready} onClick={onConfirm}>
            <Trash2 size={15} /> {t(archive ? "confirm.buttonArchive" : "confirm.buttonDelete", { n: fmt.count(plan.totals.delete) })}
          </button>
        </>
      }
    >
      <p className="confirm-lead">{t(archive ? "confirm.leadArchive" : "confirm.leadDelete", { n: fmt.count(plan.totals.delete), size: fmt.bytes(plan.totals.deleteBytes) })}</p>
      <ul className="confirm-list">
        {plan.links
          .filter((l) => l.counts.delete > 0)
          .map((link) => (
            <li key={link.linkId} className={link.massiveDelete || link.sourceLooksEmpty ? "severe" : ""}>
              <code>{link.target}</code>
              <span>{t("confirm.files", { n: fmt.count(link.counts.delete) })}</span>
              {link.sourceLooksEmpty && <em>{t("confirm.empty")}</em>}
              {!link.sourceLooksEmpty && link.massiveDelete && <em>{t("confirm.massive")}</em>}
            </li>
          ))}
      </ul>
      <p className="muted small">{t("confirm.note")}</p>
      <label className="check">
        <input type="checkbox" checked={accepted} onChange={(e) => setAccepted(e.target.checked)} />
        <span>{archive ? t("confirm.acceptArchive") : t("confirm.acceptDelete")}</span>
      </label>
      {severe && (
        <label className="field">
          <span>
            {t("confirm.type")} <b>{t("confirm.word")}</b>
          </span>
          <input value={word} autoComplete="off" spellCheck={false} onChange={(e) => setWord(e.target.value)} />
        </label>
      )}
    </Modal>
  );
}

export function Preview({ taskId, open }: { taskId: string; open?: boolean }) {
  const { go, toast, launch, run, queue, analyses, startAnalysis, stopAnalysis } = useApp();
  const entry = analyses.find((a) => a.taskId === taskId);
  const [task, setTask] = useState<Task | null>(null);
  const [linkId, setLinkId] = useState<string | null>(null);
  const [kinds, setKinds] = useState<EntryKind[]>([]);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [confirming, setConfirming] = useState(false);
  const launched = useRef(false);

  const scanning = !entry || entry.status === "running";
  const plan = entry?.status === "done" ? entry.plan : null;
  const failure = entry?.status === "failed" ? entry.error : null;

  useEffect(() => {
    api
      .tasksList()
      .then((list) => setTask(list.tasks.find((t) => t.id === taskId) ?? null))
      .catch((error) => toast(describe(String(error)), "bad"));
    if (launched.current) return;
    launched.current = true;
    if (entry?.status === "running" || (open && entry)) return;
    startAnalysis(taskId);
  }, [taskId, open, entry, startAnalysis, toast]);

  useEffect(() => {
    setKinds([]);
    setLinkId(null);
  }, [plan?.id]);

  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(search), 200);
    return () => window.clearTimeout(timer);
  }, [search]);

  const toggleKinds = (next: EntryKind[]) => {
    setKinds((current) => (current.join() === next.join() ? [] : next));
  };

  const start = async () => {
    setConfirming(false);
    try {
      if ((await launch(taskId, true)) === "queued") toast(t("queue.added", { name: task?.name ?? "" }));
      go({ name: "run" });
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const { scanned, compared, toCompare, checking: comparing } = analysisProgress(entry);
  const totals: Counts | undefined = linkId ? plan?.links.find((l) => l.linkId === linkId)?.counts : plan?.totals;
  const choice = launchChoice(run, queue, taskId);
  const busy = choice !== "start";
  const destructive = task ? fmt.isDestructive(task.mode) : false;
  // Le mode vient de la tâche ; tant qu'elle se charge, de l'analyse elle-même.
  const verdict = plan && entry ? planVerdict(plan, task?.mode ?? entry.mode) : null;
  const canRun = !!verdict && planRunnable(verdict) && choice !== "queued";
  const issues = (plan?.issues ?? []).filter((i) => i.severity !== "info");

  return (
    <div className="view preview">
      <header className="view-head">
        <div>
          <button className="crumb" onClick={() => go({ name: "home" })}>
            <ArrowLeft size={15} /> {t("home.title")}
          </button>
          <h1>{task?.name ?? t("preview.title")}</h1>
          <p>
            {task ? fmt.modeLabel(task.mode) : ""}
            {plan ? ` · ${t("preview.analyzed", { when: fmt.ago(plan.createdAt) })}` : ""}
            <span className="safe-note"> · {t("preview.safe")}</span>
          </p>
        </div>
        <div className="head-actions">
          {task && (
            <button className="btn ghost" onClick={() => go({ name: "editor", task })}>
              <Pencil size={15} /> {t("task.edit")}
            </button>
          )}
          <button className="btn ghost" disabled={scanning} onClick={() => startAnalysis(taskId)}>
            <RefreshCw size={15} /> {t("preview.rerun")}
          </button>
        </div>
      </header>

      {scanning && (
        <section className={task?.color ? "scan panel tinted" : "scan panel"} style={tintStyle(task?.color)}>
          <Clouds color={task?.color} />
          <h2>{t("preview.scanning")}</h2>
          <Beam value={toCompare > 0 ? compared / toCompare : 0} indeterminate={toCompare === 0} />
          <p>{t("preview.scanned", { n: fmt.count(scanned) })}</p>
          {toCompare > 0 && <p>{t("preview.comparing", { done: fmt.count(compared), total: fmt.count(toCompare) })}</p>}
          {comparing.length > 0 && (
            <>
              <ul className="scan-files">
                {comparing.slice(0, 4).map((name) => (
                  <li key={name} title={name}>
                    {name}
                  </li>
                ))}
              </ul>
              <p className="muted small">{t("preview.compareHint")}</p>
            </>
          )}
          <button className="btn ghost small" onClick={() => stopAnalysis(taskId)}>
            {t("preview.stop")}
          </button>
        </section>
      )}

      {failure && (
        <div className="issue error">
          <AlertCircle size={15} />
          <span>{failure}</span>
        </div>
      )}

      {plan && totals && (
        <>
          <section className={task?.color ? "tiles tinted-tiles" : "tiles"} style={tintStyle(task?.color)}>
            <Tile label={t("tile.copy")} value={totals.copy} sub={fmt.bytes(totals.copyBytes)} tone="copy" active={kinds.join() === "copy"} onClick={() => toggleKinds(["copy"])} />
            <Tile label={t("tile.update")} value={totals.update} sub={fmt.bytes(totals.updateBytes)} tone="update" active={kinds.join() === "update"} onClick={() => toggleKinds(["update"])} />
            <Tile label={t("tile.same")} value={totals.same} active={kinds.join() === "same"} onClick={() => toggleKinds(["same"])} />
            <Tile label={t("tile.ignored")} value={totals.ignored} sub={totals.ignored ? fmt.bytes(totals.ignoredBytes) : undefined} active={kinds.join() === "ignored"} onClick={() => toggleKinds(["ignored"])} />
            {destructive ? (
              <Tile label={task?.mode === "mirrorArchive" ? t("tile.archive") : t("tile.delete")} value={totals.delete} sub={totals.delete ? fmt.bytes(totals.deleteBytes) : undefined} tone="delete" active={kinds.join() === "delete"} onClick={() => toggleKinds(["delete"])} />
            ) : (
              <Tile label={t("tile.extra")} value={totals.extra} active={kinds.join() === "extra"} onClick={() => toggleKinds(["extra"])} />
            )}
            <Tile label={t("tile.problems")} value={totals.errors + totals.conflicts + totals.incomplete} tone={totals.errors + totals.conflicts > 0 ? "bad" : undefined} active={kinds.join() === "error,conflict,incomplete"} onClick={() => toggleKinds(["error", "conflict", "incomplete"])} />
          </section>

          {issues.length > 0 && (
            <section className="issues">
              {issues.map((issue, i) => (
                <div key={i} className={`issue ${issue.severity}`}>
                  {issue.severity === "error" ? <AlertCircle size={15} /> : <AlertTriangle size={15} />}
                  <span>{t(`issue.${issue.code}`, issue.params)}</span>
                  {issue.code.startsWith("drive-changed") && (
                    <button className="btn ghost small" onClick={() => go({ name: "tools", tool: "drives" })}>
                      {t("drives.open")}
                    </button>
                  )}
                </div>
              ))}
            </section>
          )}

          {plan.links
            .filter((l) => l.skipReason)
            .map((link) => (
              <div key={link.linkId} className="issue warning">
                <Info size={15} />
                <span>{t("preview.skipped", { reason: describe(link.skipReason) })}</span>
              </div>
            ))}

          <div className="preview-body">
            <aside className="preview-side">
              <section className="panel">
                <h2>{t("preview.space")}</h2>
                {plan.space.length === 0 && <p className="muted small">{t("preview.noSpace")}</p>}
                {plan.space.map((check) => (
                  <div key={check.mount} className="space-row">
                    <div className="disk-line">
                      <span className="disk-name">{check.mount}</span>
                      <span className={check.enough ? "disk-free" : "disk-free bad"}>
                        {fmt.bytes(check.needed)} / {fmt.bytes(check.available)}
                      </span>
                    </div>
                    <Beam value={check.available > 0 ? check.needed / check.available : 1} tone={check.enough ? undefined : "bad"} />
                  </div>
                ))}
                <p className="space-total">{t("preview.total", { size: fmt.bytes(plan.totals.copyBytes + plan.totals.updateBytes), n: fmt.count(plan.totals.copy + plan.totals.update) })}</p>
              </section>
              {plan.links.length > 1 && (
                <section className="panel">
                  <h2>{t("preview.links")}</h2>
                  <div className="link-tabs">
                    <button className={linkId === null ? "active" : ""} onClick={() => setLinkId(null)}>
                      {t("preview.all")}
                    </button>
                    {plan.links.map((link) => (
                      <button key={link.linkId} className={linkId === link.linkId ? "active" : ""} onClick={() => setLinkId(link.linkId)} title={`${link.source} → ${link.target}`}>
                        <span>
                          {fmt.shortPath(link.source)} → {fmt.shortPath(link.target)}
                        </span>
                        {link.counts.delete > 0 && <em>{fmt.count(link.counts.delete)}</em>}
                      </button>
                    ))}
                  </div>
                </section>
              )}
            </aside>

            <section className="preview-files panel">
              <div className="search">
                <Search size={15} />
                <input value={search} placeholder={t("preview.search")} aria-label={t("preview.search")} onChange={(e) => setSearch(e.target.value)} />
              </div>
              <FileList taskId={taskId} linkId={linkId} kinds={kinds} search={query} />
            </section>
          </div>

          <footer className="preview-foot">
            <p>{verdict && t(`preview.${verdict}`, { n: fmt.count(plan.totals.copy + plan.totals.update), d: fmt.count(plan.totals.delete) })}</p>
            <button
              className={plan.totals.delete > 0 ? "btn danger" : "btn primary"}
              disabled={!canRun}
              onClick={() => (plan.totals.delete > 0 ? setConfirming(true) : start())}
            >
              {busy ? <ListPlus size={15} /> : <Play size={15} />} {t(busy ? (plan.totals.delete > 0 ? "queue.review" : "queue.add") : plan.totals.delete > 0 ? "preview.review" : "preview.start")}
            </button>
          </footer>
        </>
      )}

      {confirming && plan && task && <ConfirmDestruction plan={plan} task={task} onClose={() => setConfirming(false)} onConfirm={start} />}
    </div>
  );
}
