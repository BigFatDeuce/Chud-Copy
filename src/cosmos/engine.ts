// Fond spatial : ciel procédural et fluide lumineux simulé sur le GPU.
//
// La simulation (advection, vorticité, projection de pression) reprend la méthode de
// WebGL Fluid Simulation, de Pavel Dobryakov, sous licence MIT (voir licenses/).
// Elle a été réécrite pour tourner en permanence derrière une application : basse résolution,
// cadence plafonnée, ciel calculé une seule fois, arrêt complet quand la fenêtre est masquée.

import type { Quality } from "../lib/types";

interface Tier {
  sim: number;
  dye: number;
  pressure: number;
  fps: number;
  ambient: number;
}

const LINUX = navigator.userAgent.includes("Linux");

const TIERS: Record<Exclude<Quality, "off">, Tier> = {
  eco: { sim: 64, dye: 256, pressure: 8, fps: 24, ambient: 5 },
  balanced: { sim: 112, dye: 512, pressure: 14, fps: 40, ambient: 3.2 },
  high: { sim: 176, dye: 1024, pressure: 20, fps: 60, ambient: 2.2 },
};

const VERTEX = `#version 300 es
precision highp float;
in vec2 aPosition;
out vec2 vUv, vL, vR, vT, vB;
uniform vec2 texelSize;
void main() {
  vUv = aPosition * .5 + .5;
  vL = vUv - vec2(texelSize.x, 0.);
  vR = vUv + vec2(texelSize.x, 0.);
  vT = vUv + vec2(0., texelSize.y);
  vB = vUv - vec2(0., texelSize.y);
  gl_Position = vec4(aPosition, 0., 1.);
}`;

const HEADER = `#version 300 es
precision highp float;
precision highp sampler2D;
in vec2 vUv, vL, vR, vT, vB;
out vec4 fragColor;
`;

const SPLAT = `${HEADER}
uniform sampler2D uTarget;
uniform float aspectRatio, radius;
uniform vec3 color;
uniform vec2 point;
void main() {
  vec2 p = vUv - point;
  p.x *= aspectRatio;
  fragColor = vec4(texture(uTarget, vUv).xyz + exp(-dot(p, p) / radius) * color, 1.);
}`;

const ADVECTION = `${HEADER}
uniform sampler2D uVelocity, uSource;
uniform vec2 texelSize;
uniform float dt, dissipation;
void main() {
  vec2 coord = vUv - dt * texture(uVelocity, vUv).xy * texelSize;
  fragColor = texture(uSource, coord) / (1. + dissipation * dt);
}`;

const DIVERGENCE = `${HEADER}
uniform sampler2D uVelocity;
void main() {
  float L = texture(uVelocity, vL).x, R = texture(uVelocity, vR).x;
  float T = texture(uVelocity, vT).y, B = texture(uVelocity, vB).y;
  vec2 C = texture(uVelocity, vUv).xy;
  if (vL.x < 0.) L = -C.x;
  if (vR.x > 1.) R = -C.x;
  if (vT.y > 1.) T = -C.y;
  if (vB.y < 0.) B = -C.y;
  fragColor = vec4(.5 * (R - L + T - B), 0., 0., 1.);
}`;

const CURL = `${HEADER}
uniform sampler2D uVelocity;
void main() {
  float L = texture(uVelocity, vL).y, R = texture(uVelocity, vR).y;
  float T = texture(uVelocity, vT).x, B = texture(uVelocity, vB).x;
  fragColor = vec4(.5 * (R - L - T + B), 0., 0., 1.);
}`;

const VORTICITY = `${HEADER}
uniform sampler2D uVelocity, uCurl;
uniform float curl, dt;
void main() {
  float L = texture(uCurl, vL).x, R = texture(uCurl, vR).x;
  float T = texture(uCurl, vT).x, B = texture(uCurl, vB).x;
  float C = texture(uCurl, vUv).x;
  vec2 force = .5 * vec2(abs(T) - abs(B), abs(R) - abs(L));
  force /= length(force) + .0001;
  force *= curl * C;
  force.y *= -1.;
  vec2 velocity = texture(uVelocity, vUv).xy + force * dt;
  fragColor = vec4(clamp(velocity, -1000., 1000.), 0., 1.);
}`;

const PRESSURE = `${HEADER}
uniform sampler2D uPressure, uDivergence;
void main() {
  float L = texture(uPressure, vL).x, R = texture(uPressure, vR).x;
  float T = texture(uPressure, vT).x, B = texture(uPressure, vB).x;
  fragColor = vec4((L + R + B + T - texture(uDivergence, vUv).x) * .25, 0., 0., 1.);
}`;

