import { useEffect, useRef, useState } from "react";
import { animateWhileVisible, rgb, useMotion } from "./motion";
import type { RunRecord } from "../lib/types";

// Brasier au pied d'un cadre en train de copier : lueur chaude, fumée qui monte, étincelles.
// Dessiné en canvas 2D, sans WebGL : un second contexte WebGL fait planter WebKitGTK sur
// certains pilotes. L'ardeur suit le débit réel et retombe en braises pendant une pause.

interface Spark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  age: number;
  size: number;
  phase: number;
}

interface Puff {
  x: number;
  y: number;
  r: number;
  vy: number;
  age: number;
  life: number;
}

const ORANGE: [number, number, number] = [1, 0.4, 0.05];

/** Ardeur du brasier : suit le débit réel, retombe en braises pendant une pause. */
export function heatOf(run: RunRecord): number {
  if (run.status !== "running") return 0.12;
  const megabytes = run.totals.speed / (1024 * 1024);
  return megabytes > 0 ? Math.min(1, 0.35 + Math.log10(Math.max(1, megabytes)) / 3.2) : 0.3;
}

const FADE_MS = 2200;

export function Blaze({ heat, color, active = true }: { heat: number; color?: string | null; active?: boolean }) {
  const motion = useMotion();
  const ref = useRef<HTMLCanvasElement>(null);
  const heatRef = useRef(heat);
  heatRef.current = heat;
  const activeRef = useRef(active);
  activeRef.current = active;
  const [shown, setShown] = useState(active);

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
    const host = canvas?.parentElement;
    if (!motion || !shown || !canvas || !host) return;
    const context = canvas.getContext("2d")!;
    const [r, g, b] = rgb(color, ORANGE).map((c) => Math.round(c * 255));
    // Le cœur des étincelles tire vers le blanc, quelle que soit la couleur de la tâche.
    const hot = `${Math.round(r + (255 - r) * 0.7)}, ${Math.round(g + (255 - g) * 0.6)}, ${Math.round(b + (255 - b) * 0.4)}`;
    const tint = `${r}, ${g}, ${b}`;
    const scale = Math.min(window.devicePixelRatio || 1, 1.5);
    const fit = () => {
      canvas.width = Math.max(1, Math.round(host.clientWidth * scale));
      canvas.height = Math.max(1, Math.round(host.clientHeight * scale));
    };
    fit();
    const resize = new ResizeObserver(fit);
    resize.observe(host);

    const sparks: Spark[] = [];
    const puffs: Puff[] = [];
    let smoothed = heatRef.current;
    let last = performance.now();
    let spawn = 0;
    let smoke = 0;

    const stop = animateWhileVisible(canvas, (now) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const target = activeRef.current ? heatRef.current : 0;
      smoothed += (target - smoothed) * Math.min(1, dt * 2);
      const { width, height } = canvas;
      const w = width / scale;
      const h = height / scale;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.globalCompositeOperation = "source-over";
      context.clearRect(0, 0, w, h);

      // Lueur ambiante au pied du cadre.
      const glow = context.createLinearGradient(0, h, 0, h * (0.35 - smoothed * 0.2));
      glow.addColorStop(0, `rgba(${tint}, ${0.18 + smoothed * 0.32})`);
      glow.addColorStop(0.45, `rgba(${tint}, ${0.05 + smoothed * 0.1})`);
      glow.addColorStop(1, `rgba(${tint}, 0)`);
      context.fillStyle = glow;
      context.fillRect(0, 0, w, h);

      // Fumée : des volutes douces qui montent, gonflent et s'effacent.
      if (activeRef.current) smoke += dt * (1.5 + smoothed * 4);
      while (smoke >= 1) {
        smoke -= 1;
        puffs.push({ x: Math.random() * w, y: h + 10, r: 14 + Math.random() * 26, vy: 14 + Math.random() * 22, age: 0, life: 3 + Math.random() * 2.5 });
      }
      for (let i = puffs.length - 1; i >= 0; i -= 1) {
        const puff = puffs[i];
        puff.age += dt;
        puff.y -= puff.vy * dt * (0.6 + smoothed);
        puff.x += Math.sin(now / 900 + puff.r) * dt * 8;
        const t = puff.age / puff.life;
        if (t >= 1) {
          puffs.splice(i, 1);
          continue;
        }
        const radius = puff.r * (1 + t * 1.8);
        const alpha = Math.sin(t * Math.PI) * (0.05 + smoothed * 0.07);
        const blob = context.createRadialGradient(puff.x, puff.y, 0, puff.x, puff.y, radius);
        blob.addColorStop(0, `rgba(${Math.round(r * 0.6 + 60)}, ${Math.round(g * 0.6 + 50)}, ${Math.round(b * 0.6 + 50)}, ${alpha})`);
        blob.addColorStop(1, "rgba(60, 50, 50, 0)");
        context.fillStyle = blob;
        context.fillRect(puff.x - radius, puff.y - radius, radius * 2, radius * 2);
      }

      // Étincelles : nombre et vitesse suivent l'ardeur.
      if (activeRef.current) spawn += dt * (6 + smoothed * 70) * (w / 600);
      while (spawn >= 1 && sparks.length < 260) {
        spawn -= 1;
        sparks.push({
          x: Math.random() * w,
          y: h - Math.random() * 8,
          vx: (Math.random() - 0.5) * 18,
          vy: -(40 + Math.random() * 90) * (0.5 + smoothed),
          life: 1.2 + Math.random() * 2.2,
          age: 0,
          size: 0.8 + Math.random() * 1.8,
          phase: Math.random() * 6.28,
        });
      }
      context.globalCompositeOperation = "lighter";
      for (let i = sparks.length - 1; i >= 0; i -= 1) {
        const spark = sparks[i];
        spark.age += dt;
        if (spark.age >= spark.life || spark.y < -10) {
          sparks.splice(i, 1);
          continue;
        }
        spark.x += (spark.vx + Math.sin(now / 300 + spark.phase) * 14) * dt;
        spark.y += spark.vy * dt;
        spark.vy *= 1 - dt * 0.25;
        const fade = 1 - spark.age / spark.life;
        const flicker = 0.65 + 0.35 * Math.sin(now / 60 + spark.phase * 7);
        const alpha = fade * flicker;
        const halo = spark.size * 4;
        const light = context.createRadialGradient(spark.x, spark.y, 0, spark.x, spark.y, halo);
        light.addColorStop(0, `rgba(${hot}, ${alpha})`);
        light.addColorStop(0.35, `rgba(${tint}, ${alpha * 0.55})`);
        light.addColorStop(1, `rgba(${tint}, 0)`);
        context.fillStyle = light;
        context.fillRect(spark.x - halo, spark.y - halo, halo * 2, halo * 2);
      }
    });
    return () => {
      stop();
      resize.disconnect();
    };
  }, [motion, color, shown]);

  if (!motion || !shown) return null;
  return <canvas ref={ref} className={active ? "fx-blaze" : "fx-blaze leaving"} aria-hidden="true" />;
}
