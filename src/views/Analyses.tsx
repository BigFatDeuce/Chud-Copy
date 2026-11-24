import { Eye, Square, X } from "lucide-react";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { t } from "../lib/i18n";
import { analysisPhase, analysisProgress, planVerdict, type AnalysisEntry, type AnalysisPhase } from "../lib/machines";
import { Beam, tintStyle } from "../components/ui";
import { Clouds } from "../effects/Clouds";

const TONE: Record<AnalysisPhase, string> = { running: "live", failed: "bad", stopped: "muted", ready: "ok" };
const LABEL: Record<AnalysisPhase, string> = { running: "status.running", failed: "status.failed", stopped: "status.cancelled", ready: "status.completed" };

/** La même phrase que le pied de l'analyse ouverte : un miroir bloqué n'est jamais annoncé « prêt ». */
function summary(entry: AnalysisEntry): string {
  if (entry.status === "failed") return entry.error;
  if (entry.status !== "done") return "";
  const { totals } = entry.plan;
  return t(`preview.${planVerdict(entry.plan, entry.mode)}`, { n: fmt.count(totals.copy + totals.update), d: fmt.count(totals.delete) });
}

function Row({ entry }: { entry: AnalysisEntry }) {
  const { go, stopAnalysis, forgetAnalysis, colors } = useApp();
  const color = colors[entry.taskId];
  const phase = analysisPhase(entry);
  const running = phase === "running";
  const { scanned, compared, toCompare, checking } = analysisProgress(entry);
  const current = checking[0];

  return (
    <article className={color ? "analysis-row tinted" : "analysis-row"} style={tintStyle(color)}>
      {running && <Clouds color={color} />}
      <div className="analysis-head">
        <div className="task-title">
          <h3>{entry.taskName}</h3>
          <span className={fmt.isDestructive(entry.mode) ? "tag danger" : "tag"}>{fmt.modeLabel(entry.mode)}</span>
        </div>
        <span className={`status-tag ${TONE[phase]}`}>{t(LABEL[phase])}</span>
      </div>
      {running ? (
        <>
          <Beam value={toCompare > 0 ? compared / toCompare : 0} indeterminate={toCompare === 0} />
          <p className="analysis-line">
            {toCompare > 0 ? t("preview.comparing", { done: fmt.count(compared), total: fmt.count(toCompare) }) : t("preview.scanned", { n: fmt.count(scanned) })}
          </p>
          {current && (
            <p className="analysis-file" title={current}>
              {current}
            </p>
          )}
        </>
      ) : (
        <p className="analysis-line">{summary(entry)}</p>
      )}
      <div className="analysis-actions">
        <button className="btn ghost small" onClick={() => go({ name: "preview", taskId: entry.taskId, open: true })}>
          <Eye size={14} /> {t("analyses.open")}
        </button>
        {running ? (
          <button className="btn ghost small" onClick={() => stopAnalysis(entry.taskId)}>
            <Square size={13} /> {t("preview.stop")}
          </button>
        ) : (
          <button className="btn ghost small" onClick={() => forgetAnalysis(entry.taskId)}>
            <X size={14} /> {t("analyses.remove")}
          </button>
        )}
      </div>
    </article>
  );
}

export function Analyses() {
  const { analyses } = useApp();
  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>{t("nav.analyses")}</h1>
          <p>{t("analyses.lead")}</p>
        </div>
      </header>
      {analyses.length === 0 && (
        <div className="empty">
          <p>{t("analyses.empty")}</p>
        </div>
      )}
      <div className="analysis-list">
        {analyses.map((entry) => (
          <Row key={entry.taskId} entry={entry} />
        ))}
      </div>
    </div>
  );
}
