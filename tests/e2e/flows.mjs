// Parcours complets sur la vraie application, avec de vrais fichiers temporaires.
// Usage : node tests/e2e/flows.mjs <exe> <dossier-temporaire> <dossier-captures>
import assert from "node:assert/strict";
import { execSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"), { bigMegabytes: 4000 });
const { d } = lab;

function tree(dir) {
  const out = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else out.push(relative(dir, path).replaceAll("\\", "/"));
    }
  };
  if (existsSync(dir)) walk(dir);
  return out.sort();
}

const passed = [];
async function step(name, body) {
  await body();
  passed.push(name);
  console.log(`ok - ${name}`);
}

const card = (page, name) => page.locator(".task-card").filter({ has: page.getByRole("heading", { name, exact: true }) });
const home = (page) => page.getByRole("button", { name: "Accueil" }).click();
const statusText = (page) => page.locator(".run-hero .status-tag").innerText();
/** Choisit un dossier récent du sélecteur, désigné par son chemin exact. */
const chip = (scope, path) => scope.locator(`.chip-choice[title=${JSON.stringify(path)}]`).first();
const tools = async (page, name) => {
  await page.locator(".rail").getByRole("button", { name: "Outils" }).click();
  await page.locator(".tool-card", { hasText: name }).click();
};
/** Choisit une valeur dans une liste déroulante de l'application, désignée par son sélecteur. */
const choose = async (page, selector, label) => {
  await page.locator(selector).click();
  await page.locator(".select-list").getByRole("option", { name: label, exact: true }).click();
};
/** Center of an element once its box stops moving (transitions finished). */
async function settledCenter(page, locator) {
  let previous;
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const box = await locator.boundingBox();
    if (previous && box && ["x", "y", "width", "height"].every((k) => Math.abs(box[k] - previous[k]) < 0.5)) {
      return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    }
    previous = box;
    await page.waitForTimeout(100);
  }
  throw new Error("element never settled");
}
const cardNames = async (page) => (await page.locator(".task-card h3").allInnerTexts()).filter((name) => !name.includes("importée"));
let expectedOrder = [];

/** Lance l'analyse depuis l'accueil, qui ne change pas d'onglet, puis l'ouvre. */
async function analyze(page, name) {
  const target = card(page, name).first();
  await target.getByRole("button", { name: "Analyser" }).click();
  await target.locator(".btn.analyzing, .btn.analyzed").first().click();
}

async function untilRunEnds(page, timeout = 120000) {
  await page.waitForFunction(() => {
    const tag = document.querySelector(".run-hero .status-tag")?.textContent ?? "";
    return tag && !["En cours", "En pause", "Arrêt en cours"].includes(tag);
  }, undefined, { timeout });
}

