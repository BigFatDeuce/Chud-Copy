import { fr, type Catalog } from "../i18n/fr";
import { en } from "../i18n/en";
import { zh } from "../i18n/zh";
import { hi } from "../i18n/hi";
import { es } from "../i18n/es";
import { ar } from "../i18n/ar";
import { bn } from "../i18n/bn";
import { pt } from "../i18n/pt";
import { ru } from "../i18n/ru";
import { ur } from "../i18n/ur";

export type Language = "en" | "zh" | "hi" | "es" | "ar" | "fr" | "bn" | "pt" | "ru" | "ur";
export type LanguageSetting = Language | "auto";

/** Les dix langues les plus parlées, chacune écrite dans sa propre langue. */
export const LANGUAGES: { id: Language; name: string; rtl?: boolean }[] = [
  { id: "en", name: "English" },
  { id: "zh", name: "简体中文" },
  { id: "hi", name: "हिन्दी" },
  { id: "es", name: "Español" },
  { id: "ar", name: "العربية", rtl: true },
  { id: "fr", name: "Français" },
  { id: "bn", name: "বাংলা" },
  { id: "pt", name: "Português" },
  { id: "ru", name: "Русский" },
  { id: "ur", name: "اردو", rtl: true },
];

const CATALOGS: Record<Language, Catalog> = { en, zh, hi, es, ar, fr, bn, pt, ru, ur };

let current: Language = "en";

export function detectLanguage(): Language {
  const wanted = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of wanted) {
    const base = tag.toLowerCase().split("-")[0] as Language;
    if (base in CATALOGS) return base;
  }
  return "en";
}

export function applyLanguage(setting: LanguageSetting): Language {
  current = setting === "auto" ? detectLanguage() : setting;
  const rtl = LANGUAGES.find((l) => l.id === current)?.rtl;
  document.documentElement.lang = current;
  document.documentElement.dir = rtl ? "rtl" : "ltr";
  return current;
}

export function locale(): string {
  return CATALOGS[current].locale;
}

function isolate(value: string): string {
  // Un chemin en écriture latine ne doit pas se mélanger à une phrase de droite à gauche.
  return document.documentElement.dir === "rtl" ? `⁨${value}⁩` : value;
}

export function t(key: keyof Catalog | (string & {}), params: Record<string, string | number> = {}): string {
  const catalog = CATALOGS[current] as Record<string, string>;
  const template = catalog[key] ?? (CATALOGS.en as Record<string, string>)[key] ?? key;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => (name in params ? isolate(String(params[name])) : match));
}

/** Traduit un message venu du cœur, un code avec ses paramètres. Tout autre texte passe tel quel. */
export function describe(text: string | null | undefined): string {
  if (!text) return "";
  if (!text.startsWith('{"code"')) return text;
  try {
    const message = JSON.parse(text) as { code: string; params?: Record<string, string> };
    return t(message.code, message.params ?? {});
  } catch {
    return text;
  }
}
