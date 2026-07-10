# Getting Started with Krita VCS

From opening the app for the first time to saving your first version — plus how Krita VCS keeps
your work safe along the way.

[← Back to Krita VCS](SITE_CONTENT.md)

---

## Step by step

1. **Install and open.** [Download for free →] and launch — no account, no sign-in.
2. **Pick a project folder.** Top-bar switcher → **Create repository** (name it, choose a
   folder) or **Browse existing repository…** (a folder you're already tracking). Picking a
   folder you've already set up just opens it — nothing gets reset.
3. **Save your first version.** Open **Changes** → write a short note about what you did →
   **Commit version**. Everything in the folder is saved, so there's nothing to pick and choose.
4. **Compare versions.** Open **History** → click any version → see a visual, side-by-side
   comparison of your layers, no code involved. Or swipe between the two with the slider, and
   click a layer for its details.
5. **Try branching, merging, or restoring.** Optional — see below.

![Screenshot placeholder — repository switcher with Create/Browse options](placeholder)
![Screenshot placeholder — Changes view with commit message + Commit version button](placeholder)

---

## Using each feature

- **Changes** — where you save. Write a note, hit Commit. Locks briefly while saving so nothing
  gets interrupted.
- **History** — every version you've saved. Click one to see what changed.
- **Branches** — separate lines of work. Click one to switch to it. Hover for **Merge into
  current** (bring it into what you're on now) or **Delete**. **New branch** starts a fresh line,
  optionally starting from another one.
- **Comparing versions** — side-by-side or a swipe slider, with zoom and pan that stay in sync
  between both sides. The eye icon highlights exactly what changed — a precise outline of the
  changed pixels, or a simpler box around the changed area. Click a layer to see its details.
- **Undo** — the ⋯ menu next to Changes or History. Removes only your most recent save; those
  changes come back as unsaved work, ready to save again.
- **Restore** — pick any older version and bring its files back, saved as a brand-new version.
  Nothing older is ever deleted.
- **Settings** (gear icon) — Artist view, title bar style, your name, and 8 color themes. Per
  project: how much space preview images may use, a "compact storage" option that shrinks
  history for files with lots of small edits, a "low-memory" option that uses less memory (a
  little slower in exchange), and Clean up storage.
- **Clean up storage** — shows you exactly how much space would be freed before you confirm
  anything. Only ever clears old, unreachable leftovers — never your current work or anything
  still visible in your history.

---

## How Krita VCS keeps your work safe

- **Won't switch or merge if you have unsaved changes.** You'll get a prompt with a shortcut
  straight to Changes, so nothing you've done gets lost or mixed into the wrong line of work.
- **Never silently overwrites a conflict.** If the same artwork changed in two places at once,
  Krita VCS keeps the incoming version and marks the file for you to review — it never guesses.
- **Can't delete your main line of work, or the one you're currently on.** Switch to another
  branch first if you want to remove the one you're using.
- **Won't undo a save that something else depends on.** If a later save or another branch
  still needs it, undo is blocked so nothing gets orphaned.
- **Nothing happens if there's nothing to do.** Saving with no changes, or restoring the version
  you're already on, simply does nothing.
- **Only one save happens at a time.** If you're also using the Krita plugin, the two can never
  save at once — you'll see a brief "please wait" instead of any risk of a mixed-up save.
- **Removing a project defaults to the safe choice.** "Remove from list" just forgets it here —
  your files and history stay untouched. Deleting the folder for good asks you to type its name
  first, so it's never one accidental click away.
- **Cleaning up always shows you first.** You'll see exactly what would be freed before anything
  is actually deleted.

---

_Screenshots throughout this page are placeholders — final captures to be added._
