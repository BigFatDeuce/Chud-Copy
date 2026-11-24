import { useEffect, useState } from "react";
import { useApp } from "../lib/context";

const reduced = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Vrai quand les effets animés sont permis : réglage d'animation actif et mouvement non réduit. */
export function useMotion(): boolean {
  const { settings } = useApp();
  const [calm, setCalm] = useState(reduced);
  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const change = () => setCalm(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);
  return settings.quality !== "off" && !calm;
}

/** Appelle `frame` à chaque image tant que l'élément est visible à l'écran. */
export function animateWhileVisible(element: Element, frame: (now: number) => void): () => void {
  let id = 0;
  let visible = false;
  const loop = (now: number) => {
    frame(now);
    id = requestAnimationFrame(loop);
  };
  const observer = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    cancelAnimationFrame(id);
    if (visible) id = requestAnimationFrame(loop);
  });
  observer.observe(element);
  return () => {
    observer.disconnect();
    cancelAnimationFrame(id);
  };
}

/** Une couleur #rrggbb en composantes de 0 à 1. */
export function rgb(hex: string | null | undefined, fallback: [number, number, number]): [number, number, number] {
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return fallback;
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255) as [number, number, number];
}
