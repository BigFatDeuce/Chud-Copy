import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { CopyPlus, FolderPlus, ListTree, Power, Trash2, X } from "lucide-react";
import { pickFolders } from "../lib/api";
import * as t from "../lib/tasks";
import * as fmt from "../lib/format";
import { t as tr } from "../lib/i18n";
import { Select } from "../components/ui";
import type { Disk, Link, PathState, Report, Task } from "../lib/types";

type Side = "source" | "destination";

interface Curve {
  id: string;
  d: string;
  enabled: boolean;
  bad: boolean;
}

function bezier(x1: number, y1: number, x2: number, y2: number): string {
  const bend = Math.max(36, (x2 - x1) * 0.55);
  return `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`;
}

const LABELLED_STATES: PathState[] = ["missing", "denied", "notADirectory", "unresponsive", "willBeCreated"];

export function RouteMap({ task, report, disks, onChange }: { task: Task; report?: Report; disks: Disk[]; onChange: (task: Task) => void }) {
  const stage = useRef<HTMLDivElement>(null);
  const [curves, setCurves] = useState<Curve[]>([]);
  const [armed, setArmed] = useState<{ side: Side; id: string } | null>(null);
  const [rubber, setRubber] = useState<string | null>(null);
  const [dropSide, setDropSide] = useState<Side | null>(null);
  const taskRef = useRef(task);
  taskRef.current = task;

  const portCenter = useCallback((side: Side, id: string) => {
    const root = stage.current;
    const port = root?.querySelector<HTMLElement>(`[data-port="${side}:${id}"]`);
    if (!root || !port) return null;
    const a = root.getBoundingClientRect();
    const b = port.getBoundingClientRect();
    return { x: b.left + b.width / 2 - a.left, y: b.top + b.height / 2 - a.top };
  }, []);

  const measure = useCallback(() => {
    const next: Curve[] = [];
    for (const link of task.links) {
      const from = portCenter("source", link.sourceId);
      const to = portCenter("destination", link.destinationId);
      if (!from || !to) continue;
      const state = report?.links.find((l) => l.id === link.id);
      next.push({ id: link.id, d: bezier(from.x, from.y, to.x, to.y), enabled: link.enabled, bad: state ? !state.ready && link.enabled : false });
    }
    setCurves(next);
  }, [task.links, report, portCenter]);

  useLayoutEffect(measure, [measure, task.sources, task.destinations]);

  useEffect(() => {
    if (!stage.current) return;
    const observer = new ResizeObserver(measure);
    observer.observe(stage.current);
    return () => observer.disconnect();
  }, [measure]);

  // Les dossiers glissés depuis l'explorateur arrivent par la fenêtre native.
  useEffect(() => {
    const sideAt = (x: number, y: number): Side | null => {
      const scale = window.devicePixelRatio || 1;
      const element = document.elementFromPoint(x / scale, y / scale);
      const zone = element?.closest<HTMLElement>("[data-drop]");
      return (zone?.dataset.drop as Side) ?? null;
    };
    const stop = getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "over") setDropSide(sideAt(payload.position.x, payload.position.y));
      else if (payload.type === "leave") setDropSide(null);
      else if (payload.type === "drop") {
        const side = sideAt(payload.position.x, payload.position.y) ?? "source";
        setDropSide(null);
        onChange(side === "source" ? t.addSources(taskRef.current, payload.paths) : t.addDestinations(taskRef.current, payload.paths));
      }
    });
    return () => {
      stop.then((fn) => fn());
    };
  }, [onChange]);

  const connect = (side: Side, id: string) => {
    if (!armed || armed.side === side) {
      setArmed(armed?.id === id ? null : { side, id });
      return;
    }
    const sourceId = side === "source" ? id : armed.id;
    const destinationId = side === "destination" ? id : armed.id;
    const existing = task.links.find((l) => l.sourceId === sourceId && l.destinationId === destinationId);
    onChange(existing ? t.removeLink(task, existing.id) : t.addLink(task, sourceId, destinationId));
    setArmed(null);
  };

  const startDrag = (event: React.PointerEvent, side: Side, id: string) => {
    const root = stage.current;
    const origin = portCenter(side, id);
    if (!root || !origin) return;
    const startX = event.clientX;
    const startY = event.clientY;
    let dragged = false;
    const move = (e: PointerEvent) => {
      if (Math.abs(e.clientX - startX) + Math.abs(e.clientY - startY) > 6) dragged = true;
      if (!dragged) return;
      const bounds = root.getBoundingClientRect();
      const x = e.clientX - bounds.left;
      const y = e.clientY - bounds.top;
      setRubber(side === "source" ? bezier(origin.x, origin.y, x, y) : bezier(x, y, origin.x, origin.y));
    };
    const up = (e: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setRubber(null);
      if (!dragged) {
        connect(side, id);
        return;
      }
      const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-endpoint]");
      if (!hit) return;
      const [otherSide, otherId] = hit.dataset.endpoint!.split(":") as [Side, string];
      if (otherSide === side) return;
      const sourceId = side === "source" ? id : otherId;
      const destinationId = side === "destination" ? id : otherId;
      onChange(t.addLink(taskRef.current, sourceId, destinationId));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const add = async (side: Side, defaultPath?: string) => {
    const paths = await pickFolders(defaultPath);
    if (paths.length) onChange(side === "source" ? t.addSources(task, paths) : t.addDestinations(task, paths));
  };

  const stateOf = (side: Side, id: string): PathState | undefined => {
    const links = task.links.filter((l) => (side === "source" ? l.sourceId : l.destinationId) === id);
    for (const link of links) {
      const state = report?.links.find((l) => l.id === link.id);
      if (state) return side === "source" ? state.sourceState : state.targetState;
    }
    return undefined;
  };

  const column = (side: Side) => {
    const items = side === "source" ? task.sources : task.destinations;
    const title = side === "source" ? tr("route.sources") : tr("route.destinations");
    const remove = side === "source" ? t.removeSource : t.removeDestination;
    return (
      <div className={`route-column ${side} ${dropSide === side ? "drop" : ""}`} data-drop={side}>
        <header>
          <h3>{title}</h3>
          <button className="icon-btn labelled" onClick={() => add(side)}>
            <FolderPlus size={15} /> {tr("route.add")}
          </button>
        </header>
        <ul>
          {items.map((item) => {
            const state = stateOf(side, item.id);
            const label = state && LABELLED_STATES.includes(state) ? tr(`state.${state}`) : undefined;
            const isArmed = armed?.side === side && armed.id === item.id;
            return (
              <li key={item.id} data-endpoint={`${side}:${item.id}`} className={isArmed ? "endpoint armed" : "endpoint"}>
                {side === "destination" && <Port side={side} id={item.id} armed={isArmed} onPointerDown={startDrag} />}
                <div className="endpoint-text">
                  <span className="endpoint-name">{fmt.shortPath(item.path)}</span>
                  <span className="endpoint-path" title={item.path}>
                    {fmt.ellipsize(item.path, 44)}
                  </span>
                  {label && <span className={`endpoint-state ${state}`}>{label}</span>}
                </div>
                <button className="icon-btn subtle" aria-label={tr("common.remove", { name: item.path })} onClick={() => onChange(remove(task, item.id))}>
                  <X size={14} />
                </button>
                {side === "source" && <Port side={side} id={item.id} armed={isArmed} onPointerDown={startDrag} />}
              </li>
            );
          })}
        </ul>
        {items.length === 0 && (
          <button className="route-empty" onClick={() => add(side)}>
            <FolderPlus size={20} />
            <span>{tr("route.drop")}</span>
          </button>
        )}
      </div>
    );
  };

  return (
    <div className="route-map">
      <div className="route-stage" ref={stage} dir="ltr">
        <svg className="route-lines" aria-hidden="true">
          <defs>
            <linearGradient id="route" gradientUnits="userSpaceOnUse" x1="30%" y1="0" x2="70%" y2="0">
              <stop offset="0" stopColor="#3fdcff" />
              <stop offset="1" stopColor="#9a6bff" />
            </linearGradient>
          </defs>
          {curves.map((curve) => (
            <g key={curve.id} className={curve.bad ? "curve bad" : curve.enabled ? "curve" : "curve off"}>
              <path d={curve.d} className="halo" />
              <path d={curve.d} className="line" />
            </g>
          ))}
          {rubber && <path d={rubber} className="rubber" />}
        </svg>
        {column("source")}
        <div className="route-gap" />
        {column("destination")}
      </div>

      {armed && <p className="route-hint">{tr("route.hint")}</p>}

      {disks.length > 0 && (
        <div className="disk-chips" aria-label={tr("route.disks")}>
          {disks.map((disk) => (
            <div key={disk.mount} className="disk-chip">
              <span>{disk.mount}</span>
              <small>{fmt.bytes(disk.available)}</small>
              <button onClick={() => add("source", disk.mount)}>{tr("route.asSource")}</button>
              <button onClick={() => add("destination", disk.mount)}>{tr("route.asDestination")}</button>
            </div>
          ))}
        </div>
      )}

      {task.links.length > 0 && <LinkList task={task} report={report} onChange={onChange} />}
    </div>
  );
}

