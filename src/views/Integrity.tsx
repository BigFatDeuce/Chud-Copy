import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Bot, CheckCheck, FileDown, ShieldCheck, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import { copyText, integrityPrompt } from "../lib/ai";
import { useApp } from "../lib/context";
import * as fmt from "../lib/format";
import { describe, t } from "../lib/i18n";
import { CHECK_KEY, IDLE, NO_CHECK, type CheckState } from "../lib/machines";
import { Modal } from "../components/ui";
import { JobBar, PathPicker, ResultHead } from "../components/tools";
import type { FindingKind, IntegrityReference } from "../lib/types";

const ORDER: FindingKind[] = ["corrupt", "unreadable", "missing", "modified", "added"];

export function IntegrityView() {
  const { toast, keep, progress, settings, saveSettings, go } = useApp();
  const [state, setState] = keep<CheckState>(CHECK_KEY, NO_CHECK);
  const [references, setReferences] = useState<IntegrityReference[]>([]);
  const [filter, setFilter] = useState<FindingKind | null>(null);
  const [forgetting, setForgetting] = useState<IntegrityReference | null>(null);
  const { root, job } = state;
  const running = job.phase === "running";
  const report = job.phase === "done" ? job.result : null;
  const reference = references.find((r) => r.root === root);

  const refresh = () => api.integrityReferences().then(setReferences).catch(() => {});
  useEffect(() => {
    refresh();
  }, [report]);

  const choose = (path: string) => {
    if (running) return;
    setState({ root: path, job: IDLE });
    setFilter(null);
  };

  const start = async () => {
    if (!root) return;
    const checked = root;
    setState({ root: checked, job: { phase: "running" } });
    try {
      const result = await api.integrityStart(checked);
      // Un contrôle arrêté rend un rapport partiel, affiché tant que le même dossier est choisi.
      setState((current) => (current.root === checked ? { root: checked, job: { phase: "done", result } } : current));
    } catch (error) {
      setState((current) => (current.root === checked && current.job.phase === "running" ? { root: checked, job: IDLE } : current));
      toast(describe(String(error)), "bad");
    }
  };

  // Retirer un contrôle en cours l'arrête ; retirer un résultat le fait seulement disparaître de l'écran.
  const dismiss = () => {
    if (running && root) api.integrityCancel(root).catch(() => {});
    setState({ root, job: IDLE });
  };

  const accept = async () => {
    if (!root) return;
    try {
      await api.integrityAccept(root);
    } catch (error) {
      toast(describe(String(error)), "bad");
      return;
    }
    setState((current) => (current.job.phase === "done" ? { ...current, job: { phase: "done", result: { ...current.job.result, saved: true } } } : current));
    toast(t("integrity.accepted"));
  };

  const forget = async (target: IntegrityReference) => {
    setForgetting(null);
    try {
      await api.integrityForget(target.root);
    } catch (error) {
      toast(describe(String(error)), "bad");
      return;
    }
    saveSettings({ ...settings, recentPaths: (settings.recentPaths ?? []).filter((p) => p !== target.root) });
    if (target.root === root) setState({ root, job: IDLE });
    refresh();
    toast(t("integrity.forgotten"));
  };

  const exportSums = async () => {
    if (!root) return;
    const target = await save({ defaultPath: "chudcopy.md5", filters: [{ name: "MD5", extensions: ["md5"] }] });
    if (!target) return;
    try {
      toast(t("integrity.exported", { n: fmt.count(await api.integrityExport(root, target)) }));
    } catch (error) {
      toast(describe(String(error)), "bad");
    }
  };

  const askAi = async () => {
    if (!report) return;
    try {
      await copyText(integrityPrompt(report));
      toast(t("ai.copied"));
    } catch {
      toast(t("ai.copyFailed"), "bad");
    }
  };

  const findings = report?.findings.filter((f) => !filter || f.kind === filter) ?? [];
  const problems = report ? report.counts.corrupt + report.counts.unreadable : 0;

  return (
    <div className="view tool">
      <header className="view-head">
        <div>
          <button className="crumb" onClick={() => go({ name: "tools" })}>
            <ArrowLeft size={15} /> {t("nav.tools")}
          </button>
          <h1>{t("nav.integrity")}</h1>
          <p>{t("integrity.lead")}</p>
        </div>
      </header>

      <section className="panel tool-target">
        <h2>{t("integrity.target")}</h2>
        <PathPicker value={root} onPick={choose} recent={references.map((r) => r.root)} disabled={running} />
        {root && (
          <div className="tool-row">
            <span className="muted small">{reference ? t("integrity.baseline", { when: fmt.date(reference.createdAt) }) : t("integrity.noBaseline")}</span>
            {!running && (
              <button className="btn primary" onClick={start}>
                <ShieldCheck size={15} /> {reference ? t("integrity.verify") : t("integrity.create")}
              </button>
            )}
          </div>
        )}
        <p className="muted small">{t("integrity.safe")}</p>
      </section>

      {running && root && <JobBar title={t("integrity.reading")} progress={progress[`integrity:${root}`]} onStop={dismiss} />}

      {report && (
        <>
          <section className={problems ? "panel verdict bad" : "panel verdict ok"}>
            <ResultHead title={report.cancelled ? t("integrity.stopped") : problems ? t("integrity.problems", { n: fmt.count(problems) }) : report.baseline ? t("integrity.clean") : t("integrity.first")} onDismiss={dismiss} />
            <p className="muted">{t("integrity.summary", { n: fmt.count(report.files), size: fmt.bytes(report.bytes), time: fmt.duration(report.elapsed) })}</p>
            <div className="tiles">
              {ORDER.map((kind) => (
                <button key={kind} className={`tile ${kind === "corrupt" || kind === "unreadable" ? "bad" : ""} ${filter === kind ? "active" : ""}`} disabled={report.counts[kind] === 0} onClick={() => setFilter(filter === kind ? null : kind)}>
                  <span className="tile-value">{fmt.count(report.counts[kind])}</span>
                  <span className="tile-label">{t(`finding.${kind}`)}</span>
                </button>
              ))}
              <div className="tile">
                <span className="tile-value">{fmt.count(report.counts.intact)}</span>
                <span className="tile-label">{t("finding.intact")}</span>
              </div>
            </div>
            <div className="tool-actions">
              {!report.saved && !report.cancelled && (
                <button className="btn ghost" onClick={accept}>
                  <CheckCheck size={15} /> {t("integrity.accept")}
                </button>
              )}
              <button className="btn ghost" onClick={exportSums}>
                <FileDown size={15} /> {t("integrity.export")}
              </button>
              <button className="btn primary" onClick={askAi}>
                <Bot size={15} /> {t("ai.button")}
              </button>
            </div>
            {!report.saved && !report.cancelled && <p className="muted small">{t("integrity.acceptHint")}</p>}
          </section>

          {findings.length > 0 && (
            <section className="panel">
              <ul className="finding-list">
                {findings.slice(0, 500).map((finding) => (
                  <li key={`${finding.kind}:${finding.rel}`} className={finding.kind}>
                    <b>{t(`finding.${finding.kind}`)}</b>
                    <code className="file-path">{finding.rel}</code>
                    {finding.detail && <small>{finding.detail}</small>}
                  </li>
                ))}
              </ul>
            </section>
          )}
        </>
      )}

      {references.length > 0 && (
        <section className="panel">
          <h2>{t("integrity.references")}</h2>
          <ul className="reference-list">
            {references.map((item) => (
              <li key={item.root}>
                <button className="reference-open" disabled={running} onClick={() => choose(item.root)}>
                  <code className="file-path" dir="ltr">
                    {item.root}
                  </code>
                  <small>{t("integrity.referenceInfo", { n: fmt.count(item.files), size: fmt.bytes(item.bytes), when: fmt.ago(item.createdAt) })}</small>
                </button>
                <button className="icon-btn" aria-label={t("integrity.forget")} title={t("integrity.forget")} disabled={running && item.root === root} onClick={() => setForgetting(item)}>
                  <Trash2 size={15} />
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {forgetting && (
        <Modal
          title={t("integrity.forgetTitle")}
          onClose={() => setForgetting(null)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setForgetting(null)}>
                {t("common.keep")}
              </button>
              <button className="btn danger" onClick={() => forget(forgetting)}>
                {t("integrity.forget")}
              </button>
            </>
          }
        >
          <p>{t("integrity.forgetBody", { path: forgetting.root })}</p>
        </Modal>
      )}
    </div>
  );
}
