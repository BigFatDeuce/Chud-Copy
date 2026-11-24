const LATEST_RELEASE = "https://api.github.com/repos/BigFatDeuce/Chud-Copy/releases/latest";

/** Vrai si `latest` est plus récente que `current`. Les numéros se comparent un à un : 1.10 suit 1.9. */
export function isNewer(latest: string, current: string): boolean {
  const parts = (version: string) => version.split(".").map((part) => parseInt(part, 10) || 0);
  const [a, b] = [parts(latest), parts(current)];
  for (let i = 0; i < Math.max(a.length, b.length); i += 1) {
    const difference = (a[i] ?? 0) - (b[i] ?? 0);
    if (difference !== 0) return difference > 0;
  }
  return false;
}

/** Interroge GitHub. Appelée uniquement sur un clic : l'application ne contacte rien d'elle-même. */
export async function fetchLatestVersion(): Promise<string> {
  const response = await fetch(LATEST_RELEASE, { headers: { Accept: "application/vnd.github+json" } });
  if (!response.ok) throw new Error(String(response.status));
  const release = (await response.json()) as { tag_name: string };
  return release.tag_name.replace(/^v/, "");
}
