// Laboratoire de test : arborescences factices et tâches enregistrées, dans un dossier temporaire.
// Rien ici ne touche à un vrai disque ni aux données réelles de l'application.
import { randomBytes, randomUUID } from "node:crypto";
import { closeSync, mkdirSync, openSync, rmSync, utimesSync, writeFileSync, writeSync } from "node:fs";
import { dirname, join } from "node:path";

const id = () => randomUUID().replaceAll("-", "");

function file(path, content, when) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  if (when) utimesSync(path, when, when);
}

function bigFile(path, megabytes) {
  mkdirSync(dirname(path), { recursive: true });
  const block = randomBytes(1024 * 1024);
  const fd = openSync(path, "w");
  for (let i = 0; i < megabytes; i += 1) writeSync(fd, block);
  closeSync(fd);
}

function task(name, mode, pairs, extra = {}) {
  const sources = [];
  const destinations = [];
  const links = [];
  for (const [source, destination, subfolder] of pairs) {
    let s = sources.find((e) => e.path === source);
    if (!s) sources.push((s = { id: id(), path: source }));
    let d = destinations.find((e) => e.path === destination);
    if (!d) destinations.push((d = { id: id(), path: destination }));
    links.push({ id: id(), sourceId: s.id, destinationId: d.id, subfolder: subfolder ?? "", enabled: true });
  }
  const now = Date.now();
  return { schema: 1, id: id(), name, description: "", sources, destinations, links, mode, createdAt: now, updatedAt: now, ...extra };
}

export function buildLab(root, { bigMegabytes = 400 } = {}) {
  rmSync(root, { recursive: true, force: true });
  const disks = join(root, "disques");
  const data = join(root, "donnees");
  const d = (...parts) => join(disks, ...parts);

  const old = new Date("2023-05-14T10:00:00");
  for (let i = 0; i < 240; i += 1) file(d("D", "Photos", `20${20 + (i % 5)}`, `vacances_${i}.jpg`), randomBytes(2048 + i * 300), old);
  for (let i = 0; i < 60; i += 1) file(d("D", "Documents", `rapport été ${i}.txt`), `contenu ${i}\n`.repeat(40), old);
  file(d("D", "Documents", "日本語 🚀 notes.md"), "unicode");
  bigFile(d("E", "Projets", "maquette.bin"), bigMegabytes);
  for (let i = 0; i < 40; i += 1) file(d("E", "Projets", "src", `module_${i}.rs`), `// module ${i}\n`.repeat(30), old);
  file(d("F", "Saves", "partie.sav"), randomBytes(4096), old);

  // Une destination déjà partiellement à jour, avec des fichiers à retirer en miroir.
  for (let i = 0; i < 20; i += 1) file(d("H", "Projets", "src", `module_${i}.rs`), `// module ${i}\n`.repeat(30), old);
  file(d("H", "Projets", "src", "module_3.rs"), "// ancienne version\n", new Date("2022-01-01"));
  for (let i = 0; i < 14; i += 1) file(d("H", "Projets", "obsolete", `ancien_${i}.log`), `ancien ${i}\n`);
  file(d("H", "Projets", "notes-a-supprimer.txt"), "à retirer");

  // Un dossier de téléchargements à nettoyer : installateurs périmés, téléchargement interrompu.
  const ancient = new Date("2024-01-10T10:00:00");
  file(d("K", "Downloads", "vlc-3.0.20-win64.exe"), randomBytes(3000), ancient);
  file(d("K", "Downloads", "vlc-3.0.21-win64.exe"), randomBytes(3100));
  file(d("K", "Downloads", "gimp-2.10-setup.msi"), randomBytes(5000), ancient);
  file(d("K", "Downloads", "film.mkv.crdownload"), randomBytes(8000), ancient);
  file(d("K", "Downloads", "facture-2024.pdf"), randomBytes(1200), ancient);
  file(d("K", "Downloads", "recent.pdf"), randomBytes(1200));

  // Une tâche à trente liaisons : l'écran de copie doit rester lisible avec une longue liste.
  const many = [];
  for (let i = 0; i < 30; i += 1) {
    for (let j = 0; j < 3; j += 1) file(d("M", `Dossier ${i}`, `fichier_${j}.txt`), `contenu ${i}-${j}
`.repeat(20), old);
    many.push([d("M", `Dossier ${i}`), d("N", "Multi", `Dossier ${i}`)]);
  }

  const tasks = [
    task("Sauvegarde PC", "update", [
      [d("D", "Photos"), d("G", "Backup", "Photos")],
      [d("D", "Documents"), d("G", "Backup", "Documents")],
      [d("E", "Projets"), d("H", "Backup", "Projets")],
      [d("F", "Saves"), d("H", "Backup", "Saves")],
    ], { description: "Tous les dossiers importants, vers les deux disques de sauvegarde." }),
    task("Archivage disques", "update", [
      [d("D", "Documents"), d("H", "Stockage"), "Documents"],
      [d("F", "Saves"), d("H", "Stockage"), "Saves"],
    ]),
    task("Miroir Projets", "mirror", [[d("E", "Projets"), d("H", "Projets")]], { description: "Image exacte du dossier de travail." }),
    task("Clé externe", "addOnly", [[d("D", "Photos"), "Q:\\NonBranchee\\Photos"]]),
    {
      ...task("À relier", "update", []),
      sources: [{ id: "src-a", path: d("D", "Photos") }, { id: "src-b", path: d("D", "Documents") }],
      destinations: [{ id: "dst-a", path: d("J", "Commun") }],
    },
    task("Beaucoup de dossiers", "update", many),
  ];
  mkdirSync(join(data, "tasks"), { recursive: true });
  writeFileSync(join(data, "settings.json"), JSON.stringify({ language: "fr", recentPaths: [d("F", "Saves"), d("D", "Photos"), d("G", "Backup", "Photos"), d("E", "Projets"), d("H", "Projets"), d("K", "Downloads")] }));
  for (const entry of tasks) writeFileSync(join(data, "tasks", `${entry.id}.json`), JSON.stringify(entry, null, 2));
  return { root, disks, data, tasks, d };
}
