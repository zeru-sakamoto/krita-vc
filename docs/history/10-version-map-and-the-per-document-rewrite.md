# The Version Map, and the per-document rewrite (v2.0.0)

**Timeframe:** 2026-08-26 – 2026-08-27 · **Commits:** `fb65ca2` … `d4d821d`

The biggest architectural pivot in the project's history, in two parts that happened to land back
to back.

**Part one: the Version Map.** The list-based History and Branches tabs are replaced by a
pannable, zoomable canvas of versions built on React Flow (`fb65ca2` "Add Version Map as the
default history view"), which gains per-branch lanes the same day (`e542a35`) and a
`create_branch_at` backend operation for forking a new branch from an arbitrary past commit ("go
back to version 5 and try a different direction") rather than only from the current tip. A
minimap visual-bug fix follows (`94547eb`).

**Part two: one document, one history.** The next commit is the rewrite itself: `8b59e1b`
"Rework tracking to one document = one history, plus UI polish." Its own message states the
change directly: replace the folder-wide repository model with per-`.kra` tracking, where each
artwork gets its own self-contained store (`objects/`, `chains/`, `cache/`, `commits.log`,
`branches.json`, `stashes.json`, `config.json`) inside a shared `.kvc/<slug>/` container, addressed
via a new `root`/`store` path split on `Repo` and an app-global, relocatable store root. Only
`.kra` files are tracked from this point on. Standalone palette files (tracked since
[04](04-settings-theming-palettes-and-the-first-plugin.md)) are dropped, though a `.kra`'s own
*embedded* palettes are still diffed off the `.kra` itself. Scanning a document collapses from a
directory walk to a single `stat`. Settings gains a "where version history is kept" control, and a
missing/unmounted drive (`StoreUnreachable`) is made a distinct failure state from "never
versioned" (`NotARepo`), so the two can't be conflated into an accidental empty store. **There is
no migration path**: v1 folder repositories are simply unreadable by this version, which the
commit message notes was acceptable because the project had no users yet.

Branch actions and pick-a-version branching are then wired directly into the new Version Map
(`d4d821d`), closing the loop between the two halves of this era: a version picker that only
exists because the Map gives you something to click on.

Changing the unit of versioning from "a folder of files" to "one artwork" touched more of the
codebase than anything else in this history. It's why standalone palette tracking was removed, why
the scanner got cheaper, and why `Repository.id` becoming a document path let ~40 Tauri commands
take the new model with zero signature changes.

**See also:** [`per-document-tracking.md`](../per-document-tracking.md) for the full mechanics of
the shipped model (`store_dir_for`, the container layout, the three failure states);
[`frontend-architecture.md`](../frontend-architecture.md#version-map) for how the Version Map
works today.
