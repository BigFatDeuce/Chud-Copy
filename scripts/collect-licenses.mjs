// Rassemble les textes de licence distribués avec l'application.
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const out = join(root, "licenses");
mkdirSync(out, { recursive: true });

const REMOTE = {
  "rclone-COPYING.txt": "https://raw.githubusercontent.com/rclone/rclone/v1.75.2/COPYING",
  "WebGL-Fluid-Simulation-LICENSE.txt": "https://raw.githubusercontent.com/PavelDoGreat/WebGL-Fluid-Simulation/master/LICENSE",
};

for (const [name, url] of Object.entries(REMOTE)) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} : ${response.status}`);
  writeFileSync(join(out, name), await response.text());
}

const LOCAL = {
  "Sora-OFL.txt": "@fontsource-variable/sora/LICENSE",
  "JetBrainsMono-OFL.txt": "@fontsource-variable/jetbrains-mono/LICENSE",
  "lucide-LICENSE.txt": "lucide-react/LICENSE",
  "React-LICENSE.txt": "react/LICENSE",
  "Tauri-LICENSE-MIT.txt": "@tauri-apps/api/LICENSE-MIT",
};

for (const [name, path] of Object.entries(LOCAL)) {
  const source = join(root, "node_modules", path);
  if (!existsSync(source)) throw new Error(`Licence absente : ${source}`);
  copyFileSync(source, join(out, name));
}

// Chud Copy itself is GPL-3.0-or-later: its object code must ship with the licence text.
copyFileSync(join(root, "LICENSE"), join(out, "Chud-Copy-LICENSE.txt"));

writeFileSync(
  join(out, "LISEZ-MOI.txt"),
  `Chud Copy : logiciel libre sous licence GPL-3.0-or-later (Chud-Copy-LICENSE.txt).
Code source : https://github.com/BigFatDeuce/Chud-Copy

Composants tiers distribués avec Chud Copy

rclone 1.75.2 : moteur de copie embarqué, licence MIT, Nick Craig-Wood et contributeurs.
WebGL Fluid Simulation : méthode de simulation reprise pour le fond animé, licence MIT, Pavel Dobryakov.
Tauri : socle de l'application, licence MIT ou Apache 2.0.
React : licence MIT.
Lucide : icônes, licence ISC.
Sora et JetBrains Mono : polices, licence SIL Open Font 1.1.
`,
);
console.log(`Licences écrites dans ${out}`);
