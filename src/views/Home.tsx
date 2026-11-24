import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { AlarmClock, AlertTriangle, Copy, Download, FolderInput, GripVertical, ListPlus, MoreHorizontal, Pencil, Play, Plus, ScanSearch, ShieldAlert, Sparkles, Trash2, Upload } from "lucide-react";
import { api, pickExportFile, pickImportFile } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { analysisProgress, launchChoice, liveRun } from "../lib/machines";
import { ColorSwatches, DiskRow, Modal, RouteGlyph, tintStyle } from "../components/ui";
import { Blaze, heatOf } from "../effects/Blaze";
import { QueuePanel } from "./Queue";
import { PowerToggle } from "../components/power";
import type { BrokenFile, LastRun, Report, RunRecord, Task } from "../lib/types";

function missingPaths(report?: Report): Set<string> {
  const out = new Set<string>();
  for (const link of report?.links ?? []) {
    if (["missing", "unresponsive", "denied"].includes(link.sourceState)) out.add(link.source);
    if (["missing", "unresponsive", "denied"].includes(link.targetState)) out.add(link.target);
  }
  return out;
}

const DAY = 24 * 3600 * 1000;

/** Jours de retard sur le rappel choisi, ou 0. Une tâche jamais lancée compte depuis sa création. */
function overdueDays(task: Task, last?: LastRun): number {
  if (!task.reminderDays) return 0;
  const since = Math.floor((Date.now() - (last?.startedAt ?? task.createdAt)) / DAY);
  return since >= task.reminderDays ? since : 0;
}

let reminded = false;

/** Tâches dans l'ordre choisi ; les nouvelles, absentes de cet ordre, viennent en tête. */
function arrange(tasks: Task[], saved: string[] | undefined): Task[] {
  const order = saved ?? [];
  const known = order.map((id) => tasks.find((task) => task.id === id)).filter((task): task is Task => !!task);
  return [...tasks.filter((task) => !order.includes(task.id)), ...known];
}

interface Handle {
  onPointerDown(event: ReactPointerEvent<HTMLButtonElement>): void;
  onKeyDown(event: KeyboardEvent<HTMLButtonElement>): void;
}

interface CardProps {
  task: Task;
  last?: LastRun;
  onChanged: () => void;
  handle: Handle;
  cardRef: (element: HTMLElement | null) => void;
  dragging: boolean;
}

