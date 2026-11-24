import { shortPath } from "./format";
import type { Endpoint, Link, Task } from "./types";

export function uid(): string {
  return crypto.randomUUID().replaceAll("-", "");
}

export function blankTask(): Task {
  return {
    schema: 1,
    id: "",
    name: "",
    description: "",
    sources: [],
    destinations: [],
    links: [],
    mode: "update",
    options: {
      emptyDirs: true,
      includeHidden: true,
      symlinks: "skip",
      preserveMetadata: true,
      compare: "sizeAndTime",
      skipNewer: false,
      verifyAfter: false,
      retries: 3,
      transfers: 4,
      multiThread: false,
      parallelVolumes: true,
      allowMerge: false,
    },
    filters: {
      includeFiles: [],
      excludeFiles: [],
      includeDirs: [],
      excludeDirs: [],
      includeExtensions: [],
      excludeExtensions: [],
      minSize: null,
      maxSize: null,
      newerThan: null,
      olderThan: null,
      excludeSystem: true,
      excludeJunk: false,
    },
    color: null,
    reminderDays: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

/** Nom de sous-dossier tiré d'une source, y compris pour une racine de disque. */
export function folderName(path: string): string {
  const name = shortPath(path).replace(/[:\\/]/g, "");
  return /^[A-Za-z]$/.test(name) ? `Disque_${name}` : name || "racine";
}

function uniqueName(base: string, taken: Set<string>): string {
  let name = base;
  let n = 2;
  while (taken.has(name.toLowerCase())) {
    name = `${base}_${n}`;
    n += 1;
  }
  taken.add(name.toLowerCase());
  return name;
}

/**
 * Sépare les sources qui partagent une destination : chacune reçoit son sous-dossier,
 * pour qu'aucune arborescence ne se mêle à une autre sans décision explicite.
 */
export function separateSharedDestinations(task: Task): Task {
  if (task.options.allowMerge) return task;
  const links = task.links.map((link) => ({ ...link }));
  for (const destination of task.destinations) {
    const shared = links.filter((l) => l.destinationId === destination.id);
    if (shared.length < 2) continue;
    const taken = new Set(shared.filter((l) => l.subfolder).map((l) => l.subfolder.toLowerCase()));
    for (const link of shared) {
      if (link.subfolder) continue;
      const source = task.sources.find((s) => s.id === link.sourceId);
      link.subfolder = uniqueName(source ? folderName(source.path) : "source", taken);
    }
  }
  return { ...task, links };
}

export function addLink(task: Task, sourceId: string, destinationId: string): Task {
  if (task.links.some((l) => l.sourceId === sourceId && l.destinationId === destinationId)) return task;
  const link: Link = { id: uid(), sourceId, destinationId, subfolder: "", enabled: true };
  return separateSharedDestinations({ ...task, links: [...task.links, link] });
}

export function removeLink(task: Task, linkId: string): Task {
  return { ...task, links: task.links.filter((l) => l.id !== linkId) };
}

export function duplicateLink(task: Task, linkId: string): Task {
  const original = task.links.find((l) => l.id === linkId);
  if (!original) return task;
  const taken = new Set(task.links.filter((l) => l.destinationId === original.destinationId).map((l) => l.subfolder.toLowerCase()));
  const source = task.sources.find((s) => s.id === original.sourceId);
  const copy: Link = { ...original, id: uid(), subfolder: uniqueName(`${source ? folderName(source.path) : "copie"}_copie`, taken) };
  return { ...task, links: [...task.links, copy] };
}

function endpoints(existing: Endpoint[], paths: string[]): Endpoint[] {
  const added = paths.filter((p) => !existing.some((e) => e.path === p)).map((path) => ({ id: uid(), path }));
  return [...existing, ...added];
}

/** Ajoute des sources et relie d'office ce qui ne laisse aucune ambiguïté. */
export function addSources(task: Task, paths: string[]): Task {
  const sources = endpoints(task.sources, paths);
  let next: Task = { ...task, sources };
  const fresh = sources.filter((s) => !task.sources.includes(s));
  if (next.destinations.length === 1) {
    for (const source of fresh) next = addLink(next, source.id, next.destinations[0].id);
  } else {
    next = pairFreeEndpoints(next);
  }
  return next;
}

export function addDestinations(task: Task, paths: string[]): Task {
  const destinations = endpoints(task.destinations, paths);
  return pairFreeEndpoints({ ...task, destinations });
}

/** Associe dans l'ordre les sources et les destinations encore libres. */
function pairFreeEndpoints(task: Task): Task {
  let next = task;
  const freeSources = next.sources.filter((s) => !next.links.some((l) => l.sourceId === s.id));
  const freeDestinations = next.destinations.filter((d) => !next.links.some((l) => l.destinationId === d.id));
  for (let i = 0; i < Math.min(freeSources.length, freeDestinations.length); i += 1) {
    next = addLink(next, freeSources[i].id, freeDestinations[i].id);
  }
  return next;
}

export function removeSource(task: Task, id: string): Task {
  return { ...task, sources: task.sources.filter((s) => s.id !== id), links: task.links.filter((l) => l.sourceId !== id) };
}

export function removeDestination(task: Task, id: string): Task {
  return { ...task, destinations: task.destinations.filter((d) => d.id !== id), links: task.links.filter((l) => l.destinationId !== id) };
}

export function joinTarget(root: string, subfolder: string): string {
  if (!subfolder.trim()) return root;
  const separator = root.includes("\\") ? "\\" : "/";
  return `${root.replace(/[\\/]+$/, "")}${separator}${subfolder.trim()}`;
}
