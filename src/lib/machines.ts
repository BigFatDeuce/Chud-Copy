// Machines à états de l'interface, en un seul endroit.
//
// Chaque machine décrit ses états par un type, et ses questions (« est-ce en cours ? », « peut-on
// lancer ? ») par une fonction pure. Les vues ne recomposent jamais ces réponses elles-mêmes : une
// règle changée ici vaut pour tous les écrans. Le cœur Rust reste la seule autorité sur les
// opérations ; ce fichier ne fait que lire ce qu'il publie. Voir « Machines à états » dans
// docs/ARCHITECTURE.md.

import { isDestructive } from "./format";
import type { IntegrityReport, Mode, Plan, PlanProgress, QueueState, RunRecord, RunStatus } from "./types";

// ---------------------------------------------------------------- Opération
//
//   running ⇄ paused        (pause, reprise)
//   running | paused → cancelling → cancelled
//   running → completed | completedWithErrors | failed
//   (application fermée brutalement) → interrupted, posé au lancement suivant
//
// Miroir de `RunStatus::is_active` dans `core/src/history.rs`.

/** Une opération vivante occupe le moteur : rien d'autre ne démarre, la fermeture demande confirmation. */
export function isLive(status: RunStatus | null | undefined): boolean {
  return status === "running" || status === "paused" || status === "cancelling";
}

/** L'opération en cours, ou rien si aucune ne tourne. */
export function liveRun(run: RunRecord | null): RunRecord | null {
  return run && isLive(run.status) ? run : null;
}

/** Ce qu'une transition de statut signale à l'utilisateur : une fin heureuse, une fin à problème, ou rien. */
export function ending(before: RunStatus | null, after: RunStatus): "good" | "bad" | null {
  if (!isLive(before) || isLive(after)) return null;
  if (after === "completed") return "good";
  // Une annulation est un choix de l'utilisateur, pas un problème à signaler.
  return after === "cancelled" ? null : "bad";
}

// ---------------------------------------------------------------- Lancement
//
// Une seule opération à la fois : selon l'état du moteur et de la file, le même bouton lance,
// met en file, ou constate que la tâche y est déjà.

export type LaunchChoice = "start" | "queue" | "queued";

export function launchChoice(run: RunRecord | null, queue: QueueState, taskId: string): LaunchChoice {
  if (!liveRun(run)) return "start";
  // Une entrée refusée (avec sa raison) ne compte pas : la tâche peut être remise en file.
  return queue.items.some((item) => item.taskId === taskId && !item.error) ? "queued" : "queue";
}

// ---------------------------------------------------------------- Analyse
//
//   running → done (plan complet, ou plan arrêté avant la fin) | failed
//
// Une analyse terminée porte toujours son plan, une analyse échouée toujours sa raison : le type
// l'impose, il n'y a plus de combinaison à vérifier à la main.

interface AnalysisBase {
  taskId: string;
  taskName: string;
  mode: Mode;
  progress: Record<string, PlanProgress>;
}

export type AnalysisOutcome = { status: "done"; plan: Plan } | { status: "failed"; error: string };
export type AnalysisEntry = AnalysisBase & ({ status: "running" } | AnalysisOutcome);

export type AnalysisPhase = "running" | "failed" | "stopped" | "ready";

export function analysisPhase(entry: AnalysisEntry): AnalysisPhase {
  if (entry.status !== "done") return entry.status;
  return entry.plan.cancelled ? "stopped" : "ready";
}

/** Avancement cumulé de toutes les liaisons d'une analyse. */
export function analysisProgress(entry: AnalysisEntry | undefined) {
  const links = Object.values(entry?.progress ?? {});
  return {
    scanned: links.reduce((sum, p) => sum + p.listed, 0),
    compared: links.reduce((sum, p) => sum + p.checks, 0),
    toCompare: links.reduce((sum, p) => sum + p.totalChecks, 0),
    checking: links.flatMap((p) => p.checking),
  };
}

// ---------------------------------------------------------------- Verdict d'un plan
//
// Du plus bloquant au moins bloquant. Les trois premiers interdisent de lancer ; les textes sont
// les clés `preview.<verdict>`.

export type PlanVerdict = "failed" | "unseen" | "stopped" | "upToDate" | "ready" | "readyArchive" | "readyDelete";

