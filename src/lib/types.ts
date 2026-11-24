// Miroir des structures Rust de `chud-core`.

import type { LanguageSetting } from "./i18n";

export type Mode = "update" | "full" | "addOnly" | "mirror" | "mirrorArchive";
export type Symlinks = "skip" | "follow" | "preserve";
export type Compare = "sizeAndTime" | "checksum" | "sizeOnly";

export interface Options {
  emptyDirs: boolean;
  includeHidden: boolean;
  symlinks: Symlinks;
  preserveMetadata: boolean;
  compare: Compare;
  skipNewer: boolean;
  verifyAfter: boolean;
  retries: number;
  transfers: number;
  multiThread: boolean;
  parallelVolumes: boolean;
  allowMerge: boolean;
}

export interface Filters {
  includeFiles: string[];
  excludeFiles: string[];
  includeDirs: string[];
  excludeDirs: string[];
  includeExtensions: string[];
  excludeExtensions: string[];
  minSize: number | null;
  maxSize: number | null;
  newerThan: string | null;
  olderThan: string | null;
  excludeSystem: boolean;
  excludeJunk: boolean;
}

export interface Endpoint {
  id: string;
  path: string;
  /** Le disque reconnu à cet endroit : une opération est refusée si un autre disque prend sa place. */
  identity?: DriveIdentity | null;
}

export interface DriveIdentity {
  id: string;
  label: string;
  fileSystem: string;
  mount: string;
  kind: "local" | "removable" | "network" | "unknown";
  total: number;
  learnedAt: number;
}

export type DriveCheck = { state: "unknown" } | { state: "same" } | { state: "absent" } | { state: "different"; found: DriveIdentity };

export interface DriveRow {
  taskId: string;
  taskName: string;
  endpointId: string;
  role: "source" | "destination";
  path: string;
  identity: DriveIdentity | null;
  check: DriveCheck;
}

export interface Link {
  id: string;
  sourceId: string;
  destinationId: string;
  subfolder: string;
  enabled: boolean;
}

export interface Task {
  schema: number;
  id: string;
  name: string;
  description: string;
  sources: Endpoint[];
  destinations: Endpoint[];
  links: Link[];
  mode: Mode;
  options: Options;
  filters: Filters;
  color: string | null;
  reminderDays: number | null;
  createdAt: number;
  updatedAt: number;
}

export interface BrokenFile {
  path: string;
  reason: string;
}

export interface TaskList {
  tasks: Task[];
  broken: BrokenFile[];
}

export type Severity = "error" | "warning" | "info";

export interface Issue {
  severity: Severity;
  code: string;
  params: Record<string, string>;
  linkId: string | null;
}

export type PathState = "ready" | "willBeCreated" | "missing" | "notADirectory" | "denied" | "unresponsive";

export interface LinkReport {
  id: string;
  source: string;
  target: string;
  sourceState: PathState;
  targetState: PathState;
  ready: boolean;
}

export interface Report {
  issues: Issue[];
  links: LinkReport[];
  runnable: boolean;
}

export interface Disk {
  name: string;
  mount: string;
  total: number;
  available: number;
  removable: boolean;
  fileSystem: string;
}

export interface FriendlyError {
  category: string;
  title: string;
  path: string | null;
  detail: string;
}

export type EntryKind = "copy" | "update" | "same" | "ignored" | "delete" | "extra" | "error" | "conflict" | "incomplete";

export interface Counts {
  copy: number;
  copyBytes: number;
  update: number;
  updateBytes: number;
  same: number;
  ignored: number;
  ignoredBytes: number;
  delete: number;
  deleteBytes: number;
  extra: number;
  errors: number;
  conflicts: number;
  incomplete: number;
}

export interface LinkPlan {
  linkId: string;
  source: string;
  target: string;
  analyzed: boolean;
  skipReason: string | null;
  counts: Counts;
  errors: FriendlyError[];
  failed: boolean;
  massiveDelete: boolean;
  sourceLooksEmpty: boolean;
  unseenFiles: number;
}

export interface SpaceCheck {
  mount: string;
  available: number;
  needed: number;
  enough: boolean;
}

export interface Plan {
  id: string;
  taskId: string;
  taskName: string;
  taskUpdatedAt: number;
  createdAt: number;
  totals: Counts;
  links: LinkPlan[];
  space: SpaceCheck[];
  issues: Issue[];
  cancelled: boolean;
}

