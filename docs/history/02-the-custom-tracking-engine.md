# The custom file-tracking engine

Dates: 2026-06-29 to 2026-07-02. Commits: `1763886` to `4144dac`.

Right after `git2` is dropped ([01](01-origins-and-the-git2-prototype.md)), the project builds a
file-tracking system from scratch, shaped around `.kra` files instead of generic blobs (`1763886`,
"Developed File Tracking System for .kra files"). Three days later two follow-up commits add
working layer and composite diffing plus UI optimizations (`4cf885d`, `4144dac`).

Those four days produce the two things everything downstream depends on: content-addressed storage
keyed to Krita's own layer and tile structure, and a diff view that shows an artist what changed
inside a painting instead of just "this file is different". The project's central bet, versioning a
`.kra` at the tile and layer level rather than as one archive, is running code by 2026-07-02.
Everything after it, from branching ([03](03-branching-and-early-performance.md)) to layer-subset
staging ([12](12-layer-subset-staging.md)), builds on this storage model.

See also: [`version-control.md`](../version-control.md) for the current tile-delta storage format,
the chain shards, and the `.kra` decomposition this era introduced.
