import { useEffect, useRef, useState } from "react";
import { useMotion } from "./motion";

// Veille : après trois minutes sans geste, une pluie de glyphes bleus voile l'application.
// Le moindre mouvement, clic ou touche la dissipe. Elle ne touche à aucune opération.

const IDLE_MS = 3 * 60 * 1000;
const GLYPHS = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789Z*+-<>¦=:.";
const EVENTS = ["pointermove", "pointerdown", "keydown", "wheel", "touchstart"] as const;

export function IdleRain() {
  const motion = useMotion();
  const [raining, setRaining] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    if (!motion) return;
    let timer = window.setTimeout(() => setRaining(true), IDLE_MS);
    const wake = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setRaining(true), IDLE_MS);
      setRaining((current) => {
        if (current) setLeaving(true);
        return current;
      });
    };
    for (const name of EVENTS) window.addEventListener(name, wake, { passive: true, capture: true });
    return () => {
      window.clearTimeout(timer);
      for (const name of EVENTS) window.removeEventListener(name, wake, { capture: true });
    };
  }, [motion]);

  useEffect(() => {
    if (!leaving) return;
    const timer = window.setTimeout(() => {
      setRaining(false);
      setLeaving(false);
    }, 650);
    return () => window.clearTimeout(timer);
  }, [leaving]);

  useEffect(() => {
    const canvas = ref.current;
    if (!raining || !canvas) return;
    const context = canvas.getContext("2d")!;
    const scale = Math.min(window.devicePixelRatio || 1, 1.5);
    const cell = 18;
    let columns: { y: number; speed: number; trail: number; glyphs: string[]; layer: number }[] = [];
    const pick = () => GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
    const fit = () => {
      canvas.width = Math.round(window.innerWidth * scale);
      canvas.height = Math.round(window.innerHeight * scale);
      const count = Math.ceil(window.innerWidth / cell);
      columns = Array.from({ length: count }, () => ({
        y: -Math.random() * window.innerHeight,
        speed: 70 + Math.random() * 160,
        trail: 8 + Math.floor(Math.random() * 18),
        glyphs: Array.from({ length: 40 }, pick),
        layer: Math.random() < 0.35 ? 0.55 : 1,
      }));
    };
    fit();
    window.addEventListener("resize", fit);
    let last = performance.now();
    let frame = 0;
    const loop = (now: number) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      context.setTransform(scale, 0, 0, scale, 0, 0);
      context.clearRect(0, 0, window.innerWidth, window.innerHeight);
      context.textAlign = "center";
      columns.forEach((column, i) => {
        column.y += column.speed * column.layer * dt;
        if (column.y - column.trail * cell > window.innerHeight) {
          column.y = -Math.random() * 300;
          column.speed = 70 + Math.random() * 160;
        }
        if (Math.random() < 0.04) column.glyphs[Math.floor(Math.random() * column.glyphs.length)] = pick();
        const size = cell * (column.layer < 1 ? 0.75 : 1);
        context.font = `${Math.round(size * 0.9)}px "JetBrains Mono Variable", monospace`;
        const x = i * cell + cell / 2;
        const head = Math.floor(column.y / size);
        for (let k = 0; k < column.trail; k += 1) {
          const row = head - k;
          if (row < 0) continue;
          const alpha = (1 - k / column.trail) * column.layer;
          context.fillStyle = k === 0 ? `rgba(196, 222, 255, ${alpha})` : `rgba(68, 116, 255, ${alpha * 0.85})`;
          context.fillText(column.glyphs[row % column.glyphs.length], x, row * size);
        }
        // La tête éclaire faiblement la page sous elle.
        const glow = context.createRadialGradient(x, head * size, 0, x, head * size, 60);
        glow.addColorStop(0, `rgba(43, 106, 255, ${0.14 * column.layer})`);
        glow.addColorStop(1, "rgba(43, 106, 255, 0)");
        context.fillStyle = glow;
        context.fillRect(x - 60, head * size - 60, 120, 120);
      });
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", fit);
    };
  }, [raining]);

  if (!motion || !raining) return null;
  return <canvas ref={ref} className={leaving ? "fx-rain leaving" : "fx-rain"} aria-hidden="true" />;
}
