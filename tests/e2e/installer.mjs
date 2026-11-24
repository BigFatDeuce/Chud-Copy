// Installe réellement le paquet Windows dans un dossier temporaire, lance l'application, désinstalle.
// Usage : node tests/e2e/installer.mjs <setup.exe> <dossier-temporaire> <dossier-captures>
import assert from "node:assert/strict";
import { execFileSync, execSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [setup, root, shots] = process.argv.slice(2);
const target = join(root, "installation");
rmSync(target, { recursive: true, force: true });
const uninstallKey = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Chud Copy";

execFileSync(setup, ["/S", `/D=${target}`], { stdio: "ignore" });
await new Promise((r) => setTimeout(r, 2500));

for (const file of ["Chud Copy.exe", "rclone.exe", "licenses/rclone-COPYING.txt", "licenses/WebGL-Fluid-Simulation-LICENSE.txt", "licenses/Tauri-LICENSE-MIT.txt"]) {
  assert.ok(existsSync(join(target, file)), `absent de l'installation : ${file}`);
}
console.log("ok - fichiers installés, moteur et licences compris");

const lab = buildLab(join(root, "lab"), { bigMegabytes: 2 });
const session = await launch({ exe: join(target, "Chud Copy.exe"), dataDir: lab.data, shots });
try {
  await session.page.waitForSelector(".task-card", { timeout: 15000 });
  const info = await session.page.evaluate(() => window.__TAURI_INTERNALS__.invoke("app_info"));
  assert.match(info.engineVersion, /^rclone v1\.75\.2/);
  console.log(`ok - application installée lancée, moteur ${info.engineVersion}`);
} finally {
  await session.close();
  await new Promise((r) => setTimeout(r, 1500));
}

execFileSync(join(target, "uninstall.exe"), ["/S"], { stdio: "ignore" });
await new Promise((r) => setTimeout(r, 3500));
assert.equal(existsSync(join(target, "Chud Copy.exe")), false, "l'exécutable doit disparaître");
let registered = true;
try {
  execSync(`reg query "${uninstallKey}"`, { stdio: "ignore" });
} catch {
  registered = false;
}
assert.equal(registered, false, "l'entrée de désinstallation doit disparaître");
assert.ok(existsSync(join(lab.data, "tasks")), "les données de l'utilisateur doivent rester");
console.log("ok - désinstallé proprement, données utilisateur conservées");
