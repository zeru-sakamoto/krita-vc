"""The "Version Control" docker: changelist + commit + discard/set-aside + branch switch for
the .kvc repo the active document lives in. History browsing/restore and repo init stay in the
desktop app.

The engine only sees the disk; Krita's canvas only memory. Two flows keep them in step, each
load-bearing and documented at its own definition: `_save_tracked` (memory -> disk, on focus,
on ⟳, before a commit) and `_rebuild_docs` (disk -> memory, after an op rewrites files under
an open document).
"""

import functools
import os
import traceback

from krita import DockWidget, Krita
from PyQt5.QtCore import QTimer, Qt
from PyQt5.QtWidgets import (
    QApplication,
    QFileDialog,
    QHBoxLayout,
    QInputDialog,
    QLabel,
    QLineEdit,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QStackedWidget,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from . import kvc_client as kvc

# Friendly labels for the scan's U/M/D statuses, matching the desktop app's Artist Mode
# wording (see src/lib/friendly.ts) rather than raw git-style letters.
STATUS_LABELS = {"U": "Added", "M": "Modified", "D": "Deleted"}

REFRESH_MS = 1500

PAGE_NO_BINARY = 0
PAGE_EMPTY = 1
PAGE_MAIN = 2


def _stash_title(stash):
    """One-line name for a set-aside, for the picker. Mirrors the desktop's stashTitle/
    stashSummary (src/components/vcs/StashDialogs.tsx): the label, else the files it holds."""
    files = [os.path.basename(c.get("path", "?")) for c in (stash.get("changes") or [])]
    name = (stash.get("label") or "").strip()
    if not name:
        name = ", ".join(files[:2]) + (f" +{len(files) - 2}" if len(files) > 2 else "")
    return f"{name or 'Empty'} — {len(files)} {'file' if len(files) == 1 else 'files'}"


def guard(method):
    """Keep exceptions out of Qt.

    Everything below is reached from C++ — the poll timer or a click — and PyQt5 aborts
    the process on an exception escaping a slot. So a missing kvc, a closed document or
    a malformed repo has to end as a label, never as a dead Krita with unsaved art in it.
    """

    # Qt over-supplies args — clicked/triggered emit a `checked` bool — and this wrapper's
    # *args makes PyQt pass it straight through to a slot that declares none (TypeError).
    # Truncate to the positional params the method actually takes past self.
    extra = method.__code__.co_argcount - 1

    @functools.wraps(method)
    def wrapper(self, *args, **kwargs):
        try:
            return method(self, *args[:extra], **kwargs)
        except kvc.KvcError as e:
            self._show_error(str(e))
        except Exception as e:
            # No Python Console Docker to read? The type+message goes in the panel and the
            # full trace to a file the user can actually find.
            try:
                with open(
                    os.path.join(os.path.expanduser("~"), "krita-vc-error.log"),
                    "a",
                    encoding="utf-8",
                ) as f:
                    f.write(traceback.format_exc() + "\n")
            except OSError:
                pass
            self._show_error(f"{type(e).__name__}: {e}")

    return wrapper


class VcDocker(DockWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Version Control")

        self.repo_root = None
        self.doc_path = None
        self.busy = False
        # Source of truth for the ticks: the list widget is rebuilt from this, never the
        # reverse, so a poll landing mid-edit can't wipe one. New paths arrive ticked, so
        # ignoring the boxes commits everything — what this docker did before they existed.
        self.checked = set()
        self._shown_paths = None
        self.stash_count = 0

        self.stack = QStackedWidget()
        self.setWidget(self.stack)

        self.stack.addWidget(self._build_not_configured_page())
        self.stack.addWidget(self._build_empty_page())
        self.stack.addWidget(self._build_main_page())

        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        self.timer.start(REFRESH_MS)

        # Focus, not an event filter: focus lands on our child widgets and QEvent.FocusIn
        # won't reach the dock. PyQt drops this connection when the docker dies (bound method
        # of a QObject), so there's nothing to disconnect.
        app = QApplication.instance()
        if app:
            app.focusChanged.connect(self._on_focus_changed)

    # --- Krita DockWidget API ------------------------------------------------------

    def canvasChanged(self, canvas):
        # Required override; the poll timer (not canvas signals, which vary across
        # Krita versions) drives all state updates. Uses a timer poll, switch to
        # a real change signal only if 1.5s staleness ever becomes a complaint.
        pass

    # --- page builders --------------------------------------------------------------

    def _build_not_configured_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.addStretch()
        label = QLabel(
            "The kvc command-line tool wasn't found.\n"
            "Locate it (installed with the Krita VC desktop app)."
        )
        label.setWordWrap(True)
        label.setAlignment(Qt.AlignCenter)
        browse = QPushButton("Locate kvc…")
        browse.clicked.connect(self._browse_for_binary)
        layout.addWidget(label)
        layout.addWidget(browse, alignment=Qt.AlignCenter)
        layout.addStretch()
        return page

    def _build_empty_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.addStretch()
        self.empty_label = QLabel("")
        self.empty_label.setWordWrap(True)
        self.empty_label.setAlignment(Qt.AlignCenter)
        layout.addWidget(self.empty_label)
        layout.addStretch()
        return page

    def _build_main_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)

        # Branch row: current branch (menu: switch / new branch…) + repo name + options menu.
        top = QHBoxLayout()
        self.branch_button = QToolButton()
        self.branch_button.setToolButtonStyle(Qt.ToolButtonTextOnly)
        self.branch_button.setPopupMode(QToolButton.InstantPopup)
        self.repo_label = QLabel("")
        self.repo_label.setAlignment(Qt.AlignRight)
        self.refresh_button = QToolButton()
        self.refresh_button.setText("⟳")
        self.refresh_button.setToolTip("Save open documents and rescan")
        self.refresh_button.setToolButtonStyle(Qt.ToolButtonTextOnly)
        self.refresh_button.clicked.connect(self._on_refresh_clicked)
        self.options_button = QToolButton()
        self.options_button.setText("⋯")
        self.options_button.setToolTip("Panel options")
        self.options_button.setToolButtonStyle(Qt.ToolButtonTextOnly)
        self.options_button.setPopupMode(QToolButton.InstantPopup)
        top.addWidget(self.branch_button)
        top.addStretch()
        top.addWidget(self.repo_label)
        top.addWidget(self.refresh_button)
        top.addWidget(self.options_button)
        layout.addLayout(top)

        self.status_label = QLabel("")
        self.status_label.setWordWrap(True)
        layout.addWidget(self.status_label)

        layout.addWidget(QLabel("Changes"))
        self.changes_list = QListWidget()
        self.changes_list.itemChanged.connect(self._on_item_checked)
        layout.addWidget(self.changes_list, stretch=1)

        author_row = QHBoxLayout()
        author_row.addWidget(QLabel("Author"))
        self.author_edit = QLineEdit(
            Krita.instance().readSetting(kvc.SETTINGS_GROUP, "authorName", "") or "You"
        )
        self.author_edit.editingFinished.connect(self._on_author_changed)
        author_row.addWidget(self.author_edit)
        layout.addLayout(author_row)

        self.message_box = QPlainTextEdit()
        self.message_box.setPlaceholderText("What changed in this version…")
        self.message_box.setFixedHeight(70)
        layout.addWidget(self.message_box)

        buttons = QHBoxLayout()
        self.commit_button = QPushButton("Commit")
        self.commit_button.clicked.connect(self._on_commit)
        buttons.addWidget(self.commit_button)
        layout.addLayout(buttons)

        return page

    @guard
    def _browse_for_binary(self):
        exe_filter = "Executable (*.exe)" if os.name == "nt" else "All files (*)"
        path, _ = QFileDialog.getOpenFileName(self.stack, "Locate kvc", "", exe_filter)
        if not path:
            return
        try:
            kvc.verify_binary(path)
        except kvc.KvcError as e:
            # The only spot a modal is safe: the user just opened a dialog. Errors from
            # the poll timer go to a label instead — a popup every 1.5s is unusable.
            QMessageBox.warning(self.stack, "Krita VC", str(e))
            return
        kvc.set_binary_path(path)
        self.refresh()

    # --- state refresh ---------------------------------------------------------------

    def _show_error(self, message):
        self.status_label.setText(f"⚠ {message}")

    def _show_empty(self, message):
        self.repo_root = None
        self.empty_label.setText(message)
        self.stack.setCurrentIndex(PAGE_EMPTY)

    def _document_state(self):
        """(path, has_unsaved_changes) for the active document, or (None, False).

        Krita hands back a wrapper whose C++ document may already be gone (closed
        between polls); touching it raises RuntimeError rather than returning None.
        """
        try:
            doc = Krita.instance().activeDocument()
            if doc is None:
                return None, False
            return doc.fileName() or None, bool(doc.modified())
        except (RuntimeError, AttributeError):
            return None, False

    @guard
    def refresh(self):
        if self.busy:
            return
        if not kvc.get_binary_path():
            self.stack.setCurrentIndex(PAGE_NO_BINARY)
            return

        doc_path, dirty = self._document_state()
        self.doc_path = doc_path

        if not doc_path:
            return self._show_empty(
                "Open a saved document to see its versions.\n"
                "New documents need saving to a folder first."
            )
        # Only .kra documents are tracked (scan::is_supported) — anything else opened in
        # Krita has nothing to commit, so say so rather than offering a dead Commit button.
        if os.path.splitext(doc_path)[1].lower() != ".kra":
            return self._show_empty(
                "Krita VC tracks .kra documents.\n"
                f"Save “{os.path.basename(doc_path)}” as .kra to version it."
            )

        self.repo_root = kvc.find_repo(doc_path)
        if not self.repo_root:
            return self._show_empty(
                "This document isn't version-controlled.\n"
                "Open it in the Krita VC app to start tracking."
            )

        self.stack.setCurrentIndex(PAGE_MAIN)
        self.repo_label.setText(os.path.basename(self.repo_root))

        if dirty:
            # Mostly seen while painting — focusing this panel saves. It means the canvas is
            # ahead of the changelist below.
            self.status_label.setText("● Unsaved changes")
        else:
            self.status_label.setText("✓ Saved")

        try:
            result = kvc.status(self.repo_root)
            branch_result = kvc.branches(self.repo_root)
        except kvc.KvcError as e:
            self._show_error(str(e))
            self.commit_button.setEnabled(False)
            return

        # .get throughout: a version-skewed or half-written store shouldn't be a KeyError
        # in a Qt slot.
        changes = result.get("changes") or []
        self.stash_count = result.get("stashes") or 0
        self._populate_changes(changes)
        self._populate_branch_menu(branch_result)
        self._populate_options_menu(len(changes))

        # Not gated on `dirty`: committing saves first. Nothing ticked commits nothing though,
        # so don't offer it — the desktop's "commit everything anyway?" confirm has no
        # equivalent here by design.
        can_write = len(changes) > 0 and len(self.checked) > 0
        self.commit_button.setEnabled(can_write)

    @guard
    def _on_item_checked(self, item):
        path = item.data(Qt.UserRole)
        if not path:
            return
        if item.checkState() == Qt.Checked:
            self.checked.add(path)
        else:
            self.checked.discard(path)

    def _populate_changes(self, changes):
        paths = [str(c.get("path", "")) for c in changes]
        # Drop ticks for files that stopped being dirty (committed, discarded, set aside),
        # and tick anything new — so the default stays "commit everything".
        self.checked &= set(paths)
        self.checked |= {p for p in paths if p not in (self._shown_paths or [])}
        if paths == self._shown_paths:
            return  # same files: leave the widget (and the user's ticks) alone
        self._shown_paths = paths

        # blockSignals: addItem/setCheckState both fire itemChanged, which would write the
        # widget's state back over self.checked as we build it.
        self.changes_list.blockSignals(True)
        self.changes_list.clear()
        for change in changes:
            status = str(change.get("status", ""))
            label = STATUS_LABELS.get(status, status or "Changed")
            path = str(change.get("path", "?"))
            item = QListWidgetItem(f"{label} — {path}")
            item.setData(Qt.UserRole, path)
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Checked if path in self.checked else Qt.Unchecked)
            self.changes_list.addItem(item)
        self.changes_list.blockSignals(False)

    def _selected_paths(self):
        """Checked paths, or None when everything is checked — None means "the whole working
        tree" to commit/stash, which is the exact call this docker made before checkboxes."""
        paths = [p for p in (self._shown_paths or []) if p in self.checked]
        return None if len(paths) == len(self._shown_paths or []) else paths

    def _populate_branch_menu(self, branch_result):
        self.branch_button.setText(f"⌥ {branch_result.get('current') or '?'} ▾")

        old_menu = self.branch_button.menu()
        if old_menu is not None and old_menu.isVisible():
            # Never rebuild the menu the user is currently pointing at — deleting an open
            # popup out from under Qt crashes it, and it would flicker every poll anyway.
            return

        # Rebuilt every refresh (branch list can change from outside Krita too) — drop
        # the previous menu explicitly, setMenu() alone doesn't delete the old one.
        menu = QMenu(self.branch_button)
        for b in branch_result.get("branches") or []:
            if b.get("current") or not b.get("name"):
                continue
            action = menu.addAction(b["name"])
            action.triggered.connect(lambda _, name=b["name"]: self._on_switch(name))
        menu.addSeparator()
        menu.addAction("New branch…").triggered.connect(self._on_new_branch)
        self.branch_button.setMenu(menu)
        if old_menu is not None:
            old_menu.deleteLater()

    def _populate_options_menu(self, n_changes):
        """Three groups, mirroring the desktop's panel-options menu: discard, set aside,
        bring back. Same don't-rebuild-a-visible-popup rule as the branch menu."""
        old_menu = self.options_button.menu()
        if old_menu is not None and old_menu.isVisible():
            return

        menu = QMenu(self.options_button)
        discard = menu.addAction("Discard checked changes")
        discard.triggered.connect(self._on_discard)
        discard.setEnabled(n_changes > 0)
        menu.addSeparator()
        aside = menu.addAction("Set aside checked changes…")
        aside.triggered.connect(self._on_set_aside)
        aside.setEnabled(n_changes > 0)
        menu.addSeparator()
        latest = menu.addAction(
            "Bring back latest" + (f"  ({self.stash_count} set aside)" if self.stash_count else "")
        )
        latest.triggered.connect(self._on_pop_latest)
        latest.setEnabled(self.stash_count > 0)
        pick = menu.addAction("Bring back…")
        pick.triggered.connect(self._on_pick_stash)
        pick.setEnabled(self.stash_count > 0)
        self.options_button.setMenu(menu)
        if old_menu is not None:
            old_menu.deleteLater()

    # --- actions -----------------------------------------------------------------

    def _set_busy(self, busy):
        self.busy = busy
        self.commit_button.setEnabled(not busy)
        self.branch_button.setEnabled(not busy)
        self.options_button.setEnabled(not busy)
        self.refresh_button.setEnabled(not busy)

    @guard
    def _on_refresh_clicked(self):
        saved, failed = self._save_tracked()
        self.refresh()  # sets status_label itself, so report after it, not before
        if failed:
            self._show_error(f"Couldn't save {', '.join(failed)}.")
        elif saved:
            self.status_label.setText(f"Saved {', '.join(saved)} ✓")

    # --- keeping disk and canvas in step ---------------------------------------------

    def _repo_docs(self):
        """{abspath: (Document, has_unsaved_changes)} for open .kra documents inside this repo.

        Reads fileName/modified in one try, same reason as `_document_state`: Krita's wrapper
        outlives its C++ document and raises on touch.

        `.kra` only — saving a .png/.jpg can raise an export dialog, which would hang the UI
        thread `_save_tracked` runs on, and this VCS doesn't track them anyway.
        """
        docs = {}
        try:
            open_docs = Krita.instance().documents()
        except (RuntimeError, AttributeError):
            return docs
        for doc in open_docs:
            try:
                path = doc.fileName()
                dirty = bool(doc.modified())
            except (RuntimeError, AttributeError):
                continue
            if not path or os.path.splitext(path)[1].lower() != ".kra":
                continue
            if kvc.in_repo(self.repo_root, path):
                docs[os.path.abspath(path)] = (doc, dirty)
        return docs

    # memory -> disk
    def _save_tracked(self):
        """Write every modified tracked document to disk. Returns (saved, failed) basenames.

        The engine only sees the disk, so unsaved work is invisible to a commit — without this
        the changelist describes the last Ctrl+S rather than the canvas.
        """
        docs = {p: d for p, (d, dirty) in self._repo_docs().items() if dirty}
        if not docs:
            return [], []
        saved, failed = [], []
        # save() spins the event loop; busy keeps the 1.5s poll off a half-written .kra.
        self._set_busy(True)
        try:
            for path, doc in docs.items():
                ok = bool(doc.save())
                doc.waitForDone()
                (saved if ok else failed).append(os.path.basename(path))
        finally:
            self._set_busy(False)
        return saved, failed

    def _contains(self, widget):
        # isAncestorOf() is False for the widget itself, hence the identity test.
        return widget is not None and (widget is self or self.isAncestorOf(widget))

    @guard
    def _on_focus_changed(self, old, new):
        """Save on the way into the docker: reaching for this panel is the artist saying they're
        done painting for a moment, which is exactly when their work should hit the disk."""
        if self.busy or not self.repo_root:
            return
        # Only on entry — not while tabbing between our own widgets, and not on the way out.
        if not self._contains(new) or self._contains(old):
            return
        saved, failed = self._save_tracked()
        if failed:
            self._show_error(f"Couldn't save {', '.join(failed)}.")
        if saved or failed:
            self.refresh()

    # disk -> memory
    def _rebuild_docs(self, op):
        """Run a file-rewriting op, then reopen any open document whose file it changed.

        Krita's copy goes stale when switch/discard/stash rewrite the .kra, yet still reports
        modified() == False — so the next Ctrl+S would write the old art back over the new
        state and silently undo the op. Refusing while a doc is unsaved is the other half: the
        engine's dirty-tree guard only sees the disk, so in-memory work would sail past it and
        then be destroyed by the reopen.
        """
        docs = self._repo_docs()
        unsaved = sorted(os.path.basename(p) for p, (_, dirty) in docs.items() if dirty)
        if unsaved:
            self._show_error(f"Save (Ctrl+S) or undo your changes in {', '.join(unsaved)} first.")
            return False
        before = {path: kvc.stat_key(path) for path in docs}

        self._set_busy(True)
        try:
            op()
        finally:
            self._set_busy(False)

        for path, (doc, _) in docs.items():
            if kvc.stat_key(path) != before[path]:
                self._reopen(path, doc)
        return True

    def _reopen(self, path, doc):
        doc.setBatchmode(True)
        doc.setModified(False)  # nothing to lose: _rebuild_docs refused to run if there was
        doc.close()
        if not os.path.exists(path):
            return  # a switch or discard can legitimately take the file away
        fresh = Krita.instance().openDocument(path)
        window = Krita.instance().activeWindow()
        if fresh and window:
            window.addView(fresh)

    def _require_repo(self):
        """A menu left open across a document close can still fire at a stale repo."""
        if self.repo_root:
            return True
        self._show_error("No version-controlled document is open.")
        return False

    @guard
    def _on_commit(self):
        message = self.message_box.toPlainText().strip()
        if not message:
            self._show_error("Enter a message before committing.")
            return
        self._commit_with_message(message)

    @guard
    def _on_author_changed(self):
        Krita.instance().writeSetting(
            kvc.SETTINGS_GROUP, "authorName", self.author_edit.text().strip() or "You"
        )

    def _commit_with_message(self, message):
        if not self._require_repo():
            return
        # Save before committing rather than trusting the focus hook to have fired first: a
        # keyboard-driven edit while this panel already holds focus never triggers it.
        saved, failed = self._save_tracked()
        if failed:
            self._show_error(f"Couldn't save {', '.join(failed)} — nothing committed.")
            return
        if saved:
            # The just-saved files only became changes now: without this, the changelist (and
            # so self.checked, and so _selected_paths) predates the save and would commit
            # everything except the work we just wrote out.
            self.refresh()
            # save() spun the event loop, so the doc may have closed and left refresh() with
            # no repo — re-check rather than shelling out to `--repo None`.
            if not self._require_repo():
                return
        author = self.author_edit.text().strip() or "You"
        self._set_busy(True)
        try:
            # Committing doesn't rewrite the working tree, so no reopen needed here.
            kvc.commit(self.repo_root, message, author, self._selected_paths())
            self.message_box.setPlainText("")
            self.status_label.setText("Saved version ✓")
        except kvc.KvcError as e:
            self._show_error(str(e))
        finally:
            self._set_busy(False)
            self.refresh()

    @guard
    def _on_discard(self):
        if not self._require_repo():
            return
        paths = self._selected_paths()
        n = len(paths) if paths is not None else len(self._shown_paths or [])
        if not n:
            self._show_error("Tick the changes you want to discard.")
            return
        # The confirm is the only guard left: auto-save means the work is on disk, so
        # _rebuild_docs' unsaved-changes refusal won't fire. Say what's actually lost.
        if not self._confirm(
            "Discard changes?",
            f"This reverts {n} {'file' if n == 1 else 'files'} to their last committed version. "
            "Everything you've done since — saved or not — is lost, and it won't be in the "
            "reopened document's undo history.",
        ):
            return
        if self._rebuild_docs(lambda: kvc.discard(self.repo_root, paths)):
            self.status_label.setText("Discarded ✓")
        self.refresh()

    @guard
    def _on_set_aside(self):
        if not self._require_repo():
            return
        paths = self._selected_paths()
        if paths is not None and not paths:
            self._show_error("Tick the changes you want to set aside.")
            return
        label, ok = QInputDialog.getText(
            self.stack, "Set aside", "What's this? (optional)"
        )
        if not ok:
            return
        author = self.author_edit.text().strip() or "You"
        if self._rebuild_docs(
            lambda: kvc.stash(self.repo_root, author, label.strip(), paths)
        ):
            self.status_label.setText("Set aside ✓")
        self.refresh()

    @guard
    def _on_pop_latest(self):
        if not self._require_repo():
            return
        stashes = kvc.stash_list(self.repo_root).get("stashes") or []
        if not stashes:
            self._show_error("Nothing has been set aside.")
            return
        self._pop(stashes[0])  # newest-first, per the CLI's stash-list ordering

    @guard
    def _on_pick_stash(self):
        if not self._require_repo():
            return
        stashes = kvc.stash_list(self.repo_root).get("stashes") or []
        if not stashes:
            self._show_error("Nothing has been set aside.")
            return
        # Numbered because getItem hands back the chosen *text*, not its row: two unlabelled
        # set-asides of the same file would render identically and index() would pop the wrong one.
        labels = [f"{i + 1}. {_stash_title(s)}" for i, s in enumerate(stashes)]
        choice, ok = QInputDialog.getItem(
            self.stack, "Bring back", "Set aside:", labels, 0, False
        )
        if ok and choice in labels:
            self._pop(stashes[labels.index(choice)])

    def _pop(self, stash):
        stash_id = stash.get("id") or ""
        try:
            if self._rebuild_docs(lambda: kvc.stash_pop(self.repo_root, stash_id)):
                self.status_label.setText("Brought back ✓")
        except kvc.KvcError as e:
            # Its own prefix, deliberately distinct from the dirty-tree one — the stash is
            # left intact, so the way out is to deal with the working file first.
            if not str(e).startswith("stash conflict"):
                raise
            self._show_error(f"{e}. Your set-aside work is still safe.")
        finally:
            self.refresh()

    def _confirm(self, title, text):
        return (
            QMessageBox.warning(
                self.stack,
                title,
                text,
                QMessageBox.Yes | QMessageBox.Cancel,
                QMessageBox.Cancel,
            )
            == QMessageBox.Yes
        )

    @guard
    def _on_switch(self, name):
        if not self._require_repo():
            return
        try:
            self._rebuild_docs(lambda: kvc.switch(self.repo_root, name))
        except kvc.KvcError as e:
            # The engine's dirty-tree error carries a stable "unsaved changes" prefix
            # (see src-tauri/src/error.rs) — same contract the desktop frontend matches.
            if "unsaved changes" not in str(e):
                raise
            # Same escape hatch the desktop offers: set the work aside, then switch. Minus its
            # "go to Changes" option, since the changelist is already on screen.
            if self._ask_set_aside_and_switch(name):
                return
            self._show_error("Save & commit your changes first.")
        finally:
            self.refresh()

    def _ask_set_aside_and_switch(self, name):
        """Offer to stash everything and retry the blocked switch. True if it went through."""
        box = QMessageBox(self.stack)
        box.setWindowTitle("Krita VC")
        box.setText(
            f"You have changes that aren't saved to a version yet.\n\n"
            f"Set them aside to switch to “{name}”? You can bring them back any time."
        )
        set_aside = box.addButton("Set aside && switch", QMessageBox.AcceptRole)
        box.addButton("Cancel", QMessageBox.RejectRole)
        box.exec_()
        if box.clickedButton() is not set_aside:
            return False
        author = self.author_edit.text().strip() or "You"

        def op():
            kvc.stash(self.repo_root, author, "", None)
            kvc.switch(self.repo_root, name)

        return self._rebuild_docs(op)

    @guard
    def _on_new_branch(self):
        if not self._require_repo():
            return
        name, ok = QInputDialog.getText(self.stack, "New branch", "Branch name:")
        if not ok or not name.strip():
            return
        self._set_busy(True)
        try:
            kvc.create_branch(self.repo_root, name.strip())
        except kvc.KvcError as e:
            self._show_error(str(e))
        finally:
            self._set_busy(False)
            self.refresh()
