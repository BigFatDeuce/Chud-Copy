import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { applyLanguage, describe, t, type Language } from "./i18n";
import { fetchLatestVersion, isNewer } from "./updates";
import goodSound from "../assets/sounds/good.mp3";
import badSound from "../assets/sounds/bad.mp3";
import { Cosmos } from "../cosmos/engine";

// Sous Linux, WebKitGTK lit l'audio avec GStreamer : dans l'AppImage, sa version embarquée ne sait pas
// charger les greffons du système et le processus web plante. Le son passe donc par le système.
const linux = /Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent);

function playSound(kind: "good" | "bad") {
  if (linux) {
    api.playSound(kind).catch(() => {});
    return;
  }
  const audio = new Audio(kind === "good" ? goodSound : badSound);
  audio.volume = 0.3;
  audio.play().catch(() => {});
}
import { ending, launchChoice, SHUTDOWN_OFF, shutdownStep, type AnalysisEntry, type AnalysisOutcome, type JobTool, type Shutdown } from "./machines";
import type { AppInfo, Disk, PlanProgress, Quality, QueueState, RunRecord, RunStatus, Settings, Task } from "./types";

export type { AnalysisEntry } from "./machines";

export type Route =
  | { name: "home" }
  | { name: "editor"; task?: Task }
  /** `open` affiche l'analyse existante au lieu d'en relancer une. */
  | { name: "preview"; taskId: string; open?: boolean }
  | { name: "analyses" }
  | { name: "run" }
  | { name: "history"; runId?: string }
  | { name: "integrity" }
  | { name: "health" }
  | { name: "tools"; tool?: ToolName }
  | { name: "settings" };

/** Les outils à travail d'arrière-plan (`JobTool`), plus ceux qui répondent aussitôt. */
export type ToolName = JobTool | "restore" | "drives";

/** Progression d'un contrôle ou d'un outil, reçue même quand sa vue n'est pas affichée. */
export interface JobProgress {
  phase?: string;
  files: number;
  totalFiles: number;
  bytes: number;
  totalBytes: number;
  current: string;
}

export interface Toast {
  id: number;
  message: string;
  tone: "ok" | "bad";
}

interface AppContext {
  route: Route;
  go(route: Route): void;
  /** Permet à une vue de retenir la navigation, par exemple pour des modifications non enregistrées. */
  guard: (check: ((proceed: () => void) => boolean) | null) => void;
  info: AppInfo | null;
  disks: Disk[];
  refreshDisks(): void;
  settings: Settings;
  saveSettings(settings: Settings): void;
  language: Language;
  run: RunRecord | null;
  setRun(run: RunRecord | null): void;
  /** Lance la tâche, ou la met en file si une opération occupe déjà le moteur. */
  launch(taskId: string, usePlan: boolean): Promise<"started" | "queued">;
  /** Couleur de chaque tâche, pour teinter ses cadres dans tous les onglets. */
  colors: Record<string, string | null>;
  rememberTasks(tasks: Task[]): void;
  queue: QueueState;
  analyses: AnalysisEntry[];
  startAnalysis(taskId: string): Promise<void>;
  stopAnalysis(taskId: string): void;
  forgetAnalysis(taskId: string): void;
  /** Analyses terminées que l'utilisateur n'a pas encore ouvertes. */
  unseen: Set<string>;
  /** État des outils, conservé quand on change d'onglet. */
  keep<T>(key: string, initial: T): [T, (next: T | ((current: T) => T)) => void];
  progress: Record<string, JobProgress>;
  toast(message: string, tone?: Toast["tone"]): void;
  toasts: Toast[];
  cosmos: Cosmos | null;
  cosmosFailed: boolean;
  /** Extinction de l'ordinateur à la fin des copies, voir `Shutdown` dans machines.ts. */
  shutdown: Shutdown;
  setShutdown(next: Shutdown): void;
}

const Context = createContext<AppContext | null>(null);

export function useApp(): AppContext {
  const value = useContext(Context);
  if (!value) throw new Error("Contexte manquant");
  return value;
}

