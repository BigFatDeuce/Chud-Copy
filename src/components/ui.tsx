import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Ban, ChevronDown, HardDrive, Usb, X } from "lucide-react";
import iconUrl from "../assets/logo.svg";
import * as fmt from "../lib/format";
import { t } from "../lib/i18n";
import type { Disk, Task } from "../lib/types";

export function Mark({ size = 24 }: { size?: number }) {
  return <img className="mark" src={iconUrl} width={size} height={size} alt="" draggable={false} />;
}

/** Couleurs proposées pour le cadre des tâches, accordées au ciel de l'application. */
export const TASK_COLORS = ["#4fe3ff", "#3a7bff", "#9a6bff", "#ff4f82", "#ff8a3d", "#ffc56b", "#3ddc97", "#c9d4ff"];

/** Variables de teinte d'une tâche : le cadre garde sa transparence, seule sa couleur change. */
export function tintStyle(color?: string | null): CSSProperties | undefined {
  return color ? ({ "--task-color": color } as CSSProperties) : undefined;
}

export function ColorSwatches({ value, onPick }: { value: string | null; onPick: (color: string | null) => void }) {
  const custom = value && !TASK_COLORS.includes(value.toLowerCase()) ? value : null;
  return (
    <div className="swatches">
      <button type="button" className={value ? "swatch none" : "swatch none selected"} aria-label={t("color.none")} title={t("color.none")} aria-pressed={!value} onClick={() => onPick(null)}>
        <Ban size={13} />
      </button>
      {TASK_COLORS.map((color) => (
        <button
          type="button"
          key={color}
          className={value?.toLowerCase() === color ? "swatch selected" : "swatch"}
          style={{ "--swatch": color } as CSSProperties}
          aria-label={color}
          aria-pressed={value?.toLowerCase() === color}
          onClick={() => onPick(color)}
        />
      ))}
      <label className={custom ? "swatch custom selected" : "swatch custom"} title={t("color.custom")} style={custom ? ({ "--swatch": custom } as CSSProperties) : undefined}>
        <input type="color" aria-label={t("color.custom")} value={value ?? "#4fe3ff"} onChange={(e) => onPick(e.target.value)} />
      </label>
    </div>
  );
}

export function Modal({ title, children, actions, onClose, tone }: { title: string; children: ReactNode; actions?: ReactNode; onClose: () => void; tone?: "danger" }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  // Rendue à la racine de la page : une fenêtre ouverte depuis une carte passe devant toutes les autres.
  return createPortal(
    <div className="modal-veil" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <section className={tone === "danger" ? "modal danger" : "modal"} role="dialog" aria-modal="true" aria-label={title}>
        <header>
          <h2>{title}</h2>
          <button className="icon-btn" aria-label={t("common.close")} onClick={onClose}>
            <X size={17} />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {actions && <footer>{actions}</footer>}
      </section>
    </div>,
    document.body,
  );
}

export interface Choice<T> {
  value: T;
  label: string;
}

const LIST_GAP = 4;
const LIST_MARGIN = 8;
const LIST_MAX = 420;

/**
 * Liste déroulante dessinée par l'application. Celle du système est peinte par le bureau, hors de
 * portée des styles : sous Linux (WebKitGTK) elle sortait en texte clair sur fond blanc, illisible.
 * Clavier : flèches, Début, Fin, Entrée ou Espace pour choisir, Échap pour refermer, une lettre
 * pour sauter au choix qui commence par elle.
 */
