// Parcourt les vues principales dans chaque langue sur la vraie application.
// Signale tout texte qui déborde de son bouton, et capture chaque langue.
// Usage : node tests/e2e/languages.mjs <exe> <dossier-temporaire> <dossier-captures>
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"), { bigMegabytes: 1 });
const session = await launch({ exe, dataDir: lab.data, shots });
const { page, shot } = session;

const LANGUAGES = ["en", "zh", "hi", "es", "ar", "fr", "bn", "pt", "ru", "ur"];

// Un bouton dont le texte est coupé, ou un élément qui fait défiler la page sur le côté.
const overflows = () =>
  page.evaluate(() => {
    const found = [];
    for (const element of document.querySelectorAll(".btn, .tag, .tile-label, .rail-item span, .status-tag, .phase, dt, .mode-name, .quality button span")) {
      if (element.scrollWidth > element.clientWidth + 1 && element.clientWidth > 0) found.push(element.textContent.trim().slice(0, 40));
    }
    const view = document.querySelector(".view");
    if (view && view.scrollWidth > view.clientWidth + 1) found.push("[défilement horizontal de la vue]");
    return found;
  });

// Mots français de l'interface : s'ils apparaissent ailleurs qu'en français, un texte a été oublié.
const FRENCH = ["Annuler", "Enregistrer", "Analyser", "Exécuter", "Réglages", "Fermer", "Retour", "Conserver", "Supprimer", "Fichiers", "Liaisons"];
const leftovers = (language) =>
  page.evaluate(
    ([words, skip]) => (skip ? [] : words.filter((word) => document.body.innerText.includes(word))),
    [FRENCH, language === "fr"],
  );

// Un texte latin retourné par l'écriture de droite à gauche : le disque doit rester lisible.
const reversed = () => page.evaluate(() => /\\:[A-Z]/.test(document.body.innerText));

const anyTask = (name) => page.locator(".task-card").nth(name);
const home = () => page.locator(".rail-item").first().click();
const report = {};

try {
  await page.waitForSelector(".task-card", { timeout: 15000 });
  for (const language of LANGUAGES) {
    await page.evaluate((value) => window.__TAURI_INTERNALS__.invoke("settings_set", { value: { quality: "off", language: value } }), language);
    await page.reload();
    await page.waitForSelector(".task-card", { timeout: 15000 });
    await page.waitForTimeout(500);
    const issues = [];

    const dir = await page.evaluate(() => document.documentElement.dir);
    if (dir !== (["ar", "ur"].includes(language) ? "rtl" : "ltr")) issues.push(`direction ${dir}`);
    issues.push(...(await overflows()).map((text) => `accueil: ${text}`));
    issues.push(...(await leftovers(language)).map((word) => `francais accueil: ${word}`));
    if (await reversed()) issues.push("chemin retourne");
    await shot(`lang-${language}-1-accueil`);

    await anyTask(1).locator(".menu-anchor .icon-btn").click();
    await page.locator(".menu button").first().click();
    await page.waitForSelector(".route-stage");
    await page.waitForTimeout(400);
    issues.push(...(await overflows()).map((text) => `editeur: ${text}`));
    issues.push(...(await leftovers(language)).map((word) => `francais editeur: ${word}`));
    await shot(`lang-${language}-2-editeur`);
    await home();
    await page.waitForSelector(".task-card");

    const mirror = page.locator(".task-card", { has: page.locator(".tag.danger") }).first();
    await mirror.locator("footer .btn").first().click();
    // L'analyse se lance sans quitter l'accueil : le même bouton l'ouvre ensuite.
    await mirror.locator(".btn.analyzing, .btn.analyzed").first().click();
    await page.waitForSelector(".tiles", { timeout: 20000 });
    await page.waitForTimeout(500);
    issues.push(...(await overflows()).map((text) => `analyse: ${text}`));
    issues.push(...(await leftovers(language)).map((word) => `francais analyse: ${word}`));
    if (await reversed()) issues.push("chemin retourne");
    await shot(`lang-${language}-3-analyse`);

    await page.locator(".preview-foot .btn").click();
    await page.waitForSelector(".modal.danger");
    await page.waitForTimeout(250);
    issues.push(...(await overflows()).map((text) => `confirmation: ${text}`));
    issues.push(...(await leftovers(language)).map((word) => `francais confirmation: ${word}`));
    await shot(`lang-${language}-4-confirmation`);
    await page.keyboard.press("Escape");

    await page.locator(".rail-item").last().click();
    await page.waitForSelector(".quality");
    await page.waitForTimeout(250);
    issues.push(...(await overflows()).map((text) => `reglages: ${text}`));
    issues.push(...(await leftovers(language)).map((word) => `francais reglages: ${word}`));
    await shot(`lang-${language}-5-reglages`);
    await home();

    report[language] = issues;
    console.log(`${language} : ${issues.length ? issues.join(" | ") : "ok"}`);
  }
  console.log("Erreurs console :", session.errors.length ? session.errors.slice(0, 5) : "aucune");
  writeFileSync(join(shots, "languages.json"), JSON.stringify(report, null, 2));
} finally {
  await session.close();
}
