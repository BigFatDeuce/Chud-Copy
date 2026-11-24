// Mesure le coût du fond animé, application au repos, pour chaque niveau de qualité.
// Usage : node tests/e2e/idle.mjs <exe> <dossier-temporaire> <dossier-captures>
import { execSync } from "node:child_process";
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"), { bigMegabytes: 1 });
const run = await launch({ exe, dataDir: lab.data, shots });
const { page } = run;

function cpuSeconds() {
  // L'application et ses processus WebView, pas ceux du reste de la machine.
  const script = `$ids=@(${run.app.pid}); $all=Get-CimInstance Win32_Process; $grow=$true; while($grow){$grow=$false; foreach($p in $all){ if($ids -contains $p.ParentProcessId -and $ids -notcontains $p.ProcessId){$ids+=$p.ProcessId;$grow=$true} } }; $t=0; foreach($i in $ids){ $g=Get-Process -Id $i -ErrorAction SilentlyContinue; if($g){$t+=$g.TotalProcessorTime.TotalSeconds} }; $t`;
  return Number(execSync(`powershell -NoProfile -Command "${script}"`, { encoding: "utf8" }).trim().replace(",", "."));
}

const fps = () =>
  page.evaluate(
    () =>
      new Promise((resolve) => {
        let frames = 0;
        const start = performance.now();
        const tick = () => {
          frames += 1;
          if (performance.now() - start < 2000) requestAnimationFrame(tick);
          else resolve(Math.round(frames / 2));
        };
        requestAnimationFrame(tick);
      }),
  );

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });
  await page.getByRole("button", { name: "Réglages" }).click();
  for (const [label, name] of [["Désactivé", /Désactivé/], ["Économe", /Économe/], ["Équilibré", /Équilibré/], ["Élevé", /Élevé/]]) {
    await page.getByRole("radio", { name }).click();
    await page.waitForTimeout(1500);
    const before = cpuSeconds();
    const started = Date.now();
    await page.waitForTimeout(6000);
    const percent = ((cpuSeconds() - before) / ((Date.now() - started) / 1000)) * 100;
    console.log(`${label.padEnd(10)} CPU ${percent.toFixed(1).padStart(5)} % d'un cœur   affichage ${await fps()} i/s`);
  }
} finally {
  await run.close();
}