export function Select<T extends string | number>({ value, choices, onChange, disabled, label }: { value: T; choices: Choice<T>[]; onChange: (value: T) => void; disabled?: boolean; label?: string }) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  // Entrée et Espace sont traitées à l'appui : le clic que le navigateur en déduit ensuite est ignoré.
  const keyed = useRef(false);
  const id = useId();
  const selected = choices.findIndex((choice) => choice.value === value);

  const show = () => {
    if (disabled || choices.length === 0) return;
    setActive(Math.max(0, selected));
    setOpen(true);
  };
  const pick = (index: number) => {
    setOpen(false);
    const choice = choices[index];
    if (choice && choice.value !== value) onChange(choice.value);
  };

  // La liste se place sous le bouton, ou au-dessus s'il manque de place, sans jamais sortir de la fenêtre.
  const place = () => {
    const menu = list.current;
    const anchor = trigger.current;
    if (!menu || !anchor) return;
    const box = anchor.getBoundingClientRect();
    const below = window.innerHeight - box.bottom - LIST_GAP - LIST_MARGIN;
    const above = box.top - LIST_GAP - LIST_MARGIN;
    menu.style.minWidth = `${box.width}px`;
    menu.style.maxHeight = `${LIST_MAX}px`;
    const up = menu.offsetHeight > below && above > below;
    menu.style.maxHeight = `${Math.max(96, Math.min(LIST_MAX, up ? above : below))}px`;
    const rtl = getComputedStyle(anchor).direction === "rtl";
    const left = rtl ? box.right - menu.offsetWidth : box.left;
    menu.style.left = `${Math.max(LIST_MARGIN, Math.min(left, window.innerWidth - menu.offsetWidth - LIST_MARGIN))}px`;
    menu.style.top = up ? "" : `${box.bottom + LIST_GAP}px`;
    menu.style.bottom = up ? `${window.innerHeight - box.top + LIST_GAP}px` : "";
  };

  useLayoutEffect(() => {
    if (open) place();
  }, [open]);

  useEffect(() => {
    if (open) list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [open, active]);

  // Un clic ailleurs referme la liste. Si la page défile ou change de taille, la liste suit son
  // bouton : la fermer la ferait disparaître au moindre mouvement de la molette.
  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null) => target instanceof Node && (!!list.current?.contains(target) || !!trigger.current?.contains(target));
    const onDown = (event: MouseEvent) => {
      if (!inside(event.target)) setOpen(false);
    };
    const onScroll = (event: Event) => {
      if (!(event.target instanceof Node && list.current?.contains(event.target))) place();
    };
    const close = () => setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", place);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", place);
      window.removeEventListener("blur", close);
    };
  }, [open]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    const last = choices.length - 1;
    const move = (index: number) => {
      event.preventDefault();
      if (open) setActive(Math.max(0, Math.min(last, index)));
      else show();
    };
    switch (event.key) {
      case "ArrowDown":
        return move(active + 1);
      case "ArrowUp":
        return move(active - 1);
      case "Home":
        return open ? move(0) : undefined;
      case "End":
        return open ? move(last) : undefined;
      case "Enter":
      case " ":
        event.preventDefault();
        keyed.current = true;
        return open ? pick(active) : show();
      case "Escape":
        if (!open) return;
        // Une fenêtre modale autour ne doit pas se fermer en même temps que la liste.
        event.stopPropagation();
        return setOpen(false);
      case "Tab":
        return setOpen(false);
    }
    if (event.key.length !== 1 || event.ctrlKey || event.metaKey || event.altKey) return;
    const letter = event.key.toLocaleLowerCase();
    const from = open ? active : selected;
    const order = choices.map((_, index) => (from + 1 + index) % choices.length);
    const found = order.find((index) => choices[index].label.toLocaleLowerCase().startsWith(letter));
    if (found === undefined) return;
    if (open) setActive(found);
    else pick(found);
  };

  return (
    <>
      <button
        type="button"
        ref={trigger}
        className="select"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-activedescendant={open ? `${id}-${active}` : undefined}
        aria-label={label}
        disabled={disabled}
        onClick={() => {
          if (keyed.current) return;
          if (open) setOpen(false);
          else show();
        }}
        onKeyDown={onKeyDown}
        onKeyUp={(event) => {
          if (event.key === " ") event.preventDefault();
          // Relâché après le clic éventuel, qui part pendant le traitement de cette touche.
          window.setTimeout(() => {
            keyed.current = false;
          });
        }}
        onBlur={() => {
          keyed.current = false;
        }}
      >
        <span className="select-value">{choices[selected]?.label ?? ""}</span>
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {open &&
        // Rendue à la racine de la page : aucun cadre ne la rogne, et elle passe devant tout le reste.
        createPortal(
          <ul className="select-list" role="listbox" id={id} ref={list} onMouseDown={(event) => event.preventDefault()}>
            {choices.map((choice, index) => (
              <li
                key={String(choice.value)}
                id={`${id}-${index}`}
                role="option"
                aria-selected={index === selected}
                className={index === active ? "active" : undefined}
                onMouseMove={() => index !== active && setActive(index)}
                onClick={() => pick(index)}
              >
                {choice.label}
              </li>
            ))}
          </ul>,
          document.body,
        )}
    </>
  );
}

export function Toggle({ checked, onChange, label, hint, disabled }: { checked: boolean; onChange: (value: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className={disabled ? "toggle disabled" : "toggle"}>
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="track" aria-hidden="true" />
      <span className="toggle-text">
        <span>{label}</span>
        {hint && <small>{hint}</small>}
      </span>
    </label>
  );
}

export function Chips({ values, onChange, placeholder, label }: { values: string[]; onChange: (values: string[]) => void; placeholder: string; label: string }) {
  const [draft, setDraft] = useState("");
  const id = useId();
  const commit = () => {
    const items = draft.split(/[,;]/).map((v) => v.trim()).filter(Boolean);
    if (items.length) onChange([...values, ...items.filter((v) => !values.includes(v))]);
    setDraft("");
  };
  return (
    <div className="chips-field">
      <label htmlFor={id}>{label}</label>
      <div className="chips">
        {values.map((value) => (
          <span key={value} className="chip">
            {value}
            <button aria-label={t("common.remove", { name: value })} onClick={() => onChange(values.filter((v) => v !== value))}>
              <X size={12} />
            </button>
          </span>
        ))}
        <input
          id={id}
          value={draft}
          placeholder={values.length ? "" : placeholder}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              commit();
            } else if (e.key === "Backspace" && !draft && values.length) {
              onChange(values.slice(0, -1));
            }
          }}
        />
      </div>
    </div>
  );
}

