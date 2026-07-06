// Domain types for the Krita VCS UI.

export type FileStatus = "M" | "A" | "D" | "U" | "R" | "C";
// M = Modified, A = Added, D = Deleted, U = Untracked, R = Renamed, C = Conflicted

export interface FileChange {
  path: string;
  status: FileStatus;
}

export type BranchKind = "local" | "current";

export interface Branch {
  name: string;
  kind: BranchKind;
  /** Tip commit id; null/absent for a branch with no commits yet. */
  tip?: string | null;
}

/** A local repository — a folder the user has designated. Local-only (no remotes). */
export interface Repository {
  id: string;
  /** Display name (usually the folder's basename). */
  name: string;
  /** Absolute filesystem path. */
  path: string;
}

export interface Commit {
  id: string;
  /** Short hash, e.g. "a1b2c3d" */
  hash: string;
  message: string;
  author: string;
  /** ISO timestamp */
  timestamp: string;
  changes: FileChange[];
  /**
   * Parent commit ids (the lineage used to draw the history graph). Ordered with
   * the first parent as the mainline; a length > 1 marks a merge commit. The root
   * commit has `[]`.
   */
  parents: string[];
  /** Branch this commit belongs to — used for graph lane coloring/labels. */
  branch?: string;
}

export type DiffLineKind = "add" | "del" | "context" | "hunk";

export interface DiffLine {
  kind: DiffLineKind;
  /** Line number in the old file (omitted for added/hunk lines) */
  oldLine?: number;
  /** Line number in the new file (omitted for deleted/hunk lines) */
  newLine?: number;
  text: string;
}

/** Text (code-style) diff — used for non-art files: palettes, config, etc. */
export interface TextDiff {
  kind: "text";
  path: string;
  status: FileStatus;
  lines: DiffLine[];
}

export type BlendMode = "normal" | "multiply" | "screen" | "overlay" | "add";
export type LayerChange = "added" | "removed" | "modified" | "unchanged";

/** Which artwork state a canvas should render. */
export type DiffState = "before" | "after";

/** Normalized (0..1) rect over the artwork viewBox, for the "box" highlight mode. */
export interface ChangeRegion {
  x: number;
  y: number;
  w: number;
  h: number;
  label?: string;
}

export interface ArtLayer {
  id: string;
  name: string;
  /** 0..100 */
  opacity: number;
  blendMode: BlendMode;
  change: LayerChange;
  /** Inner SVG markup for the layer at each state. null = layer absent in that state. */
  before: string | null; // null when change === "added"
  after: string | null; // null when change === "removed"
}

/** Visual diff for an art (.kra) file: a layer stack + before/after imagery. */
export interface ArtDiff {
  kind: "art";
  path: string; // e.g. "characters/hero.kra"
  status: FileStatus;
  /** Artwork pixel dims → SVG viewBox. */
  width: number;
  height: number;
  /** Ordered bottom→top (stacking order). */
  layers: ArtLayer[];
  /** Changed-region boxes for the highlight overlay. */
  regions: ChangeRegion[];
  /**
   * Composite (whole-image) markup for each state, used for the "Composite" view when the
   * backend supplies it (real `.kra` mergedimage.png). When absent, the composite is derived
   * by stacking `layers`. null = no such state (added/removed file).
   */
  beforeImage?: string | null;
  afterImage?: string | null;
  /**
   * Changed-pixel mask markup (an `<image>` sized to the viewBox): transparent except where
   * the before/after composites differ, tinted accent. Drives the "pixels" highlight mode.
   * Computed by the backend off the composite, so it's present from the first `commit_diff`
   * (before per-layer rasters stream in). null/absent for added/removed files.
   */
  diffImage?: string | null;
  /**
   * SVG path data (normalized 0..1) outlining the changed pixels' silhouette. Scaled to the
   * viewBox and stroked dashed in the "pixels" highlight — hugs the change, not a bounding box.
   */
  diffOutline?: string | null;
}

export type SwatchChange = "added" | "removed" | "modified" | "unchanged";

export interface PaletteSwatch {
  name: string;
  /** Hex color in "before" state. null when change === "added". */
  before: string | null;
  /** Hex color in "after" state. null when change === "removed". */
  after: string | null;
  change: SwatchChange;
}

/** Structured diff for a color palette (.gpl) file. */
export interface PaletteDiff {
  kind: "palette";
  path: string; // e.g. "palettes/skin-tones.gpl"
  status: FileStatus;
  /** Number of columns (from .gpl "Columns:" header). */
  columns: number;
  /** All swatches in the palette — unchanged and changed alike. */
  swatches: PaletteSwatch[];
}

export type DiffEntry = ArtDiff | TextDiff | PaletteDiff;

/** A working-tree change with its staged/unstaged state (Changes tab). */
export interface WorkingChange {
  change: FileChange;
  staged: boolean;
}