const GRADIENT = `${HEADER}
uniform sampler2D uPressure, uVelocity;
void main() {
  float L = texture(uPressure, vL).x, R = texture(uPressure, vR).x;
  float T = texture(uPressure, vT).x, B = texture(uPressure, vB).x;
  fragColor = vec4(texture(uVelocity, vUv).xy - vec2(R - L, T - B), 0., 1.);
}`;

const CLEAR = `${HEADER}
uniform sampler2D uTexture;
uniform float value;
void main() { fragColor = value * texture(uTexture, vUv); }`;

// Calculé une fois par taille de fenêtre : le bruit fractal est trop coûteux pour chaque image.
const SKY = `${HEADER}
uniform vec2 resolution;
uniform float seed;
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7)) + seed) * 43758.5453); }
float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p), u = f * f * (3. - 2. * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), u.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), u.x), u.y);
}
float fbm(vec2 p) {
  float v = 0., a = .5;
  mat2 r = mat2(.8, -.6, .6, .8);
  for (int i = 0; i < 6; i++) { v += a * noise(p); p = r * p * 2.03; a *= .5; }
  return v;
}
float stars(vec2 uv, float scale, float threshold) {
  vec2 p = uv * scale, id = floor(p), f = fract(p) - .5;
  float h = hash(id);
  if (h < threshold) return 0.;
  vec2 o = vec2(hash(id + 3.1), hash(id + 7.7)) - .5;
  float d = length(f - o * .7);
  return smoothstep(.09 * (h - threshold) / (1. - threshold) + .012, 0., d) * (.4 + .6 * hash(id + 1.3));
}
void main() {
  float aspect = resolution.x / resolution.y;
  vec2 uv = vec2(vUv.x * aspect, vUv.y);
  vec2 q = vec2(fbm(uv * 1.4), fbm(uv * 1.4 + 5.2));
  float n = fbm(uv * 1.5 + q * 1.7);
  float wisps = pow(fbm(uv * 2.6 + q * 2.4 + 9.), 2.2);

  vec3 c = mix(vec3(.012, .014, .034), vec3(.022, .03, .085), vUv.y * .4 + n * .6);
  vec2 a = vUv - vec2(.82, .84), b = vUv - vec2(.16, .1), d = vUv - vec2(.58, .34);
  c += vec3(.20, .09, .42) * exp(-dot(a, a) * 4.5) * (.35 + n);
  c += vec3(.04, .16, .36) * exp(-dot(b, b) * 4.) * (.3 + n);
  c += vec3(.03, .08, .2) * exp(-dot(d, d) * 7.) * wisps * 2.;
  c += vec3(.10, .05, .24) * wisps * .8;
  c *= .55 + .45 * smoothstep(1.25, .25, length(vUv - .5) * 1.5);
  fragColor = vec4(c, 1.);
}`;

const DISPLAY = `${HEADER}
uniform sampler2D uDye, uSky;
uniform vec2 parallax, screen;
uniform float time;
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7)) + 4.2) * 43758.5453); }
// Étoiles en pixels d'écran : un point net par cellule, dont l'éclat respire à son propre rythme.
float stars(vec2 px, float cell, float threshold, float drift) {
  vec2 p = px / cell, id = floor(p), f = fract(p) - .5;
  float h = hash(id);
  if (h < threshold) return 0.;
  vec2 o = vec2(hash(id + 3.1), hash(id + 7.7)) - .5;
  float d = length(f - o * .7) * cell;
  float size = mix(.6, 1.6, (h - threshold) / (1. - threshold));
  float twinkle = .55 + .45 * sin(time * (.6 + hash(id + 5.3) * 2.2) + hash(id + 9.1) * 6.283);
  return smoothstep(size + .9, size * .35, d) * mix(1., twinkle, drift) * (.45 + .55 * hash(id + 1.3));
}
void main() {
  vec3 dye = texture(uDye, vUv).rgb;
  vec3 sky = texture(uSky, vUv * .985 + .0075 + parallax).rgb;
  vec2 px = gl_FragCoord.xy + parallax * screen * 1.6;
  float scale = max(screen.y / 1080., .75);
  float s = stars(px, 9. * scale, .985, .5) * .7 + stars(px + 311., 17. * scale, .975, .9) + stars(px + 977., 37. * scale, .965, 1.) * 1.35;
  vec3 tint = mix(vec3(.75, .85, 1.), vec3(1., .86, .95), hash(floor(px / (60. * scale))));
  vec3 c = sky + dye * .9 + s * tint * .85;
  c = 1. - exp(-c * 1.6);
  float grain = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
  fragColor = vec4(c + (grain - .5) / 255., 1.);
}`;

