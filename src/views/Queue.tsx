import { ArrowDown, ArrowUp, Play, X } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { tintStyle } from "../components/ui";

export function QueuePanel() {
  const { queue, colors } = useApp();
  if (queue.items.length === 0) return null;
  return (
    <section className="panel queue">
      <h2>
        {t("queue.title")} <span className="count">{queue.items.length}</span>
      </h2>
      {queue.held && <p className="queue-held">{t("queue.held")}</p>}
      <ol className="queue-list">
        {queue.items.map((item, index) => (
          <li key={item.id} className={colors[item.taskId] ? "tinted-row" : undefined} style={tintStyle(colors[item.taskId])}>
            <span className="queue-rank">{index + 1}</span>
            <span className="queue-name">
              <b title={item.taskName}>{item.taskName}</b>
              <small>{fmt.modeLabel(item.mode)}</small>
              {item.error && <small className="danger-text">{describe(item.error)}</small>}
            </span>
            <button className="icon-btn" aria-label={t("queue.up")} title={t("queue.up")} disabled={index === 0} onClick={() => api.queueShift(item.id, true)}>
              <ArrowUp size={15} />
            </button>
            <button className="icon-btn" aria-label={t("queue.down")} title={t("queue.down")} disabled={index === queue.items.length - 1} onClick={() => api.queueShift(item.id, false)}>
              <ArrowDown size={15} />
            </button>
            <button className="icon-btn" aria-label={t("queue.remove")} title={t("queue.remove")} onClick={() => api.queueRemove(item.id)}>
              <X size={15} />
            </button>
          </li>
        ))}
      </ol>
      <p className="muted small">{t("queue.note")}</p>
      <div className="queue-actions">
        <button className="btn ghost small" onClick={() => api.queueClear()}>
          {t("queue.clear")}
        </button>
        {queue.held && (
          <button className="btn primary small" onClick={() => api.queueResume()}>
            <Play size={14} /> {t("queue.resume")}
          </button>
        )}
      </div>
    </section>
  );
}
