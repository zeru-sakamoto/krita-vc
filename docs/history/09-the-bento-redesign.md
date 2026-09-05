# The Bento redesign

**Timeframe:** 2026-08-23 – 2026-08-30 · **Commits:** `558fd6a` … `470ccff` (+ `8250fb1`, `c9e28e2`)

The app's visual language changes wholesale: from a flat, VS Code-style look to a tactile "Bento
Box Neumorphism" design. Prep work (`558fd6a` "Prepared App for UI Redesign") precedes the actual
style swap (`56f706c` "Design Change from VSCode Flat Style to Bento Box Neumorphism"), then a
polish pass on header heights, focus rings, loading skeletons, and themed selects (`1fcc980`), a
commit adding scoped-repository-check-with-cancel alongside more tactile form controls (`2f75fa7`),
a custom `Tooltip` component replacing native `title=` attributes app-wide (`699dbe2`), and a
matching UI update to the Krita plugin so its docker doesn't visually diverge from the desktop app
(`c9e28e2`). A CPU-budget dropdown becomes a slider with a fixed backdrop-filter flicker
(`7674a0d`), and the era closes with a large audit commit, `470ccff` "Enforce the bento/tactile
design system app-wide, and animate popups out," sweeping every screen against `DESIGN.md` for
type-scale, icon-scale, transition-timing/easing, and button/toggle consistency (~250 arbitrary
`text-[Npx]` values replaced by a six-step scale, ~90 literal icon sizes by four, nine button
treatments folded back into `Button`/`IconButton`), and replacing the artwork-switcher dropdown
with a searchable modal. A trailing workflow bug fix closes the era (`8250fb1`).

Two fixes in that audit came from the same root cause: a theme derived from another theme rather
than designed. The two light themes turned out to be inverted dark ones. Studio-light's text and
accent were literally Krita Blue's background and accent lifted verbatim, so both were rebuilt as
standalone palettes and renamed **Gallery** and **Overcast**. And the
Version Map's background grid vanished entirely on the True Black theme, because the grid color
was mixed *toward black* against a background that was already `#000` and could not get darker;
mixing toward `--color-text-muted` instead gives every theme real contrast from one formula.

This era's date range overlaps with the [Version Map and per-document rewrite](10-version-map-and-the-per-document-rewrite.md)
and the [backup/restore overhaul](11-backup-restore-overhaul.md). All three were being worked on
in the same roughly one-week window, which is why `content/RELEASE_NOTES.md` bundles the redesign,
the Version Map, and multi-artwork backup into one v2.0.0 release rather than three.

**See also:** [`../../DESIGN.md`](../../DESIGN.md) for the resulting design system;
[`frontend-architecture.md`](../frontend-architecture.md) for the UI primitives (`Button`,
`IconButton`, `Modal`, `Tooltip`, etc.) this era established as the app's only button/toggle types.