export interface PlanProgress {
  linkId: string;
  listed: number;
  checks: number;
  totalChecks: number;
  checking: string[];
  finished: boolean;
}

export interface PlanQuery {
  linkId: string | null;
  kinds: EntryKind[];
  search: string;
  offset: number;
  limit: number;
}

export interface FileSide {
  size: number;
  modified: number | null;
}

export interface PlanRow {
  linkId: string;
  rel: string;
  kind: EntryKind;
  size: number | null;
  note: string | null;
  source: FileSide | null;
  target: FileSide | null;
}

export interface PlanPage {
  total: number;
  rows: PlanRow[];
}

export type RunStatus = "running" | "paused" | "cancelling" | "completed" | "completedWithErrors" | "failed" | "cancelled" | "interrupted";
export type LinkPhase = "waiting" | "copying" | "verifying" | "done" | "failed" | "cancelled" | "skipped";

export interface LinkFile {
  name: string;
  size: number;
  bytes: number;
  percentage: number;
  speed: number;
}

export interface LinkRecord {
  id: string;
  source: string;
  target: string;
  phase: LinkPhase;
  status: "completed" | "completedWithErrors" | "failed" | "cancelled" | null;
  bytes: number;
  totalBytes: number;
  transfers: number;
  totalTransfers: number;
  checks: number;
  totalChecks: number;
  deletes: number;
  errors: number;
  speed: number;
  averageSpeed: number;
  eta: number | null;
  elapsed: number;
  verifyChecked: number;
  verifyTotal: number;
  verifyFailures: number;
  current: LinkFile[];
  message: string | null;
}

export interface Totals {
  bytes: number;
  totalBytes: number;
  transfers: number;
  totalTransfers: number;
  deletes: number;
  errors: number;
  speed: number;
  averageSpeed: number;
  eta: number | null;
  totalKnown: boolean;
}

export interface RunError extends Partial<FriendlyError> {
  linkId: string;
  target: string;
}

export interface RunRecord {
  id: string;
  taskId: string;
  taskName: string;
  mode: Mode;
  status: RunStatus;
  startedAt: number;
  endedAt: number | null;
  elapsed: number;
  links: LinkRecord[];
  totals: Totals;
  errors: RunError[];
  errorsTotal: number;
}

export interface QueuedRun {
  id: string;
  taskId: string;
  taskName: string;
  mode: Mode;
  error: string | null;
}

export interface QueueState {
  items: QueuedRun[];
  held: boolean;
}

export interface LogPage {
  total: number;
  lines: string[];
}

export interface AppInfo {
  version: string;
  engine: string;
  engineVersion: string;
  dataDir: string;
  platform: string;
  /** Qui dessine la barre de titre : le bureau (« native ») ou l'application (« custom »). */
  chrome: "native" | "custom";
  interrupted: string[];
}

export type Quality = "off" | "eco" | "balanced" | "high";

export interface Settings {
  quality: Quality;
  language: LanguageSetting;
  /** Ordre choisi pour les tâches de l'accueil. */
  taskOrder?: string[];
  /** Masque chemins et noms de fichiers derrière une poussière, révélée au survol. */
  discreet?: boolean;
  /** Sons de fin d'opération. Actifs sauf refus explicite. */
  sounds?: boolean;
  /** Recherche une nouvelle version au lancement, au plus une fois par jour. */
  autoUpdate?: boolean;
  lastUpdateCheck?: number;
  /** Derniers dossiers choisis dans les outils. */
  recentPaths?: string[];
}

export interface LastRun {
  status: RunStatus;
  startedAt: number;
}

export type FindingKind = "corrupt" | "unreadable" | "modified" | "missing" | "added";

export interface IntegrityProgress {
  files: number;
  totalFiles: number;
  bytes: number;
  totalBytes: number;
  current: string;
}

export interface IntegrityReport {
  root: string;
  startedAt: number;
  elapsed: number;
  files: number;
  bytes: number;
  baseline: number | null;
  counts: Record<FindingKind | "intact", number>;
  findings: { kind: FindingKind; rel: string; detail: string | null }[];
  cancelled: boolean;
  saved: boolean;
}

export type Verdict = "healthy" | "warning" | "failing" | "unknown";

