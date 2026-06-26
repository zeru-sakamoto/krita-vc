// =============================================================================
// === MOCK PALETTE DATA — structured PaletteDiff entries for the UI.        ===
// === Replace when a real parser/backend is wired up.                       ===
// =============================================================================

import type { PaletteDiff } from "../types";

// Full Skin Tones palette for commit c6.
// The real changes in c6: "midtone" shifted slightly warm, "highlight-warm" was added.
// Everything else is unchanged context.
export const PALETTE_DIFFS: Record<string, PaletteDiff> = {
  skin_tones_c6: {
    kind: "palette",
    path: "palettes/skin-tones.gpl",
    status: "M",
    columns: 4,
    swatches: [
      { name: "deep-shadow", before: "#2C1A10", after: "#2C1A10", change: "unchanged" },
      { name: "shadow", before: "#5C3520", after: "#5C3520", change: "unchanged" },
      { name: "dark", before: "#8B5635", after: "#8B5635", change: "unchanged" },
      { name: "base", before: "#C48050", after: "#C48050", change: "unchanged" },
      { name: "midtone", before: "#EDBC9C", after: "#F0BFA0", change: "modified" },
      { name: "light", before: "#F5D4B8", after: "#F5D4B8", change: "unchanged" },
      { name: "highlight", before: "#FAE8D5", after: "#FAE8D5", change: "unchanged" },
      { name: "highlight-warm", before: null, after: "#FFE0C4", change: "added" },
      { name: "blush", before: "#D4896A", after: "#D4896A", change: "unchanged" },
      { name: "lip", before: "#B86050", after: "#B86050", change: "unchanged" },
      { name: "warm-accent", before: "#C87848", after: "#C87848", change: "unchanged" },
      { name: "cool-accent", before: "#A06858", after: "#A06858", change: "unchanged" },
    ],
  },
};
