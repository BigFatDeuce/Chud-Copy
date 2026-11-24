// Récupère le binaire rclone embarqué dans l'application.
// Version et empreintes sont épinglées : une mise à jour du moteur est un choix explicite,
// jamais un effet de bord de la compilation.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION = "v1.75.2";
const TARGETS = {
  "x86_64-pc-windows-msvc": {
    platform: "windows-amd64",
    sha256: "8a9db2bad1dc79a65b3348fd857069598e2aab7cf4d118b77b1df23055aff818",
    exe: ".exe",
  },
  "x86_64-unknown-linux-gnu": {
    platform: "linux-amd64",
    sha256: "349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702",
    exe: "",
  },
};

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const binaries = join(root, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });

const requested = process.argv.slice(2);
const hostTarget = process.platform === "win32" ? "x86_64-pc-windows-msvc" : "x86_64-unknown-linux-gnu";
const targets = requested.length ? requested : [hostTarget];

for (const triple of targets) {
  const spec = TARGETS[triple];
  if (!spec) throw new Error(`Cible inconnue : ${triple}`);
  const out = join(binaries, `rclone-${triple}${spec.exe}`);
  if (existsSync(out)) {
    console.log(`rclone déjà présent : ${out}`);
    continue;
  }
  const name = `rclone-${VERSION}-${spec.platform}`;
  const url = `https://downloads.rclone.org/${VERSION}/${name}.zip`;
  console.log(`Téléchargement de ${url}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Échec du téléchargement : ${res.status}`);
  const data = Buffer.from(await res.arrayBuffer());
  const digest = createHash("sha256").update(data).digest("hex");
  if (digest !== spec.sha256) throw new Error(`Empreinte invalide pour ${name} : ${digest}`);

  const work = mkdtempSync(join(tmpdir(), "chud-rclone-"));
  try {
    const zip = join(work, "rclone.zip");
    writeFileSync(zip, data);
    if (process.platform === "win32") {
      execFileSync("powershell.exe", ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${zip}' -DestinationPath '${work}'`]);
    } else {
      execFileSync("unzip", ["-q", zip, "-d", work]);
    }
    const dir = join(work, name);
    copyFileSync(join(dir, `rclone${spec.exe}`), out);
    if (!spec.exe) chmodSync(out, 0o755);
    console.log(`rclone ${VERSION} prêt : ${out}`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
