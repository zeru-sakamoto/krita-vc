# The custom file-tracking engine

**Timeframe:** 2026-06-29 – 2026-07-02 · **Commits:** `1763886` … `4144dac`

Immediately after dropping `git2` ([01](01-origins-and-the-git2-prototype.md)), the project builds
a file-tracking system from scratch, purpose-fit to `.kra` files instead of generic blobs
(`1763886` "Developed File Tracking System for .kra files"). Three days later, that engine gains
functional layer-and-composite diffing plus UI optimizations in two follow-up commits (`4cf885d`,
`4144dac`), landing the two things everything downstream depends on: content-addressed storage
keyed to Krita's internal layer/tile structure, and a diff view that can actually show an artist
what changed inside a painting rather than just "this file is different."

Four days of work, and the project's central bet is running code: version-control a `.kra` at the
tile/layer level, not the archive level. Everything from branching
([03](03-branching-and-early-performance.md)) to layer-subset staging
([12](12-layer-subset-staging.md)) builds on the storage model established here.

**See also:** [`version-control.md`](../version-control.md) for the current tile-delta storage
format, chain shards, and the `.kra` decomposition this era introduced.
