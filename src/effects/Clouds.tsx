import { useEffect, useRef } from "react";
import { animateWhileVisible, rgb, useMotion } from "./motion";

// Brouillard d'analyse : quelques couches d'un même bruit, qui dérivent à des vitesses différentes.
// Le curseur l'écarte sur son passage, et la trouée se referme doucement.

let tile: HTMLCanvasElement | null = null;

/** Bruit fractal raccordable, calculé une seule fois pour tous les cadres. */
function fogTile(): HTMLCanvasElement {
  if (tile) return tile;
  const size = 256;
  const lattice = (n: number) => {
    const values = new Float32Array(n * n);
    for (let i = 0; i < values.length; i += 1) values[i] = Math.random();
    return (x: number, y: number) => {
      const x0 = Math.floor(x);
      const y0 = Math.floor(y);
      const fx = x - x0;
      const fy = y - y0;
      const sx = fx * fx * (3 - 2 * fx);
      const sy = fy * fy * (3 - 2 * fy);
      const at = (a: number, b: number) => values[((b % n) + n) % n * n + (((a % n) + n) % n)];
      const top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * sx;
      const bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * sx;
      return top + (bottom - top) * sy;
    };
  };
  const octaves = [4, 8, 16, 32].map((n) => ({ n, sample: lattice(n) }));
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const context = canvas.getContext("2d")!;
  const image = context.createImageData(size, size);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      let value = 0;
      let weight = 0.55;
      for (const { n, sample } of octaves) {
        value += sample((x / size) * n, (y / size) * n) * weight;
        weight *= 0.5;
      }
      const alpha = Math.max(0, Math.min(1, (value - 0.38) * 2.1));
      const index = (y * size + x) * 4;
      image.data[index] = image.data[index + 1] = image.data[index + 2] = 255;
      image.data[index + 3] = alpha * alpha * 255;
    }
  }
  context.putImageData(image, 0, 0);
  tile = canvas;
  return canvas;
}

interface Gust {
  x: number;
  y: number;
  strength: number;
}

export function Clouds({ color }: { color?: string | null }) {
  const motion = useMotion();
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    const host = canvas?.parentElement;
    if (!motion || !canvas || !host) return;
    const context = canvas.getContext("2d")!;
    const fog = fogTile();
    const [r, g, b] = rgb(color, [0.72, 0.83, 1]);
    const tint = document.createElement("canvas");
    tint.width = tint.height = fog.width;
    const tinted = tint.getContext("2d")!;
    tinted.drawImage(fog, 0, 0);
    tinted.globalCompositeOperation = "source-in";
    tinted.fillStyle = `rgb(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)})`;
    tinted.fillRect(0, 0, tint.width, tint.height);

    const gusts: Gust[] = [];
    const scale = Math.min(window.devicePixelRatio || 1, 1.5);
    const fit = () => {
      canvas.width = Math.max(1, Math.round(host.clientWidth * scale));
      canvas.height = Math.max(1, Math.round(host.clientHeight * scale));
    };
    fit();
    const resize = new ResizeObserver(fit);
    resize.observe(host);
    const move = (event: PointerEvent) => {
      const box = host.getBoundingClientRect();
      gusts.push({ x: (event.clientX - box.left) * scale, y: (event.clientY - box.top) * scale, strength: 1 });
      if (gusts.length > 40) gusts.shift();
    };
    host.addEventListener("pointermove", move);

    const layers = [
      { size: 340, speed: 9, alpha: 0.2 },
      { size: 520, speed: -6, alpha: 0.16 },
      { size: 220, speed: 14, alpha: 0.12 },
    ];
    let last = performance.now();
    const stop = animateWhileVisible(canvas, (now) => {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const { width, height } = canvas;
      context.globalCompositeOperation = "source-over";
      context.clearRect(0, 0, width, height);
      for (const layer of layers) {
        const size = layer.size * scale;
        const offsetX = (((now / 1000) * layer.speed * scale) % size + size) % size;
        const offsetY = (((now / 1000) * layer.speed * 0.35 * scale) % size + size) % size;
        context.globalAlpha = layer.alpha;
        for (let x = -offsetX; x < width; x += size) {
          for (let y = -offsetY; y < height; y += size) context.drawImage(tint, x, y, size, size);
        }
      }
      // Le brouillard s'épaissit vers les bords : le centre, où se lisent les chiffres, reste clair.
      context.globalAlpha = 1;
      context.globalCompositeOperation = "destination-out";
      const clearing = context.createRadialGradient(width / 2, height / 2, 0, width / 2, height / 2, Math.max(width, height) * 0.55);
      clearing.addColorStop(0, "rgba(0,0,0,0.55)");
      clearing.addColorStop(1, "rgba(0,0,0,0)");
      context.fillStyle = clearing;
      context.fillRect(0, 0, width, height);
      for (let i = gusts.length - 1; i >= 0; i -= 1) {
        const gust = gusts[i];
        const radius = 70 * scale;
        const hole = context.createRadialGradient(gust.x, gust.y, 0, gust.x, gust.y, radius);
        hole.addColorStop(0, `rgba(0,0,0,${0.9 * gust.strength})`);
        hole.addColorStop(1, "rgba(0,0,0,0)");
        context.fillStyle = hole;
        context.fillRect(gust.x - radius, gust.y - radius, radius * 2, radius * 2);
        gust.strength -= dt * 0.7;
        if (gust.strength <= 0) gusts.splice(i, 1);
      }
    });
    return () => {
      stop();
      resize.disconnect();
      host.removeEventListener("pointermove", move);
    };
  }, [motion, color]);

  if (!motion) return null;
  return <canvas ref={ref} className="fx-clouds" aria-hidden="true" />;
}
