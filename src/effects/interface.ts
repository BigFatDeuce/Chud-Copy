// Effets d'interface globaux : rien ici ne déclenche d'opération, seuls des styles changent.

const PRESSABLE = ".btn, .icon-btn, .rail-item, .rail-live, .tile, .mode, .quality button, .link-tabs button";

/** Brève déchirure numérique sur le bouton pressé : tranches décalées et couches rouge et cyan séparées. */
export function installGlitch(): () => void {
  const press = (event: PointerEvent) => {
    if (!document.body.classList.contains("motion")) return;
    const target = (event.target as Element | null)?.closest(PRESSABLE);
    if (!(target instanceof HTMLElement) || target.matches(":disabled")) return;
    target.classList.remove("glitching");
    void target.offsetWidth;
    target.classList.add("glitching");
    target.addEventListener("animationend", () => target.classList.remove("glitching"), { once: true });
  };
  window.addEventListener("pointerdown", press, { capture: true });
  return () => window.removeEventListener("pointerdown", press, { capture: true });
}

/**
 * Mode discret : le grain du filtre SVG est retiré quelques fois par seconde pour que la
 * poussière scintille. Le texte réel reste dans la page et se révèle au survol.
 */
export function installDust(): () => void {
  const turbulence = document.getElementById("dust-grain");
  const scatter = document.getElementById("dust-scatter");
  let seed = 1;
  const timer = window.setInterval(() => {
    if (!document.body.classList.contains("discreet") || !document.body.classList.contains("motion")) return;
    seed = (seed % 97) + 1;
    turbulence?.setAttribute("seed", String(seed));
    scatter?.setAttribute("seed", String(seed + 13));
  }, 140);
  return () => window.clearInterval(timer);
}