let session = await launch({ exe, dataDir: lab.data, shots });
let { page, shot } = session;

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });

  await step("aucune opération ne démarre à l'ouverture", async () => {
    await page.waitForTimeout(1500);
    assert.equal(await page.locator(".rail-live").count(), 0);
    assert.deepEqual(tree(d("G")), []);
  });

  await step("un disque absent garde la tâche et la signale", async () => {
    await card(page, "Clé externe").getByText("indisponible").waitFor();
    assert.equal(await card(page, "Clé externe").getByRole("button", { name: "Exécuter" }).isDisabled(), true);
  });

  await step("les tâches se réordonnent au glisser, puis au clavier", async () => {
    const before = await cardNames(page);
    const handle = await page.locator(".task-card").nth(0).locator(".drag-handle").boundingBox();
    const target = await page.locator(".task-card").nth(2).boundingBox();
    await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
    await page.mouse.down();
    await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 14 });
    await page.waitForTimeout(450);
    await shot("02-glisser");
    await page.mouse.up();
    await page.waitForTimeout(600);
    const dropped = await cardNames(page);
    assert.notEqual(dropped.indexOf(before[0]), 0, dropped.join());
    assert.deepEqual([...dropped].sort(), [...before].sort());
    let saved = JSON.parse(readFileSync(join(lab.data, "settings.json"), "utf8")).taskOrder;
    assert.deepEqual(saved.map((id) => lab.tasks.find((t) => t.id === id)?.name), dropped, "l'ordre affiché est celui enregistré");

    const moved = dropped[2];
    await page.locator(".task-card").nth(2).locator(".drag-handle").focus();
    await page.keyboard.press("ArrowUp");
    await page.waitForTimeout(600);
    expectedOrder = await cardNames(page);
    assert.equal(expectedOrder[1], moved);
    saved = JSON.parse(readFileSync(join(lab.data, "settings.json"), "utf8")).taskOrder;
    assert.deepEqual(saved.map((id) => lab.tasks.find((t) => t.id === id)?.name), expectedOrder);
  });

  await step("une couleur de cadre se choisit et suit la tâche", async () => {
    await card(page, "Sauvegarde PC").locator("[aria-label=\"Plus d'actions\"]").click();
    await page.locator(".menu .swatch[aria-label='#ff4f82']").click();
    await page.locator(".task-card.tinted").filter({ hasText: "Sauvegarde PC" }).waitFor();
    const border = await card(page, "Sauvegarde PC").evaluate((el) => getComputedStyle(el).borderTopColor);
    assert.notEqual(border, await card(page, "Archivage disques").evaluate((el) => getComputedStyle(el).borderTopColor));
    assert.deepEqual(await cardNames(page), expectedOrder, "changer la couleur ne déplace pas la tâche");
    await page.waitForTimeout(300);
    await shot("03-couleur");
  });

  await step("le mode discret brouille les chemins et se révèle au survol", async () => {
    await page.getByRole("button", { name: "Discret" }).click();
    await page.waitForFunction(() => document.body.classList.contains("discreet"));
    const filter = await page.locator(".glyph text").first().evaluate((el) => getComputedStyle(el).filter);
    assert.ok(filter.includes("dust"), filter);
    await page.waitForTimeout(300);
    await shot("04-discret");
    await page.getByRole("button", { name: "Discret" }).click();
    await page.waitForFunction(() => !document.body.classList.contains("discreet"));
    // L'enregistrement des réglages est asynchrone : on attend qu'il soit fait.
    const saved = () => JSON.parse(readFileSync(join(lab.data, "settings.json"), "utf8")).discreet;
    for (let i = 0; i < 20 && saved() !== false; i += 1) await page.waitForTimeout(100);
    assert.equal(saved(), false);
  });

  await step("créer des liaisons au clic puis au glisser", async () => {
    await card(page, "À relier").locator("[aria-label=\"Plus d'actions\"]").click();
    await page.getByRole("menuitem", { name: "Modifier" }).click();
    await page.waitForSelector(".route-stage");
    const sourcePorts = page.locator(".route-column.source .port");
    const destinationPort = page.locator(".route-column.destination .port").first();

    await sourcePorts.nth(0).click();
    await destinationPort.click();
    await page.waitForSelector(".link-row");
    assert.equal(await page.locator(".link-row").count(), 1);

    // A linked port grows through a CSS transition: aim at its settled center, not a corner.
    const from = await settledCenter(page, sourcePorts.nth(1));
    const to = await settledCenter(page, destinationPort);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(to.x, to.y, { steps: 8 });
    await page.mouse.up();
    await page.waitForFunction(() => document.querySelectorAll(".link-row").length === 2);

    // Deux sources vers une même destination : chacune reçoit son propre sous-dossier.
    const subfolders = await page.locator(".link-sub").evaluateAll((inputs) => inputs.map((i) => i.value));
    assert.deepEqual(subfolders.sort(), ["Documents", "Photos"]);
    await page.waitForTimeout(500);
    await shot("10-liaisons");
  });

  await step("une collision bloque l'analyse", async () => {
    await page.locator(".link-sub").nth(0).fill("Commun");
    await page.locator(".link-sub").nth(1).fill("commun");
    await page.locator(".issue.error").first().waitFor();
    assert.equal(await page.getByRole("button", { name: "Analyser" }).isDisabled(), true);
    await page.locator(".link-sub").nth(1).fill("Autre");
    await page.waitForFunction(() => !document.querySelector(".editor-foot .issue.error"));
  });

  await step("modifications non enregistrées retenues puis sauvegardées", async () => {
    await home(page);
    await page.getByRole("heading", { name: "Modifications non enregistrées" }).waitFor();
    await page.locator(".modal").getByRole("button", { name: "Enregistrer", exact: true }).click();
    await page.waitForSelector(".task-card");
    const saved = lab.tasks.find((t) => t.name === "À relier");
    const stored = JSON.parse(readFileSync(join(lab.data, "tasks", `${saved.id}.json`), "utf8"));
    assert.equal(stored.links.length, 2);
  });

  await step("une notification ne couvre aucun bouton", async () => {
    await card(page, "Archivage disques").locator("[aria-label=\"Plus d'actions\"]").click();
    await page.getByRole("menuitem", { name: "Modifier" }).click();
    await page.waitForSelector(".route-stage");
    await page.getByLabel("Description").fill("Essai de notification");
    await page.getByRole("button", { name: "Enregistrer", exact: true }).click();
    await page.waitForSelector(".toast");
    const covered = await page.evaluate(() => {
      const toast = document.querySelector(".toast").getBoundingClientRect();
      return [...document.querySelectorAll("button")]
        .filter((button) => {
          const box = button.getBoundingClientRect();
          return box.width > 0 && box.left < toast.right && box.right > toast.left && box.top < toast.bottom && box.bottom > toast.top;
        })
        .map((button) => button.textContent.trim());
    });
    assert.deepEqual(covered, []);
    await page.getByRole("button", { name: "Retour", exact: true }).click();
    await page.waitForSelector(".task-card");
  });

  await step("miroir : analyse, confirmation, suppressions exactes", async () => {
    await card(page, "Miroir Projets").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForSelector(".tile.delete", { timeout: 30000 });
    assert.ok(existsSync(d("H", "Projets", "notes-a-supprimer.txt")), "l'analyse ne doit rien supprimer");
    await page.getByRole("button", { name: "Vérifier et lancer" }).click();
    const launchButton = page.locator(".modal.danger .btn.danger");
    assert.equal(await launchButton.isDisabled(), true);
    await page.locator(".modal.danger input[type=checkbox]").check();
    // Plus de la moitié du dossier ? Le mot de confirmation devient obligatoire.
    if (await page.locator(".modal.danger .field input").count()) await page.locator(".modal.danger .field input").fill("SUPPRIMER");
    await launchButton.click();
    await page.waitForSelector(".run-hero");
    await page.waitForFunction(() => document.querySelector(".percent") !== null);
    await page.waitForTimeout(1200);
    await shot("11-execution");
    await untilRunEnds(page);
    assert.equal(await statusText(page), "Terminée");
    await page.waitForTimeout(600);
    await shot("12-termine");
    assert.deepEqual(tree(d("H", "Projets")), tree(d("E", "Projets")));
  });

  await step("rapport et journal consultables", async () => {
    await page.getByRole("button", { name: "Voir le rapport" }).click();
    await page.waitForSelector(".journal .log");
    await page.getByLabel("Rechercher dans le journal").fill("Deleted");
    await page.waitForFunction(() => document.querySelector(".log")?.textContent?.includes("notes-a-supprimer"), undefined, { timeout: 5000 });
    await page.waitForTimeout(300);
    await shot("13-rapport");
  });

  await step("une comparaison longue montre sa progression", async () => {
    await home(page);
    await page.waitForSelector(".task-card");
    await page.evaluate(async () => {
      const { tasks } = await window.__TAURI_INTERNALS__.invoke("tasks_list");
      const task = tasks.find((t) => t.name === "Miroir Projets");
      await window.__TAURI_INTERNALS__.invoke("task_save", { task: { ...task, options: { ...task.options, compare: "checksum" } } });
    });
    await analyze(page, "Miroir Projets");
    await page.locator(".scan-files li", { hasText: "maquette.bin" }).waitFor({ timeout: 20000 });
    await page.getByText("Fichiers comparés").waitFor();
    assert.equal(await page.locator(".scan .fx-clouds").count(), 1, "l'analyse en cours se voile de nuages");
    await shot("18-comparaison");
    await page.getByRole("button", { name: "Arrêter l'analyse" }).click();
    await page.waitForSelector(".tiles", { timeout: 20000 });
  });

  await step("plusieurs analyses en parallèle, suivies dans leur vue", async () => {
    await home(page);
    await page.waitForSelector(".task-card");
    await analyze(page, "Miroir Projets");
    await page.locator(".scan-files li", { hasText: "maquette.bin" }).waitFor({ timeout: 20000 });
    await home(page);
    await page.waitForSelector(".task-card");
    await analyze(page, "Archivage disques");
    await page.waitForSelector(".tiles", { timeout: 20000 });
    await page.getByRole("button", { name: "Analyses" }).click();
    const mirror = page.locator(".analysis-row", { hasText: "Miroir Projets" });
    await mirror.waitFor();
    assert.equal(await page.locator(".analysis-row").count(), 2);
    assert.equal(await mirror.locator(".status-tag").innerText(), "En cours", "la seconde analyse doit tourner pendant la première");
    assert.equal(await page.locator(".rail-badge").innerText(), "1");
    await shot("19-analyses");
    await mirror.getByRole("button", { name: "Arrêter l'analyse" }).click();
    await mirror.locator(".status-tag", { hasText: "Annulée" }).waitFor({ timeout: 20000 });
    await page.locator(".analysis-row", { hasText: "Archivage disques" }).getByRole("button", { name: "Ouvrir" }).click();
    await page.waitForSelector(".tiles");
    assert.equal(await page.locator(".scan").count(), 0, "ouvrir une analyse ne doit pas la relancer");
  });

  await step("une analyse lancée de l'accueil y reste, puis l'onglet Analyses s'allume", async () => {
    await home(page);
    await page.waitForSelector(".task-card");
    await card(page, "Archivage disques").getByRole("button", { name: "Analyser" }).click();
    await card(page, "Archivage disques").locator(".btn.analyzed").waitFor({ timeout: 20000 });
    assert.equal(await page.locator(".task-grid").count(), 1, "l'accueil reste affiché");
    await page.locator(".rail-item.attention").waitFor();
    await page.waitForTimeout(300);
    await shot("19b-analyse-terminee");
    await page.getByRole("button", { name: "Analyses" }).click();
    await page.waitForFunction(() => !document.querySelector(".rail-item.attention"));
  });

  await step("pause et reprise réelles depuis l'interface", async () => {
    await home(page);
    await card(page, "Sauvegarde PC").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForSelector(".run-hero");
    await page.waitForFunction(() => document.querySelector(".lp-current li"), undefined, { timeout: 20000 });
    assert.equal(await page.locator(".run-hero.tinted .fx-blaze").count(), 1, "le cadre en copie brûle, dans la couleur de sa tâche");
    assert.equal(await page.locator(".titlebar .fx-laser").count(), 1, "le laser passe derrière le titre pendant la copie");
    await shot("13b-flammes");
    await page.getByRole("button", { name: "Pause" }).click();
    await page.getByRole("button", { name: "Reprendre" }).waitFor();
    await page.waitForTimeout(1500);
    const read = () => page.locator(".metrics dd").nth(2).innerText();
    const before = await read();
    await page.waitForTimeout(1500);
    assert.equal(await read(), before, "le compteur doit s'arrêter pendant la pause");
    assert.equal(await statusText(page), "En pause");
    await shot("14-pause");
    await page.getByRole("button", { name: "Reprendre" }).click();
    await page.getByRole("button", { name: "Pause" }).waitFor();
  });

  await step("annulation : aucun fichier incomplet en destination", async () => {
    await page.getByRole("button", { name: "Annuler" }).click();
    await page.locator(".modal").getByRole("button", { name: "Annuler l'opération" }).click();
    await untilRunEnds(page, 30000);
    assert.equal(await statusText(page), "Annulée");
    const files = tree(d("H", "Backup", "Projets"));
    assert.equal(files.filter((f) => f.includes(".partial")).length, 0, files.join());
    assert.equal(files.includes("maquette.bin"), false);
  });

  await step("file d'attente : une annulation la retient, la reprise la relance", async () => {
    await home(page);
    await card(page, "Sauvegarde PC").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForFunction(() => document.querySelector(".lp-current li"), undefined, { timeout: 20000 });
    await home(page);
    await card(page, "Archivage disques").getByRole("button", { name: "Mettre en file" }).click();
    await page.locator(".queue").getByText("Archivage disques").waitFor();
    assert.equal(await page.locator(".queue").getByText("File en pause").count(), 0);
    assert.equal(await card(page, "Archivage disques").getByRole("button", { name: "En file" }).isDisabled(), true);
    await page.waitForTimeout(400);
    await shot("15-file-attente");

    await page.locator(".rail-live").click();
    await page.getByRole("button", { name: "Annuler", exact: true }).click();
    await page.locator(".modal").getByRole("button", { name: "Annuler l'opération" }).click();
    await untilRunEnds(page, 30000);
    await page.locator(".queue").getByText("File en pause").waitFor({ timeout: 5000 });
    await page.waitForTimeout(1500);
    assert.deepEqual(tree(d("H", "Stockage")), [], "une annulation ne doit pas lancer la suite");
    assert.equal(await statusText(page), "Annulée");

    await page.getByRole("button", { name: "Reprendre la file" }).click();
    await page.waitForFunction(() => !document.querySelector(".queue"), undefined, { timeout: 10000 });
    await page.getByRole("heading", { name: "Archivage disques" }).waitFor({ timeout: 10000 });
    await untilRunEnds(page, 30000);
    assert.ok(tree(d("H", "Stockage")).includes("Saves/partie.sav"));
  });

  await step("une tâche déjà à jour se lance et se marque terminée", async () => {
    await home(page);
    await page.waitForSelector(".task-card");
    await analyze(page, "Archivage disques");
    await page.waitForSelector(".tiles", { timeout: 20000 });
    await page.getByRole("button", { name: "Lancer l'opération" }).click();
    await untilRunEnds(page, 30000);
    assert.equal(await statusText(page), "Terminée");
  });

  await step("une copie à trente liaisons garde son en-tête entier", async () => {
    await home(page);
    await card(page, "Beaucoup de dossiers").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForSelector(".run-hero");
    await untilRunEnds(page, 60000);
    assert.equal(await statusText(page), "Terminée");
    assert.equal(await page.locator(".link-progress").count(), 30);
    // Avant la correction, la vue serrait l'en-tête pour loger la liste : son titre sortait de sa case.
    const hero = await page.locator(".run-hero").evaluate((el) => ({ client: el.clientHeight, scroll: el.scrollHeight }));
    assert.ok(hero.scroll <= hero.client + 1, `en-tête écrasé : ${JSON.stringify(hero)}`);
    for (const selector of [".run-title h1", ".run-result", ".metrics", ".run-actions"]) {
      const inside = await page.evaluate((sel) => {
        const box = document.querySelector(sel).getBoundingClientRect();
        const frame = document.querySelector(".run-hero").getBoundingClientRect();
        return box.top >= frame.top - 1 && box.bottom <= frame.bottom + 1;
      }, selector);
      assert.ok(inside, `${selector} déborde de l'en-tête`);
    }
    assert.equal(tree(d("N", "Multi")).length, 90);
    await shot("26-trente-liaisons");
  });

  await step("l'extinction en fin de copie s'arme, compte, et s'annule", async () => {
    const witness = join(lab.data, "extinction-simulee.txt");
    await home(page);
    const toggle = page.locator(".home-side .power-toggle input");
    // Cocher après une copie déjà finie n'éteint rien.
    await page.locator(".home-side .power-toggle label").click();
    assert.equal(await toggle.isChecked(), true);
    await page.waitForTimeout(1500);
    assert.equal(await page.locator(".modal", { hasText: "Extinction du PC" }).count(), 0);
    // Une copie démarre puis se termine : le compte à rebours s'affiche, et s'annule.
    rmSync(d("N", "Multi"), { recursive: true, force: true });
    await card(page, "Beaucoup de dossiers").getByRole("button", { name: "Exécuter" }).click();
    await page.locator(".modal", { hasText: "Extinction dans" }).waitFor({ timeout: 60000 });
    await shot("27-extinction");
    await page.getByRole("button", { name: "Annuler l'extinction" }).click();
    await page.waitForTimeout(1500);
    assert.equal(await page.locator(".modal", { hasText: "Extinction du PC" }).count(), 0);
    assert.equal(existsSync(witness), false, "une extinction annulée n'éteint rien");
    await home(page);
    assert.equal(await toggle.isChecked(), false, "l'annulation décoche la case");
    // Une copie annulée par l'utilisateur désarme l'extinction.
    await page.locator(".home-side .power-toggle label").click();
    await card(page, "Sauvegarde PC").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForFunction(() => document.querySelector(".lp-current li"), undefined, { timeout: 20000 });
    await page.locator(".run-actions").getByRole("button", { name: "Annuler" }).click();
    await page.locator(".modal .btn.danger").click();
    await untilRunEnds(page, 30000);
    await page.waitForTimeout(1500);
    assert.equal(await page.locator(".modal", { hasText: "Extinction du PC" }).count(), 0);
    assert.equal(existsSync(witness), false);
    // « Éteindre maintenant » demande l'extinction au cœur (simulée ici).
    await home(page);
    await page.locator(".home-side .power-toggle label").click();
    rmSync(d("N", "Multi"), { recursive: true, force: true });
    await card(page, "Beaucoup de dossiers").getByRole("button", { name: "Exécuter" }).click();
    await page.locator(".modal", { hasText: "Extinction dans" }).waitFor({ timeout: 60000 });
    await page.getByRole("button", { name: "Éteindre maintenant" }).click();
    await page.waitForFunction(() => !document.querySelector(".modal"), undefined, { timeout: 10000 });
    assert.ok(existsSync(witness), "l'extinction a bien été demandée");
    rmSync(witness);
  });

  await step("export et import réels", async () => {
    const task = lab.tasks[0];
    const file = join(lab.root, "export.chudcopy");
    await page.evaluate(async ([id, path]) => window.__TAURI_INTERNALS__.invoke("tasks_export", { ids: [id], path }), [task.id, file]);
    assert.ok(statSync(file).size > 100);
    const imported = await page.evaluate(async (path) => window.__TAURI_INTERNALS__.invoke("tasks_import", { path }), file);
    assert.equal(imported.length, 1);
    assert.notEqual(imported[0].id, task.id);
  });

  await step("le réglage d'animation s'applique et persiste", async () => {
    await page.getByRole("button", { name: "Réglages" }).click();
    await page.getByRole("radio", { name: /Désactivé/ }).click();
    await page.waitForTimeout(400);
    await shot("15-reglages");
    const version = await page.evaluate(async () => (await window.__TAURI_INTERNALS__.invoke("app_info")).version);
    // Réponse simulée : le test ne dépend ni du réseau ni de la dernière version publiée.
    await page.evaluate((current) => {
      window.fetch = async () => new Response(JSON.stringify({ tag_name: `v${current}` }), { status: 200 });
    }, version);
    await page.getByRole("button", { name: "Rechercher une mise à jour" }).click();
    await page.getByText(`dernière version (${version})`).waitFor({ timeout: 15000 });
    await page.evaluate(() => {
      window.fetch = async () => new Response(JSON.stringify({ tag_name: "v9.9.9" }), { status: 200 });
    });
    await page.getByRole("button", { name: "Rechercher une mise à jour" }).click();
    await page.getByText("Version 9.9.9 disponible").waitFor({ timeout: 5000 });
    await page.getByRole("button", { name: "Télécharger" }).waitFor();
    await page.evaluate(() => {
      window.fetch = async () => {
        throw new Error("hors ligne");
      };
    });
    await page.getByRole("button", { name: "Rechercher une mise à jour" }).click();
    await page.getByText("Impossible de vérifier").waitFor({ timeout: 5000 });
    assert.equal(JSON.parse(readFileSync(join(lab.data, "settings.json"), "utf8")).quality, "off");
    await page.getByRole("radio", { name: /Équilibré/ }).click();
  });

  await step("fermeture brutale en pleine copie", async () => {
    await home(page);
    // Le gros fichier doit être en cours de copie au moment de couper : sa copie précédente est retirée du laboratoire.
    rmSync(d("H", "Backup", "Projets", "maquette.bin"), { force: true });
    await card(page, "Sauvegarde PC").getByRole("button", { name: "Exécuter" }).click();
    await page.waitForFunction(() => document.querySelector(".lp-current li"), undefined, { timeout: 20000 });
    await page.waitForTimeout(300);
    const pid = session.app.pid;
    execSync(`taskkill /PID ${pid} /F`, { stdio: "ignore" });
    await session.close();
    await new Promise((r) => setTimeout(r, 1800));
    const engine = join(exe, "..", "rclone.exe").toLowerCase();
    const running = execSync(`powershell -NoProfile -Command "Get-Process rclone -ErrorAction SilentlyContinue | ForEach-Object { $_.Path }; exit 0"`, { encoding: "utf8" });
    assert.equal(running.toLowerCase().includes(engine), false, "le moteur doit s'arrêter avec l'application");
  });

  session = await launch({ exe, dataDir: lab.data, shots });
  ({ page, shot } = session);
  await page.waitForSelector(".task-card", { timeout: 15000 });

  await step("au redémarrage tout est là, rien ne repart seul", async () => {
    assert.ok((await page.locator(".task-card").count()) >= 5);
    assert.deepEqual(await cardNames(page), expectedOrder, "l'ordre choisi survit aux copies et au redémarrage");
    assert.match(await card(page, "Sauvegarde PC").getAttribute("class"), /tinted/, "la couleur survit au redémarrage");
    await page.waitForTimeout(1500);
    assert.equal(await page.locator(".rail-live").count(), 0);
    await page.getByRole("button", { name: "Historique" }).click();
    await page.getByText("Interrompue", { exact: true }).first().waitFor();
    await page.waitForTimeout(400);
    await shot("16-historique");
  });

  await step("le reste d'une copie coupée est signalé à l'analyse", async () => {
    await home(page);
    await analyze(page, "Sauvegarde PC");
    await page.waitForSelector(".tiles", { timeout: 30000 });
    const partial = tree(d("H", "Backup", "Projets")).filter((f) => f.endsWith(".partial"));
    if (partial.length) {
      await page.getByRole("button", { name: /Erreurs et conflits/ }).click();
      await page.getByText("Copie interrompue").first().waitFor();
      await page.waitForTimeout(300);
      await shot("17-copie-interrompue");
    }
  });

  await step("un miroir validé en file attend son tour puis s'exécute", async () => {
    const extra = d("H", "Projets", "en-trop.txt");
    writeFileSync(extra, "à retirer");
    await home(page);
    await card(page, "Sauvegarde PC").first().getByRole("button", { name: "Exécuter" }).click();
    await page.waitForFunction(() => document.querySelector(".lp-current li"), undefined, { timeout: 20000 });
    // En pause, la copie reste en cours aussi longtemps que dure l'analyse.
    await page.getByRole("button", { name: "Pause" }).click();
    await page.getByRole("button", { name: "Reprendre" }).waitFor();
    await home(page);
    await analyze(page, "Miroir Projets");
    // The 4 GB file is re-read on both sides to compare content: slow disks need more than 30 s.
    await page.waitForSelector(".tiles", { timeout: 120000 });
    await page.getByRole("button", { name: "Vérifier et mettre en file" }).click();
    await page.locator(".modal input[type=checkbox]").check();
    await page.locator(".modal .btn.danger").click();
    await page.locator(".queue").getByText("Miroir Projets").waitFor();
    assert.equal(existsSync(extra), true, "rien ne se supprime avant son tour");
    await page.getByRole("button", { name: "Reprendre", exact: true }).click();
    await page.getByRole("heading", { name: "Miroir Projets" }).waitFor({ timeout: 120000 });
    await page.waitForFunction(() => !document.querySelector(".queue"));
    await untilRunEnds(page, 180000);
    assert.equal(await statusText(page), "Terminée");
    assert.equal(existsSync(extra), false);
    assert.equal(existsSync(d("E", "Projets", "maquette.bin")), true);
  });

  await step("tout l'historique s'efface, après confirmation seulement", async () => {
    await page.getByRole("button", { name: "Historique" }).click();
    await page.waitForSelector(".history-list li");
    const before = await page.locator(".history-list li").count();
    assert.ok(before > 1);
    await page.getByRole("button", { name: "Tout effacer" }).click();
    await page.waitForTimeout(300);
    await shot("18-effacer-historique");
    await page.locator(".modal").getByRole("button", { name: "Conserver" }).click();
    assert.equal(await page.locator(".history-list li").count(), before);
    await page.getByRole("button", { name: "Tout effacer" }).click();
    await page.locator(".modal").getByRole("button", { name: "Tout effacer" }).click();
    await page.getByText("Vos opérations apparaîtront ici").waitFor({ timeout: 5000 });
    assert.equal((await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("history_list"))).length, 0);
    assert.equal(await page.getByRole("button", { name: "Tout effacer" }).count(), 0);
    await home(page);
    await page.waitForSelector(".task-card");
    assert.ok((await page.locator(".task-card").count()) >= 5);
    assert.ok(tree(d("G", "Backup")).length > 0, "les copies restent en place");
    const status = await card(page, "Sauvegarde PC").locator(".task-status").innerText();
    assert.notEqual(status.trim(), "Jamais exécutée", "la dernière exécution reste affichée");
  });

  await step("le contrôle d'intégrité repère une corruption silencieuse", async () => {
    const folder = d("F", "Saves");
    await tools(page, "Intégrité");
    await page.locator(".chip-choice", { hasText: "Saves" }).click();
    await page.getByRole("button", { name: "Créer la référence" }).click();
    await page.getByText("Référence créée").waitFor({ timeout: 20000 });
    assert.deepEqual(tree(folder), ["partie.sav"], "rien n'est écrit dans le dossier contrôlé");

    // Un octet change, la taille et la date non : c'est ainsi que se présente une corruption.
    const file = join(folder, "partie.sav");
    const stats = statSync(file);
    const bytes = readFileSync(file);
    bytes[10] ^= 0xff;
    writeFileSync(file, bytes);
    utimesSync(file, stats.atime, stats.mtime);

    await page.getByRole("button", { name: "Vérifier" }).click();
    await page.getByText("Problèmes détectés : 1").waitFor({ timeout: 20000 });
    await page.locator(".finding-list li.corrupt", { hasText: "partie.sav" }).waitFor();
    await page.waitForTimeout(300);
    await shot("20-integrite");
    await page.getByRole("button", { name: "Compte rendu pour une IA" }).click();
    await page.getByText(/^Compte rendu copié/).waitFor({ timeout: 5000 });
  });

  await step("un contrôle reste affiché en changeant d'onglet, et sa référence se supprime", async () => {
    await home(page);
    await tools(page, "Intégrité");
    await page.getByText("Problèmes détectés : 1").waitFor();
    assert.equal(await page.locator(".reference-list li").count(), 1);
    await page.locator(".reference-list li").getByRole("button", { name: "Supprimer la référence" }).click();
    await page.locator(".modal").getByRole("button", { name: "Supprimer la référence" }).click();
    await page.getByText("Référence supprimée").waitFor();
    await page.waitForFunction(() => !document.querySelector(".reference-list li"), undefined, { timeout: 5000 });
    assert.deepEqual(tree(d("F", "Saves")), ["partie.sav"]);
  });

  await step("comparer une sauvegarde à sa source, sans rien copier", async () => {
    await tools(page, "Comparer deux dossiers");
    const sides = page.locator(".compare-sides .path-picker");
    await chip(sides.nth(0), d("D", "Photos")).click();
    await chip(sides.nth(1), d("G", "Backup", "Photos")).click();
    await page.getByRole("button", { name: "Comparer", exact: true }).click();
    await page.getByText("Les deux dossiers sont identiques").waitFor({ timeout: 30000 });
    await page.waitForTimeout(300);
    await shot("21-comparer");
  });

  await step("les doublons se trouvent par leur contenu", async () => {
    await tools(page, "Doublons");
    await chip(page, d("D", "Photos")).click();
    await chip(page, d("G", "Backup", "Photos")).click();
    await choose(page, ".field.inline .select", "1 Ko");
    await page.getByRole("button", { name: "Chercher les doublons" }).click();
    await page.getByText("Groupes de doublons : 240").waitFor({ timeout: 60000 });
    assert.ok((await page.locator(".dup-group").count()) > 10);
    await page.waitForTimeout(400);
    await shot("22-doublons");
  });

  // CHUD_TEST_SHARE : racine d'un vrai partage réseau de test, vide et jetable (ex. un lecteur
  // connecté à un dossier temporaire). Sans corbeille du système, la suppression doit être immédiate.
  if (process.env.CHUD_TEST_SHARE) await step("des doublons sur un partage réseau partent aussitôt dans sa corbeille", async () => {
    const share = process.env.CHUD_TEST_SHARE;
    const dir = join(share, "doublons-reseau");
    mkdirSync(dir, { recursive: true });
    const data = Buffer.alloc(32 * 1024 * 1024, 7);
    writeFileSync(join(dir, "a.bin"), data);
    writeFileSync(join(dir, "b.bin"), data);
    await tools(page, "Doublons");
    while (await page.locator(".root-list button").count()) await page.locator(".root-list button").first().click();
    await page.locator(".typed-path input").fill(dir);
    await page.locator(".typed-path").getByRole("button", { name: "Utiliser" }).click();
    await page.getByRole("button", { name: "Chercher les doublons" }).click();
    await page.getByText(/^Groupes de doublons : 1$/).waitFor({ timeout: 60000 });
    await page.locator(".dup-group").getByRole("button", { name: "Mettre les autres à la corbeille" }).click();
    await page.locator(".modal").getByText("_Corbeille Chud Copy", { exact: false }).waitFor();
    const started = Date.now();
    await page.locator(".modal").getByRole("button", { name: "Mettre 1 à la corbeille" }).click();
    await page.getByText(/Pas de corbeille à cet emplacement/).waitFor({ timeout: 10000 });
    assert.ok(Date.now() - started < 5000, `suppression trop lente : ${Date.now() - started} ms`);
    assert.equal(readdirSync(dir).filter((name) => name.endsWith(".bin")).length, 1, "une seule copie reste");
    assert.equal(tree(join(share, "_Corbeille Chud Copy")).filter((p) => p.endsWith(".bin")).length, 1, "la copie retirée est récupérable");
    await page.locator(".dup-group").waitFor({ state: "detached" });
    await shot("30-doublons-reseau");
  });

  await step("la carte de l'espace se parcourt", async () => {
    await tools(page, "Espace disque");
    await chip(page, d("E", "Projets")).click();
    await page.getByRole("button", { name: "Cartographier" }).click();
    await page.waitForSelector(".tm-cell", { timeout: 30000 });
    await page.waitForTimeout(600);
    await shot("23-espace");
    // Chaque nom affiché tient dans sa case, et aucune case n'est une bande étirée illisible.
    const cells = await page.locator(".tm-cell").evaluateAll((all) =>
      all.map((el) => {
        const box = el.getBoundingClientRect();
        const name = el.querySelector(".tm-name");
        return { w: box.width, h: box.height, named: !!name, fits: !name || name.scrollWidth <= name.clientWidth + 1 || name.textContent.length > 8 };
      }),
    );
    for (const cell of cells.filter((c) => c.named)) assert.ok(cell.w >= 72 && cell.h >= 48 && cell.fits, JSON.stringify(cell));
    assert.ok(await page.locator(".space-crumbs .space-open").isVisible(), "le dossier affiché s'ouvre dans l'explorateur");
    const rows = await page.locator(".space-list li").count();
    assert.equal(await page.locator(".space-list li .icon-btn").count(), rows, "chaque élément s'ouvre dans l'explorateur");
    await page.locator(".space-list .space-enter", { hasText: "src" }).click();
    await page.locator(".space-crumbs button", { hasText: "src" }).waitFor();
    await page.waitForSelector(".tm-cell");
    await page.locator(".space-crumbs button").first().click();
    await page.locator(".space-list .space-enter", { hasText: "src" }).waitFor();
  });

  await step("le test de lecture lit tout et ne trouve rien d'anormal", async () => {
    await tools(page, "Test de lecture");
    await chip(page, d("E", "Projets")).click();
    await page.getByRole("button", { name: "Lancer le test" }).click();
    await page.getByText("Aucun problème de lecture").waitFor({ timeout: 120000 });
    await page.waitForTimeout(500);
    await shot("24-lecture");
  });

  await step("une version archivée se retrouve et se liste", async () => {
    const archived = d("H", "Projets", "_Archives Chud Copy", "2026-11-02 10h00 00", "src", "ancien.rs");
    mkdirSync(dirname(archived), { recursive: true });
    writeFileSync(archived, "// ancienne version");
    await tools(page, "Restaurer depuis les archives");
    await chip(page, d("H", "Projets")).click();
    await page.locator(".version-list button", { hasText: "2026-11-02 10h00 00" }).click();
    await page.locator(".archive-files li", { hasText: "src/ancien.rs" }).waitFor();
    await page.getByRole("button", { name: "Tout sélectionner" }).click();
    await page.getByText("Sélectionnés : 1").waitFor();
    await shot("25-restaurer");
  });

  await step("le conseiller de nettoyage propose sans jamais rien supprimer seul", async () => {
    await tools(page, "Conseiller de nettoyage");
    await chip(page, d("K", "Downloads")).click();
    await page.getByRole("button", { name: "Analyser", exact: true }).click();
    await page.getByText("4 fichiers à examiner").waitFor({ timeout: 30000 });
    const kinds = await page.locator(".cleanup-list .kind-badge").allInnerTexts();
    assert.deepEqual(kinds.sort(), ["Anciennes versions", "Installateurs", "Téléchargements inachevés", "Vieux téléchargements"].sort());
    assert.equal(await page.locator(".cleanup-list li", { hasText: "vlc-3.0.21" }).count(), 0, "la version la plus récente reste");
    await page.locator(".cleanup-list li", { hasText: "gimp-2.10-setup.msi" }).locator("input").check();
    await page.getByText("1 cochés").waitFor();
    await page.getByRole("button", { name: "Mettre à la corbeille" }).click();
    await page.locator(".modal").getByText("gimp-2.10-setup.msi").waitFor();
    await shot("26-nettoyage");
    // Le test n'envoie rien dans la vraie corbeille : il annule.
    await page.locator(".modal").getByRole("button", { name: "Annuler" }).click();
    assert.equal(tree(d("K", "Downloads")).length, 6, "rien n'a été retiré");
  });

  await step("le testeur réseau mesure un dossier et retire son dossier de test", async () => {
    await tools(page, "Testeur réseau et NAS");
    await page.locator(".typed-path input").fill(d("E", "Projets"));
    await page.getByRole("button", { name: "Utiliser" }).click();
    await page.getByText("Tester aussi l'écriture").click();
    await choose(page, ".field.inline .select", "64 Mo");
    await page.getByRole("button", { name: "Lancer le test" }).click();
    await page.getByText("Partage accessible").waitFor({ timeout: 120000 });
    await page.locator(".net-stat", { hasText: "Données relues intactes" }).getByText("Oui").waitFor();
    assert.ok(!readdirSync(d("E", "Projets")).some((name) => name.startsWith(".chudcopy-network-test")), "dossier de test retiré");
    await page.waitForTimeout(400);
    await shot("27-reseau");
  });

  await step("la garde d'identité bloque un autre disque jusqu'à confirmation", async () => {
    // Le disque mémorisé pour la destination est remplacé par un disque inconnu.
    await page.evaluate(async () => {
      const invoke = window.__TAURI_INTERNALS__.invoke;
      const { tasks } = await invoke("tasks_list");
      const task = tasks.find((t) => t.name === "Miroir Projets");
      const fake = { id: "DISQUE-ETRANGER", label: "ETRANGER", fileSystem: "NTFS", mount: "Z:\\", kind: "removable", total: 1000, learnedAt: 1 };
      await invoke("task_save", { task: { ...task, destinations: task.destinations.map((d) => ({ ...d, identity: fake })) } });
    });
    await page.getByRole("button", { name: "Historique" }).click();
    await home(page);
    const alert = card(page, "Miroir Projets").locator(".drive-alert");
    await alert.waitFor({ timeout: 30000 });
    assert.ok(await card(page, "Miroir Projets").getByRole("button", { name: "Exécuter" }).isDisabled(), "copie bloquée");
    await shot("28-carte-autre-disque");
    await alert.click();
    const row = page.locator(".drive-list li.different");
    await row.waitFor({ timeout: 30000 });
    await shot("29-garde-disques");
    await row.getByRole("button", { name: "C'est le bon disque" }).click();
    await page.locator(".modal").getByRole("button", { name: "Oui, c'est le bon disque" }).click();
    await page.getByText("Disque mémorisé.").waitFor();
    await page.locator(".drive-list li.different").waitFor({ state: "detached", timeout: 30000 });
    assert.ok((await page.locator(".drive-list li.same").count()) > 0);
  });

  await step("un rappel signale une sauvegarde en retard, sans rien lancer", async () => {
    await page.evaluate(async () => {
      const { tasks } = await window.__TAURI_INTERNALS__.invoke("tasks_list");
      const task = tasks.find((t) => t.name === "Archivage disques");
      await window.__TAURI_INTERNALS__.invoke("task_save", { task: { ...task, reminderDays: 1 } });
    });
    const runs = join(lab.data, "last-runs.json");
    const last = JSON.parse(readFileSync(runs, "utf8"));
    const id = lab.tasks.find((t) => t.name === "Archivage disques").id;
    last[id].startedAt = Date.now() - 3 * 24 * 3600 * 1000;
    writeFileSync(runs, JSON.stringify(last));
    await page.getByRole("button", { name: "Historique" }).click();
    await home(page);
    await card(page, "Archivage disques").getByText("En retard : 3 j").waitFor();
    assert.equal(await page.locator(".rail-live").count(), 0, "un rappel ne lance jamais rien");
  });

  await step("la santé des disques s'affiche sans rien lancer seule", async () => {
    await page.getByRole("button", { name: "Santé disques" }).click();
    await page.getByRole("button", { name: "Analyser les disques" }).waitFor();
    assert.equal(await page.locator(".disk-health").count(), 0);
    await page.getByRole("button", { name: "Réglages" }).click();
    await page.getByText("Son de fin d'opération").waitFor();
    await page.getByText("Rechercher les mises à jour au lancement").waitFor();
  });

  await step("un dossier déposé rejoint la colonne visée", async () => {
    await home(page);
    await card(page, "Archivage disques").locator("[aria-label=\"Plus d'actions\"]").click();
    await page.getByRole("menuitem", { name: "Modifier" }).click();
    await page.waitForSelector(".route-stage");
    await page.waitForTimeout(500);
    const box = await page.locator(".route-column.destination").boundingBox();
    const scale = await page.evaluate(() => window.devicePixelRatio);
    // Le glissement réel vient de l'explorateur ; ici on envoie l'événement que Tauri lui substitue.
    await page.evaluate(
      ([path, x, y]) => window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event: "tauri://drag-drop", payload: { paths: [path], position: { x, y } } }),
      [d("J", "Depot"), (box.x + box.width / 2) * scale, (box.y + box.height - 30) * scale],
    );
    await page.locator(".route-column.destination .endpoint-name", { hasText: "Depot" }).waitFor({ timeout: 5000 });
  });

  if (process.env.CHUD_TEST_VISIBLE) await step("la boîte de dialogue native s'ouvre", async () => {
    await page.locator(".route-column.source").getByRole("button", { name: "Ajouter" }).click();
    await page.waitForTimeout(1800);
    const script = join(dirname(fileURLToPath(import.meta.url)), "dialog-open.ps1");
    const open = execSync(`powershell -NoProfile -ExecutionPolicy Bypass -File "${script}" -ProcessId ${session.app.pid}`, { encoding: "utf8" }).trim();
    assert.equal(open, "True");
  });

  console.log("Erreurs console :", session.errors.length ? session.errors : "aucune");
  assert.equal(session.errors.length, 0);
  console.log(`\n${passed.length} parcours réussis.`);
} catch (error) {
  await shot("erreur").catch(() => {});
  console.error(`Réussis avant l'échec : ${passed.length}`);
  throw error;
} finally {
  await session.close();
}
