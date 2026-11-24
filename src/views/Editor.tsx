import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, AlertTriangle, ChevronDown, Info, Save, ScanSearch, ShieldAlert } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { blankTask } from "../lib/tasks";
import { Chips, ColorSwatches, Modal, Select, Toggle } from "../components/ui";
import { RouteMap } from "./RouteMap";
import type { Filters, Mode, Options, Report, Task } from "../lib/types";

const MODE_ORDER: Mode[] = ["update", "full", "addOnly", "mirrorArchive", "mirror"];
const MEGABYTE = 1024 * 1024;
/** Rappels proposés, en jours ; 0 pour aucun. */
const REMINDER_DAYS = [0, 1, 3, 7, 14, 30];

function Disclosure({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <section className={open ? "disclosure open" : "disclosure"}>
      <button aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>
          {title}
          {hint && <small>{hint}</small>}
        </span>
        <ChevronDown size={17} />
      </button>
      {open && <div className="disclosure-body">{children}</div>}
    </section>
  );
}

function activeFilterCount(filters: Filters): number {
  const lists = [filters.includeFiles, filters.excludeFiles, filters.includeDirs, filters.excludeDirs, filters.includeExtensions, filters.excludeExtensions];
  const values = [filters.minSize, filters.maxSize, filters.newerThan, filters.olderThan, filters.excludeJunk ? true : null];
  return lists.filter((l) => l.length > 0).length + values.filter((v) => v !== null).length;
}

function FiltersPanel({ filters, onChange }: { filters: Filters; onChange: (filters: Filters) => void }) {
  const set = <K extends keyof Filters>(key: K, value: Filters[K]) => onChange({ ...filters, [key]: value });
  const size = (value: string) => (value === "" ? null : Math.round(Number(value) * MEGABYTE));
  return (
    <div className="filters">
      <p className="muted small">{t("filters.help")}</p>
      <div className="filter-columns">
        <div>
          <h4>{t("filters.exclude")}</h4>
          <Chips label={t("filters.extensions")} placeholder="tmp, log, bak" values={filters.excludeExtensions} onChange={(v) => set("excludeExtensions", v)} />
          <Chips label={t("filters.names")} placeholder="Thumbs.db, draft*" values={filters.excludeFiles} onChange={(v) => set("excludeFiles", v)} />
          <Chips label={t("filters.folders")} placeholder="node_modules, cache" values={filters.excludeDirs} onChange={(v) => set("excludeDirs", v)} />
        </div>
        <div>
          <h4>{t("filters.include")}</h4>
          <Chips label={t("filters.extensions")} placeholder="jpg, png, raw" values={filters.includeExtensions} onChange={(v) => set("includeExtensions", v)} />
          <Chips label={t("filters.names")} placeholder="report*" values={filters.includeFiles} onChange={(v) => set("includeFiles", v)} />
          <Chips label={t("filters.folders")} placeholder="Photos/2024" values={filters.includeDirs} onChange={(v) => set("includeDirs", v)} />
        </div>
      </div>
      <div className="field-row">
        <label className="field">
          <span>{t("filters.minSize")}</span>
          <input type="number" min="0" step="any" value={filters.minSize === null ? "" : filters.minSize / MEGABYTE} onChange={(e) => set("minSize", size(e.target.value))} />
        </label>
        <label className="field">
          <span>{t("filters.maxSize")}</span>
          <input type="number" min="0" step="any" value={filters.maxSize === null ? "" : filters.maxSize / MEGABYTE} onChange={(e) => set("maxSize", size(e.target.value))} />
        </label>
        <label className="field">
          <span>{t("filters.after")}</span>
          <input type="date" value={filters.newerThan ?? ""} onChange={(e) => set("newerThan", e.target.value || null)} />
        </label>
        <label className="field">
          <span>{t("filters.before")}</span>
          <input type="date" value={filters.olderThan ?? ""} onChange={(e) => set("olderThan", e.target.value || null)} />
        </label>
      </div>
      <Toggle checked={filters.excludeSystem} onChange={(v) => set("excludeSystem", v)} label={t("filters.system")} hint={t("filters.systemHint")} />
    </div>
  );
}

