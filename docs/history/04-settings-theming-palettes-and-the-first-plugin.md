# Settings, theming, palette tracking, and the first Krita plugin

**Timeframe:** 2026-07-07 – 2026-07-13 · **Commits:** `20599a6` … `5226db4`

The broadest single stretch of the early project: foundational UX surface area is built out
alongside a genuinely new tracked-content type, in roughly this order:

- Diff-overlay bug fixes (`20599a6`) close out the composite/tile diffing work from
  [02](02-the-custom-tracking-engine.md).
- The **Settings Modal** arrives (`882d5db`), moving what had been hardcoded configuration into
  user-facing preferences, alongside a fix to the Inspector panel's displayed info.
- A **theme selector** and a cursor-pointer fix for clickable buttons follow within the same day
  (`f5d0bbe`, `49e1c36`).
- The **first version of the Krita plugin**, the PyKrita "Version Control" docker, ships
  (`89ec69a` "Developed VC Docker for Krita"), giving artists commit/branch actions without
  leaving Krita for the first time. The same commit quietly introduces the **`kvc` CLI**
  (`src-tauri/src/bin/kvc.rs` first appears here), a second Tauri-free binary over the same engine
  crate. The plugin can't call Tauri commands from inside Krita's Python runtime, so it shells out
  to `kvc` instead. That constraint is why the crate builds an `rlib` and why the engine has two
  binary targets to this day.
- A **custom title bar** (`38e55e4`) replaces reliance on the OS-native window frame, alongside an
  app-name capitalization fix (`e1dfc6d`) and a security-and-performance pass (`277d769`).
- Site-content updates land (`240ce52`, `b4f1b21`), the project's public-facing copy starting to
  track its actual feature set.
- **Color palette tracking** ships (`9f49419` "Developed Color Palette Tracking - Supports: .gpl,
  .kpl, .aco, .ase"), the first entirely new file type the engine version-controls, alongside
  `.kra`. It's followed two days later by a bug-fix pass (`86aa695`) that also adds a connector
  line in the history graph back to the version a rollback restored from, and an Inspector-panel
  fix with a new app logo (`5226db4`).

Palette tracking here is standalone-file tracking (a `.gpl`/`.kpl`/`.aco`/`.ase` on its own gets a
history); it's only later, in the [per-document rewrite](10-version-map-and-the-per-document-rewrite.md),
that standalone palettes stop being trackable and palette diffing narrows to palettes *embedded
inside* a `.kra`.

**See also:** [`frontend-architecture.md`](../frontend-architecture.md) for Settings, theming, and
the custom title bar as they exist today; [`version-control.md`](../version-control.md#palette-diffs)
for palette diffing; [`../../krita-plugin/README.md`](../../krita-plugin/README.md) for the plugin.
