# Settings, theming, palette tracking, and the first Krita plugin

Dates: 2026-07-07 to 2026-07-13. Commits: `20599a6` to `5226db4`.

This is the broadest stretch of the early project. A lot of basic UI arrives next to the first new
kind of tracked file, roughly in this order.

`20599a6` fixes the pixel and box diff overlays, closing out the diffing work from
[02](02-the-custom-tracking-engine.md). The Settings modal follows (`882d5db`) and moves what had
been hard-coded configuration into preferences the user can change, along with a fix to what the
Inspector shows. A theme selector (`f5d0bbe`) and a fix for clickable buttons that didn't show a
pointer cursor (`49e1c36`) land the same day.

`89ec69a` ("Developed VC Docker for Krita") ships the first Krita plugin: the PyKrita "Version
Control" docker, the first way to commit or switch branches without leaving Krita. Its first
version also had a one-tap "Checkpoint" button that committed with a generated message; that button
was removed again in `af689cd` ([05](05-staging-stashing-and-the-v1-release.md)). The same commit
quietly adds the `kvc` CLI (`src-tauri/src/bin/kvc.rs` first appears here), a second binary over the
same engine with no Tauri dependency. A Python plugin running inside Krita can't call Tauri
commands, so it runs `kvc` as a subprocess instead. That constraint is why the crate builds an
`rlib` and has two binary targets to this day.

`38e55e4` adds the custom title bar, so the app no longer depends on the operating system's window
frame. `e1dfc6d` changes the capitalization of the app's name, and it is also the commit the first
pre-release was tagged at (`beta`, published as "KVC v1.0-beta" on 2026-07-09, app version 0.1.0).
A security and performance pass follows (`277d769`), then two updates to the marketing site's copy
(`240ce52`, `b4f1b21`).

`9f49419` ("Developed Color Palette Tracking - Supports: .gpl, .kpl, .aco, .ase") makes palettes
the first file type the engine versions besides `.kra`. Two days later `86aa695` fixes palette
bugs and draws a connector in the history graph from a rolled-back version to the version it was
restored from, and `5226db4` fixes the Inspector on the Changes panel and changes the app logo.

The palette tracking of this era is for standalone files: a `.gpl`, `.kpl`, `.aco` or `.ase` gets a
history of its own. The [per-document rewrite](10-version-map-and-the-per-document-rewrite.md) later
stops tracking standalone palettes, and palette diffing narrows to the palettes embedded inside a
`.kra`.

See also: [`frontend-architecture.md`](../frontend-architecture.md) for Settings, themes and the
custom title bar as they are today; [`version-control.md`](../version-control.md#palette-diffs) for
palette diffing; [`krita-plugin/README.md`](../../krita-plugin/README.md) for the plugin.
