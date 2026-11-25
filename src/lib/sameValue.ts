/**
 * Structural equality for the plain JSON-shaped values the backend sends (diff entries, layers).
 * Used to keep the previous object when a refresh brings back the same data, so memoized
 * consumers see no change. Identity is checked first; the stringify only runs on a refetch.
 */
export function sameValue(a: unknown, b: unknown): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}
