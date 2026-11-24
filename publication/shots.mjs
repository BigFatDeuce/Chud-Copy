// Captures de présentation : vraie application, données fictives, lecteurs virtuels S: à V:.
import { randomBytes, randomUUID } from "node:crypto";
import { closeSync, mkdirSync, openSync, rmSync, utimesSync, writeFileSync, writeSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [project, exe, root, shots] = process.argv.slice(2);
const { launch } = await import(pathToFileURL(join(project, "tests/e2e/driver.mjs")).href);

const id = () => randomUUID().replaceAll("-", "");
const old = new Date("2025-03-14T10:00:00");
const data = join(root, "data");

function file(path, content, when = old) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  utimesSync(path, when, when);
}

function big(path, megabytes) {
  mkdirSync(dirname(path), { recursive: true });
  const block = randomBytes(1024 * 1024);
  const fd = openSync(path, "w");
  for (let i = 0; i < megabytes; i += 1) writeSync(fd, block);
  closeSync(fd);
}

function task(name, mode, pairs, description = "") {
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
  return { schema: 1, id: id(), name, description, sources, destinations, links, mode, createdAt: Date.now(), updatedAt: Date.now() };
}

const disk = (letter, ...parts) => join(root, "disks", letter, ...parts);
for (let i = 0; i < 180; i += 1) file(disk("S", "Photos", `20${21 + (i % 5)}`, `IMG_${4000 + i}.jpg`), randomBytes(4096 + i * 400));
for (let i = 0; i < 48; i += 1) file(disk("S", "Documents", "Invoices", `invoice-${1000 + i}.pdf`), randomBytes(3000));
for (let i = 0; i < 24; i += 1) file(disk("T", "Projects", "app", "src", `module_${i}.rs`), `// module ${i}\n`.repeat(40));
big(disk("T", "Projects", "renders", "final-cut.mov"), 7000);
file(disk("T", "Saves", "slot-1.sav"), randomBytes(8192));
for (let i = 0; i < 12; i += 1) file(disk("V", "Projects", "app", "src", `module_${i}.rs`), `// module ${i}\n`.repeat(40));
for (let i = 0; i < 14; i += 1) file(disk("V", "Projects", "old-builds", `build-${i}.log`), `log ${i}\n`);
file(disk("V", "Projects", "scratch-notes.txt"), "to remove");
mkdirSync(disk("U"), { recursive: true });

const tasks = [
  task("PC Backup", "update", [
    ["S:\\Photos", "U:\\Backup\\Photos"],
    ["S:\\Documents", "U:\\Backup\\Documents"],
    ["T:\\Projects", "V:\\Backup\\Projects"],
    ["T:\\Saves", "V:\\Backup\\Saves"],
  ], "Everything that matters, to both backup disks."),
  task("Archive Drive", "addOnly", [
    ["S:\\Documents", "U:\\Archive", "Documents"],
    ["T:\\Saves", "U:\\Archive", "Saves"],
  ]),
  task("Project Mirror", "mirror", [["T:\\Projects", "V:\\Projects"]], "An exact image of the working folder."),
  task("Travel SSD", "update", [["S:\\Photos", "X:\\Photos"]]),
];
mkdirSync(join(data, "tasks"), { recursive: true });
writeFileSync(join(data, "settings.json"), JSON.stringify({ language: "en", quality: "balanced" }));
for (const entry of tasks) writeFileSync(join(data, "tasks", `${entry.id}.json`), JSON.stringify(entry, null, 2));

const session = await launch({ exe, dataDir: data, shots });
const { page, shot } = session;
const card = (name) => page.locator(".task-card").filter({ has: page.getByRole("heading", { name, exact: true }) });

// Les vrais disques de la machine n'ont rien à faire sur une page publique.
const hideDisks = () =>
  page.evaluate(() => {
    document.querySelectorAll(".disk-chips").forEach((e) => (e.style.display = "none"));
    const panels = document.querySelectorAll(".home-side .panel");
    if (panels[1]) panels[1].style.display = "none";
  });

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });
  await card("Archive Drive").getByRole("button", { name: "Run" }).click();
  await page.getByRole("button", { name: "View report" }).waitFor({ timeout: 30000 });
  await page.getByRole("button", { name: "Home" }).click();
  await page.waitForSelector(".task-card");
  await page.waitForTimeout(2500);
  await hideDisks();
  await page.waitForTimeout(300);
  await shot("home");

  await card("PC Backup").locator("[aria-label='More actions']").click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  await page.waitForSelector(".route-stage");
  await page.waitForTimeout(900);
  await hideDisks();
  await page.waitForTimeout(300);
  await shot("editor");
  await page.getByRole("button", { name: "Back", exact: true }).click();

  await page.waitForSelector(".task-card");
  await card("Project Mirror").getByRole("button", { name: "Analyze" }).click();
  await page.waitForSelector(".tiles", { timeout: 30000 });
  await page.waitForTimeout(900);
  await shot("analysis");

  await page.getByRole("button", { name: "Home" }).click();
  await page.waitForSelector(".task-card");
  await card("PC Backup").getByRole("button", { name: "Run" }).click();
  await page.waitForSelector(".lp-current li", { timeout: 30000 });
  await page.waitForTimeout(4200);
  await shot("transfer");
  console.log("Erreurs console :", session.errors.length ? session.errors : "aucune");
  console.log(await page.locator(".disk-list").count());
} finally {
  await session.close();
}