function Port({ side, id, armed, onPointerDown }: { side: Side; id: string; armed: boolean; onPointerDown: (e: React.PointerEvent, side: Side, id: string) => void }) {
  return <button className={armed ? "port armed" : "port"} data-port={`${side}:${id}`} aria-label={tr("route.connect")} onPointerDown={(e) => { e.preventDefault(); onPointerDown(e, side, id); }} />;
}

function LinkList({ task, report, onChange }: { task: Task; report?: Report; onChange: (task: Task) => void }) {
  const update = (id: string, patch: Partial<Link>) => onChange({ ...task, links: task.links.map((l) => (l.id === id ? { ...l, ...patch } : l)) });
  const issuesOf = (id: string) => report?.issues.filter((i) => i.linkId === id && i.severity === "error") ?? [];
  return (
    <section className="link-list">
      <h3>
        <ListTree size={15} /> {tr("route.links")}
      </h3>
      <p className="muted small">{tr("route.linksHint")}</p>
      {task.links.map((link) => {
        const source = task.sources.find((s) => s.id === link.sourceId);
        const destination = task.destinations.find((d) => d.id === link.destinationId);
        if (!source || !destination) return null;
        const blocked = issuesOf(link.id).length > 0;
        return (
          <div key={link.id} className={`link-row ${link.enabled ? "" : "off"} ${blocked ? "blocked" : ""}`}>
            <span className="link-source" title={source.path}>
              {fmt.shortPath(source.path)}
            </span>
            <span className="link-arrow" aria-hidden="true" />
            <Select<string>
              value={link.destinationId}
              label={tr("route.linkDestination")}
              onChange={(destinationId) => onChange(t.separateSharedDestinations({ ...task, links: task.links.map((l) => (l.id === link.id ? { ...l, destinationId } : l)) }))}
              choices={task.destinations.map((d) => ({ value: d.id, label: fmt.ellipsize(d.path, 46) }))}
            />
            <input className="link-sub" value={link.subfolder} placeholder={tr("route.subPlaceholder")} aria-label={tr("route.subLabel")} onChange={(e) => update(link.id, { subfolder: e.target.value })} />
            <code className="link-target" title={t.joinTarget(destination.path, link.subfolder)}>
              {fmt.ellipsize(t.joinTarget(destination.path, link.subfolder), 84)}
            </code>
            <div className="link-actions">
              <button className={link.enabled ? "icon-btn subtle" : "icon-btn subtle on"} aria-label={link.enabled ? tr("route.disable") : tr("route.enable")} title={link.enabled ? tr("route.disableShort") : tr("route.enableShort")} onClick={() => update(link.id, { enabled: !link.enabled })}>
                <Power size={14} />
              </button>
              <button className="icon-btn subtle" aria-label={tr("route.duplicate")} title={tr("route.duplicateShort")} onClick={() => onChange(t.duplicateLink(task, link.id))}>
                <CopyPlus size={14} />
              </button>
              <button className="icon-btn subtle" aria-label={tr("route.delete")} title={tr("route.deleteShort")} onClick={() => onChange(t.removeLink(task, link.id))}>
                <Trash2 size={14} />
              </button>
            </div>
          </div>
        );
      })}
    </section>
  );
}