function defaultQuality(): Quality {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "off" : "balanced";
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [route, setRoute] = useState<Route>({ name: "home" });
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [disks, setDisks] = useState<Disk[]>([]);
  const [settings, setSettings] = useState<Settings>({ quality: defaultQuality(), language: "auto" });
  const [language, setLanguage] = useState<Language>(() => applyLanguage("auto"));
  const [ready, setReady] = useState(false);
  const [run, setRun] = useState<RunRecord | null>(null);
  const [colors, setColors] = useState<Record<string, string | null>>({});
  const [queue, setQueue] = useState<QueueState>({ items: [], held: false });
  const [analyses, setAnalyses] = useState<AnalysisEntry[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [cosmos, setCosmos] = useState<Cosmos | null>(null);
  const [cosmosFailed, setCosmosFailed] = useState(false);
  const [shutdown, setShutdown] = useState<Shutdown>(SHUTDOWN_OFF);
  const guardRef = useRef<((proceed: () => void) => boolean) | null>(null);
  const toastId = useRef(0);
  const starting = useRef(new Set<string>());
  const [unseen, setUnseen] = useState<Set<string>>(new Set());
  const routeRef = useRef(route);
  routeRef.current = route;
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const runRef = useRef(run);
  runRef.current = run;
  const queueRef = useRef(queue);
  queueRef.current = queue;
  const runStatus = useRef<RunStatus | null>(null);
  const kept = useRef(new Map<string, unknown>());
  const [version, bump] = useState(0);
  const [progress, setProgress] = useState<Record<string, JobProgress>>({});

  // L'état des outils vit ici et non dans leur vue : un contrôle continue et reste affiché
  // quand on va voir ailleurs, jusqu'à ce que l'utilisateur le retire.
  const keep = useCallback(<T,>(key: string, initial: T): [T, (next: T | ((current: T) => T)) => void] => {
    const value = (kept.current.has(key) ? kept.current.get(key) : initial) as T;
    const set = (next: T | ((current: T) => T)) => {
      const current = (kept.current.has(key) ? kept.current.get(key) : initial) as T;
      kept.current.set(key, typeof next === "function" ? (next as (c: T) => T)(current) : next);
      bump((n) => n + 1);
    };
    return [value, set];
  }, []);

  const go = useCallback((next: Route) => {
    const check = guardRef.current;
    if (check && check(() => setRoute(next))) return;
    setRoute(next);
  }, []);

  const guard = useCallback((check: ((proceed: () => void) => boolean) | null) => {
    guardRef.current = check;
  }, []);

  const refreshDisks = useCallback(() => {
    api.disksList().then(setDisks).catch(() => {});
  }, []);

  const toast = useCallback((message: string, tone: Toast["tone"] = "ok") => {
    toastId.current += 1;
    const id = toastId.current;
    setToasts((list) => [...list, { id, message, tone }]);
    window.setTimeout(() => setToasts((list) => list.filter((t) => t.id !== id)), 5200);
  }, []);

  const rememberTasks = useCallback((tasks: Task[]) => {
    setColors(Object.fromEntries(tasks.map((task) => [task.id, task.color ?? null])));
  }, []);

  const saveSettings = useCallback((next: Settings) => {
    setSettings(next);
    setLanguage(applyLanguage(next.language));
    api.settingsSet(next).catch(() => {});
  }, []);

  // Les analyses appartiennent à l'application, pas à un écran : elles survivent à la navigation.
  const startAnalysis = useCallback(async (taskId: string) => {
    if (starting.current.has(taskId)) return;
    starting.current.add(taskId);
    // Seule transition possible depuis « en cours » : vers un plan, ou vers une raison d'échec.
    const settle = (outcome: AnalysisOutcome) =>
      setAnalyses((list) => list.map((a) => (a.taskId === taskId ? { taskId: a.taskId, taskName: a.taskName, mode: a.mode, progress: a.progress, ...outcome } : a)));
    let listed = false;
    try {
      const task = (await api.tasksList()).tasks.find((t) => t.id === taskId);
      if (!task) return;
      const entry: AnalysisEntry = { taskId, taskName: task.name, mode: task.mode, status: "running", progress: {} };
      setAnalyses((list) => [entry, ...list.filter((a) => a.taskId !== taskId)]);
      listed = true;
      settle({ status: "done", plan: await api.planStart(taskId) });
    } catch (error) {
      settle({ status: "failed", error: describe(String(error)) });
    } finally {
      starting.current.delete(taskId);
      // Terminée hors de sa vue : l'onglet Analyses le signale jusqu'à ce qu'on l'ouvre.
      // Une tâche introuvable n'a rien laissé dans la liste : rien à signaler.
      const current = routeRef.current;
      const watching = current.name === "analyses" || (current.name === "preview" && current.taskId === taskId);
      if (listed && !watching) setUnseen((set) => new Set(set).add(taskId));
    }
  }, []);

  const launch = useCallback(async (taskId: string, usePlan: boolean) => {
    if (launchChoice(runRef.current, queueRef.current, taskId) !== "start") {
      await api.queueAdd(taskId, usePlan);
      return "queued" as const;
    }
    // L'écran d'exécution ne doit pas montrer la fin de l'opération précédente en attendant la nouvelle.
    setRun(null);
    await api.runStart(taskId, usePlan);
    return "started" as const;
  }, []);

  const stopAnalysis = useCallback((taskId: string) => {
    api.planCancel(taskId).catch(() => {});
  }, []);

  const forgetAnalysis = useCallback((taskId: string) => {
    setAnalyses((list) => list.filter((a) => a.taskId !== taskId));
    api.planForget(taskId).catch(() => {});
  }, []);

  useEffect(() => {
    const stop = listen<PlanProgress & { taskId: string }>("plan:progress", ({ payload }) =>
      setAnalyses((list) =>
        list.map((entry) => {
          if (entry.taskId !== payload.taskId) return entry;
          const previous = entry.progress[payload.linkId];
          const next = payload.finished && previous ? { ...previous, checking: [], finished: true } : payload;
          return { ...entry, progress: { ...entry.progress, [payload.linkId]: next } };
        }),
      ),
    );
    return () => {
      stop.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const canvas = document.getElementById("cosmos") as HTMLCanvasElement;
    const instance = Cosmos.create(canvas);
    if (instance) setCosmos(instance);
    else {
      canvas.style.display = "none";
      setCosmosFailed(true);
    }
    Promise.all([api.settingsGet(), api.tasksList()])
      .then(([saved, list]) => {
        rememberTasks(list.tasks);
        // Les versions 1.0 n'existaient qu'en français : leurs utilisateurs le restent à la mise à jour.
        const language = saved.language ?? (list.tasks.length > 0 ? "fr" : "auto");
        setSettings((current) => ({ ...current, ...saved, language }));
        setLanguage(applyLanguage(language));
      })
      .catch(() => {})
      .finally(() => setReady(true));
    api.appInfo().then(setInfo).catch(() => {});
    api
      .runCurrent()
      .catch(() => null)
      .then((current) => {
        if (current) {
          setRun(current);
          setRoute({ name: "run" });
        }
      });
    api.queueGet().then(setQueue).catch(() => {});
    refreshDisks();
    const onFocus = () => refreshDisks();
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(refreshDisks, 15000);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [refreshDisks, rememberTasks]);

  useEffect(() => {
    cosmos?.setQuality(settings.quality);
  }, [cosmos, settings.quality]);

  useEffect(() => {
    if (route.name === "analyses") setUnseen(new Set());
    else if (route.name === "preview") setUnseen((set) => (set.has(route.taskId) ? new Set([...set].filter((id) => id !== route.taskId)) : set));
  }, [route]);

  // Recherche automatique de mise à jour : seulement si l'utilisateur l'a cochée, au plus une fois par jour.
  const checked = useRef(false);
  useEffect(() => {
    if (!ready || !info || checked.current || !settings.autoUpdate) return;
    checked.current = true;
    if (Date.now() - (settings.lastUpdateCheck ?? 0) < 20 * 3600 * 1000) return;
    fetchLatestVersion()
      .then((latest) => {
        saveSettings({ ...settingsRef.current, lastUpdateCheck: Date.now() });
        if (isNewer(latest, info.version)) toast(t("update.notice", { version: latest }));
      })
      .catch(() => {});
  }, [ready, info, settings.autoUpdate, settings.lastUpdateCheck, saveSettings, toast]);

  useEffect(() => {
    const stop = listen<RunRecord>("run:update", (event) => {
      const next = event.payload.status;
      // Un son à la fin de chaque opération, au tiers de sa puissance : réussite ou problème.
      const kind = ending(runStatus.current, next);
      if (kind && settingsRef.current.sounds !== false) playSound(kind);
      runStatus.current = next;
      setRun(event.payload);
    });
    const stopQueue = listen<QueueState>("queue:update", (event) => setQueue(event.payload));
    const stopCheck = listen<JobProgress & { root: string }>("integrity:progress", ({ payload }) => setProgress((all) => ({ ...all, [`integrity:${payload.root}`]: payload })));
    const stopTool = listen<JobProgress & { id: string }>("tool:progress", ({ payload }) => setProgress((all) => ({ ...all, [payload.id]: payload })));
    return () => {
      stop.then((fn) => fn());
      stopQueue.then((fn) => fn());
      stopCheck.then((fn) => fn());
      stopTool.then((fn) => fn());
    };
  }, []);

  // L'extinction suit l'opération et la file : elle s'arme, compte, ou se désarme d'elle-même.
  useEffect(() => {
    setShutdown((state) => shutdownStep(state, run, queue, Date.now()));
  }, [run, queue]);

  const value = useMemo(
    () => ({ route, go, guard, info, disks, refreshDisks, settings, saveSettings, language, run, setRun, launch, colors, rememberTasks, queue, analyses, startAnalysis, stopAnalysis, forgetAnalysis, unseen, keep, progress, toast, toasts, cosmos, cosmosFailed, shutdown, setShutdown }),
    [route, go, guard, info, disks, refreshDisks, settings, saveSettings, language, run, launch, colors, rememberTasks, queue, analyses, startAnalysis, stopAnalysis, forgetAnalysis, unseen, keep, version, progress, toast, toasts, cosmos, cosmosFailed, shutdown],
  );
  return <Context.Provider value={value}>{ready ? children : null}</Context.Provider>;
}
