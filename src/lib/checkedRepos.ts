/** Which repo paths have ever had "Check for problems" run on them, for the "never checked" scope. */
const KEY = "krita-vc:checked-repos";

function readSet(): Set<string> {
  if (typeof localStorage === "undefined") return new Set();
  try {
    const raw = localStorage.getItem(KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? new Set(parsed) : new Set();
  } catch {
    return new Set();
  }
}

export function hasBeenChecked(path: string): boolean {
  return readSet().has(path);
}

export function markChecked(path: string): void {
  if (typeof localStorage === "undefined") return;
  const set = readSet();
  if (set.has(path)) return;
  set.add(path);
  localStorage.setItem(KEY, JSON.stringify([...set]));
}