function AdvancedPanel({ options, onChange }: { options: Options; onChange: (options: Options) => void }) {
  const set = <K extends keyof Options>(key: K, value: Options[K]) => onChange({ ...options, [key]: value });
  const bounded = (value: string, min: number, max: number) => Math.max(min, Math.min(max, Number(value) || min));
  return (
    <div className="advanced">
      <div className="field-row">
        <label className="field">
          <span>{t("adv.compare")}</span>
          <Select
            value={options.compare}
            onChange={(compare) => set("compare", compare)}
            choices={[
              { value: "sizeAndTime", label: t("adv.compareSize") },
              { value: "checksum", label: t("adv.compareContent") },
              { value: "sizeOnly", label: t("adv.compareSizeOnly") },
            ]}
          />
        </label>
        <label className="field">
          <span>{t("adv.symlinks")}</span>
          <Select
            value={options.symlinks}
            onChange={(symlinks) => set("symlinks", symlinks)}
            choices={[
              { value: "skip", label: t("adv.linkSkip") },
              { value: "follow", label: t("adv.linkFollow") },
              { value: "preserve", label: t("adv.linkKeep") },
            ]}
          />
        </label>
        <label className="field">
          <span>{t("adv.retries")}</span>
          <input type="number" min="1" max="20" value={options.retries} onChange={(e) => set("retries", bounded(e.target.value, 1, 20))} />
        </label>
        <label className="field">
          <span>{t("adv.transfers")}</span>
          <input type="number" min="1" max="32" value={options.transfers} onChange={(e) => set("transfers", bounded(e.target.value, 1, 32))} />
          <small>{t("adv.transfersHint")}</small>
        </label>
      </div>
      <div className="toggle-grid">
        <Toggle checked={options.preserveMetadata} onChange={(v) => set("preserveMetadata", v)} label={t("adv.attrs")} hint={t("adv.attrsHint")} />
        <Toggle checked={options.skipNewer} onChange={(v) => set("skipNewer", v)} label={t("adv.skipNewer")} hint={t("adv.skipNewerHint")} />
        <Toggle checked={options.parallelVolumes} onChange={(v) => set("parallelVolumes", v)} label={t("adv.parallel")} hint={t("adv.parallelHint")} />
        <Toggle checked={options.multiThread} onChange={(v) => set("multiThread", v)} label={t("adv.multi")} hint={t("adv.multiHint")} />
        <Toggle checked={options.allowMerge} onChange={(v) => set("allowMerge", v)} label={t("adv.merge")} hint={t("adv.mergeHint")} />
      </div>
    </div>
  );
}

