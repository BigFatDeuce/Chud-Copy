// Chaque traduction doit garder exactement les variables du français : perdre {path}
// retirerait un chemin d'un message de sécurité, sans erreur de compilation.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "i18n");

function load(file) {
  const entries = new Map();
  for (const line of readFileSync(join(dir, file), "utf8").split("\n")) {
    const match = /^\s*"([^"]+)": "(.*)",$/.exec(line);
    if (match) entries.set(match[1], match[2]);
  }
  return entries;
}

const vars = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join();
const reference = load("fr.ts");
const problems = [];

for (const file of readdirSync(dir)) {
  if (file === "fr.ts") continue;
  const catalog = load(file);
  for (const [key, text] of reference) {
    if (!catalog.has(key)) problems.push(`${file} : clé absente ${key}`);
    else if (vars(catalog.get(key)) !== vars(text)) problems.push(`${file} : variables différentes pour ${key}`);
    else if (key !== "locale" && key !== "units" && !catalog.get(key).trim()) problems.push(`${file} : texte vide ${key}`);
  }
  for (const key of catalog.keys()) if (!reference.has(key)) problems.push(`${file} : clé inconnue ${key}`);
}

if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`${reference.size} clés vérifiées dans ${readdirSync(dir).length} langues.`);
