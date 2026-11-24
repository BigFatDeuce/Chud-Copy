import { useState } from "react";
import { ArrowLeft, FolderSearch, HardDrive, Square, X } from "lucide-react";
import { api, pickFolders } from "../lib/api";
import { useApp, type JobProgress, type ToolName } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { IDLE, jobKey, type Job, type JobTool } from "../lib/machines";
import { Beam } from "./ui";

/** Choix d'un disque, d'un dossier récent, d'un dossier quelconque ou d'un chemin tapé
 * (« \\nas\partage », « /mnt/nas ») : un partage réseau non connecté n'apparaît dans aucune liste. */
export function PathPicker({ value, onPick, recent = [], disabled, label }: { value: string | null; onPick: (path: string) => void; recent?: string[]; disabled?: boolean; label?: string }) {
  const { disks, settings, saveSettings } = useApp();
  const [typed, setTyped] = useState("");
  const pick = (path: string) => {
    // Un dossier déjà proposé garde sa place : les pastilles ne bougent pas sous le curseur.
    const known = settings.recentPaths ?? [];
    if (!known.includes(path) && !disks.some((disk) => disk.mount === path)) saveSettings({ ...settings, recentPaths: [path, ...known].slice(0, 8) });
    onPick(path);
  };
  const browse = async () => {
    const [path] = await pickFolders(value ?? undefined);
    if (path) pick(path);
  };
  const paths = [...new Set([...recent, ...(settings.recentPaths ?? [])])];
  return (
    <div className="path-picker">
      {label && <span className="picker-label">{label}</span>}
      <div className="target-choices">
        {disks.map((disk) => (
          <button key={disk.mount} className={value === disk.mount ? "chip-choice active" : "chip-choice"} disabled={disabled} onClick={() => pick(disk.mount)}>
            <HardDrive size={14} /> {disk.mount}
          </button>
        ))}
        {paths
          .filter((path) => !disks.some((disk) => disk.mount === path))
          .map((path) => (
            <button key={path} className={value === path ? "chip-choice active" : "chip-choice"} disabled={disabled} onClick={() => pick(path)} title={path}>
              <FolderSearch size={14} /> <span className="file-path">{fmt.shortPath(path)}</span>
            </button>
          ))}
        <button className="btn ghost small" disabled={disabled} onClick={browse}>
          <FolderSearch size={14} /> {t("integrity.browse")}
        </button>
      </div>
      <form
        className="typed-path"
        onSubmit={(e) => {
          e.preventDefault();
          const path = typed.trim();
          if (path) {
            pick(path);
            setTyped("");
          }
        }}
      >
        <input value={typed} onChange={(e) => setTyped(e.target.value)} placeholder={t("net.typed")} aria-label={t("net.typed")} dir="ltr" disabled={disabled} />
        <button className="btn ghost small" disabled={!typed.trim() || disabled}>
          {t("net.use")}
        </button>
      </form>
      {value && (
        <code className="picked file-path" dir="ltr">
          {value}
        </code>
      )}
    </div>
  );
}

/** Progression d'un outil en cours, avec arrêt. */
export function JobBar({ progress, onStop, title, detail }: { progress?: JobProgress; onStop: () => void; title: string; detail?: string }) {
  const known = !!progress && progress.totalBytes > 0 && progress.phase !== "list";
  return (
    <section className="panel job-bar">
      <div className="job-head">
        <h2>{title}</h2>
        <button className="btn ghost small" onClick={onStop}>
          <Square size={13} /> {t("integrity.stop")}
        </button>
      </div>
      <Beam value={known ? progress.bytes / progress.totalBytes : 0} indeterminate={!known} />
      <p>
        {detail !== undefined
          ? detail
          : !progress || progress.phase === "list"
          ? t("tools.listing", { n: fmt.count(progress?.totalFiles ?? 0) })
          : t("integrity.progress", { done: fmt.count(progress.files), total: fmt.count(progress.totalFiles), size: fmt.bytes(progress.bytes), all: fmt.bytes(progress.totalBytes) })}
      </p>
      {progress?.current && <p className="analysis-file file-path">{progress.current}</p>}
    </section>
  );
}