export function Editor({ initial }: { initial?: Task }) {
  const { go, guard, disks, toast, refreshDisks } = useApp();
  const [task, setTask] = useState<Task>(initial ?? blankTask());
  const [junk, setJunk] = useState<string[]>([]);
  useEffect(() => {
    api.junkList().then(setJunk).catch(() => {});
  }, []);
  const [report, setReport] = useState<Report>();
  const [saved, setSaved] = useState(JSON.stringify(initial ?? blankTask()));
  const [leaving, setLeaving] = useState<(() => void) | null>(null);
  const dirty = JSON.stringify(task) !== saved;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  useEffect(() => {
    guard((proceed) => {
      if (!dirtyRef.current) return false;
      setLeaving(() => proceed);
      return true;
    });
    return () => guard(null);
  }, [guard]);

  useEffect(refreshDisks, [refreshDisks]);

  useEffect(() => {
    // Une validation ancienne, ralentie par un partage réseau, ne remplace jamais la plus récente.
    let current = true;
    const timer = window.setTimeout(() => {
      api
        .taskValidate(task)
        .then((report) => current && setReport(report))
        .catch(() => {});
    }, 220);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [task]);

  const onChange = useCallback((next: Task) => setTask(next), []);
  const issues = report?.issues ?? [];
  const blocking = issues.some((i) => i.severity === "error");
  const filterCount = activeFilterCount(task.filters);
  
  const sorted = useMemo(() => {
    const rank = { error: 0, warning: 1, info: 2 };
    return [...issues].sort((a, b) => rank[a.severity] - rank[b.severity]);
  }, [issues]);

  const save = async (): Promise<Task | null> => {
    if (!task.name.trim()) {
      toast(t("editor.nameRequired"), "bad");
      return null;
    }
    try {
      const stored = await api.taskSave(task);
      setTask(stored);
      setSaved(JSON.stringify(stored));
      dirtyRef.current = false;
      toast(t("editor.savedToast"));
      return stored;
    } catch (error) {
      toast(describe(String(error)), "bad");
      return null;
    }
  };

  return (
    <div className="view editor">
      <header className="editor-head">
        <input className="title-input" value={task.name} placeholder={t("task.nameLabel")} aria-label={t("task.nameLabel")} onChange={(e) => setTask({ ...task, name: e.target.value })} />
        <div className="color-field">
          <span>{t("color.label")}</span>
          <ColorSwatches value={task.color} onPick={(color) => setTask({ ...task, color })} />
        </div>
        <input className="description-input" value={task.description} placeholder={t("editor.descPlaceholder")} aria-label={t("editor.descLabel")} onChange={(e) => setTask({ ...task, description: e.target.value })} />
      </header>

      <div className="editor-grid">
        <div className="editor-main">
          <RouteMap task={task} report={report} disks={disks} onChange={onChange} />
        </div>

        <aside className="editor-side">
          <section className="panel">
            <h2>{t("editor.mode")}</h2>
            <div className="modes" role="radiogroup" aria-label={t("editor.modeAria")}>
              {MODE_ORDER.map((id) => {
                const danger = fmt.isDestructive(id);
                const selected = task.mode === id;
                return (
                  <button key={id} role="radio" aria-checked={selected} className={`mode ${selected ? "selected" : ""} ${danger ? "danger" : ""}`} onClick={() => setTask({ ...task, mode: id })}>
                    <span className="mode-name">{fmt.modeLabel(id)}</span>
                    {selected && <span className="mode-summary">{fmt.modeSummary(id)}</span>}
                  </button>
                );
              })}
            </div>
            {fmt.isDestructive(task.mode) && (
              <div className="issue danger-note">
                <ShieldAlert size={15} />
                <span>{task.mode === "mirror" ? t("editor.dangerMirror") : t("editor.dangerArchive")}</span>
              </div>
            )}
          </section>

          <section className="panel">
            <h2>{t("editor.essentials")}</h2>
            <Toggle checked={task.options.verifyAfter} onChange={(v) => setTask({ ...task, options: { ...task.options, verifyAfter: v } })} label={t("editor.verify")} hint={t("editor.verifyHint")} />
            <Toggle checked={task.options.emptyDirs} onChange={(v) => setTask({ ...task, options: { ...task.options, emptyDirs: v } })} label={t("editor.emptyDirs")} />
            <Toggle checked={task.options.includeHidden} onChange={(v) => setTask({ ...task, options: { ...task.options, includeHidden: v } })} label={t("editor.hidden")} />
            <Toggle checked={task.filters.excludeJunk} onChange={(v) => setTask({ ...task, filters: { ...task.filters, excludeJunk: v } })} label={t("editor.junk")} hint={t("editor.junkHint")} />
            <label className="field inline reminder">
              <span>{t("reminder.label")}</span>
              <Select<number>
                value={task.reminderDays ?? 0}
                onChange={(days) => setTask({ ...task, reminderDays: days || null })}
                choices={REMINDER_DAYS.map((days) => ({ value: days, label: days ? t("reminder.days", { n: days }) : t("reminder.never") }))}
              />
            </label>
            {junk.length > 0 && (
              <div className={task.filters.excludeJunk ? "junk-list" : "junk-list off"} aria-label={t("editor.junkList")}>
                {junk.map((pattern) => (
                  <code key={pattern}>{pattern}</code>
                ))}
              </div>
            )}
          </section>

          <Disclosure title={t("editor.filters")} hint={filterCount ? t("editor.filtersActive", { n: filterCount }) : t("editor.filtersNone")}>
            <FiltersPanel filters={task.filters} onChange={(filters) => setTask({ ...task, filters })} />
          </Disclosure>

          <Disclosure title={t("editor.advanced")}>
            <AdvancedPanel options={task.options} onChange={(options) => setTask({ ...task, options })} />
          </Disclosure>
        </aside>
      </div>

      <footer className="editor-foot">
        <div className="issues" aria-live="polite">
          {sorted.slice(0, 4).map((issue, i) => (
            <div key={i} className={`issue ${issue.severity}`}>
              {issue.severity === "error" ? <AlertCircle size={15} /> : issue.severity === "warning" ? <AlertTriangle size={15} /> : <Info size={15} />}
              <span>{t(`issue.${issue.code}`, issue.params)}</span>
            </div>
          ))}
          {sorted.length > 4 && <div className="muted small">{t("editor.more", { n: sorted.length - 4 })}</div>}
          {sorted.length === 0 && <span className="muted small">{dirty ? t("editor.unsaved") : task.id ? t("editor.saved") : ""}</span>}
        </div>
        <div className="foot-actions">
          <button className="btn ghost" onClick={() => go({ name: "home" })}>
            {dirty ? t("common.cancel") : t("common.back")}
          </button>
          <button className="btn ghost" disabled={!dirty} onClick={save}>
            <Save size={15} /> {t("editor.save")}
          </button>
          <button
            className="btn primary"
            disabled={blocking}
            onClick={async () => {
              const stored = dirty || !task.id ? await save() : task;
              if (stored) go({ name: "preview", taskId: stored.id });
            }}
          >
            <ScanSearch size={15} /> {t("task.analyze")}
          </button>
        </div>
      </footer>

      {leaving && (
        <Modal
          title={t("editor.unsaved")}
          onClose={() => setLeaving(null)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setLeaving(null)}>
                {t("leave.stay")}
              </button>
              <button
                className="btn ghost"
                onClick={() => {
                  guard(null);
                  leaving();
                }}
              >
                {t("leave.discard")}
              </button>
              <button
                className="btn primary"
                onClick={async () => {
                  if (await save()) {
                    guard(null);
                    leaving();
                  }
                }}
              >
                {t("editor.save")}
              </button>
            </>
          }
        >
          <p>{t("leave.body")}</p>
        </Modal>
      )}
    </div>
  );
}
