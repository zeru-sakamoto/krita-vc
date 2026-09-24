# The Bento redesign

Dates: 2026-08-23 to 2026-08-30. Commits: `558fd6a` to `8250fb1`, interleaved with eras
[10](10-version-map-and-the-per-document-rewrite.md) and [11](11-backup-restore-overhaul.md).

The app's look changes wholesale, from a flat VS Code style to a tactile design the commits call
"Bento Box Neumorphism". Prep work (`558fd6a`, "Prepared App for UI Redesign") comes before the
switch itself (`56f706c`, "Design Change from VSCode Flat Style to Bento Box Neumorphism"). Then:

- `1fcc980` polishes header heights, focus rings, loading skeletons and themed selects. It also
  adds a Performance tab to Settings and moves "Background CPU use" into it from Storage, where
  [07](07-cpu-headroom-v1.1.md) had put it.
- `2f75fa7` lets "Check for problems" run over more than one artwork, with a cancel between
  artworks, and makes more form controls tactile.
- `699dbe2` adds a custom `Tooltip` component and replaces native `title=` tooltips across the app.
- `c9e28e2` updates the Krita docker's UI. Its colors are now read from Krita's active palette
  instead of being hard-coded, so the docker follows whichever Krita theme the artist uses. The
  same commit adds `scripts/update-installed-kvc.ps1`, a helper that copies a fresh `kvc` build
  over the installed one so plugin work doesn't need a full reinstall.
- `7674a0d` turns the CPU budget dropdown into a slider and fixes a backdrop-filter flicker.

The era closes with a large audit, `470ccff` ("Enforce the bento/tactile design system app-wide,
and animate popups out"). It checks every screen against `DESIGN.md`: about 250 arbitrary
`text-[Npx]` sizes become a six-step type scale, about 90 literal icon sizes become four, nine
button treatments fold back into `Button` and `IconButton`, transitions get explicit timing, popups
animate out as well as in, and the artwork switcher's dropdown becomes a searchable dialog. The
commit message says its findings and measurements are in `docs/design-audit.md`, but that file was
never committed, on any branch.

Two of the audit's fixes had the same cause: a theme derived from another theme instead of designed
on its own. The two light themes turned out to be inverted dark ones (the old "studio-light" used
Krita Blue's background and accent colors unchanged for its text and accent), so both were rebuilt
as standalone palettes and renamed Gallery and Overcast. And the Version Map's background grid
disappeared on the True Black theme, because the grid color was mixed toward black on a background
that was already `#000`. Mixing toward `--color-text-muted` instead gives every theme a visible grid
from one formula.

`8250fb1` ("Workflow Bug Fix") updates the release workflow's actions and Node version, and
`app-v2.0.0` is tagged at that commit. The redesign, the Version Map and the per-document rewrite
([10](10-version-map-and-the-per-document-rewrite.md)) and the new backup and restore
([11](11-backup-restore-overhaul.md)) were all built in the same week, which is why v2.0.0 ships
them together.

See also: [`DESIGN.md`](../../DESIGN.md) for the resulting design system, and
[`frontend-architecture.md`](../frontend-architecture.md) for the UI primitives (`Button`,
`IconButton`, `Modal`, `Tooltip` and the rest) that this era made the app's only buttons and
toggles.
