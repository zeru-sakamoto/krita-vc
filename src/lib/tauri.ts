/** True when running inside the Tauri webview (vs. a plain browser `npm run dev`). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
