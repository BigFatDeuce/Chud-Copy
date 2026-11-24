import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { CleanupItem, CleanupOptions, CleanupPlace, CleanupReport, DriveRow, DuplicateGroup, NetworkReport, Recycled, TrashOutcome } from "./types";
import type { AppInfo, ArchiveVersion, ArchivedFile, CompareReport, Disk, DuplicateReport, HealthReport, IntegrityReference, IntegrityReport, LastRun, LogPage, ReadReport, SpaceNode, Plan, PlanPage, PlanQuery, QueueState, Report, RunRecord, Settings, Task, TaskList } from "./types";

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  playSound: (kind: "good" | "bad") => invoke<boolean>("play_sound", { kind }),
  tasksList: () => invoke<TaskList>("tasks_list"),
  taskSave: (task: Task) => invoke<Task>("task_save", { task }),
  taskDelete: (id: string) => invoke<void>("task_delete", { id }),
  taskDuplicate: (id: string) => invoke<Task>("task_duplicate", { id }),
  tasksExport: (ids: string[], path: string) => invoke<number>("tasks_export", { ids, path }),
  tasksImport: (path: string) => invoke<Task[]>("tasks_import", { path }),
  taskValidate: (task: Task) => invoke<Report>("task_validate", { task }),
  disksList: () => invoke<Disk[]>("disks_list"),
  planStart: (taskId: string) => invoke<Plan>("plan_start", { taskId }),
  planCancel: (taskId: string) => invoke<void>("plan_cancel", { taskId }),
  planForget: (taskId: string) => invoke<void>("plan_forget", { taskId }),
  releasesOpen: () => invoke<void>("releases_open"),
  planQuery: (taskId: string, query: PlanQuery) => invoke<PlanPage>("plan_query", { taskId, query }),
  runStart: (taskId: string, usePlan: boolean) => invoke<string>("run_start", { taskId, usePlan }),
  runPause: () => invoke<void>("run_pause"),
  runResume: () => invoke<void>("run_resume"),
  runCancel: () => invoke<void>("run_cancel"),
  runCurrent: () => invoke<RunRecord | null>("run_current"),
  queueGet: () => invoke<QueueState>("queue_get"),
  queueAdd: (taskId: string, usePlan: boolean) => invoke<void>("queue_add", { taskId, usePlan }),
  queueRemove: (id: string) => invoke<void>("queue_remove", { id }),
  queueShift: (id: string, earlier: boolean) => invoke<void>("queue_shift", { id, earlier }),
  queueClear: () => invoke<void>("queue_clear"),
  queueResume: () => invoke<void>("queue_resume"),
  historyList: () => invoke<RunRecord[]>("history_list"),
  historyGet: (id: string) => invoke<RunRecord>("history_get", { id }),
  historyDelete: (id: string) => invoke<void>("history_delete", { id }),
  historyClear: () => invoke<number>("history_clear"),
  tasksLastRuns: () => invoke<Record<string, LastRun>>("tasks_last_runs"),
  junkList: () => invoke<string[]>("junk_list"),
  archiveReveal: (target: string) => invoke<void>("archive_reveal", { target }),
  integrityStart: (root: string) => invoke<IntegrityReport>("integrity_start", { root }),
  integrityCancel: (root: string) => invoke<void>("integrity_cancel", { root }),
  integrityBaseline: (root: string) => invoke<number | null>("integrity_baseline", { root }),
  integrityAccept: (root: string) => invoke<void>("integrity_accept", { root }),
  integrityExport: (root: string, target: string) => invoke<number>("integrity_export", { root, target }),
  healthScan: () => invoke<HealthReport>("health_scan"),
  integrityReferences: () => invoke<IntegrityReference[]>("integrity_references"),
  integrityForget: (root: string) => invoke<void>("integrity_forget", { root }),
  toolCancel: (id: string) => invoke<void>("tool_cancel", { id }),
  toolCompare: (id: string, left: string, right: string, content: boolean) => invoke<CompareReport>("tool_compare", { id, left, right, content }),
  toolDuplicates: (id: string, roots: string[], minSize: number) => invoke<DuplicateReport>("tool_duplicates", { id, roots, minSize }),
  toolSpace: (id: string, root: string) => invoke<SpaceNode>("tool_space", { id, root }),
  toolRead: (id: string, root: string) => invoke<ReadReport>("tool_read", { id, root }),
  toolCleanup: (id: string, roots: string[], options: CleanupOptions) => invoke<CleanupReport>("tool_cleanup", { id, roots, options }),
  cleanupPlaces: () => invoke<CleanupPlace[]>("cleanup_places"),
  cleanupTrash: (items: CleanupItem[]) => invoke<TrashOutcome>("cleanup_trash", { items }),
  toolNetwork: (id: string, path: string, options: { writeTest: boolean; size: number }) => invoke<NetworkReport>("tool_network", { id, path, options }),
  drivesStatus: () => invoke<DriveRow[]>("drives_status"),
  driveAccept: (taskId: string, endpointId: string) => invoke<Task>("drive_accept", { taskId, endpointId }),
  duplicatesTrash: (group: DuplicateGroup, keep: string, remove: string[]) => invoke<Recycled>("duplicates_trash", { group, keep, remove }),
  archiveVersions: (target: string) => invoke<ArchiveVersion[]>("archive_versions", { target }),
  archiveFiles: (target: string, archive: string, stamp: string) => invoke<ArchivedFile[]>("archive_files", { target, archive, stamp }),
  archiveRestore: (target: string, archive: string, stamp: string, rels: string[], destination: string) =>
    invoke<string[]>("archive_restore", { target, archive, stamp, rels, destination }),
  logRead: (id: string, search: string, offset: number, limit: number) => invoke<LogPage>("log_read", { id, search, offset, limit }),
  logReveal: (id: string) => invoke<void>("log_reveal", { id }),
  dataReveal: () => invoke<void>("data_reveal"),
  pathReveal: (path: string) => invoke<void>("path_reveal", { path }),
  systemPowerOff: () => invoke<void>("system_power_off"),
  settingsGet: () => invoke<Partial<Settings>>("settings_get"),
  settingsSet: (value: Settings) => invoke<void>("settings_set", { value }),
  appQuit: () => invoke<void>("app_quit"),
};

export async function pickFolders(defaultPath?: string): Promise<string[]> {
  const picked = await open({ directory: true, multiple: true, defaultPath });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

const EXPORT_FILTER = [{ name: "Tâches Chud Copy", extensions: ["chudcopy"] }];

export async function pickExportFile(name: string): Promise<string | null> {
  return save({ defaultPath: `${name}.chudcopy`, filters: EXPORT_FILTER });
}

export async function pickImportFile(): Promise<string | null> {
  const picked = await open({ multiple: false, filters: [...EXPORT_FILTER, { name: "JSON", extensions: ["json"] }] });
  return typeof picked === "string" ? picked : null;
}
