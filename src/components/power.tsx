// Extinction de l'ordinateur à la fin des copies : la case qui l'arme, et le compte à rebours qui
// laisse une minute pour l'annuler. Les règles sont dans `Shutdown` (lib/machines.ts).

import { useEffect, useRef, useState } from "react";
import { Power } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/context";
import { describe, t } from "../lib/i18n";
import { armShutdown, liveRun, SHUTDOWN_OFF } from "../lib/machines";
import { Modal, Toggle } from "./ui";

/** La case « Éteindre le PC à la fin des copies ». */
export function PowerToggle() {
  const { shutdown, setShutdown, run } = useApp();
  const armed = shutdown.phase !== "off";
  const hint = !armed ? t("power.hint") : liveRun(run) ? t("power.armedLive") : t("power.armedIdle");
  return (
    <div className={armed ? "power-toggle armed" : "power-toggle"}>
      <Power size={15} aria-hidden="true" />
      <Toggle checked={armed} label={t("power.label")} hint={hint} onChange={(on) => setShutdown(on ? armShutdown(run) : SHUTDOWN_OFF)} />
    </div>
  );
}

/** Le compte à rebours, affiché par-dessus tout écran. Fermer la fenêtre annule l'extinction. */
export function PowerCountdown() {
  const { shutdown, setShutdown, toast } = useApp();
  const [now, setNow] = useState(() => Date.now());
  const asked = useRef(false);
  const counting = shutdown.phase === "counting";

  useEffect(() => {
    if (!counting) {
      asked.current = false;
      return;
    }
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [counting]);

  const powerOff = () => {
    if (asked.current) return;
    asked.current = true;
    api
      .systemPowerOff()
      .then(() => setShutdown(SHUTDOWN_OFF))
      .catch((error) => {
        setShutdown(SHUTDOWN_OFF);
        toast(describe(String(error)), "bad");
      });
  };

  const left = counting ? Math.max(0, Math.ceil((shutdown.until - now) / 1000)) : 0;
  useEffect(() => {
    if (counting && left === 0) powerOff();
  });

  if (!counting) return null;
  const cancel = () => setShutdown(SHUTDOWN_OFF);
  return (
    <Modal
      title={t("power.title")}
      onClose={cancel}
      actions={
        <>
          <button className="btn primary" onClick={cancel}>
            {t("power.cancel")}
          </button>
          <button className="btn ghost danger-text" onClick={powerOff}>
            <Power size={14} /> {t("power.now")}
          </button>
        </>
      }
    >
      <p className="power-count" aria-live="polite">
        {t("power.countdown", { n: left })}
      </p>
      <p>{t("power.body")}</p>
    </Modal>
  );
}