export function planVerdict(plan: Plan, mode: Mode): PlanVerdict {
  const destructive = isDestructive(mode);
  // Un miroir ne part jamais d'une analyse incomplète : il supprimerait des fichiers encore présents.
  if (destructive && plan.links.some((link) => link.failed)) return "failed";
  if (destructive && plan.links.some((link) => link.unseenFiles > 0)) return "unseen";
  if (plan.cancelled) return "stopped";
  const { copy, update, delete: removed } = plan.totals;
  if (copy + update + removed === 0) return "upToDate";
  if (removed === 0) return "ready";
  return mode === "mirrorArchive" ? "readyArchive" : "readyDelete";
}

export function planRunnable(verdict: PlanVerdict): boolean {
  return verdict !== "failed" && verdict !== "unseen" && verdict !== "stopped";
}

// ---------------------------------------------------------------- Travaux en arrière-plan
//
//   idle → running → done → idle (résultat retiré)
//                  ↘ idle (échec, signalé par un message)
//
// L'état vit dans l'application et non dans la vue : un outil continue quand on change d'onglet.

export type Job<R> = { phase: "idle" } | { phase: "running" } | { phase: "done"; result: R };

export const IDLE: Job<never> = { phase: "idle" };

/** Les outils qui tournent en arrière-plan. La barre latérale et l'accueil des outils lisent cette liste. */
export const JOB_TOOLS = ["compare", "duplicates", "space", "read", "cleanup", "network"] as const;
export type JobTool = (typeof JOB_TOOLS)[number];

export function isJobTool(id: string): id is JobTool {
  return (JOB_TOOLS as readonly string[]).includes(id);
}

export function jobKey(id: JobTool): string {
  return `tool:${id}`;
}

/** Contrôle d'intégrité : un dossier choisi, et le travail qui porte dessus. */
export interface CheckState {
  root: string | null;
  job: Job<IntegrityReport>;
}

export const CHECK_KEY = "integrity";
export const NO_CHECK: CheckState = { root: null, job: IDLE };

/** Lecture d'un état conservé par l'application (`keep` du contexte). */
type Keep = <T>(key: string, initial: T) => [T, unknown];

export function jobIsRunning(keep: Keep, id: JobTool): boolean {
  return keep<Job<unknown>>(jobKey(id), IDLE)[0].phase === "running";
}

/** Nombre d'outils et de contrôles en cours, pour la pastille de la barre latérale. */
export function backgroundJobs(keep: Keep): number {
  const tools = JOB_TOOLS.filter((id) => jobIsRunning(keep, id)).length;
  const check = keep<CheckState>(CHECK_KEY, NO_CHECK)[0].job.phase === "running" ? 1 : 0;
  return tools + check;
}

// ---------------------------------------------------------------- Extinction en fin de copie
//
//   off → armed (case cochée) → counting (tout est fini) → extinction demandée au cœur
//   armed | counting → off (case décochée, compte à rebours annulé, ou copie annulée)
//   counting → armed (une opération repart, par exemple la suivante de la file)
//
// Jamais enregistrée : un lancement précédent ne peut pas éteindre l'ordinateur. Le cœur
// revérifie qu'aucune opération ne tourne ni n'attend avant d'éteindre.

export type Shutdown = { phase: "off" } | { phase: "armed"; sawRun: boolean } | { phase: "counting"; until: number };

export const SHUTDOWN_OFF: Shutdown = { phase: "off" };

/** Délai laissé pour annuler, en secondes, entre la fin des copies et l'extinction. */
export const SHUTDOWN_DELAY = 60;

/** Tout est fini : plus d'opération vivante, plus rien en attente dans la file. */
export function allDone(run: RunRecord | null, queue: QueueState): boolean {
  return !liveRun(run) && !queue.items.some((item) => !item.error);
}

/** Armer l'extinction : une opération déjà en cours compte comme vue. */
export function armShutdown(run: RunRecord | null): Shutdown {
  return { phase: "armed", sawRun: !!liveRun(run) };
}

/**
 * Prochain état de l'extinction selon l'opération et la file. Il faut avoir vu une opération
 * tourner depuis que la case est cochée : cocher après une copie finie n'éteint rien. Une copie
 * annulée désarme, puisque l'utilisateur est devant l'écran.
 */
export function shutdownStep(state: Shutdown, run: RunRecord | null, queue: QueueState, now: number): Shutdown {
  if (state.phase === "off") return state;
  const done = allDone(run, queue);
  if (state.phase === "counting") return done ? state : { phase: "armed", sawRun: true };
  if (liveRun(run)) return state.sawRun ? state : { phase: "armed", sawRun: true };
  if (!state.sawRun || !done) return state;
  if (run?.status === "cancelled") return SHUTDOWN_OFF;
  return { phase: "counting", until: now + SHUTDOWN_DELAY * 1000 };
}
