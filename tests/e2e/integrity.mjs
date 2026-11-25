// Integrity check on the real application when part of the folder cannot be read.
// The unread files must never be reported missing, and an incomplete first check must wait for
// the user's consent before becoming the reference.
// Usage: node tests/e2e/integrity.mjs <exe> <temp-folder> <screenshots-folder>
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { buildLab } from "./lab.mjs";
import { launch } from "./driver.mjs";

const [exe, root, shots] = process.argv.slice(2);
const lab = buildLab(join(root, "lab"), { bigMegabytes: 2 });
const folder = lab.d("F", "Saves");
const locked = join(folder, "verrou");
mkdirSync(locked, { recursive: true });
writeFileSync(join(locked, "cache.dat"), "unread");

// "Everyone" (S-1-1-0) may no longer list this folder, whatever the account or the language.
const deny = () => execFileSync("icacls", [locked, "/deny", "*S-1-1-0:(RD)"], { stdio: "ignore" });
const allow = () => execFileSync("icacls", [locked, "/remove:d", "*S-1-1-0"], { stdio: "ignore" });

const run = await launch({ exe, dataDir: lab.data, shots });
const { page, shot } = run;
const tiles = (label) => page.locator(".tile", { hasText: label }).locator(".tile-value").innerText();

try {
  deny();
  assert.throws(() => readdirSync(locked), "the denied access did not take effect: this test would prove nothing");

  await page.waitForSelector(".task-card", { timeout: 15000 });
  await page.locator(".rail").getByRole("button", { name: "Outils" }).click();
  await page.locator(".tool-card", { hasText: "Intégrité" }).click();
  await page.locator(".chip-choice", { hasText: "Saves" }).click();
  await page.getByRole("button", { name: "Créer la référence" }).click();

  await page.getByText("Problèmes détectés : 1").waitFor({ timeout: 30000 });
  assert.equal(await tiles("Illisibles"), "1");
  await page.getByRole("button", { name: "Accepter comme nouvelle référence" }).waitFor();
  assert.equal(await page.locator(".reference-list li").count(), 0, "an incomplete first check is not a reference yet");
  await shot("integrity-1-partial");
  console.log("ok - an incomplete first check waits for consent");

  await page.getByRole("button", { name: "Accepter comme nouvelle référence" }).click();
  await page.getByText("Nouvelle référence enregistrée").waitFor();
  await page.waitForFunction(() => document.querySelectorAll(".reference-list li").length === 1, undefined, { timeout: 5000 });
  console.log("ok - the user can accept it knowingly");

  // Checked again while the folder is still unreadable: nothing inside it is called missing.
  await page.getByRole("button", { name: "Vérifier" }).click();
  await page.getByText("Problèmes détectés : 1").waitFor({ timeout: 30000 });
  assert.equal(await tiles("Disparus"), "0");
  await shot("integrity-2-again");
  console.log("ok - unread files are never reported missing");

  assert.deepEqual(run.errors, [], "console errors");
  console.log("Erreurs console : aucune");
} finally {
  allow();
  await run.close();
}