function TaskCard({ task, last, onChanged, handle, cardRef, dragging }: CardProps) {
  const { go, toast, run, launch, queue, analyses, startAnalysis, unseen } = useApp();
  const [report, setReport] = useState<Report>();
  const [menu, setMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Seule la validation de la version actuelle de la tâche s'affiche, même si une plus ancienne,
    // ralentie par un partage réseau, répond après elle.
    let current = true;
    api
      .taskValidate(task)
      .then((report) => current && setReport(report))
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [task]);

  useEffect(() => {
    if (!menu) return;
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setMenu(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [menu]);

  const danger = fmt.isDestructive(task.mode);
  const missing = missingPaths(report);
  const unavailable = report ? report.links.filter((l) => !l.ready).length : 0;
  const blocking = report?.issues.some((i) => i.severity === "error");
  const driveAlert = report?.issues.find((i) => i.code.startsWith("drive-changed"));
  const choice = launchChoice(run, queue, task.id);
  const busy = choice !== "start";
  const live = liveRun(run)?.taskId === task.id;
  // Une analyse déjà ouverte n'est plus proposée : le bouton en relance une.
  const found = analyses.find((entry) => entry.taskId === task.id);
  const analysis = found && (found.status === "running" || unseen.has(task.id)) ? found : undefined;
  const analyzing = analysis?.status === "running";
  const { compared, toCompare } = analysisProgress(analysis);
  const percent = toCompare > 0 ? Math.floor((compared / toCompare) * 100) : null;

  // L'analyse se lance sans quitter l'accueil ; une fois là, le bouton y mène.
  const analyze = () => {
    if (analysis) go({ name: "preview", taskId: task.id, open: true });
    else startAnalysis(task.id);
  };

  const execute = async () => {
    if (danger) {
      go({ name: "preview", taskId: task.id });
      return;
    }
    try {
      if ((await launch(task.id, false)) === "queued") toast(t("queue.added", { name: task.name }));
      else go({ name: "run" });
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const recolor = async (color: string | null) => {
    try {
      await api.taskSave({ ...task, color });
      onChanged();
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  /** Une action du menu qui échoue le dit, au lieu de ne rien faire en silence. */
  const attempt = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const rename = async () => {
    const name = renaming?.trim();
    setRenaming(null);
    if (!name || name === task.name) return;
    try {
      await api.taskSave({ ...task, name });
      onChanged();
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  return (
    <article ref={cardRef} className={`task-card ${task.color ? "tinted" : ""} ${dragging ? "dragging" : ""} ${menu ? "menu-open" : ""}`} style={tintStyle(task.color)}>
      {run && run.taskId === task.id && <Blaze heat={heatOf(run)} color={task.color} active={live} />}
      <header>
        <button className="drag-handle" aria-label={t("task.move", { name: task.name })} title={t("task.moveHint")} onPointerDown={handle.onPointerDown} onKeyDown={handle.onKeyDown}>
          <GripVertical size={16} />
        </button>
        <div className="task-title">
          {renaming === null ? (
            <h3>{task.name}</h3>
          ) : (
            <input className="inline-input" autoFocus value={renaming} aria-label={t("task.nameLabel")} onChange={(e) => setRenaming(e.target.value)} onBlur={rename} onKeyDown={(e) => e.key === "Enter" && rename()} />
          )}
          <span className={danger ? "tag danger" : "tag"}>{fmt.modeLabel(task.mode)}</span>
        </div>
        <div className="menu-anchor" ref={menuRef}>
          <button className="icon-btn" aria-label={t("task.more")} aria-expanded={menu} onClick={() => setMenu(!menu)}>
            <MoreHorizontal size={18} />
          </button>
          {menu && (
            <div className="menu" role="menu">
              <button role="menuitem" onClick={() => go({ name: "editor", task })}>
                <Pencil size={15} /> {t("task.edit")}
              </button>
              <button role="menuitem" onClick={() => { setMenu(false); setRenaming(task.name); }}>
                <FolderInput size={15} /> {t("task.rename")}
              </button>
              <button role="menuitem" onClick={() => { setMenu(false); attempt(async () => { await api.taskDuplicate(task.id); onChanged(); }); }}>
                <Copy size={15} /> {t("task.duplicate")}
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  attempt(async () => {
                    const path = await pickExportFile(task.name);
                    if (!path) return;
                    await api.tasksExport([task.id], path);
                    toast(t("task.exported"));
                  });
                }}
              >
                <Upload size={15} /> {t("task.export")}
              </button>
              <div className="menu-colors" role="group" aria-label={t("color.label")}>
                <ColorSwatches
                  value={task.color}
                  onPick={(color) => {
                    setMenu(false);
                    recolor(color);
                  }}
                />
              </div>
              <button role="menuitem" className="danger" onClick={() => { setMenu(false); setConfirmDelete(true); }}>
                <Trash2 size={15} /> {t("task.delete")}
              </button>
            </div>
          )}
        </div>
      </header>

      {task.description && <p className="task-description">{task.description}</p>}
      <RouteGlyph task={task} missing={missing} />

      <footer>
        <div className="task-status">
          {driveAlert ? (
            <button className="note bad drive-alert" title={t(`issue.${driveAlert.code}`, driveAlert.params)} onClick={() => go({ name: "tools", tool: "drives" })}>
              <ShieldAlert size={14} /> {t("drives.cardAlert")}
            </button>
          ) : overdueDays(task, last) > 0 ? (
            <span className="note warn overdue">
              <AlarmClock size={14} /> {t("reminder.overdue", { n: overdueDays(task, last) })}
            </span>
          ) : unavailable > 0 ? (
            <span className="note warn">
              <AlertTriangle size={14} /> {t("task.unavailable", { n: unavailable })}
            </span>
          ) : last ? (
            <span className="note">
              <i className={`dot ${fmt.statusTone(last.status)}`} /> {fmt.statusLabel(last.status)} {fmt.ago(last.startedAt)}
            </span>
          ) : (
            <span className="note">{t("task.never")}</span>
          )}
        </div>
        <button className={analyzing ? "btn ghost small analyzing" : analysis ? "btn ghost small analyzed" : "btn ghost small"} disabled={blocking} onClick={analyze} title={analysis && !analyzing ? t("task.seeAnalysis") : undefined}>
          <ScanSearch size={15} /> {analyzing ? (percent === null ? t("task.analyzing") : `${percent} %`) : analysis ? t("task.seeAnalysis") : t("task.analyze")}
        </button>
        <button className={danger ? "btn small warn-solid" : "btn primary small"} disabled={blocking || !report?.runnable || choice === "queued"} onClick={execute}>
          {busy ? <ListPlus size={14} /> : <Play size={14} />} {t(choice === "queued" ? "queue.inQueue" : choice === "queue" ? "queue.add" : "task.run")}
        </button>
      </footer>

      {confirmDelete && (
        <Modal
          title={t("task.deleteTitle")}
          onClose={() => setConfirmDelete(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirmDelete(false)}>
                {t("common.keep")}
              </button>
              <button
                className="btn danger"
                onClick={() => {
                  setConfirmDelete(false);
                  attempt(async () => {
                    await api.taskDelete(task.id);
                    onChanged();
                  });
                }}
              >
                {t("task.deleteConfirm")}
              </button>
            </>
          }
        >
          <p>{t("task.deleteBody", { name: task.name })}</p>
        </Modal>
      )}
    </article>
  );
}

export function Home() {
  const { go, disks, toast, settings, saveSettings, rememberTasks } = useApp();
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [broken, setBroken] = useState<BrokenFile[]>([]);
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [lastRuns, setLastRuns] = useState<Record<string, LastRun>>({});
  const [order, setOrder] = useState<string[] | null>(null);
  const [dragged, setDragged] = useState<string | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const cards = useRef(new Map<string, HTMLElement>());
  const slots = useRef(new Map<string, { x: number; y: number }>());
  const pointer = useRef({ x: 0, y: 0, grabX: 0, grabY: 0 });

  const ordered = useMemo(() => {
    const base = arrange(tasks ?? [], settings.taskOrder);
    if (!order) return base;
    return order.map((id) => base.find((task) => task.id === id)).filter((task): task is Task => !!task);
  }, [tasks, settings.taskOrder, order]);
  const ids = ordered.map((task) => task.id).join();

  const load = useCallback(async () => {
    let list;
    try {
      list = await api.tasksList();
    } catch (error) {
      toast(describe(String(error)), "bad");
      return;
    }
    rememberTasks(list.tasks);
    setTasks(list.tasks);
    setBroken(list.broken);
    // L'historique se lit en parallèle, une fois les tâches affichées.
    const [runs, last] = await Promise.all([
      api.historyList().catch(() => [] as Awaited<ReturnType<typeof api.historyList>>),
      api.tasksLastRuns().catch(() => ({}) as Awaited<ReturnType<typeof api.tasksLastRuns>>),
    ]);
    setRuns(runs);
    setLastRuns(last);
    // Un rappel par ouverture de l'application : la tâche n'est jamais lancée à la place de l'utilisateur.
    const late = list.tasks.filter((task) => overdueDays(task, last[task.id]) > 0);
    if (!reminded && late.length) toast(t("reminder.toast", { n: late.length, names: late.map((task) => task.name).join(", ") }));
    reminded = true;
  }, [rememberTasks, toast]);

  /** La carte saisie suit le pointeur, à partir de sa place dans la grille. */
  const follow = (card: HTMLElement) => {
    const box = grid.current!.getBoundingClientRect();
    const { x, y, grabX, grabY } = pointer.current;
    card.style.transition = "none";
    card.style.transform = `translate(${x - box.left - grabX - card.offsetLeft}px, ${y - box.top - grabY - card.offsetTop}px)`;
  };

  // Animation FLIP : chaque carte part de son ancienne place et glisse jusqu'à la nouvelle.
  useLayoutEffect(() => {
    for (const [id, card] of cards.current) {
      const now = { x: card.offsetLeft, y: card.offsetTop };
      const before = slots.current.get(id);
      slots.current.set(id, now);
      if (id === dragged) {
        follow(card);
        continue;
      }
      if (!before || (before.x === now.x && before.y === now.y)) continue;
      card.style.transition = "none";
      card.style.transform = `translate(${before.x - now.x}px, ${before.y - now.y}px)`;
      requestAnimationFrame(() => {
        card.style.transition = "transform 0.38s cubic-bezier(0.2, 0.8, 0.2, 1)";
        card.style.transform = "";
      });
    }
  }, [ids, dragged]);

  const saveOrder = (next: string[]) => saveSettings({ ...settings, taskOrder: next });

  const handle = (id: string): Handle => ({
    onPointerDown(event) {
      const card = cards.current.get(id);
      if (!card || !grid.current || event.button !== 0) return;
      event.preventDefault();
      // Réordonner déplace les cartes dans la page, ce qui ferait perdre une capture du pointeur : on écoute la fenêtre.

      const box = grid.current.getBoundingClientRect();
      pointer.current = { x: event.clientX, y: event.clientY, grabX: event.clientX - box.left - card.offsetLeft, grabY: event.clientY - box.top - card.offsetTop };
      // L'ordre en cours de glissement vit ici ; React n'en reçoit que des copies à afficher.
      let list = ordered.map((task) => task.id);
      // Les cartes n'ont pas toutes la même hauteur : après un échange, la grille se recompose
      // et le pointeur peut se retrouver sur une autre carte. Il doit bouger avant un nouvel échange.
      let swappedAt = { x: -1e6, y: -1e6 };
      setOrder(list);
      setDragged(id);
      const onMove = (e: PointerEvent) => {
        pointer.current.x = e.clientX;
        pointer.current.y = e.clientY;
        follow(card);
        const area = grid.current!.getBoundingClientRect();
        const px = e.clientX - area.left;
        const py = e.clientY - area.top;
        if (Math.hypot(e.clientX - swappedAt.x, e.clientY - swappedAt.y) < 28) return;
        for (const [other, element] of cards.current) {
          if (other === id) continue;
          const inside = px >= element.offsetLeft && px <= element.offsetLeft + element.offsetWidth && py >= element.offsetTop && py <= element.offsetTop + element.offsetHeight;
          if (!inside) continue;
          const next = list.filter((x) => x !== id);
          next.splice(list.indexOf(other), 0, id);
          if (next.join() !== list.join()) {
            list = next;
            swappedAt = { x: e.clientX, y: e.clientY };
            setOrder(next);
          }
          break;
        }
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
        card.style.transition = "transform 0.32s cubic-bezier(0.2, 0.8, 0.2, 1)";
        card.style.transform = "";
        setDragged(null);
        saveOrder(list);
        setOrder(null);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    onKeyDown(event) {
      const step = ({ ArrowUp: -1, ArrowLeft: -1, ArrowDown: 1, ArrowRight: 1 } as Record<string, number>)[event.key];
      if (!step) return;
      event.preventDefault();
      const list = ordered.map((task) => task.id);
      const index = list.indexOf(id);
      const next = list.filter((x) => x !== id);
      next.splice(Math.max(0, Math.min(next.length, index + step)), 0, id);
      saveOrder(next);
      requestAnimationFrame(() => cards.current.get(id)?.querySelector<HTMLElement>(".drag-handle")?.focus());
    },
  });

  useEffect(() => {
    load().catch((e) => toast(describe(String(e)), "bad"));
  }, [load, toast]);

  const importTasks = async () => {
    const path = await pickImportFile();
    if (!path) return;
    try {
      const imported = await api.tasksImport(path);
      toast(t("home.imported", { n: imported.length }));
      load();
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const exportAll = async () => {
    if (!tasks?.length) return;
    const path = await pickExportFile(t("home.title"));
    if (!path) return;
    try {
      await api.tasksExport(tasks.map((t) => t.id), path);
      toast(t("home.exported", { n: tasks.length }));
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  return (
    <div className="view home">
      <section className="home-main">
        <header className="view-head">
          <div>
            <h1>{t("home.title")}</h1>
            <p>{tasks?.length ? t("home.lead") : t("home.leadEmpty")}</p>
          </div>
          <div className="head-actions">
            <button className="icon-btn labelled" onClick={importTasks} title={t("home.importTitle")}>
              <Download size={16} /> {t("home.import")}
            </button>
            {!!tasks?.length && (
              <button className="icon-btn labelled" onClick={exportAll} title={t("home.exportTitle")}>
                <Upload size={16} /> {t("home.export")}
              </button>
            )}
            <button className="btn primary" onClick={() => go({ name: "editor" })}>
              <Plus size={16} /> {t("home.new")}
            </button>
          </div>
        </header>

        {broken.map((file) => (
          <div key={file.path} className="issue warning">
            <AlertTriangle size={15} />
            <span>
              {t("home.broken")} <code>{file.path}</code>
            </span>
          </div>
        ))}

        {tasks && tasks.length === 0 && (
          <div className="empty">
            <Sparkles size={26} />
            <h2>{t("home.firstTitle")}</h2>
            <p>{t("home.firstBody")}</p>
            <button className="btn primary" onClick={() => go({ name: "editor" })}>
              <Plus size={16} /> {t("home.create")}
            </button>
          </div>
        )}

        <div className={dragged ? "task-grid sorting" : "task-grid"} ref={grid}>
          {ordered.map((task) => (
            <TaskCard
              key={task.id}
              task={task}
              last={lastRuns[task.id]}
              onChanged={load}
              handle={handle(task.id)}
              dragging={dragged === task.id}
              cardRef={(element) => {
                if (element) cards.current.set(task.id, element);
                else cards.current.delete(task.id);
              }}
            />
          ))}
        </div>
      </section>

      <aside className="home-side">
        <section className="panel">
          <PowerToggle />
        </section>
        <QueuePanel />
        <section className="panel">
          <h2>{t("home.recent")}</h2>
          {runs.length === 0 && <p className="muted">{t("home.noRecent")}</p>}
          <ul className="run-list">
            {runs.slice(0, 6).map((run) => (
              <li key={run.id}>
                <button onClick={() => go({ name: "history", runId: run.id })}>
                  <i className={`dot ${fmt.statusTone(run.status)}`} />
                  <span className="run-name">{run.taskName}</span>
                  <span className="run-meta">
                    {fmt.ago(run.startedAt)} · {fmt.bytes(run.totals.bytes)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
        <section className="panel">
          <h2>{t("home.disks")}</h2>
          <div className="disk-list">
            {disks.map((disk) => (
              <DiskRow key={disk.mount} disk={disk} />
            ))}
          </div>
        </section>
      </aside>
    </div>
  );
}