interface Program {
  program: WebGLProgram;
  uniforms: Record<string, WebGLUniformLocation>;
}

interface Target {
  texture: WebGLTexture;
  fbo: WebGLFramebuffer;
  width: number;
  height: number;
  texel: [number, number];
}

interface DoubleTarget {
  read: Target;
  write: Target;
  texel: [number, number];
  width: number;
  height: number;
  swap(): void;
}

const PALETTE: [number, number, number][] = [
  [0.1, 0.62, 1],
  [0.2, 0.86, 1],
  [0.34, 0.33, 1],
  [0.56, 0.24, 0.98],
];

export class Cosmos {
  private gl: WebGL2RenderingContext;
  private tier: Tier | null = null;
  private frame = 0;
  private last = 0;
  private accumulator = 0;
  private ambientClock = 0;
  private flow = 0;
  private flowClock = 0;
  private parallax = [0, 0];
  private parallaxGoal = [0, 0];
  private pointer: { x: number; y: number } | null = null;
  private sky: Target | null = null;
  private velocity!: DoubleTarget;
  private dye!: DoubleTarget;
  private pressure!: DoubleTarget;
  private divergence!: Target;
  private curl!: Target;
  private programs: Record<string, Program>;
  private internal: { color: number; red: number; rg: number };
  private resizeTimer = 0;
  private disposed = false;

