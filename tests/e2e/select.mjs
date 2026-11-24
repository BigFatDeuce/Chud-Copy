// Listes déroulantes de l'application : aucune liste du système ne doit rester, et celles de
// l'application doivent s'ouvrir, se piloter à la souris comme au clavier, et rester dans la fenêtre.
// Usage : node tests/e2e/select.mjs <exe> <dossier-temporaire> <dossier-captures>
import assert from "node:assert/strict";
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"), { bigMegabytes: 2 });
const run = await launch({ exe, dataDir: lab.data, shots });
const { page, shot } = run;

const natives = () => page.locator("select").count();
const list = page.locator(".select-list");

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });

  // Éditeur : comparaison des fichiers, à la souris.
  await page.locator(".task-card").first().locator('[aria-label="Plus d\'actions"]').click();
  await page.getByRole("menuitem", { name: "Modifier" }).click();
  await page.waitForSelector(".route-stage");
  assert.equal(await natives(), 0, "liste du système dans l'éditeur");
  await page.getByRole("button", { name: "Réglages avancés" }).click();
  const compare = page.getByRole("combobox", { name: "Comparaison des fichiers" });
  const original = (await compare.innerText()).trim();
  await compare.click();
  await list.waitFor();
  await shot("select-1-editeur");
  const box = await list.boundingBox();
  const view = page.viewportSize() ?? (await page.evaluate(() => ({ width: innerWidth, height: innerHeight })));
  assert.ok(box.x >= 0 && box.y >= 0 && box.x + box.width <= view.width && box.y + box.height <= view.height, "la liste sort de la fenêtre");
  const colors = await list.evaluate((el) => {
    const item = el.querySelector("li");
    return { back: getComputedStyle(el).backgroundColor, text: getComputedStyle(item).color };
  });
  assert.notEqual(colors.back, colors.text, "texte et fond de même couleur");
  await list.getByRole("option", { name: "Contenu complet (lit chaque fichier)" }).click();
  assert.equal(await list.count(), 0, "la liste doit se refermer après un choix");
  assert.match(await compare.innerText(), /Contenu complet/);
  console.log("ok - éditeur : choix à la souris, liste dans la fenêtre et lisible");

  // Au clavier : ouvrir, descendre, valider ; Échap referme sans rien changer.
  await compare.focus();
  await page.keyboard.press("Enter");
  await list.waitFor();
  await page.keyboard.press("Escape");
  assert.equal(await list.count(), 0, "Échap doit refermer la liste");
  assert.match(await compare.innerText(), /Contenu complet/);
  await page.keyboard.press("Space");
  await list.waitFor();
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  assert.equal(await list.count(), 0);
  assert.match(await compare.innerText(), /Taille et date/);
  await page.keyboard.press("ArrowDown");
  await list.waitFor();
  await page.mouse.click(5, 300);
  assert.equal(await list.count(), 0, "un clic ailleurs doit refermer la liste");
  console.log("ok - éditeur : clavier, Échap et clic ailleurs");

  // La comparaison retrouve sa valeur d'origine : l'éditeur n'a alors rien à enregistrer.
  await compare.click();
  await list.getByRole("option", { name: original, exact: true }).click();
  await page.getByRole("button", { name: "Retour" }).click();
  await page.waitForSelector(".task-card");

  // Réglages : la langue change réellement l'interface.
  await page.locator(".rail").getByRole("button", { name: "Réglages" }).click();
  assert.equal(await natives(), 0, "liste du système dans les réglages");
  await page.getByRole("combobox", { name: "Langue" }).click();
  await list.waitFor();
  await shot("select-2-langue");
  await list.getByRole("option", { name: "English" }).click();
  await page.locator(".rail").getByRole("button", { name: "Settings" }).waitFor();
  await page.getByRole("combobox", { name: "Language" }).click();
  await list.getByRole("option", { name: "Français" }).click();
  await page.locator(".rail").getByRole("button", { name: "Réglages" }).waitFor();
  console.log("ok - réglages : la langue se choisit dans la liste de l'application");

  // Outils : chaque écran qui avait une liste du système.
  for (const tool of ["Doublons", "Conseiller de nettoyage", "Testeur réseau et NAS"]) {
    await page.locator(".rail").getByRole("button", { name: "Outils" }).click();
    await page.locator(".tool-card", { has: page.locator(".tool-name", { hasText: tool }) }).click();
    await page.locator(".select").first().waitFor();
    assert.equal(await natives(), 0, `liste du système dans l'outil ${tool}`);
  }
  console.log("ok - outils : plus aucune liste du système");

  assert.deepEqual(run.errors, [], "erreurs dans la console");
  console.log("ok - aucune erreur dans la console");
} finally {
  await run.close();
}
