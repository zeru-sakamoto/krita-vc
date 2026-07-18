"""Thin subprocess wrapper around the `kvc` companion CLI (src-tauri/src/bin/kvc.rs).

Every function here shells out to the `kvc` binary and returns its parsed JSON, or
raises KvcError with a message fit to show directly in the docker. No state is kept
here beyond the binary path setting — the CLI and the .kvc store on disk are the only
source of truth, same as the desktop app.

KvcError is the only exception this module may raise: its callers are Qt slots, where
anything else aborts Krita.
"""

import json
import os
import shutil
import subprocess

from krita import Krita

SETTINGS_GROUP = "kritavc"
_EXE = "kvc.exe" if os.name == "nt" else "kvc"

# Reads are near-instant. A commit hashes and stores a whole .kra, so it gets minutes;
# past that something is wrong and a message beats a frozen Krita. Both calls block the
# UI thread — move to QProcess if the freeze during commit ever gets noticed.
READ_TIMEOUT = 30
WRITE_TIMEOUT = 300

# Without this the poll spawns a console window flash every 1.5s behind Krita's UI.
_CREATION_FLAGS = getattr(subprocess, "CREATE_NO_WINDOW", 0) if os.name == "nt" else 0


class KvcError(Exception):
    pass


_verified_paths = set()


def _verified(path):
    """True if `path` passes the kvc identity check, caching the answer per path. Auto-discovered
    binaries (PATH / install dirs) go through here so a rogue `kvc` planted earlier on PATH isn't
    run blindly; the check spawns kvc once per distinct path, then the result is remembered."""
    if path in _verified_paths:
        return True
    try:
        verify_binary(path)
    except KvcError:
        return False
    _verified_paths.add(path)
    return True


def get_binary_path():
    """Explicit user-set path first, then a short list of likely install locations.

    The user-set path was already identity-checked by the file picker that stored it, so it's
    trusted directly; auto-discovered candidates are verified before use (a failing check falls
    through to the next candidate rather than running an unknown executable)."""
    saved = Krita.instance().readSetting(SETTINGS_GROUP, "kvcPath", "")
    if saved and os.path.isfile(saved):
        return saved

    found = shutil.which(_EXE)
    if found and _verified(found):
        return found

    # The desktop app installs its own binaries alongside itself; check the common
    # per-user and per-machine locations rather than guessing an exact version path.
    for base in (os.environ.get("LOCALAPPDATA"), os.environ.get("PROGRAMFILES")):
        if not base:
            continue
        candidate = os.path.join(base, "krita-vc", _EXE)
        if os.path.isfile(candidate) and _verified(candidate):
            return candidate

    return None


def set_binary_path(path):
    Krita.instance().writeSetting(SETTINGS_GROUP, "kvcPath", path or "")


def verify_binary(path):
    """Raise KvcError unless `path` really is the kvc CLI.

    Run with no arguments the CLI prints its own usage as JSON — a free identity check,
    so picking the wrong executable fails at the file picker instead of turning every
    later call into "unexpected kvc output".
    """
    if not path or not os.path.isfile(path):
        raise KvcError("That file doesn't exist.")
    raw, _ = _exec(path, [], 10)
    try:
        data = json.loads(raw)
    except ValueError:
        data = None
    if not isinstance(data, dict) or "usage: kvc" not in str(data.get("error", "")):
        raise KvcError(
            "That isn't the kvc tool. Pick the kvc executable installed with the Krita VC app."
        )


def in_repo(root, path):
    """True if `path` is inside `root`. Both are normalized first — a plain string prefix
    test would match "/art/repo2/a.kra" against root "/art/repo"."""
    if not root or not path:
        return False
    try:
        root = os.path.abspath(root)
        path = os.path.abspath(path)
    except (OSError, ValueError):
        return False
    return path.startswith(root + os.sep)


def stat_key(path):
    """(mtime, size) for `path`, or None if it's gone — how the docker tells whether an
    operation actually rewrote a file it has open. Missing file is a real answer, not an error."""
    try:
        st = os.stat(path)
    except OSError:
        return None
    return (st.st_mtime_ns, st.st_size)


def find_repo(start_path):
    """Walk up from a file (or folder) looking for a `.kvc` directory. None if none found."""
    if not start_path:
        return None
    try:
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
    except (OSError, ValueError):
        # Unreadable or malformed path (Krita can hand back a URL-ish name for some
        # documents) — not version-controlled as far as we're concerned.
        return None


def _exec(binary, args, timeout):
    """Run `binary`, returning (raw output, exit code). Launch failures become KvcError."""
    try:
        proc = subprocess.run(
            [binary] + args,
            capture_output=True,
            text=True,
            # kvc emits UTF-8 JSON; without this, text=True decodes with the Windows locale
            # codepage (cp1252) and mangles non-ASCII repo paths / branch names / messages.
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            creationflags=_CREATION_FLAGS,
        )
    except subprocess.TimeoutExpired:
        raise KvcError(
            f"kvc didn't finish within {timeout}s. Close the Krita VC app if it's open, then retry."
        )
    except (OSError, ValueError, subprocess.SubprocessError) as e:
        raise KvcError(f"couldn't run kvc: {e}")
    return (proc.stdout if proc.returncode == 0 else proc.stderr), proc.returncode


def _parse(raw, code):
    text = (raw or "").strip()
    try:
        data = json.loads(text)
    except ValueError:
        raise KvcError(f"unexpected kvc output: {text[:200] or '(empty)'}")
    if not isinstance(data, dict):
        raise KvcError(f"unexpected kvc output: {text[:200]}")
    if code != 0:
        raise KvcError(str(data.get("error") or "unknown error"))
    return data


def _run(args, timeout=READ_TIMEOUT):
    binary = get_binary_path()
    if not binary:
        raise KvcError("kvc CLI not found. Set its location in the docker settings.")
    return _parse(*_exec(binary, args, timeout))


def _paths_flag(paths):
    """`--paths` is a JSON array, and omitted entirely for "everything" — passing a literal
    "null" would just be a parse error on the other side."""
    return ["--paths", json.dumps(paths)] if paths is not None else []


def status(repo):
    return _run(["status", "--repo", repo])


def commit(repo, message, author, paths=None):
    return _run(
        ["commit", "--repo", repo, "--message", message, "--author", author]
        + _paths_flag(paths),
        WRITE_TIMEOUT,
    )


def discard(repo, paths=None):
    return _run(["discard", "--repo", repo] + _paths_flag(paths), WRITE_TIMEOUT)


def stash(repo, author, label="", paths=None):
    return _run(
        ["stash", "--repo", repo, "--author", author, "--label", label] + _paths_flag(paths),
        WRITE_TIMEOUT,
    )


def stash_pop(repo, stash_id):
    return _run(["stash-pop", "--repo", repo, "--id", stash_id], WRITE_TIMEOUT)


def stash_list(repo):
    return _run(["stash-list", "--repo", repo])


def branches(repo):
    return _run(["branches", "--repo", repo])


def switch(repo, name):
    return _run(["switch", "--repo", repo, "--branch", name], WRITE_TIMEOUT)


def create_branch(repo, name, base=None):
    args = ["create-branch", "--repo", repo, "--name", name]
    if base:
        args += ["--base", base]
    return _run(args, WRITE_TIMEOUT)
