# First-launch welcome and application tour

Two one-time overlays greet a new install. The welcome asks for a name and a theme before anything
else. The tour then walks through the shell once a painting is open. Both follow the same pattern
as Artist Mode and the custom title bar: a React context plus a `localStorage` flag, with a replay
button in Settings → Appearance.

## First-launch welcome

[`src/lib/onboarding.tsx`](../src/lib/onboarding.tsx) and
[`src/components/shell/OnboardingOverlay.tsx`](../src/components/shell/OnboardingOverlay.tsx)
implement a full-window, two-step welcome ("Welcome to krita-vc"). Step one asks for the artist's
name, with a note that nothing leaves the computer. That's true: the app is local-only and the name
lives in `localStorage`. Step two picks a theme from preview cards.

- **Gating.** The flag is `krita-vc:onboarding-completed`, the same shape as the tour's. Installs
  that predate the welcome skip it if the tour was completed (`krita-vc:tour-completed`) or a
  non-empty name is set. It has to be non-empty because `AuthorNameProvider` writes the key as `""`
  on first mount, so the key merely existing would silently skip a fresh install that was closed
  halfway through the welcome.
- **Answers save live** through `useAuthorName` and `useTheme`; only the completion flag waits for
  "Skip" or "Get started". Clicking a card re-skins the whole app, which makes the app itself the
  preview.
- **Full-window, not a `Modal`.** Escape or a click on the scrim would lose it for good. `AppShell`
  renders it next to `BusyOverlay`, on `--z-onboarding` above the tour. It covers `TopBar`, which is
  the window's title bar when the custom one is on, so the welcome carries its own drag strip and
  `WindowControls`.
- **Sequencing with the tour.** `RepoShell` only calls the tour's `beginIfFirstTime()` once the
  welcome is inactive, so the tour starts right after "Get started" instead of stacking under it.
- **Preview cards** paint a theme that isn't active by re-scoping its tokens onto the card.
  `global.css`'s theme blocks match `html[data-theme="x"], [data-theme-preview="x"]`, and Charcoal,
  which lives in `@theme`, has a preview-only copy of its identity tokens. Only identity tokens follow
  the card. Variables derived at `:root` (`--shadow-*`, `--glass-*`, `--color-state-*`) were resolved
  from the active theme, so the card's small drawing of the app uses plain fills and borders.
- Replay it from Settings → Appearance → "Replay welcome".

## Application tour

A first-launch spotlight walkthrough of the shell, shown once and never again automatically.

[`src/lib/tour.tsx`](../src/lib/tour.tsx) (`TourProvider`, `useTour()`) holds a linear
`TOUR_STEPS` array of 35 steps (`{tourId, title, body, view?, when?}`) and a `stepIndex` state
machine (`next`, `back`, `skip`, `restart`, `beginIfFirstTime`). Completion is the `localStorage`
flag `krita-vc:tour-completed`. `RepoShell` calls `beginIfFirstTime()` once on mount (after the
welcome, see above), and it does nothing once the flag is set. A step with a `view` calls
`setActiveView` as a side effect, so the tour can walk through Changes, the Version Map, History,
Branches and Performance without the user switching tabs.

### Steps that gate themselves

Not every step applies to every shell. A step whose target isn't in the DOM leaves `TourOverlay`
with nothing to spotlight, and it renders `null`: no card, no Next, no Skip. That was a real bug when
the Version Map became the default and History and Branches went behind the Legacy toggle; seven
steps in a row pointed at nothing and the tour went blank. So a step can carry a
`when?: (c: TourConditions) => boolean` predicate over four facts the shell knows:

- `legacy`: the History and Branches tabs exist (off by default);
- `hasVersions`: the map draws nodes rather than its empty state;
- `hasOtherBranches`: the map's "All lines" toggle is rendered;
- `dirty`: the map draws its pending-version preview.

A plain predicate, rather than a key registry, lets negation (`map-empty`) and compound gates
(`map-all-lines`) work without extra syntax. `RepoShell` reports the conditions live in an effect
instead of the provider taking a snapshot at start: the tour fires from a mount effect while
`useCommits` and `useBranches` are still loading, so a snapshot would drop every map step on a
painting that does have history. The cursor stays an index into `TOUR_STEPS`, not into the filtered
list (which resizes underneath it), while `stepIndex` and `totalSteps` come from the filtered list,
so "Step N of M" matches what the user will see. Of the 35 steps, a fresh install sees 19. A painting
with versions and Legacy off sees 24 to 26, depending on unsaved changes and a second branch, and
turning Legacy on adds eight more.

### The Version Map steps

