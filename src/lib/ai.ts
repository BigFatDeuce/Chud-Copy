import * as fmt from "./format";
import { t } from "./i18n";
import type { HealthReport, IntegrityReport } from "./types";

/** Copie un texte dans le presse-papiers, avec un repli pour les vues web qui refusent l'API. */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const done = document.execCommand("copy");
    area.remove();
    if (!done) throw new Error("clipboard");
  }
}

/** Compte rendu d'un contrôle d'intégrité, à donner tel quel à une IA. */
export function integrityPrompt(report: IntegrityReport): string {
  const lines = report.findings.slice(0, 400).map((f) => `- ${f.kind}: ${f.rel}${f.detail ? ` (${f.detail})` : ""}`);
  return [
    t("ai.integrityIntro"),
    "",
    `Folder: ${report.root}`,
    `Checked: ${new Date(report.startedAt).toISOString()} — ${report.files} files, ${fmt.bytes(report.bytes)}, ${Math.round(report.elapsed)} s${report.cancelled ? " (stopped before the end)" : ""}`,
    `Reference: ${report.baseline ? new Date(report.baseline).toISOString() : "none, this check is the first one"}`,
    `Counts: ${JSON.stringify(report.counts)}`,
    "Legend: corrupt = content changed while size and date did not; unreadable = read error; modified = date changed; missing; added.",
    "",
    lines.length ? "Findings:" : "No finding.",
    ...lines,
    report.findings.length > lines.length ? `… ${report.findings.length - lines.length} more not listed.` : "",
    "",
    t("ai.outro"),
  ]
    .filter((line, i, all) => line !== "" || all[i - 1] !== "")
    .join("\n");
}

/** Compte rendu SMART, données brutes comprises. */
export function healthPrompt(report: HealthReport): string {
  const disks = report.disks.map((disk) => ({
    name: disk.name,
    model: disk.model,
    kind: disk.kind,
    size: fmt.bytes(disk.size),
    verdictFromChudCopy: disk.verdict,
    notes: disk.notes,
    raw: disk.raw,
  }));
  return [
    t("ai.healthIntro"),
    "",
    `Source: ${report.tool}${report.limited ? " (limited: administrator access was refused)" : ""}${report.missingTool ? " (smartctl not installed)" : ""}`,
    `Date: ${new Date().toISOString()}`,
    "",
    "```json",
    JSON.stringify(disks, null, 1),
    "```",
    "",
    t("ai.outro"),
  ].join("\n");
}