export function Beam({ value, tone, indeterminate }: { value: number; tone?: "ok" | "bad" | "warn" | "muted"; indeterminate?: boolean }) {
  return (
    <div className={`beam ${tone ?? ""} ${indeterminate ? "unknown" : ""}`} role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={indeterminate ? undefined : Math.round(value * 100)}>
      {!indeterminate && <div className="beam-fill" style={{ transform: `scaleX(${Math.max(0, Math.min(1, value))})` }} />}
    </div>
  );
}

export function DiskRow({ disk }: { disk: Disk }) {
  const used = disk.total > 0 ? (disk.total - disk.available) / disk.total : 0;
  const tone = used > 0.95 ? "bad" : used > 0.85 ? "warn" : undefined;
  const Icon = disk.removable ? Usb : HardDrive;
  return (
    <div className="disk">
      <Icon size={16} />
      <div className="disk-text">
        <div className="disk-line">
          <span className="disk-name">{disk.mount}</span>
          <span className="disk-free">{t("disk.freeOf", { size: fmt.bytes(disk.available), total: fmt.bytes(disk.total) })}</span>
        </div>
        <Beam value={used} tone={tone} />
      </div>
    </div>
  );
}

function truncate(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/** Le disque compte autant que le dossier : `D: Photos` et `G: Photos` ne sont pas la même chose. */
function nodeLabel(path: string): string {
  const drive = /^([A-Za-z]):/.exec(path);
  if (!drive) return fmt.shortPath(path);
  const name = fmt.shortPath(path.slice(2));
  return /^[\\/]*$/.test(path.slice(2)) ? `${drive[1]}:` : `${drive[1]}: ${name}`;
}

const VISIBLE = 4;

/** Petite carte des liaisons d'une tâche, qui remplace une liste de chemins. */
export function RouteGlyph({ task, missing }: { task: Task; missing?: Set<string> }) {
  const sources = task.sources.slice(0, VISIBLE);
  const destinations = task.destinations.slice(0, VISIBLE);
  const rows = Math.max(sources.length, destinations.length, 1);
  const rowHeight = 22;
  const height = rows * rowHeight + 2;
  const y = (index: number, total: number) => (height - total * rowHeight) / 2 + index * rowHeight + rowHeight / 2;
  const leftX = 126;
  const rightX = 234;
  const id = useId();
  return (
    <svg className="glyph" viewBox={`0 0 360 ${height}`} role="img" aria-label={t("glyph.aria", { s: task.sources.length, d: task.destinations.length })}>
      <defs>
        {/* En unités de l'espace SVG : une liaison droite a une boîte de hauteur nulle, où un dégradé relatif ne se dessine pas. */}
        <linearGradient id={id} gradientUnits="userSpaceOnUse" x1={leftX} y1="0" x2={rightX} y2="0">
          <stop offset="0" stopColor="#3fdcff" />
          <stop offset="1" stopColor="#9a6bff" />
        </linearGradient>
      </defs>
      {task.links.map((link) => {
        const from = sources.findIndex((s) => s.id === link.sourceId);
        const to = destinations.findIndex((d) => d.id === link.destinationId);
        if (from < 0 || to < 0) return null;
        const y1 = y(from, sources.length);
        const y2 = y(to, destinations.length);
        return <path key={link.id} d={`M ${leftX} ${y1} C ${leftX + 54} ${y1}, ${rightX - 54} ${y2}, ${rightX} ${y2}`} fill="none" stroke={`url(#${id})`} strokeWidth="1.4" opacity={link.enabled ? 0.9 : 0.25} />;
      })}
      {sources.map((source, i) => (
        <g key={source.id} className={missing?.has(source.path) ? "absent" : undefined}>
          <text x={leftX - 10} y={y(i, sources.length) + 4} textAnchor="end">
            {truncate(nodeLabel(source.path), 18)}
          </text>
          <circle cx={leftX} cy={y(i, sources.length)} r="3.2" />
        </g>
      ))}
      {destinations.map((destination, i) => (
        <g key={destination.id} className={missing?.has(destination.path) ? "absent" : undefined}>
          <circle cx={rightX} cy={y(i, destinations.length)} r="3.2" className="end" />
          <text x={rightX + 10} y={y(i, destinations.length) + 4}>
            {truncate(nodeLabel(destination.path), 18)}
          </text>
        </g>
      ))}
      {task.sources.length > VISIBLE && (
        <text x="0" y={height - 2} className="more">
          +{task.sources.length - VISIBLE}
        </text>
      )}
    </svg>
  );
}
