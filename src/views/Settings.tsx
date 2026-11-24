import { useState } from "react";
import { Download, FolderOpen, RefreshCw } from "lucide-react";
import { Select, Toggle } from "../components/ui";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import { LANGUAGES, t, type LanguageSetting } from "../lib/i18n";
import { fetchLatestVersion, isNewer } from "../lib/updates";
import type { Quality } from "../lib/types";

const QUALITIES: Quality[] = ["off", "eco", "balanced", "high"];

type UpdateState = { step: "idle" | "checking" | "latest" | "failed" } | { step: "available"; version: string };

function Updates({ current }: { current: string }) {
  const { settings, saveSettings } = useApp();
  const [state, setState] = useState<UpdateState>({ step: "idle" });

  const check = async () => {
    setState({ step: "checking" });
    try {
      const version = await fetchLatestVersion();
      setState(isNewer(version, current) ? { step: "available", version } : { step: "latest" });
    } catch {
      setState({ step: "failed" });
    }
  };

  return (
    <section className="panel">
      <h2>{t("update.section")}</h2>
      <div className="update-row">
        <button className="btn ghost" disabled={state.step === "checking" || !current} onClick={check}>
          <RefreshCw size={15} /> {state.step === "checking" ? t("update.checking") : t("update.check")}
        </button>
        {state.step === "latest" && <span className="ok-text">{t("update.latest", { version: current })}</span>}
        {state.step === "failed" && <span className="danger-text">{t("update.failed")}</span>}
        {state.step === "available" && (
          <>
            <span className="update-new">{t("update.available", { version: state.version })}</span>
            <button className="btn primary small" onClick={() => api.releasesOpen()}>
              <Download size={14} /> {t("update.download")}
            </button>
          </>
        )}
      </div>
      <Toggle checked={!!settings.autoUpdate} onChange={(v) => saveSettings({ ...settings, autoUpdate: v })} label={t("update.auto")} hint={t("update.autoHint")} />
      <p className="muted small">{t("update.privacy")}</p>
    </section>
  );
}

export function SettingsView() {
  const { settings, saveSettings, info, cosmosFailed } = useApp();
  return (
    <div className="view settings">
      <header className="view-head">
        <div>
          <h1>{t("settings.title")}</h1>
        </div>
      </header>

      <section className="panel">
        <h2>{t("settings.language")}</h2>
        <label className="field language">
          <Select<LanguageSetting>
            label={t("settings.language")}
            value={settings.language}
            onChange={(language) => saveSettings({ ...settings, language })}
            choices={[{ value: "auto", label: t("settings.languageAuto") }, ...LANGUAGES.map((language) => ({ value: language.id, label: language.name }))]}
          />
        </label>
      </section>

      <section className="panel">
        <h2>{t("settings.animations")}</h2>
        {cosmosFailed && <p className="muted small">{t("settings.noWebgl")}</p>}
        <div className="quality" role="radiogroup" aria-label={t("settings.qualityAria")}>
          {QUALITIES.map((id) => (
            <button key={id} role="radio" aria-checked={settings.quality === id} disabled={cosmosFailed} className={settings.quality === id ? "selected" : ""} onClick={() => saveSettings({ ...settings, quality: id })}>
              <span>{t(`quality.${id}`)}</span>
              <small>{t(`quality.${id}Hint`)}</small>
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <h2>{t("settings.sounds")}</h2>
        <Toggle checked={settings.sounds !== false} onChange={(v) => saveSettings({ ...settings, sounds: v })} label={t("settings.soundsLabel")} hint={t("settings.soundsHint")} />
      </section>

      <Updates current={info?.version ?? ""} />

      <section className="panel">
        <h2>{t("settings.data")}</h2>
        <p className="muted small">{t("settings.dataHelp")}</p>
        <div className="data-row">
          <code dir="ltr">{info?.dataDir}</code>
          <button className="icon-btn labelled" onClick={() => api.dataReveal()}>
            <FolderOpen size={15} /> {t("settings.open")}
          </button>
        </div>
      </section>

      <section className="panel">
        <h2>{t("settings.about")}</h2>
        <dl className="about">
          <div>
            <dt>Chud Copy</dt>
            <dd>{info?.version}</dd>
          </div>
          <div>
            <dt>{t("settings.engine")}</dt>
            <dd dir="ltr">{info?.engineVersion}</dd>
          </div>
        </dl>
        <p className="muted small">{t("settings.credits")}</p>
      </section>
    </div>
  );
}
