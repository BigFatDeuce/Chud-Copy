import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Activity, Clock3, Eye, EyeOff, Home as HomeIcon, Minus, ScanSearch, Settings2, Square, Wrench, X, Zap } from "lucide-react";
import { api } from "./lib/api";
import { AppProvider, useApp, type Route } from "./lib/context";
import { t } from "./lib/i18n";
import { backgroundJobs, liveRun } from "./lib/machines";
import { Mark, Modal } from "./components/ui";
import { PowerCountdown } from "./components/power";
import { Home } from "./views/Home";
import { Editor } from "./views/Editor";
import { Preview } from "./views/Preview";
import { RunView } from "./views/Run";
import { History } from "./views/History";
import { Analyses } from "./views/Analyses";
import { SettingsView } from "./views/Settings";
import { IntegrityView } from "./views/Integrity";
import { HealthView } from "./views/Health";
import { ToolsView } from "./views/Tools";
import { IdleRain } from "./effects/IdleRain";
import { installDust, installGlitch } from "./effects/interface";
import { useMotion } from "./effects/motion";
import { Laser } from "./effects/Laser";

export function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}

function TitleBar() {
  const { info, settings, saveSettings, run, analyses } = useApp();
  const working = !!liveRun(run) || analyses.some((a) => a.status === "running");
  const discreet = !!settings.discreet;
  // Le cœur dit qui dessine les boutons de fenêtre. Avant sa réponse, on ne les montre pas là où
  // le bureau les fournit d'ordinaire : ils n'apparaîtraient que pour disparaître.
  const native = info ? info.chrome === "native" : /Linux/i.test(navigator.userAgent);
  const win = getCurrentWindow();
  return (
    <header className="titlebar" data-tauri-drag-region>
      <Laser active={working} />
      <div className="brand" data-tauri-drag-region>
        <Mark size={36} />
        <span data-tauri-drag-region>
          Chud <b>Copy</b>
        </span>
      </div>
      <div className="title-tools">
        <button
          className={discreet ? "censor on" : "censor"}
          aria-pressed={discreet}
          title={t("discreet.hint")}
          onClick={() => saveSettings({ ...settings, discreet: !discreet })}
        >
          <span className="censor-dust" aria-hidden="true" />
          {discreet ? <EyeOff size={14} /> : <Eye size={14} />}
          <span>{t("discreet.label")}</span>
        </button>
      </div>
      {!native && (
        <div className="window-controls">
          <button aria-label={t("win.minimize")} onClick={() => win.minimize()}>
            <Minus size={15} />
          </button>
          <button aria-label={t("win.maximize")} onClick={() => win.toggleMaximize()}>
            <Square size={12} />
          </button>
          <button aria-label={t("common.close")} className="close" onClick={() => win.close()}>
            <X size={16} />
          </button>
        </div>
      )}
    </header>
  );
}

function Rail() {
  const { route, go, run, queue, analyses, unseen, keep } = useApp();
  const analyzing = analyses.filter((a) => a.status === "running").length;
  const active = liveRun(run);
  const percent = active && active.totals.totalKnown && active.totals.totalBytes > 0 ? Math.min(100, Math.floor((active.totals.bytes / active.totals.totalBytes) * 100)) : null;
  const item = (name: Route["name"], label: string, icon: React.ReactNode, target: Route, badge = 0, attention = false) => (
    <button className={`rail-item ${route.name === name || (name === "tools" && route.name === "integrity") ? "active" : ""} ${attention ? "attention" : ""}`} onClick={() => go(target)} aria-current={route.name === name ? "page" : undefined}>
      {icon}
      <span>{label}</span>
      {badge > 0 && <em className="rail-badge">{badge}</em>}
    </button>
  );
  return (
    <nav className="rail" aria-label={t("nav.aria")}>
      {item("home", t("nav.home"), <HomeIcon size={19} />, { name: "home" })}
      {item("analyses", t("nav.analyses"), <ScanSearch size={19} />, { name: "analyses" }, analyzing || unseen.size, unseen.size > 0)}
      {item("history", t("nav.history"), <Clock3 size={19} />, { name: "history" })}
      <div className="rail-divider" />
      {item("health", t("nav.health"), <Activity size={19} />, { name: "health" })}
      {item("tools", t("nav.tools"), <Wrench size={19} />, { name: "tools" }, backgroundJobs(keep))}
      <div className="rail-grow" />
      {active && (
        <button className="rail-live" onClick={() => go({ name: "run" })}>
          <Zap size={17} />
          <span>{percent === null ? t("nav.running") : `${percent} %`}</span>
          {queue.items.length > 0 && <em className="rail-badge">{queue.items.length}</em>}
        </button>
      )}
      {item("settings", t("nav.settings"), <Settings2 size={19} />, { name: "settings" })}
    </nav>
  );
}

