// Visite de toutes les vues, avec une capture de chacune.
// Usage : node tests/e2e/tour.mjs <exe> <dossier-temporaire> <dossier-captures>
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"));
const run = await launch({ exe, dataDir: lab.data, shots });
const { page, shot } = run;

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });
  await page.waitForTimeout(2500);
  await shot("01-accueil");

  await page.locator(".task-card", { hasText: "Archivage disques" }).locator("[aria-label=\"Plus d'actions\"]").click();
  await page.getByRole("menuitem", { name: "Modifier" }).click();
  await page.waitForSelector(".route-stage");
  await page.waitForTimeout(700);
  await shot("02-editeur");
  await page.getByRole("button", { name: "Retour" }).click();

  await page.locator(".task-card", { hasText: "Miroir Projets" }).getByRole("button", { name: "Analyser" }).click();
  await page.waitForSelector(".tiles", { timeout: 20000 });
  await page.waitForTimeout(800);
  await shot("03-analyse-miroir");

  await page.getByRole("button", { name: "Vérifier et lancer" }).click();
  await page.waitForSelector(".modal.danger");
  await page.waitForTimeout(300);
  await shot("04-confirmation");
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Réglages" }).click();
  await page.waitForTimeout(500);
  await shot("05-reglages");
  console.log("Erreurs console :", run.errors.length ? run.errors : "aucune");
} finally {
  await run.close();
}
