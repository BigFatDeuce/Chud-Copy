import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftRight, ChevronRight, Copy, Fingerprint, FolderOpen, Gauge, History, Layers, Network, Play, PieChart, Plus, RefreshCw, Search, ShieldCheck, Sparkles, Trash2, X } from "lucide-react";
import { api, pickFolders } from "../lib/api";
import { useApp, type ToolName } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { isJobTool, jobIsRunning } from "../lib/machines";
import { joinTarget } from "../lib/tasks";
import { Modal, Select, Toggle } from "../components/ui";
import { Frame, JobBar, PathPicker, ResultHead, SpeedGraph, useJob } from "../components/tools";
import { CleanupTool, DrivesTool, NetworkTool } from "./ToolsMore";
import type { ArchiveVersion, ArchivedFile, CompareReport, DuplicateGroup, DuplicateReport, ReadReport, SpaceNode } from "../lib/types";

const TOOLS: { id: ToolName; icon: React.ReactNode; hue: string }[] = [
  { id: "compare", icon: <ArrowLeftRight size={22} />, hue: "#4fe3ff" },
  { id: "duplicates", icon: <Copy size={22} />, hue: "#9a6bff" },
  { id: "space", icon: <PieChart size={22} />, hue: "#ffc56b" },
  { id: "read", icon: <Gauge size={22} />, hue: "#3ddc97" },
  { id: "restore", icon: <History size={22} />, hue: "#ff8a3d" },
  { id: "cleanup", icon: <Sparkles size={22} />, hue: "#ff4f82" },
  { id: "network", icon: <Network size={22} />, hue: "#6be0ff" },
  { id: "drives", icon: <Fingerprint size={22} />, hue: "#c9a6ff" },
];