function CurrentView() {
  const { route } = useApp();
  switch (route.name) {
    case "home":
      return <Home />;
    case "editor":
      return <Editor key={route.task?.id ?? "new"} initial={route.task} />;
    case "preview":
      return <Preview key={route.taskId} taskId={route.taskId} open={route.open} />;
    case "analyses":
      return <Analyses />;
    case "run":
      return <RunView />;
    case "history":
      return <History key={route.runId ?? "list"} runId={route.runId} />;
    case "integrity":
      return <IntegrityView />;
    case "health":
      return <HealthView />;
    case "tools":
      return <ToolsView key={route.tool ?? "hub"} tool={route.tool} />;
    case "settings":
      return <SettingsView />;
  }
}

/** Filtre SVG du mode discret : le texte se défait en grains dispersés. */
function DustFilter() {
  return (
    <svg className="fx-defs" aria-hidden="true" focusable="false">
      <filter id="dust" x="-8%" y="-40%" width="116%" height="180%" colorInterpolationFilters="sRGB">
        <feTurbulence id="dust-scatter" type="fractalNoise" baseFrequency="0.9" numOctaves="1" seed="14" result="warp" />
        <feDisplacementMap in="SourceGraphic" in2="warp" scale="10" xChannelSelector="R" yChannelSelector="G" result="scattered" />
        <feTurbulence id="dust-grain" type="fractalNoise" baseFrequency="1.6" numOctaves="1" seed="1" result="grain" />
        <feColorMatrix in="grain" type="matrix" values="0 0 0 0 1  0 0 0 0 1  0 0 0 0 1  9 0 0 0 -4.6" result="mask" />
        <feComposite in="scattered" in2="mask" operator="in" result="dust" />
        <feColorMatrix in="dust" type="saturate" values="0.15" />
      </filter>
    </svg>
  );
}

function Shell() {
  const { toasts, cosmosFailed, language, queue, route, settings, cosmos, info } = useApp();
  const [closing, setClosing] = useState(false);
  const motion = useMotion();
  const previous = useRef(route.name);

  useEffect(() => installGlitch(), []);
  useEffect(() => installDust(), []);

  useEffect(() => {
    document.body.classList.toggle("motion", motion);
    document.body.classList.toggle("linux", info?.platform === "linux");
    document.body.classList.toggle("discreet", !!settings.discreet);
  }, [motion, settings.discreet, info?.platform]);

  // Changer d'onglet fait passer le ciel en vitesse lumière, brièvement.
  useEffect(() => {
    if (previous.current !== route.name) cosmos?.warp();
    previous.current = route.name;
  }, [route.name, cosmos]);

  useEffect(() => {
    const stop = listen("app:close-blocked", () => setClosing(true));
    return () => {
      stop.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    document.body.classList.toggle("no-webgl", cosmosFailed);
  }, [cosmosFailed]);

  return (
    <div className="shell" key={language}>
      <TitleBar />
      <Rail />
      <main className="stage">
        <CurrentView />
      </main>
      <DustFilter />
      <IdleRain />
      <div className="toasts" role="status">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.tone}`}>
            {t.message}
          </div>
        ))}
      </div>
      <PowerCountdown />
      {closing && (
        <Modal
          title={t("close.title")}
          onClose={() => setClosing(false)}
          actions={
            <>
              <button className="btn ghost" onClick={() => setClosing(false)}>
                {t("close.continue")}
              </button>
              <button className="btn danger" onClick={() => api.appQuit()}>
                {t("close.quit")}
              </button>
            </>
          }
        >
          <p>{t("close.body")}</p>
          {queue.items.length > 0 && <p>{t("close.queue")}</p>}
        </Modal>
      )}
    </div>
  );
}
