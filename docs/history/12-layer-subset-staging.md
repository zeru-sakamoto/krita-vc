# Layer-subset staging

Dates: 2026-09-01 to 2026-09-03. Commits: `c686231` to `e5b02ea`.

`c686231` ("Improved Refresh Triggers for file Change Detection") comes first and makes two changes
you can feel. Switching back to the app window now rescans the painting on its own, so a save in
Krita shows up without a manual rescan. And the Performance tab's storage figures are computed once
in the shell instead of every time the tab opens.

The main feature follows the next day: layer-subset staging, which saves only the ticked top-level
layers of a `.kra` instead of the whole document (`95df784`). The store keeps whole documents, so
the version is synthesized: the working file with every unticked top-level layer put back to its
last saved form. `mergedimage.png` and `preview.png` are dropped from that synthesized archive,
because they are Krita's renders of the whole stack and would show layers the version doesn't
contain. A new `layers` argument runs through `commit_snapshot` and `commit_selected` to a
`kvc commit --layers` flag. The index marks a partial commit, so the artwork keeps showing as
changed instead of the held-back layers disappearing from the Changes panel on the next scan. And
`stacked_composite_url` builds a stand-in composite for the Version Map, since a layer-subset
version has no `mergedimage.png` of its own.

About three hours later, `020d55f` makes those commits roughly three times faster (13.3 s to 4.1 s
on the benchmark; the commit title says 3.3x) and audits the docs. The biggest cost was that a
partial commit rebuilt the entire previously saved document only to throw nearly all of it away
(235 MB rebuilt to keep a couple of layers). Rebuilding only the entries the commit reads cut that
to 39 MB and made it 5.4 times faster. The commit also replaces a trick that
zeroed the index's size and mtime to force a dirty scan with an explicit `TrackedFile.partial`
flag. The trick worked, but it made every later scan read and hash the whole painting, and
`kvc status` runs on the Krita docker's 1.5-second poll. After the fix a scan takes 0.1 ms instead
of 145 ms.

The same commit audits `CLAUDE.md` and `DESIGN.md` against the code and corrects about a dozen
details that had drifted: the number of Settings tabs, an undocumented Performance tab and two
undocumented backend modules (`ops_log.rs`, `diskspace.rs`), the TopBar's height (44 px, not
36 px), the Version Map grid formula, and a spec for a docker tab strip that never existed. The
audit didn't reach `docs/frontend-architecture.md`, so the old TopBar height and grid formula
survived there.

A follow-up that night (`e5b02ea`) fixes the Changes panel reporting every layer that wasn't new
as modified. The diff paired a layer's old and new tiles by archive path, and Krita renumbers its
`layerN` data files whenever the stack changes, so adding one layer made every layer above it look
modified even when its pixels were identical. Layers are now paired by their own identity (uuid, or
name when there is none) before any tiles are compared.

See also: [`layer-staging.md`](../layer-staging.md) for how layer-subset staging works today
(`stage.rs`, `committed_subset`, `plan_pieces`), and
[`performance.md`](../performance.md#layer-subset-staging) for the measurements.
