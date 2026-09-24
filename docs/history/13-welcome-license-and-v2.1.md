# The welcome screen, a license change, and v2.1.0

Dates: 2026-09-05 to 2026-09-13. Commits: `5fccb88` to `b324391`.

`5fccb88` ("Added Historical Documentation") adds this series: chapters 01 to 12 and their index,
written from the commit log and cross-checked against `CLAUDE.md` and `RELEASE_NOTES.md`.

A week later, `975397d` adds a first-launch welcome. It is a full-window, two-step screen. The first
step asks for the artist's name, with a note that nothing leaves the computer. The second offers the
themes as preview cards, each a small drawing of the app in that theme's colors, and clicking a card
switches the whole app to it right away. Both answers save as soon as they change; a separate flag
records that the welcome is finished once the artist clicks "Get started" or "Skip". Existing
installs skip it if the tour was already completed or a name is already set. The tour now waits
until the welcome is done, so the two never stack. Because the welcome covers the custom title bar,
it carries its own drag strip and window buttons. Settings → Appearance gains a "Replay welcome"
button next to "Replay tour".

Twelve minutes later, `e2cfae9` ("Add LICENSE file") replaces the 21-line MIT license added in
`3a22621` ([05](05-staging-stashing-and-the-v1-release.md)) with the full text of the GNU General
Public License, version 3. The commit message gives no reason. Neither `package.json` nor
`src-tauri/Cargo.toml` declares a license field, and the site copy in `content/` still said MIT
after the change.

`b324391` bumps the version to 2.1.0 in `package.json`, `package-lock.json`,
`src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`. "Krita VC v2.1.0"
was published on GitHub the same day and marked as the latest release, and the site-sync pipeline
from [06](06-ci-cd-pipeline.md) pulled its installers into the marketing site.

v2.1.0 collects everything since v2.0.0: saving only the layers you pick
([12](12-layer-subset-staging.md)), the welcome screen, rescanning when you switch
back to the app (`c686231`), the fix for layers above a new one showing as modified (`e5b02ea`),
and a Performance tab that no longer recomputes its storage figures every time it opens.

See also: [`onboarding-and-tour.md`](../onboarding-and-tour.md) for how the welcome and the tour
work, and the [v2.1.0 release](https://github.com/zeru-sakamoto/krita-vc/releases/tag/app-v2.1.0)
for its notes.
