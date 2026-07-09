"""Thin subprocess wrapper around the `kvc` companion CLI (src-tauri/src/bin/kvc.rs).

Every function here shells out to the `kvc` binary and returns its parsed JSON, or
raises KvcError with a message fit to show directly in the docker. No state is kept
here beyond the binary path setting — the CLI and the .kvc store on disk are the only
source of truth, same as the desktop app.
"""

import json
import os
import shutil
import subprocess

from krita import Krita

SETTINGS_GROUP = "kritavc"
_EXE = "kvc.exe" if os.name == "nt" else "kvc"


class KvcError(Exception):
    pass


def get_binary_path():
    """Explicit user-set path first, then a short list of likely install locations."""
    saved = Krita.instance().readSetting(SETTINGS_GROUP, "kvcPath", "")
    if saved and os.path.isfile(saved):
        return saved

    found = shutil.which(_EXE)
    if found:
        return found

    # The desktop app installs its own binaries alongside itself; check the common
    # per-user and per-machine locations rather than guessing an exact version path.
    for base in (os.environ.get("LOCALAPPDATA"), os.environ.get("PROGRAMFILES")):
        if not base:
            continue
        candidate = os.path.join(base, "krita-vc", _EXE)
        if os.path.isfile(candidate):
            return candidate

    return None


def set_binary_path(path):
    Krita.instance().writeSetting(SETTINGS_GROUP, "kvcPath", path or "")


def find_repo(start_path):
    """Walk up from a file (or folder) looking for a `.kvc` directory. None if none found."""
    if not start_path:
        return None
    current = os.path.abspath(start_path)
    if os.path.isfile(current):
        current = os.path.dirname(current)
    while True:
        if os.path.isdir(os.path.join(current, ".kvc")):
            return current
        parent = os.path.dirname(current)
        if parent == current:
            return None
        current = parent


def _run(args):
    binary = get_binary_path()
    if not binary:
        raise KvcError("kvc CLI not found. Set its location in the docker settings.")
    try:
        proc = subprocess.run(
            [binary] + args,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except OSError as e:
        raise KvcError(f"failed to launch kvc: {e}")

    raw = proc.stdout if proc.returncode == 0 else proc.stderr
    try:
        data = json.loads(raw)
    except ValueError:
        raise KvcError(f"unexpected kvc output: {raw.strip() or '(empty)'}")

    if proc.returncode != 0:
        raise KvcError(data.get("error", "unknown error"))
    return data


def status(repo):
    return _run(["status", "--repo", repo])


def commit(repo, message, author):
    return _run(["commit", "--repo", repo, "--message", message, "--author", author])


def branches(repo):
    return _run(["branches", "--repo", repo])


def switch(repo, name):
    return _run(["switch", "--repo", repo, "--branch", name])


def create_branch(repo, name, base=None):
    args = ["create-branch", "--repo", repo, "--name", name]
    if base:
        args += ["--base", base]
    return _run(args)
