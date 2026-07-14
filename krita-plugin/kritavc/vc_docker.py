"""The "Version Control" docker: changelist + commit + branch switch for the .kvc repo
the active document lives in. Read-only history/restore/init stay in the desktop app —
this panel only ever writes a commit or moves a branch pointer, and only when the
active document is saved.
"""

import os
import time

from krita import DockWidget, Krita
from PyQt5.QtCore import QTimer, Qt
from PyQt5.QtWidgets import (
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QListWidget,
    QMenu,
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


class VcDocker(DockWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Version Control")

        self.repo_root = None
        self.doc_path = None
        self.busy = False

        self.stack = QStackedWidget()
        self.setWidget(self.stack)

        self.stack.addWidget(self._build_not_configured_page())
        self.stack.addWidget(self._build_empty_page())
        self.stack.addWidget(self._build_main_page())

        self.timer = QTimer(self)
        self.timer.timeout.connect(self.refresh)
        self.timer.start(REFRESH_MS)

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
        label = QLabel(
            "This document isn't version-controlled.\n"
            "Open it in the Krita VC app to start tracking."
        )
        label.setWordWrap(True)
        label.setAlignment(Qt.AlignCenter)
        layout.addWidget(label)
        layout.addStretch()
        return page

    def _build_main_page(self):
        page = QWidget()
        layout = QVBoxLayout(page)

        # Branch row: current branch (menu: switch / new branch…) + repo name.
        top = QHBoxLayout()
        self.branch_button = QToolButton()
        self.branch_button.setToolButtonStyle(Qt.ToolButtonTextOnly)
        self.branch_button.setPopupMode(QToolButton.InstantPopup)
        self.repo_label = QLabel("")
        self.repo_label.setAlignment(Qt.AlignRight)
        top.addWidget(self.branch_button)
        top.addStretch()
        top.addWidget(self.repo_label)
        layout.addLayout(top)

        self.status_label = QLabel("")
        layout.addWidget(self.status_label)

        layout.addWidget(QLabel("Changes"))
        self.changes_list = QListWidget()
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
        self.checkpoint_button = QPushButton("⚡ Checkpoint")
        self.checkpoint_button.clicked.connect(self._on_checkpoint)
        buttons.addWidget(self.commit_button)
        buttons.addWidget(self.checkpoint_button)
        layout.addLayout(buttons)

        return page

    def _browse_for_binary(self):
        exe_filter = "Executable (*.exe)" if os.name == "nt" else "All files (*)"
        path, _ = QFileDialog.getOpenFileName(self.stack, "Locate kvc", "", exe_filter)
        if path:
            kvc.set_binary_path(path)
            self.refresh()

    # --- state refresh ---------------------------------------------------------------

    def refresh(self):
        if self.busy:
            return
        if not kvc.get_binary_path():
            self.stack.setCurrentIndex(0)
            return

        doc = Krita.instance().activeDocument()
        doc_path = doc.fileName() if doc else None
        self.doc_path = doc_path
        self.repo_root = kvc.find_repo(doc_path) if doc_path else None

        if not self.repo_root:
            self.stack.setCurrentIndex(1)
            return

        self.stack.setCurrentIndex(2)
        self.repo_label.setText(os.path.basename(self.repo_root))

        dirty = bool(doc and doc.modified())
        if dirty:
            self.status_label.setText("● Unsaved changes — press Ctrl+S")
        else:
            self.status_label.setText("✓ Saved")

        try:
            result = kvc.status(self.repo_root)
            branch_result = kvc.branches(self.repo_root)
        except kvc.KvcError as e:
            self.status_label.setText(f"⚠ {e}")
            return

        self._populate_changes(result["changes"])
        self._populate_branch_menu(branch_result)

        can_write = (not dirty) and len(result["changes"]) > 0
        self.commit_button.setEnabled(can_write)
        self.checkpoint_button.setEnabled(can_write)

    def _populate_changes(self, changes):
        self.changes_list.clear()
        for change in changes:
            label = STATUS_LABELS.get(change["status"], change["status"])
            self.changes_list.addItem(f"{label} — {change['path']}")

    def _populate_branch_menu(self, branch_result):
        current = branch_result["current"]
        self.branch_button.setText(f"⌥ {current} ▾")

        # Rebuilt every refresh (branch list can change from outside Krita too) — drop
        # the previous menu explicitly, setMenu() alone doesn't delete the old one.
        old_menu = self.branch_button.menu()
        menu = QMenu(self.branch_button)
        for b in branch_result["branches"]:
            if b["current"]:
                continue
            action = menu.addAction(b["name"])
            action.triggered.connect(lambda _, name=b["name"]: self._on_switch(name))
        menu.addSeparator()
        menu.addAction("New branch…").triggered.connect(self._on_new_branch)
        self.branch_button.setMenu(menu)
        if old_menu is not None:
            old_menu.deleteLater()

    # --- actions -----------------------------------------------------------------

    def _set_busy(self, busy):
        self.busy = busy
        self.commit_button.setEnabled(not busy)
        self.checkpoint_button.setEnabled(not busy)
        self.branch_button.setEnabled(not busy)

    def _on_commit(self):
        message = self.message_box.toPlainText().strip()
        if not message:
            self.status_label.setText("⚠ Enter a message before committing.")
            return
        self._commit_with_message(message)

    def _on_checkpoint(self):
        self._commit_with_message(time.strftime("Checkpoint %H:%M"))

    def _on_author_changed(self):
        Krita.instance().writeSetting(
            kvc.SETTINGS_GROUP, "authorName", self.author_edit.text().strip() or "You"
        )

    def _commit_with_message(self, message):
        author = self.author_edit.text().strip() or "You"
        self._set_busy(True)
        try:
            kvc.commit(self.repo_root, message, author)
            self.message_box.setPlainText("")
            self.status_label.setText("Saved version ✓")
        except kvc.KvcError as e:
            self.status_label.setText(f"⚠ {e}")
        finally:
            self._set_busy(False)
            self.refresh()

    def _on_switch(self, name):
        self._set_busy(True)
        try:
            kvc.switch(self.repo_root, name)
        except kvc.KvcError as e:
            # The engine's dirty-tree error carries a stable "unsaved changes" prefix
            # (see src-tauri/src/error.rs) — same contract the desktop frontend matches.
            if "unsaved changes" in str(e):
                self.status_label.setText("⚠ Save & commit your changes first.")
            else:
                self.status_label.setText(f"⚠ {e}")
        finally:
            self._set_busy(False)
            self.refresh()

    def _on_new_branch(self):
        from PyQt5.QtWidgets import QInputDialog

        name, ok = QInputDialog.getText(self.stack, "New branch", "Branch name:")
        if not ok or not name.strip():
            return
        self._set_busy(True)
        try:
            kvc.create_branch(self.repo_root, name.strip())
        except kvc.KvcError as e:
            self.status_label.setText(f"⚠ {e}")
        finally:
            self._set_busy(False)
            self.refresh()