export interface DiskHealth {
  name: string;
  model: string;
  serial: string;
  kind: string;
  size: number;
  verdict: Verdict | null;
  temperature: number | null;
  powerOnHours: number | null;
  powerCycles: number | null;
  reallocated: number | null;
  pending: number | null;
  uncorrectable: number | null;
  crcErrors: number | null;
  wearUsed: number | null;
  mediaErrors: number | null;
  commandTimeouts: number | null;
  errorLog: number | null;
  errorsRecent: number | null;
  previousAt: number | null;
  changes: { field: string; before: number; after: number }[];
  notes: string[];
  raw: unknown;
}

export interface IntegrityReference {
  root: string;
  createdAt: number;
  files: number;
  bytes: number;
}

export interface Side {
  rel: string;
  size: number;
  modified: number;
}

export interface CompareReport {
  left: string;
  right: string;
  content: boolean;
  same: number;
  onlyLeftCount: number;
  onlyRightCount: number;
  differentCount: number;
  onlyLeft: Side[];
  onlyRight: Side[];
  different: { rel: string; left: Side; right: Side; reason: "size" | "date" | "content" }[];
  errors: [string, string][];
  cancelled: boolean;
  elapsed: number;
}

export interface DuplicateGroup {
  size: number;
  md5: string;
  files: string[];
  /** Date de chaque fichier lors de l'analyse, dans l'ordre de `files`. */
  modified?: number[];
}

/** Fichiers retirés, et corbeilles Chud Copy utilisées là où le système n'en a pas. */
export interface Recycled {
  moved: string[];
  holding: string[];
}

export interface DuplicateReport {
  roots: string[];
  scanned: number;
  groups: DuplicateGroup[];
  wasted: number;
  cancelled: boolean;
  elapsed: number;
}

export interface SpaceNode {
  name: string;
  size: number;
  files: number;
  leaf: boolean;
  /** Regroupement des petits éléments : il n'existe pas sur le disque. */
  grouped: boolean;
  children: SpaceNode[];
}

export interface ReadReport {
  root: string;
  files: number;
  bytes: number;
  elapsed: number;
  averageSpeed: number;
  samples: number[];
  slow: { rel: string; size: number; speed: number }[];
  unreadable: [string, string][];
  cancelled: boolean;
}

export interface ArchiveVersion {
  archive: string;
  stamp: string;
  files: number;
  bytes: number;
}

export interface ArchivedFile {
  rel: string;
  size: number;
  modified: number;
}

export interface HealthReport {
  tool: string;
  disks: DiskHealth[];
  missingTool: boolean;
  limited: boolean;
}

export type CleanupKind = "installer" | "superseded" | "temp" | "partial" | "download";

export interface CleanupItem {
  path: string;
  kind: CleanupKind;
  size: number;
  /** Secondes depuis 1970. */
  modified: number;
}

export interface BigFolder {
  path: string;
  size: number;
  files: number;
  newest: number;
}

export interface CleanupReport {
  roots: string[];
  scanned: number;
  scannedBytes: number;
  items: CleanupItem[];
  folders: BigFolder[];
  cancelled: boolean;
  elapsed: number;
}

export interface CleanupOptions {
  installerDays: number;
  tempDays: number;
  downloadDays: number;
  bigFolder: number;
}

export interface CleanupPlace {
  kind: "downloads" | "temp";
  path: string;
}

export interface TrashOutcome {
  trashed: string[];
  skipped: string[];
  bytes: number;
  holding: string[];
}

export interface Latency {
  samples: number[];
  failures: number;
  min: number;
  average: number;
  max: number;
  jitter: number;
}

export interface Transfer {
  bytes: number;
  seconds: number;
  speed: number;
  samples: number[];
}

export interface NetworkReport {
  path: string;
  kind: "smb" | "nfs" | "sftp" | "network" | "local";
  host: string | null;
  port: number | null;
  address: string | null;
  resolveMs: number | null;
  connect: Latency | null;
  reachable: boolean;
  readable: boolean;
  entries: number;
  listMs: number;
  metadata: Latency;
  read: Transfer | null;
  readFromTest: boolean;
  writable: boolean | null;
  write: Transfer | null;
  smallFilesPerSecond: number | null;
  verified: boolean | null;
  cleaned: boolean | null;
  error: string | null;
  cancelled: boolean;
  elapsed: number;
}
