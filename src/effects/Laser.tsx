import { useEffect, useRef, useState } from "react";
import { animateWhileVisible, useMotion } from "./motion";

// Laser bleu derrière le titre de la fenêtre pendant une copie ou une analyse : cœur blanc,
// halo bleu, légère ondulation, scintillement et grains de lumière qui filent le long du rayon.
// Canvas 2D, pour la même raison que le brasier : un seul contexte WebGL dans l'application.

const FADE_MS = 700;

export function Laser({ active }: { active: boolean }) {
  const motion = useMotion();
  const ref = useRef<HTMLCanvasElement>(null);
  const [shown, setShown] = useState(active);
  const activeRef = useRef(active);
  activeRef.current = active;

  // Le rayon s'éteint en fondu avant de disparaître.
  useEffect(() => {
    if (active) {
      setShown(true);
      return;
    }
    const timer = window.setTimeout(() => setShown(false), FADE_MS);
    return () => window.clearTimeout(timer);
  }, [active]);

  useEffect(() => {
    const canvas = ref.current;
    if (!motion || !shown || !canvas) return;
    const context = canvas.getContext("2d")!;
    const scale = Math.min(window.devicePixelRatio || 1, 2);
    const fit = () => {
      canvas.width = Math.max(1, Math.round(canvas.clientWidth * scale));
      canvas.height = Math.max(1, Math.round(canvas.clientHeight * scale));
    };
    fit();
    const resize = new ResizeObserver(fit);
    resize.observe(canvas);
    const grains = Array.from({ length: 26 }, () => ({ at: Math.random(), speed: 0.08 + Math.random() * 0.22, size: 0.6 + Math.random() * 1.4, phase: Math.random() * 6.28 }));
    let power = 0;
    let flicker = 1;
    let last = performance.now();

    const stop = animateWhileVisible(canvas, (now) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      power += ((activeRef.current ? 1 : 0) - power) * Math.min(1, dt * 3);
      if (Math.random() < dt * 9) flicker = 0.78 + Math.random() * 0.22;
      const w = canvas.width / scale;
      const h = canvas.height / scale;
      const mid = h / 2;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, w, h);
      if (power < 0.01) return;
      const t = now / 1000;
      const strength = power * flicker;
      // Le rayon s'amincit jusqu'à une pointe à chaque extrémité.
      const y = (x: number) => mid + Math.sin(x * 0.018 + t * 1.3) * 1.6 + Math.sin(x * 0.041 - t * 2.1) * 0.9;
      const taper = (x: number) => Math.sin(Math.PI * Math.min(1, Math.max(0, x / w)));

      context.globalCompositeOperation = "lighter";
      const layers = [
        { width: 22, color: "40, 110, 255", alpha: 0.05 },
        { width: 11, color: "56, 160, 255", alpha: 0.09 },
        { width: 5, color: "120, 210, 255", alpha: 0.2 },
        { width: 1.4, color: "220, 240, 255", alpha: 0.42 },
      ];
      for (const layer of layers) {
        context.beginPath();
        for (let x = 0; x <= w; x += 6) {
          const k = taper(x);
          context.lineTo(x, y(x) - (layer.width / 2) * k);
        }
        for (let x = w; x >= 0; x -= 6) {
          const k = taper(x);
          context.lineTo(x, y(x) + (layer.width / 2) * k);
        }
        context.closePath();
        context.fillStyle = `rgba(${layer.color}, ${layer.alpha * strength})`;
        context.fill();
      }

      // Grains de lumière qui filent sur le rayon.
      for (const grain of grains) {
        grain.at = (grain.at + grain.speed * dt) % 1;
        const x = grain.at * w;
        const k = taper(x);
        const glow = context.createRadialGradient(x, y(x), 0, x, y(x), grain.size * 5);
        const alpha = k * strength * 0.4 * (0.6 + 0.4 * Math.sin(t * 8 + grain.phase));
        glow.addColorStop(0, `rgba(235, 248, 255, ${alpha})`);
        glow.addColorStop(1, "rgba(56, 160, 255, 0)");
        context.fillStyle = glow;
        context.fillRect(x - grain.size * 5, y(x) - grain.size * 5, grain.size * 10, grain.size * 10);
      }
    });
    return () => {
      stop();
      resize.disconnect();
    };
  }, [motion, shown]);

  if (!motion || !shown) return null;
  return <canvas ref={ref} className={active ? "fx-laser" : "fx-laser leaving"} aria-hidden="true" />;
}
