// Pilote la vraie application par le protocole de débogage de sa WebView.
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright-core";

export async function launch({ exe, dataDir, port = 9333, shots }) {
  mkdirSync(shots, { recursive: true });
  const app = spawn(exe, [], {
    env: {
      ...process.env,
      CHUD_COPY_DATA_DIR: dataDir,
      // Aucun test ne doit pouvoir éteindre la machine : l'extinction écrit ce fichier témoin.
      CHUD_COPY_FAKE_POWER_OFF: join(dataDir, "extinction-simulee.txt"),
      // Fenêtre hors de l'écran et sans focus : l'utilisateur garde la main sur sa machine.
      // CHUD_TEST_VISIBLE=1 la montre, pour observer un test.
      ...(process.env.CHUD_TEST_VISIBLE ? {} : { CHUD_COPY_BACKGROUND: "1" }),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      WEBVIEW2_USER_DATA_FOLDER: join(dataDir, "webview"),
    },
    stdio: "ignore",
  });

  let browser;
  for (let attempt = 0; attempt < 60 && !browser; attempt += 1) {
    await new Promise((r) => setTimeout(r, 500));
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => undefined);
  }
  if (!browser) {
    app.kill();
    throw new Error("L'application n'a pas ouvert son port de débogage.");
  }
  const page = browser.contexts()[0].pages().find((p) => !p.url().startsWith("devtools"));
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  return {
    app,
    page,
    errors,
    shot: (name) => page.screenshot({ path: join(shots, `${name}.png`) }),
    async close() {
      await browser.close().catch(() => {});
      app.kill();
    },
  };
}