  private constructor(private canvas: HTMLCanvasElement, gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.internal = { color: gl.RGBA16F, red: gl.R16F, rg: gl.RG16F };
    this.programs = {
      splat: this.link(SPLAT),
      advection: this.link(ADVECTION),
      divergence: this.link(DIVERGENCE),
      curl: this.link(CURL),
      vorticity: this.link(VORTICITY),
      pressure: this.link(PRESSURE),
      gradient: this.link(GRADIENT),
      clear: this.link(CLEAR),
      sky: this.link(SKY),
      display: this.link(DISPLAY),
    };
    const quad = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, quad);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, -1, 1, 1, 1, 1, -1]), gl.STATIC_DRAW);
    const indices = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indices);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint16Array([0, 1, 2, 0, 2, 3]), gl.STATIC_DRAW);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.enableVertexAttribArray(0);
    gl.disable(gl.BLEND);

    window.addEventListener("resize", this.onResize);
    window.addEventListener("pointermove", this.onPointer, { passive: true });
    document.addEventListener("visibilitychange", this.onVisibility);
  }

  /** Renvoie null si le GPU ne convient pas : l'interface garde alors son dégradé statique. */
  static create(canvas: HTMLCanvasElement): Cosmos | null {
    // Le ciel fixe n'est dessiné qu'une fois : sans cette conservation, WebKitGTK l'efface aussitôt.
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, powerPreference: "low-power", preserveDrawingBuffer: true });
    if (!gl || !gl.getExtension("EXT_color_buffer_float") && !gl.getExtension("EXT_color_buffer_half_float")) return null;
    gl.getExtension("OES_texture_float_linear");
    try {
      return new Cosmos(canvas, gl);
    } catch (error) {
      console.warn("Fond animé indisponible", error);
      return null;
    }
  }

  private shader(type: number, source: string): WebGLShader {
    const gl = this.gl;
    const shader = gl.createShader(type)!;
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "shader");
    return shader;
  }

  private link(fragment: string): Program {
    const gl = this.gl;
    const program = gl.createProgram()!;
    gl.attachShader(program, this.shader(gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(program, this.shader(gl.FRAGMENT_SHADER, fragment));
    gl.bindAttribLocation(program, 0, "aPosition");
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "programme");
    const uniforms: Record<string, WebGLUniformLocation> = {};
    const total = gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS);
    for (let i = 0; i < total; i += 1) {
      const name = gl.getActiveUniform(program, i)!.name;
      uniforms[name] = gl.getUniformLocation(program, name)!;
    }
    return { program, uniforms };
  }

  private target(width: number, height: number, internal: number, format: number, filter: number): Target {
    const gl = this.gl;
    const texture = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    const type = internal === gl.RGBA8 ? gl.UNSIGNED_BYTE : gl.HALF_FLOAT;
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, format, type, null);
    const fbo = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.viewport(0, 0, width, height);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    return { texture, fbo, width, height, texel: [1 / width, 1 / height] };
  }

  private double(width: number, height: number, internal: number, format: number, filter: number): DoubleTarget {
    let read = this.target(width, height, internal, format, filter);
    let write = this.target(width, height, internal, format, filter);
    return {
      width,
      height,
      texel: read.texel,
      get read() {
        return read;
      },
      get write() {
        return write;
      },
      swap() {
        [read, write] = [write, read];
      },
    } as DoubleTarget;
  }

  private destroy(target?: Target) {
    if (!target) return;
    this.gl.deleteTexture(target.texture);
    this.gl.deleteFramebuffer(target.fbo);
  }

  private release() {
    if (!this.velocity) return;
    for (const pair of [this.velocity, this.dye, this.pressure]) {
      this.destroy(pair.read);
      this.destroy(pair.write);
    }
    this.destroy(this.divergence);
    this.destroy(this.curl);
  }

  private fieldSize(resolution: number): [number, number] {
    const aspect = Math.max(this.canvas.width, 1) / Math.max(this.canvas.height, 1);
    return aspect >= 1 ? [Math.round(resolution * aspect), resolution] : [resolution, Math.round(resolution / aspect)];
  }

  private allocate() {
    const gl = this.gl;
    const tier = this.tier;
    if (!tier) return;
    this.release();
    const [simW, simH] = this.fieldSize(tier.sim);
    const [dyeW, dyeH] = this.fieldSize(tier.dye);
    this.velocity = this.double(simW, simH, this.internal.rg, gl.RG, gl.LINEAR);
    this.dye = this.double(dyeW, dyeH, this.internal.color, gl.RGBA, gl.LINEAR);
    this.pressure = this.double(simW, simH, this.internal.red, gl.RED, gl.NEAREST);
    this.divergence = this.target(simW, simH, this.internal.red, gl.RED, gl.NEAREST);
    this.curl = this.target(simW, simH, this.internal.red, gl.RED, gl.NEAREST);
  }

  private fit() {
    // WebKitGTK compose plus lentement que Chromium : sous Linux, la définition reste celle de l'écran logique.
    const scale = Math.min(window.devicePixelRatio || 1, LINUX ? 1 : 2);
    this.canvas.width = Math.max(1, Math.round(window.innerWidth * scale));
    this.canvas.height = Math.max(1, Math.round(window.innerHeight * scale));
  }

  private paintSky() {
    const gl = this.gl;
    this.destroy(this.sky ?? undefined);
    const width = Math.min(this.canvas.width, LINUX ? 2560 : 4096);
    const height = Math.round(width * (this.canvas.height / this.canvas.width));
    this.sky = this.target(width, height, gl.RGBA8, gl.RGBA, gl.LINEAR);
    const { program, uniforms } = this.programs.sky;
    gl.useProgram(program);
    gl.uniform2f(uniforms.resolution, width, height);
    gl.uniform1f(uniforms.seed, 4.2);
    gl.uniform2f(uniforms.texelSize, 1 / width, 1 / height);
    this.draw(this.sky);
  }

  private draw(target: Target | null) {
    const gl = this.gl;
    gl.viewport(0, 0, target ? target.width : this.canvas.width, target ? target.height : this.canvas.height);
    gl.bindFramebuffer(gl.FRAMEBUFFER, target ? target.fbo : null);
    gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_SHORT, 0);
  }

  private bind(unit: number, target: Target): number {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, target.texture);
    return unit;
  }

  private onResize = () => {
    window.clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      this.fit();
      this.paintSky();
      this.allocate();
      this.render();
    }, 180);
  };

  private onPointer = (event: PointerEvent) => {
    const x = event.clientX / window.innerWidth;
    const y = 1 - event.clientY / window.innerHeight;
    this.parallaxGoal = [(x - 0.5) * 0.012, (y - 0.5) * 0.012];
    if (this.tier && this.pointer) {
      const dx = x - this.pointer.x;
      const dy = y - this.pointer.y;
      if (Math.abs(dx) + Math.abs(dy) > 0.002) {
        const color = PALETTE[1];
        this.splat(x, y, dx * 900, dy * 900, [color[0] * 0.035, color[1] * 0.035, color[2] * 0.05], 0.0009);
      }
    }
    this.pointer = { x, y };
  };

  private onVisibility = () => {
    if (document.hidden) {
      cancelAnimationFrame(this.frame);
      this.frame = 0;
    } else if (this.tier && !this.frame) {
      this.last = performance.now();
      this.frame = requestAnimationFrame(this.loop);
    }
  };

  setQuality(quality: Quality) {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.fit();
    this.tier = quality === "off" ? null : TIERS[quality];
    this.paintSky();
    if (this.tier) {
      this.allocate();
      this.ambientClock = 0;
      this.last = performance.now();
      this.frame = requestAnimationFrame(this.loop);
    } else {
      this.renderSkyOnly();
    }
  }

  private renderSkyOnly() {
    const gl = this.gl;
    const { program, uniforms } = this.programs.display;
    gl.useProgram(program);
    // Sans simulation, une texture noire tient lieu de fluide.
    const empty = this.target(4, 4, this.internal.color, gl.RGBA, gl.LINEAR);
    gl.uniform1i(uniforms.uDye, this.bind(0, empty));
    gl.uniform1i(uniforms.uSky, this.bind(1, this.sky!));
    gl.uniform2f(uniforms.parallax, 0, 0);
    gl.uniform2f(uniforms.screen, this.canvas.width, this.canvas.height);
    gl.uniform1f(uniforms.time, 0);
    this.draw(null);
    this.destroy(empty);
  }

  /** Intensité de 0 à 1, dérivée du débit réel du moteur. */
  setFlow(intensity: number) {
    this.flow = Math.max(0, Math.min(1, intensity));
  }

  /** Traînées rayonnantes au changement d'onglet, comme un passage en vitesse lumière. */
  warp() {
    if (!this.tier) return;
    for (let i = 0; i < 10; i += 1) {
      const angle = (i / 10) * Math.PI * 2 + Math.random() * 0.3;
      const x = 0.5 + Math.cos(angle) * 0.08;
      const y = 0.5 + Math.sin(angle) * 0.08;
      this.splat(x, y, Math.cos(angle) * 520, Math.sin(angle) * 520, [0.006, 0.02, 0.05], 0.0025);
    }
  }

  /** Vague de fin d'opération, teintée selon le résultat. */
  burst(tone: "ok" | "bad") {
    if (!this.tier) return;
    const color: [number, number, number] = tone === "ok" ? [0.012, 0.075, 0.08] : [0.085, 0.01, 0.035];
    for (let i = 0; i < 7; i += 1) {
      const angle = (i / 7) * Math.PI * 2;
      this.splat(0.5, 0.55, Math.cos(angle) * 300, Math.sin(angle) * 300, color, 0.006);
    }
  }

  private splat(x: number, y: number, dx: number, dy: number, color: [number, number, number], radius: number) {
    const gl = this.gl;
    const aspect = this.canvas.width / this.canvas.height;
    const { program, uniforms } = this.programs.splat;
    gl.useProgram(program);
    gl.uniform2f(uniforms.texelSize, this.velocity.texel[0], this.velocity.texel[1]);
    gl.uniform1f(uniforms.aspectRatio, aspect);
    gl.uniform2f(uniforms.point, x, y);
    gl.uniform1f(uniforms.radius, aspect > 1 ? radius * aspect : radius);
    gl.uniform1i(uniforms.uTarget, this.bind(0, this.velocity.read));
    gl.uniform3f(uniforms.color, dx, dy, 0);
    this.draw(this.velocity.write);
    this.velocity.swap();
    gl.uniform1i(uniforms.uTarget, this.bind(0, this.dye.read));
    gl.uniform3f(uniforms.color, color[0], color[1], color[2]);
    this.draw(this.dye.write);
    this.dye.swap();
  }

  private emit(dt: number) {
    const tier = this.tier!;
    // Respiration d'ambiance, très lente quand rien ne se passe.
    this.ambientClock -= dt;
    if (this.ambientClock <= 0) {
      this.ambientClock = tier.ambient * (0.7 + Math.random() * 0.6);
      const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
      const angle = Math.random() * Math.PI * 2;
      const force = 60 + Math.random() * 120;
      this.splat(Math.random(), Math.random(), Math.cos(angle) * force, Math.sin(angle) * force, [color[0] * 0.05, color[1] * 0.05, color[2] * 0.07], 0.006);
    }
    // Pendant un transfert, des courants traversent l'écran de gauche à droite.
    if (this.flow > 0) {
      this.flowClock -= dt;
      if (this.flowClock <= 0) {
        this.flowClock = 0.16 - this.flow * 0.1;
        const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
        const y = 0.2 + Math.random() * 0.6;
        const force = 240 + this.flow * 420;
        const gain = 0.012 + this.flow * 0.02;
        this.splat(0.02 + Math.random() * 0.06, y, force, (0.5 - y) * 120, [color[0] * gain, color[1] * gain, color[2] * gain], 0.0016);
      }
    }
  }

  private step(dt: number) {
    const gl = this.gl;
    const tier = this.tier!;
    const { curl, vorticity, divergence, pressure, gradient, advection, clear } = this.programs;
    const texel = this.velocity.texel;

    gl.useProgram(curl.program);
    gl.uniform2f(curl.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(curl.uniforms.uVelocity, this.bind(0, this.velocity.read));
    this.draw(this.curl);

    gl.useProgram(vorticity.program);
    gl.uniform2f(vorticity.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(vorticity.uniforms.uVelocity, this.bind(0, this.velocity.read));
    gl.uniform1i(vorticity.uniforms.uCurl, this.bind(1, this.curl));
    gl.uniform1f(vorticity.uniforms.curl, 16);
    gl.uniform1f(vorticity.uniforms.dt, dt);
    this.draw(this.velocity.write);
    this.velocity.swap();

    gl.useProgram(divergence.program);
    gl.uniform2f(divergence.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(divergence.uniforms.uVelocity, this.bind(0, this.velocity.read));
    this.draw(this.divergence);

    gl.useProgram(clear.program);
    gl.uniform1i(clear.uniforms.uTexture, this.bind(0, this.pressure.read));
    gl.uniform1f(clear.uniforms.value, 0.8);
    this.draw(this.pressure.write);
    this.pressure.swap();

    gl.useProgram(pressure.program);
    gl.uniform2f(pressure.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(pressure.uniforms.uDivergence, this.bind(0, this.divergence));
    for (let i = 0; i < tier.pressure; i += 1) {
      gl.uniform1i(pressure.uniforms.uPressure, this.bind(1, this.pressure.read));
      this.draw(this.pressure.write);
      this.pressure.swap();
    }

    gl.useProgram(gradient.program);
    gl.uniform2f(gradient.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(gradient.uniforms.uPressure, this.bind(0, this.pressure.read));
    gl.uniform1i(gradient.uniforms.uVelocity, this.bind(1, this.velocity.read));
    this.draw(this.velocity.write);
    this.velocity.swap();

    gl.useProgram(advection.program);
    gl.uniform2f(advection.uniforms.texelSize, texel[0], texel[1]);
    gl.uniform1i(advection.uniforms.uVelocity, this.bind(0, this.velocity.read));
    gl.uniform1i(advection.uniforms.uSource, this.bind(0, this.velocity.read));
    gl.uniform1f(advection.uniforms.dt, dt);
    gl.uniform1f(advection.uniforms.dissipation, 0.35);
    this.draw(this.velocity.write);
    this.velocity.swap();

    gl.uniform1i(advection.uniforms.uVelocity, this.bind(0, this.velocity.read));
    gl.uniform1i(advection.uniforms.uSource, this.bind(1, this.dye.read));
    gl.uniform1f(advection.uniforms.dissipation, 0.65);
    this.draw(this.dye.write);
    this.dye.swap();
  }

  private render() {
    if (!this.tier) {
      this.renderSkyOnly();
      return;
    }
    const gl = this.gl;
    const { program, uniforms } = this.programs.display;
    gl.useProgram(program);
    gl.uniform1i(uniforms.uDye, this.bind(0, this.dye.read));
    gl.uniform1i(uniforms.uSky, this.bind(1, this.sky!));
    gl.uniform2f(uniforms.parallax, this.parallax[0], this.parallax[1]);
    gl.uniform2f(uniforms.screen, this.canvas.width, this.canvas.height);
    gl.uniform1f(uniforms.time, performance.now() / 1000);
    this.draw(null);
  }

  private loop = (now: number) => {
    if (this.disposed || !this.tier) return;
    this.frame = requestAnimationFrame(this.loop);
    const elapsed = (now - this.last) / 1000;
    this.last = now;
    this.accumulator += Math.min(elapsed, 0.25);
    const interval = 1 / this.tier.fps;
    if (this.accumulator < interval) return;
    const dt = Math.min(this.accumulator, 1 / 20);
    this.accumulator = 0;

    this.parallax[0] += (this.parallaxGoal[0] - this.parallax[0]) * 0.04;
    this.parallax[1] += (this.parallaxGoal[1] - this.parallax[1]) * 0.04;
    this.emit(dt);
    this.step(dt);
    this.render();
  };
}