The map's steps sit between the Changes steps and the legacy History ones, matching the activity
bar's order (Changes, map, History, and so on). They cover every control the map offers.

| `tourId` | Gate | What it points at |
|---|---|---|
| `map` | none | the activity-bar icon: what the map is, and that drag pans and scroll zooms |
| `map-empty` | `!hasVersions` | the empty state, the only thing on screen on a fresh install |
| `map-version` | `hasVersions` | one version card: composite, number and note, changed-layer chips, the tip ring, and that clicking opens the full comparison (where users without Legacy first hear about the Inspector) |
| `map-preview` | `dirty` | the dashed pending-version node |
| `map-branch` | `hasVersions` | the action bar's branch switcher, including its hover-revealed merge and delete |
| `map-branch-new` | `hasVersions` | "New version line…", with the menu forced open |
| `map-pick` | `hasVersions` | "Start a line here…", pick-a-version branching |
| `map-all-lines` | `hasVersions && hasOtherBranches` | the all-lines lane toggle |
| `map-view-controls` | `hasVersions` | the zoom readout, fit-all and jump-to-newest as one spotlight, because three holes over adjacent 16 px buttons read as padding, not as teaching |
| `map-minimap` | `hasVersions` | the overview, through `MinimapViewport`'s panel |

### Spotlight targets and the overlay

[`TourOverlay`](../src/components/shell/TourOverlay.tsx) renders `null` when the tour isn't active.
Targets are plain `data-tour-id` attributes. `IconButton` and `Menu`'s `MenuItem` both take an
optional `tourId` prop that sets it, and a handful of other targets carry `data-tour-id` directly
on a wrapper: the artwork switcher, the branch badge row, the changed-layer list, the commit graph,
the commit message and button, and the Version Map's empty state, header controls, action bar and
minimap overlay. So locating a step's target needs no ref plumbing. The one target that isn't
literal markup is a map node: `VersionNodeData.tourTarget` flags the current branch's tip, which is
the node `jumpToLatest` centers on and therefore the one `onlyRenderVisibleElements` reliably keeps
mounted.

The dim-with-a-hole effect is four opaque `fixed` bands that tile the viewport around the target's
rectangle (top, bottom, left, right), plus a fifth, transparent, non-interactive div over the hole
so it never intercepts clicks. This is deliberately not a box-shadow spread or an SVG mask; both
silently failed to paint in this WebView build. Rectangle coordinates are rounded to whole pixels so
the four independently positioned bands agree on the same boundary; raw floats from
`getBoundingClientRect()` risked a hairline seam between neighboring bands. The callout card sits
beside the target for activity-bar rows and for rows inside an open dropdown (on whichever side has
room, so a card near the bottom of the window never clips), and below the target otherwise. It is
clamped on both axes, so a target in the bottom right (the minimap) or a tall one (a version card)
flips the card above the target instead of running off the screen.

Measurement retries on `requestAnimationFrame` for about 400 ms instead of measuring once. The
Version Map stays mounted under `display: none` while another tab is active (see
[version-map.md](version-map.md#mounted-once-for-the-shells-lifetime)), and React Flow re-measures
its nodes a frame or two after it becomes visible, so a single frame can read a stale or zero-size
rectangle. A target still missing at the deadline makes the tour step over it in the direction of
travel (a `dir` ref, so pressing Back doesn't bounce forward). That's the backstop that keeps a
missing target from blanking the overlay, whatever the `when` predicates miss.

Steps that spotlight a row inside a `Menu` (the panel options' undo, discard and set-aside rows, and
the map action bar's "New version line…") need that menu open while the overlay blocks the click
that would normally open it. [`Menu`](../src/components/ui/Menu.tsx) takes a `forceOpen` prop,
ORed with its normal click-toggled state so it never fights the outside-click and Escape handling,
driven by `Sidebar`'s `PANEL_OPTION_TOUR_IDS` set and by `MapActionBar`'s single-step check.

`VersionMap` closes any open version drilldown while the tour is active, because a drilldown
replaces the canvas and takes every map spotlight target with it. The first-launch tour can't hit
that, but "Replay tour" from Settings can.

### Controls

The left and right arrow keys step back and forward, as do the card's Back and Next buttons. Skip is
a press-and-hold button (`HoldToSkip`, 300 ms) so a single stray click can't dismiss the whole tour.
It keeps a native `title=` instead of the app's `Tooltip`, because the tour's `--z-tour` layer sits
above `--z-tooltip` by design and a portaled tooltip would render behind the dimming bands.

Replay the tour any time from Settings → Appearance → "Replay tour" (`useTour().restart()`). It jumps
back to step 0 without touching the completion flag until the tour reaches its end again.