/** En-tête d'un résultat, qu'on peut retirer de l'écran. */
export function ResultHead({ title, onDismiss, children }: { title: string; onDismiss: () => void; children?: React.ReactNode }) {
  return (
    <div className="result-head">
      <h2>{title}</h2>
      <div className="result-actions">
        {children}
        <button className="icon-btn" aria-label={t("tools.dismiss")} title={t("tools.dismiss")} onClick={onDismiss}>
          <X size={16} />
        </button>
      </div>
    </div>
  );
}

function Back() {
  const { go } = useApp();
  return (
    <button className="crumb" onClick={() => go({ name: "tools" })}>
      <ArrowLeft size={15} /> {t("nav.tools")}
    </button>
  );
}

export function Frame({ id, children }: { id: ToolName; children: React.ReactNode }) {
  return (
    <div className="view tool">
      <header className="view-head">
        <div>
          <Back />
          <h1>{t(`tool.${id}`)}</h1>
          <p>{t(`tool.${id}.what`)}</p>
        </div>
      </header>
      {children}
    </div>
  );
}

/** Numéro du dernier lancement de chaque outil : le résultat d'un lancement abandonné est ignoré. */
const generations = new Map<JobTool, number>();

/**
 * Lance un outil et garde son résultat dans l'application, même si l'on change d'onglet.
 * Le travail suit la machine `Job` de `lib/machines.ts` : au repos, en cours, ou terminé avec son résultat.
 */
export function useJob<R>(id: JobTool) {
  const { keep, progress, toast } = useApp();
  const [state, setState] = keep<Job<R>>(jobKey(id), IDLE);
  const running = state.phase === "running";
  const run = async (work: () => Promise<R>) => {
    const generation = (generations.get(id) ?? 0) + 1;
    generations.set(id, generation);
    const current = () => generations.get(id) === generation;
    setState({ phase: "running" });
    try {
      const result = await work();
      if (current()) setState({ phase: "done", result });
    } catch (error) {
      if (!current()) return;
      setState(IDLE);
      toast(describe(String(error)), "bad");
    }
  };
  /** Demande l'arrêt : l'outil rend alors un résultat partiel, marqué comme arrêté. */
  const stop = () => api.toolCancel(id).catch(() => {});
  /** Retire de l'écran : un travail en cours est arrêté, et son résultat ne sera pas affiché. */
  const dismiss = () => {
    if (running) api.toolCancel(id).catch(() => {});
    generations.set(id, (generations.get(id) ?? 0) + 1);
    setState(IDLE);
  };
  /** Remplace le résultat affiché, par exemple après avoir retiré des fichiers de la liste. */
  const replace = (result: R) => setState((current) => (current.phase === "done" ? { phase: "done", result } : current));
  return { running, result: state.phase === "done" ? state.result : null, progress: progress[id], run, stop, dismiss, replace };
}

export function SpeedGraph({ samples, live }: { samples: number[]; live?: boolean }) {
  if (samples.length < 2) return null;
  const max = Math.max(...samples) || 1;
  const points = samples.map((s, i) => `${(i / (samples.length - 1)) * 100},${40 - (s / max) * 36}`).join(" ");
  return (
    <svg className={live ? "speed-graph live" : "speed-graph"} viewBox="0 0 100 40" preserveAspectRatio="none" aria-hidden="true">
      <defs>
        <linearGradient id="speed-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#3ddc97" stopOpacity="0.45" />
          <stop offset="1" stopColor="#3ddc97" stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon points={`0,40 ${points} 100,40`} fill="url(#speed-fill)" />
      <polyline points={points} fill="none" stroke="#3ddc97" strokeWidth="0.6" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

