# The Version Map, and the per-document rewrite (v2.0.0)

Dates: 2026-08-26 to 2026-08-27. Commits: `fb65ca2` to `d4d821d`.

The biggest architectural change in the project's history, in two parts that landed back to back.

## Part one: the Version Map

`fb65ca2` ("Add Version Map as the default history view") replaces the list-based History and
Branches tabs as the default with a pannable, zoomable canvas of versions built on React Flow. The
same day, `e542a35` draws each branch on its own lane (behind an opt-in "show all lines" toggle) and
adds a `create_branch_at` backend operation, which starts a new branch from any past commit ("go
back to version 5 and try a different direction") instead of only from the current tip. `94547eb`
fixes a minimap drawing bug.

## Part two: one document, one history

The next commit is the rewrite: `8b59e1b`, "Rework tracking to one document = one history, plus UI
polish". Its message describes the change. The folder-wide repository model is replaced with
per-`.kra` tracking, where each artwork gets its own self-contained store (`objects/`, `chains/`,
`cache/`, `commits.log`, `branches.json`, `stashes.json`, `config.json`) inside a shared
`.kvc/<slug>/` container, addressed through a new split between the `root` and `store` paths on
`Repo`, with an app-wide store root that can be moved.

From here on only `.kra` files are tracked. Standalone palette files, tracked since
[04](04-settings-theming-palettes-and-the-first-plugin.md), are dropped, although the palettes
embedded in a `.kra` are still diffed from the `.kra` itself. Scanning a document shrinks from a
directory walk to a single `stat`. Settings gains a "Where version history is kept" control. A
drive that isn't mounted (`StoreUnreachable`) becomes a separate failure from "never versioned"
(`NotARepo`), so the app can't answer a missing drive by creating a fresh, empty history.

There is no migration. The commit message says only "No migration path" and that v1 folder
repositories can't be read. The reason is written down elsewhere: `CLAUDE.md` and
[`per-document-tracking.md`](../per-document-tracking.md) both say it was free because the app had
no users yet.

`d4d821d` then puts branch actions and pick-a-version branching on the Version Map itself.
`create_branch_at` had been written and tested but was unreachable until then, because nothing
else in the app let you pick a version.

## Why it mattered

Changing the unit of versioning from a folder of files to one artwork touched more of the codebase
than any other change in this history. It is why standalone palette tracking went away and why the
scanner got so much cheaper. It also explains a detail that looks odd from outside: the frontend's
`Repository.id` simply became the document's path, which let every one of the app's Tauri commands
(39 today) take the new model without a signature change.

See also: [`per-document-tracking.md`](../per-document-tracking.md) for the mechanics of the
shipped model (`store_dir_for`, the container layout, the three failure states), and
[`version-map.md`](../version-map.md) for how the Version Map works today.