/** Accueil des outils : à quoi sert chacun, en une phrase, avant même de l'ouvrir. */
function Hub() {
  const { go, keep } = useApp();
  const running = (id: ToolName) => (isJobTool(id) && jobIsRunning(keep, id) ? "running" : "");
  return (
    <div className="view tool">
      <header className="view-head">
        <div>
          <h1>{t("nav.tools")}</h1>
          <p>{t("tools.lead")}</p>
        </div>
      </header>
      <div className="tool-grid">
        {TOOLS.map((tool, i) => (
          <button key={tool.id} className={`tool-card ${running(tool.id)}`} style={{ "--hue": tool.hue, animationDelay: `${i * 60}ms` } as React.CSSProperties} onClick={() => go({ name: "tools", tool: tool.id })}>
            <span className="tool-orb">{tool.icon}</span>
            <span className="tool-name">{t(`tool.${tool.id}`)}</span>
            <span className="tool-what">{t(`tool.${tool.id}.what`)}</span>
            <span className="tool-when">{t(`tool.${tool.id}.when`)}</span>
          </button>
        ))}
        <button className="tool-card" style={{ "--hue": "#3a7bff", animationDelay: `${TOOLS.length * 60}ms` } as React.CSSProperties} onClick={() => go({ name: "integrity" })}>
          <span className="tool-orb">
            <ShieldCheck size={22} />
          </span>
          <span className="tool-name">{t("nav.integrity")}</span>
          <span className="tool-what">{t("tool.integrity.what")}</span>
          <span className="tool-when">{t("tool.integrity.when")}</span>
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- Comparer

function CompareTool() {
  const { keep } = useApp();
  const [form, setForm] = keep("form:compare", { left: null as string | null, right: null as string | null, content: false });
  const job = useJob<CompareReport>("compare");
  const [filter, setFilter] = useState<"left" | "right" | "different" | null>(null);
  const report = job.result;
  const rows = !report
    ? []
    : [
        ...(filter === null || filter === "different" ? report.different.map((d) => ({ kind: "different", rel: d.rel, note: t(`compare.reason.${d.reason}`) })) : []),
        ...(filter === null || filter === "left" ? report.onlyLeft.map((s) => ({ kind: "left", rel: s.rel, note: fmt.bytes(s.size) })) : []),
        ...(filter === null || filter === "right" ? report.onlyRight.map((s) => ({ kind: "right", rel: s.rel, note: fmt.bytes(s.size) })) : []),
      ];
  return (
    <Frame id="compare">
      <section className="panel">
        <div className="compare-sides">
          <PathPicker label={t("compare.leftLabel")} value={form.left} onPick={(left) => setForm((f) => ({ ...f, left }))} disabled={job.running} />
          <PathPicker label={t("compare.rightLabel")} value={form.right} onPick={(right) => setForm((f) => ({ ...f, right }))} disabled={job.running} />
        </div>
        <Toggle checked={form.content} onChange={(content) => setForm((f) => ({ ...f, content }))} label={t("compare.content")} hint={t("compare.contentHint")} />
        <div className="tool-actions">
          <button className="btn primary" disabled={!form.left || !form.right || form.left === form.right || job.running} onClick={() => job.run(() => api.toolCompare("compare", form.left!, form.right!, form.content))}>
            <Play size={15} /> {t("compare.start")}
          </button>
        </div>
        <p className="muted small">{t("tools.readOnly")}</p>
      </section>
      {job.running && <JobBar title={t("compare.running")} progress={job.progress} onStop={job.stop} />}
      {report && (
        <section className="panel">
          <ResultHead title={report.cancelled ? t("integrity.stopped") : report.differentCount + report.onlyLeftCount + report.onlyRightCount === 0 ? t("compare.identical") : t("compare.done")} onDismiss={job.dismiss} />
          <div className="tiles">
            <div className="tile ok">
              <span className="tile-value">{fmt.count(report.same)}</span>
              <span className="tile-label">{t("compare.same")}</span>
            </div>
            {(["different", "left", "right"] as const).map((kind) => {
              const n = kind === "different" ? report.differentCount : kind === "left" ? report.onlyLeftCount : report.onlyRightCount;
              return (
                <button key={kind} className={`tile ${filter === kind ? "active" : ""} ${kind === "different" ? "update" : ""}`} disabled={n === 0} onClick={() => setFilter(filter === kind ? null : kind)}>
                  <span className="tile-value">{fmt.count(n)}</span>
                  <span className="tile-label">{t(`compare.${kind}`)}</span>
                </button>
              );
            })}
          </div>
          {rows.length > 0 && (
            <ul className="finding-list">
              {rows.slice(0, 800).map((row) => (
                <li key={`${row.kind}:${row.rel}`} className={row.kind}>
                  <b>{t(`compare.${row.kind}`)}</b>
                  <code className="file-path">{row.rel}</code>
                  <small>{row.note}</small>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Doublons

const MIN_SIZES = [1024, 1024 * 1024, 10 * 1024 * 1024, 100 * 1024 * 1024];

function DuplicateCard({ group, onTrashed }: { group: DuplicateGroup; onTrashed: (removed: string[]) => void }) {
  const { toast } = useApp();
  const [keepPath, setKeepPath] = useState(group.files[0]);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const remove = group.files.filter((f) => f !== keepPath);
  const trash = async () => {
    setConfirm(false);
    setBusy(true);
    try {
      const out = await api.duplicatesTrash(group, keepPath, remove);
      toast(t("dup.trashed", { n: fmt.count(out.moved.length), size: fmt.bytes(group.size * out.moved.length) }));
      for (const path of out.holding) toast(t("trash.held", { path }));
      onTrashed(out.moved);
    } catch (error) {
      toast(describe(String(error)), "bad");
    } finally {
      setBusy(false);
    }
  };
  return (
    <article className="dup-group">
      <header>
        <b>{fmt.bytes(group.size)}</b>
        <span className="muted small">{t("dup.copies", { n: group.files.length, size: fmt.bytes(group.size * (group.files.length - 1)) })}</span>
        <button className="btn ghost small danger-text" disabled={busy} onClick={() => setConfirm(true)}>
          {busy ? <RefreshCw size={13} className="spin" /> : <Trash2 size={13} />} {busy ? t("trash.working") : t("dup.trash")}
        </button>
      </header>
      <ul>
        {group.files.map((file) => (
          <li key={file} className={file === keepPath ? "kept" : ""}>
            <label>
              <input type="radio" checked={file === keepPath} disabled={busy} onChange={() => setKeepPath(file)} />
              <span className="file-path" title={file}>
                {fmt.ellipsize(file, 110)}
              </span>
            </label>
            {file === keepPath && <em>{t("dup.kept")}</em>}
            <button className="icon-btn" aria-label={t("dup.reveal")} title={t("dup.reveal")} onClick={() => api.pathReveal(file).catch(() => {})}>
              <FolderOpen size={14} />
            </button>
          </li>
        ))}
      </ul>
      {confirm && (
        <Modal
          tone="danger"
          title={t("dup.confirmTitle")}
          onClose={() => setConfirm(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setConfirm(false)}>
                {t("common.cancel")}
              </button>
              <button className="btn danger" onClick={trash}>
                <Trash2 size={15} /> {t("dup.trashConfirm", { n: remove.length })}
              </button>
            </>
          }
        >
          <p>{t("dup.confirmBody")}</p>
          <p className="muted small">{t("trash.holdingNote")}</p>
          <ul className="confirm-list">
            {remove.map((file) => (
              <li key={file}>
                <code>{file}</code>
              </li>
            ))}
          </ul>
          <p className="muted small">{t("dup.keptNote", { path: keepPath })}</p>
        </Modal>
      )}
    </article>
  );
}

function DuplicatesTool() {
  const { keep } = useApp();
  const [form, setForm] = keep("form:duplicates", { roots: [] as string[], minSize: 1024 * 1024 });
  const [adding, setAdding] = useState<string | null>(null);
  const job = useJob<DuplicateReport>("duplicates");
  const [shown, setShown] = useState(60);
  const report = job.result;
  const add = (path: string) => {
    setAdding(null);
    if (!form.roots.includes(path)) setForm({ ...form, roots: [...form.roots, path] });
  };
  const trashed = (removed: string[]) => {
    if (!report) return;
    const gone = new Set(removed);
    const groups = report.groups
      .map((g) => {
        const kept = g.files.map((_, i) => i).filter((i) => !gone.has(g.files[i]));
        return { ...g, files: kept.map((i) => g.files[i]), modified: g.modified && kept.map((i) => g.modified![i]) };
      })
      .filter((g) => g.files.length > 1);
    job.replace({ ...report, groups, wasted: groups.reduce((sum, g) => sum + g.size * (g.files.length - 1), 0) });
  };
  return (
    <Frame id="duplicates">
      <section className="panel">
        <h2>{t("dup.where")}</h2>
        <div className="root-list">
          {form.roots.map((root) => (
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
        <label className="field inline">
          <span>{t("dup.minSize")}</span>
          <Select<number> value={form.minSize} onChange={(minSize) => setForm({ ...form, minSize })} disabled={job.running} choices={MIN_SIZES.map((size) => ({ value: size, label: fmt.bytes(size) }))} />
        </label>
        <div className="tool-actions">
          <button className="btn primary" disabled={form.roots.length === 0 || job.running} onClick={() => job.run(() => api.toolDuplicates("duplicates", form.roots, form.minSize))}>
            <Search size={15} /> {t("dup.start")}
          </button>
        </div>
        <p className="muted small">{t("dup.safe")}</p>
      </section>
      {job.running && <JobBar title={t("dup.running")} progress={job.progress} onStop={job.stop} />}
      {report && (
        <section className="panel">
          <ResultHead title={report.groups.length ? t("dup.found", { n: fmt.count(report.groups.length) }) : t("dup.none")} onDismiss={job.dismiss} />
          {report.groups.length > 0 && (
            <div className="big-figure">
              <span>{fmt.bytes(report.wasted)}</span>
              <small>{t("dup.wasted")}</small>
            </div>
          )}
          <div className="dup-list">
            {report.groups.slice(0, shown).map((group) => (
              <DuplicateCard key={group.md5 + group.size} group={group} onTrashed={trashed} />
            ))}
          </div>
          {report.groups.length > shown && (
            <button className="btn ghost" onClick={() => setShown(shown + 60)}>
              <Plus size={14} /> {t("dup.more", { n: report.groups.length - shown })}
            </button>
          )}
        </section>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Espace disque

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Disposition en carrés (« squarified treemap ») : chaque surface est proportionnelle à la taille. */
function squarify(values: number[], frame: Box): Box[] {
  let { x, y, w, h } = frame;
  const total = values.reduce((a, b) => a + b, 0);
  if (!total) return values.map(() => ({ x, y, w: 0, h: 0 }));
  const areas = values.map((v) => (v / total) * w * h);
  const boxes: Box[] = [];
  let i = 0;
  while (i < areas.length) {
    const side = Math.min(w, h) || 1;
    const worst = (row: number[]) => {
      const sum = row.reduce((a, b) => a + b, 0);
      return Math.max((side * side * Math.max(...row)) / (sum * sum), (sum * sum) / (side * side * Math.min(...row)));
    };
    const row = [areas[i]];
    let j = i + 1;
    while (j < areas.length && worst([...row, areas[j]]) <= worst(row)) row.push(areas[j++]);
    const sum = row.reduce((a, b) => a + b, 0);
    if (w >= h) {
      const width = sum / h;
      let cy = y;
      for (const area of row) {
        boxes.push({ x, y: cy, w: width, h: area / width });
        cy += area / width;
      }
      x += width;
      w -= width;
    } else {
      const height = sum / w;
      let cx = x;
      for (const area of row) {
        boxes.push({ x: cx, y, w: area / height, h: height });
        cx += area / height;
      }
      y += height;
      h -= height;
    }
    i = j;
  }
  return boxes;
}

const SPACE_HUES = ["#4fe3ff", "#9a6bff", "#3a7bff", "#ff4f82", "#ffc56b", "#3ddc97", "#ff8a3d", "#c9d4ff"];
const OTHERS_HUE = "#7d88b5";

/** En dessous de cette surface à l'écran (px²), une case ne se lit plus : elle rejoint la case « autres ». */
const CELL_MIN_AREA = 4200;

/** Une case de la carte : un élément réel, ou le regroupement de ceux trop petits pour se lire. */
interface Cell {
  node: SpaceNode | null;
  size: number;
  hue: string;
  /** Nombre d'éléments regroupés, pour la case « autres ». */
  count: number;
}

/** Nombre d'éléments que représente un nœud : le regroupement du cœur s'appelle « +N ». */
function itemCount(node: SpaceNode): number {
  return node.grouped ? Number(node.name.slice(1)) || 1 : 1;
}

/** Répartit les éléments en cases lisibles sur une surface de `area` px². */
function cellsOf(children: SpaceNode[], area: number): Cell[] {
  const total = children.reduce((sum, c) => sum + c.size, 0);
  const cells: Cell[] = [];
  const small: SpaceNode[] = [];
  children.forEach((node, i) => {
    const cell = { node, size: node.size, hue: SPACE_HUES[i % SPACE_HUES.length], count: 1 };
    // Le regroupement du cœur (« +N ») rejoint toujours la case « autres » : une seule case pour le reste.
    if (node.grouped || (total && (node.size / total) * area < CELL_MIN_AREA)) small.push(node);
    else cells.push(cell);
  });
  // Un élément seul ne gagne rien à être regroupé : il garde sa propre case.
  cells.sort((a, b) => b.size - a.size);
  if (small.length === 1 && !small[0].grouped) cells.push({ node: small[0], size: small[0].size, hue: SPACE_HUES[children.indexOf(small[0]) % SPACE_HUES.length], count: 1 });
  // La case « autres » vient en dernier, au coin de la carte, quelle que soit sa taille.
  else if (small.length > 0) cells.push({ node: null, size: small.reduce((sum, c) => sum + c.size, 0), hue: OTHERS_HUE, count: small.reduce((sum, c) => sum + itemCount(c), 0) });
  return cells;
}

/** Taille réelle d'un élément à l'écran, suivie quand la fenêtre change. */
function useMeasure(): [(element: HTMLElement | null) => void, { w: number; h: number }] {
  const [frame, setFrame] = useState({ w: 0, h: 0 });
  const observer = useRef<ResizeObserver | null>(null);
  const ref = useCallback((element: HTMLElement | null) => {
    observer.current?.disconnect();
    observer.current = null;
    if (!element) return;
    observer.current = new ResizeObserver(([entry]) => setFrame({ w: entry.contentRect.width, h: entry.contentRect.height }));
    observer.current.observe(element);
  }, []);
  return [ref, frame];
}

function SpaceTool() {
  const { keep, toast } = useApp();
  const [form, setForm] = keep("form:space", { root: null as string | null });
  const [path, setPath] = keep<string[]>("form:space:path", []);
  const job = useJob<SpaceNode>("space");
  const [mapRef, frame] = useMeasure();
  const tree = job.result;
  const node = useMemo(() => {
    let current = tree;
    for (const name of path) current = current?.children.find((c) => c.name === name && !c.leaf) ?? current;
    return current;
  }, [tree, path]);
  const children = useMemo(() => node?.children.filter((c) => c.size > 0) ?? [], [node]);
  // La disposition se calcule sur la taille réelle de la carte, en pixels : en pourcentages d'un
  // carré, les cases d'une carte large sortaient étirées en fines bandes illisibles.
  const cells = useMemo(() => cellsOf(children, frame.w * frame.h), [children, frame.w, frame.h]);
  const boxes = squarify(
    cells.map((c) => c.size),
    { x: 0, y: 0, w: frame.w, h: frame.h },
  );
  const hueOf = (child: SpaceNode) => cells.find((c) => c.node === child)?.hue ?? OTHERS_HUE;
  const location = (names: string[]) => names.reduce((at, name) => joinTarget(at, name), tree?.name ?? "");
  const reveal = (names: string[]) => api.pathReveal(location(names)).catch((e) => toast(describe(String(e)), "bad"));
  return (
    <Frame id="space">
      <section className="panel">
        <PathPicker value={form.root} onPick={(root) => setForm({ root })} disabled={job.running} />
        <div className="tool-actions">
          <button
            className="btn primary"
            disabled={!form.root || job.running}
            onClick={() => {
              setPath([]);
              job.run(() => api.toolSpace("space", form.root!));
            }}
          >
            <Layers size={15} /> {t("space.start")}
          </button>
        </div>
        <p className="muted small">{t("tools.readOnly")}</p>
      </section>
      {job.running && <JobBar title={t("space.running")} progress={job.progress} onStop={job.stop} />}
      {tree && node && (
        <section className="panel">
          <ResultHead title={t("space.total", { size: fmt.bytes(tree.size), n: fmt.count(tree.files) })} onDismiss={job.dismiss} />
          <nav className="space-crumbs">
            <button onClick={() => setPath([])}>{fmt.shortPath(tree.name)}</button>
            {path.map((name, i) => (
              <span key={i}>
                <ChevronRight size={13} />
                <button onClick={() => setPath(path.slice(0, i + 1))}>{name}</button>
              </span>
            ))}
            <span className="muted small">· {fmt.bytes(node.size)}</span>
            <button className="icon-btn labelled small space-open" onClick={() => reveal(path)}>
              <FolderOpen size={14} /> {t("space.reveal")}
            </button>
          </nav>
          <div className="treemap" key={path.join("/")} ref={mapRef}>
            {frame.w > 0 &&
              cells.map((cell, i) => {
                const box = boxes[i];
                const child = cell.node;
                const share = node.size ? cell.size / node.size : 0;
                const name = child ? child.name : t("space.others", { n: fmt.count(cell.count) });
                const folder = child && !child.leaf;
                const real = child && !child.grouped;
                return (
                  <button
                    key={child ? `${child.name}:${child.leaf}` : "others"}
                    className={folder ? "tm-cell" : child ? "tm-cell leaf" : "tm-cell leaf others"}
                    style={{ left: box.x, top: box.y, width: box.w, height: box.h, "--hue": cell.hue, animationDelay: `${Math.min(i, 20) * 25}ms` } as React.CSSProperties}
                    title={`${name} — ${fmt.bytes(cell.size)} (${Math.round(share * 100)} %)${real ? `\n${t("space.hint")}` : ""}`}
                    disabled={!folder}
                    onClick={() => folder && setPath([...path, child.name])}
                    onContextMenu={(event) => {
                      if (!real) return;
                      event.preventDefault();
                      reveal([...path, child.name]);
                    }}
                  >
                    {box.w >= 72 && box.h >= 48 ? (
                      <>
                        <span className="tm-name">{name}</span>
                        <span className="tm-size">{fmt.bytes(cell.size)}</span>
                      </>
                    ) : (
                      box.w >= 64 && box.h >= 26 && <span className="tm-size">{fmt.bytes(cell.size)}</span>
                    )}
                  </button>
                );
              })}
          </div>
          <ul className="space-list">
            {children.map((child) => (
              <li key={`${child.name}:${child.leaf}`}>
                <i style={{ background: hueOf(child) }} />
                {child.leaf ? (
                  <span className="file-path" title={child.name}>
                    {child.name}
                  </span>
                ) : (
                  <button className="file-path space-enter" title={child.name} onClick={() => setPath([...path, child.name])}>
                    {child.name}
                  </button>
                )}
                <span className="space-bar">
                  <span style={{ width: `${node.size ? (child.size / node.size) * 100 : 0}%`, background: hueOf(child) }} />
                </span>
                <b>{fmt.bytes(child.size)}</b>
                {child.grouped ? (
                  <span />
                ) : (
                  <button className="icon-btn" aria-label={t("dup.reveal")} title={t("dup.reveal")} onClick={() => reveal([...path, child.name])}>
                    <FolderOpen size={14} />
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Test de lecture

function ReadTool() {
  const { keep } = useApp();
  const [form, setForm] = keep("form:read", { root: null as string | null });
  const [live, setLive] = keep<number[]>("form:read:live", []);
  const job = useJob<ReadReport>("read");
  const last = useRef<{ bytes: number; at: number } | null>(null);

  // Courbe en direct : le débit entre deux nouvelles de progression.
  useEffect(() => {
    const progress = job.progress;
    if (!job.running || !progress || progress.phase !== "read") return;
    const now = performance.now();
    if (last.current && now - last.current.at > 300) {
      const speed = ((progress.bytes - last.current.bytes) / (now - last.current.at)) * 1000;
      if (speed >= 0) setLive((samples) => [...samples.slice(-119), speed]);
    }
    if (!last.current || now - last.current.at > 300) last.current = { bytes: progress.bytes, at: now };
  }, [job.progress, job.running]);

  const report = job.result;
  const problems = report ? report.unreadable.length + report.slow.length : 0;
  return (
    <Frame id="read">
      <section className="panel">
        <PathPicker value={form.root} onPick={(root) => setForm({ root })} disabled={job.running} />
        <div className="tool-actions">
          <button
            className="btn primary"
            disabled={!form.root || job.running}
            onClick={() => {
              setLive([]);
              last.current = null;
              job.run(() => api.toolRead("read", form.root!));
            }}
          >
            <Gauge size={15} /> {t("read.start")}
          </button>
        </div>
        <p className="muted small">{t("read.note")}</p>
      </section>
      {job.running && (
        <>
          <JobBar title={t("read.running")} progress={job.progress} onStop={job.stop} />
          <section className="panel">
            <div className="speed-now">
              <span>{live.length ? fmt.speed(live[live.length - 1]) : "—"}</span>
              <small>{t("read.speed")}</small>
            </div>
            <SpeedGraph samples={live} live />
          </section>
        </>
      )}
      {report && (
        <section className={problems ? "panel verdict bad" : "panel verdict ok"}>
          <ResultHead title={report.cancelled ? t("integrity.stopped") : problems ? t("read.problems", { n: fmt.count(problems) }) : t("read.clean")} onDismiss={job.dismiss} />
          <p className="muted">{t("read.summary", { n: fmt.count(report.files), size: fmt.bytes(report.bytes), speed: fmt.speed(report.averageSpeed), time: fmt.duration(report.elapsed) })}</p>
          <SpeedGraph samples={report.samples} />
          {report.unreadable.length > 0 && (
            <>
              <h3 className="sub">{t("read.unreadable")}</h3>
              <ul className="finding-list">
                {report.unreadable.slice(0, 300).map(([rel, error]) => (
                  <li key={rel} className="unreadable">
                    <b>{t("finding.unreadable")}</b>
                    <code className="file-path">{rel}</code>
                    <small>{error}</small>
                  </li>
                ))}
              </ul>
            </>
          )}
          {report.slow.length > 0 && (
            <>
              <h3 className="sub">{t("read.slow")}</h3>
              <p className="muted small">{t("read.slowHint")}</p>
              <ul className="finding-list">
                {report.slow.slice(0, 300).map((slow) => (
                  <li key={slow.rel} className="missing">
                    <b>{fmt.speed(slow.speed)}</b>
                    <code className="file-path">{slow.rel}</code>
                    <small>{fmt.bytes(slow.size)}</small>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}
    </Frame>
  );
}

// ---------------------------------------------------------------- Restaurer

function RestoreTool() {
  const { keep, toast } = useApp();
  const [target, setTarget] = keep<string | null>("form:restore", null);
  const [targets, setTargets] = useState<{ path: string; task: string }[]>([]);
  const [versions, setVersions] = useState<ArchiveVersion[] | null>(null);
  const [version, setVersion] = useState<ArchiveVersion | null>(null);
  const [files, setFiles] = useState<ArchivedFile[]>([]);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");

  useEffect(() => {
    api.tasksList().then(({ tasks }) => {
      const list: { path: string; task: string }[] = [];
      for (const task of tasks.filter((t) => t.mode === "mirrorArchive")) {
        for (const link of task.links) {
          const destination = task.destinations.find((d) => d.id === link.destinationId);
          if (destination) list.push({ path: joinTarget(destination.path, link.subfolder), task: task.name });
        }
      }
      setTargets(list);
    }).catch((e) => toast(describe(String(e)), "bad"));
  }, [toast]);

  useEffect(() => {
    setVersions(null);
    setVersion(null);
    setFiles([]);
    if (!target) return;
    api.archiveVersions(target).then(setVersions).catch((e) => toast(describe(String(e)), "bad"));
  }, [target, toast]);

  useEffect(() => {
    setChosen(new Set());
    setFiles([]);
    if (!target || !version) return;
    api.archiveFiles(target, version.archive, version.stamp).then(setFiles).catch((e) => toast(describe(String(e)), "bad"));
  }, [target, version, toast]);

  const visible = files.filter((f) => !search || f.rel.toLowerCase().includes(search.toLowerCase()));
  const toggle = (rel: string) => setChosen((set) => {
    const next = new Set(set);
    if (next.has(rel)) next.delete(rel);
    else next.add(rel);
    return next;
  });

  const restore = async () => {
    if (!target || !version || chosen.size === 0) return;
    const [destination] = await pickFolders();
    if (!destination) return;
    try {
      const written = await api.archiveRestore(target, version.archive, version.stamp, [...chosen], destination);
      toast(t("restore.done", { n: fmt.count(written.length) }));
      if (written[0]) api.pathReveal(written[0]).catch(() => {});
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  return (
    <Frame id="restore">
      <section className="panel">
        <h2>{t("restore.where")}</h2>
        {targets.length > 0 && (
          <div className="target-choices">
            {targets.map((item) => (
              <button key={item.path} className={target === item.path ? "chip-choice active" : "chip-choice"} onClick={() => setTarget(item.path)} title={item.path}>
                <History size={14} /> <span>{item.task}</span> <span className="file-path muted">{fmt.shortPath(item.path)}</span>
              </button>
            ))}
          </div>
        )}
        <PathPicker value={target} onPick={setTarget} />
        <p className="muted small">{t("restore.hint")}</p>
      </section>

      {versions && versions.length === 0 && <p className="issue info">{t("restore.none")}</p>}
      {versions && versions.length > 0 && (
        <div className="restore-grid">
          <section className="panel">
            <h2>{t("restore.versions")}</h2>
            <ul className="version-list">
              {versions.map((item) => (
                <li key={item.archive + item.stamp}>
                  <button className={version === item ? "active" : ""} onClick={() => setVersion(item)}>
                    <b>{item.stamp}</b>
                    <small>{t("restore.versionInfo", { n: fmt.count(item.files), size: fmt.bytes(item.bytes) })}</small>
                  </button>
                </li>
              ))}
            </ul>
          </section>
          <section className="panel">
            {version ? (
              <>
                <div className="search">
                  <Search size={15} />
                  <input value={search} placeholder={t("preview.search")} aria-label={t("preview.search")} onChange={(e) => setSearch(e.target.value)} />
                </div>
                <div className="restore-bar">
                  <button className="btn ghost small" onClick={() => setChosen(new Set(visible.map((f) => f.rel)))}>
                    {t("restore.all")}
                  </button>
                  <span className="muted small">{t("restore.chosen", { n: fmt.count(chosen.size) })}</span>
                  <button className="btn primary small" disabled={chosen.size === 0} onClick={restore}>
                    <History size={14} /> {t("restore.start")}
                  </button>
                </div>
                <ul className="archive-files">
                  {visible.slice(0, 1000).map((file) => (
                    <li key={file.rel}>
                      <label>
                        <input type="checkbox" checked={chosen.has(file.rel)} onChange={() => toggle(file.rel)} />
                        <span className="file-path">{file.rel}</span>
                      </label>
                      <small>{fmt.bytes(file.size)}</small>
                    </li>
                  ))}
                </ul>
              </>
            ) : (
              <p className="muted">{t("restore.pick")}</p>
            )}
          </section>
        </div>
      )}
    </Frame>
  );
}

export function ToolsView({ tool }: { tool?: ToolName }) {
  switch (tool) {
    case "compare":
      return <CompareTool />;
    case "duplicates":
      return <DuplicatesTool />;
    case "space":
      return <SpaceTool />;
    case "read":
      return <ReadTool />;
    case "restore":
      return <RestoreTool />;
    case "cleanup":
      return <CleanupTool />;
    case "network":
      return <NetworkTool />;
    case "drives":
      return <DrivesTool />;
    default:
      return <Hub />;
  }
}
