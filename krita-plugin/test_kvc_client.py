"""Self-check for kvc_client's error paths: `python krita-plugin/test_kvc_client.py`.

Only the failure modes matter here — every one of these used to reach a Qt slot as a
non-KvcError and abort Krita. Stubs `krita` so this runs outside Krita; no test runner
in the repo (the Rust tests are the real suite), so asserts in __main__ it is.
"""

import importlib.util
import os
import subprocess
import sys
import types

_krita = types.ModuleType("krita")
_krita.Krita = types.SimpleNamespace(
    instance=lambda: types.SimpleNamespace(
        readSetting=lambda *a: "", writeSetting=lambda *a: None
    )
)
sys.modules["krita"] = _krita

# Loaded by path, not as `kritavc.kvc_client`: the package __init__ registers a docker
# with a live Krita, which there isn't one of out here.
_spec = importlib.util.spec_from_file_location(
    "kvc_client",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "kritavc", "kvc_client.py"),
)
kvc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(kvc)


def fake_exec(out="", code=0):
    return lambda binary, args, timeout: (out, code)


def fake_run(raises):
    """Fakes subprocess.run, not _exec — _exec is what turns launch failures into KvcError."""

    def run(*args, **kwargs):
        raise raises

    return run


def expect_kvc_error(fn, needle=""):
    try:
        fn()
    except kvc.KvcError as e:
        assert needle in str(e), f"expected {needle!r} in {e!r}"
        return
    except Exception as e:
        raise AssertionError(f"leaked {type(e).__name__} instead of KvcError: {e}")
    raise AssertionError("expected KvcError, got a clean return")


def test_parse_rejects_non_json():
    # Wrong executable picked: prints its own help, not JSON.
    expect_kvc_error(lambda: kvc._parse("Usage: notepad [file]", 1), "unexpected")


def test_parse_rejects_empty():
    expect_kvc_error(lambda: kvc._parse("", 0), "(empty)")


def test_parse_rejects_non_dict():
    # .get() on a list is an AttributeError -> abort.
    expect_kvc_error(lambda: kvc._parse("[1, 2]", 1), "unexpected")


def test_parse_surfaces_cli_error():
    expect_kvc_error(lambda: kvc._parse('{"error": "unsaved changes"}', 1), "unsaved changes")


def test_parse_error_without_message():
    expect_kvc_error(lambda: kvc._parse("{}", 1), "unknown error")


def test_parse_ok():
    assert kvc._parse('{"changes": []}', 0) == {"changes": []}


def test_timeout_is_kvc_error():
    # A commit on a big .kra can legitimately outrun the timeout; that used to be an
    # uncaught TimeoutExpired straight out of a Qt slot.
    kvc.get_binary_path = lambda: "kvc"
    subprocess.run = fake_run(subprocess.TimeoutExpired("kvc", 30))
    expect_kvc_error(lambda: kvc._run(["status"]), "didn't finish")


def test_launch_failure_is_kvc_error():
    kvc.get_binary_path = lambda: "kvc"
    subprocess.run = fake_run(OSError("not executable"))
    expect_kvc_error(lambda: kvc._run(["status"]), "couldn't run kvc")


def test_missing_binary_is_kvc_error():
    kvc.get_binary_path = lambda: None
    expect_kvc_error(lambda: kvc._run(["status"]), "not found")


def test_verify_rejects_missing_file():
    expect_kvc_error(lambda: kvc.verify_binary("/nope/kvc.exe"), "doesn't exist")


def test_verify_rejects_wrong_binary():
    kvc._exec = fake_exec(out='{"error": "some other tool"}', code=1)
    expect_kvc_error(lambda: kvc.verify_binary(__file__), "isn't the kvc tool")


def test_verify_accepts_real_binary():
    kvc._exec = fake_exec(out='{"error": "usage: kvc <status|commit|...>"}', code=1)
    kvc.verify_binary(__file__)


def test_find_repo_survives_bad_paths():
    assert kvc.find_repo(None) is None
    assert kvc.find_repo("") is None
    assert kvc.find_repo("\0bad") is None


def capture_args():
    """Records the argv a wrapper builds, and returns kvc's minimal success shape."""
    seen = []

    def fake(binary, args, timeout):
        seen.append(args)
        return '{"ok": true}', 0

    kvc._exec = fake
    kvc.get_binary_path = lambda: "kvc"
    return seen


def test_paths_flag_omitted_when_none():
    # A literal "null" would reach the CLI's serde parse as a non-array and error out;
    # omitting the flag is what means "everything".
    seen = capture_args()
    kvc.commit("R", "m", "me")
    assert "--paths" not in seen[0], seen[0]


def test_paths_flag_is_json_array():
    seen = capture_args()
    kvc.commit("R", "m", "me", ["a.kra", "b/c, d.kra"])
    assert seen[0][seen[0].index("--paths") + 1] == '["a.kra", "b/c, d.kra"]', seen[0]


def test_empty_paths_list_is_not_everything():
    # [] is falsy but means "no files" — it must still send --paths, or an empty tick set
    # would silently commit the whole tree.
    seen = capture_args()
    kvc.discard("R", [])
    assert seen[0][seen[0].index("--paths") + 1] == "[]", seen[0]


def test_write_wrappers_build_expected_argv():
    seen = capture_args()
    kvc.discard("R")
    kvc.stash("R", "me", "wip")
    kvc.stash_pop("R", "s1")
    kvc.stash_list("R")
    assert seen[0] == ["discard", "--repo", "R"], seen[0]
    assert seen[1] == ["stash", "--repo", "R", "--author", "me", "--label", "wip"], seen[1]
    assert seen[2] == ["stash-pop", "--repo", "R", "--id", "s1"], seen[2]
    assert seen[3] == ["stash-list", "--repo", "R"], seen[3]


def test_in_repo_rejects_sibling_with_shared_prefix():
    root = os.path.join("art", "repo")
    assert kvc.in_repo(root, os.path.join(root, "hero.kra"))
    assert kvc.in_repo(root, os.path.join(root, "chars", "hero.kra"))
    # The bug a bare startswith() would have: a sibling folder whose name extends the root's.
    assert not kvc.in_repo(root, os.path.join("art", "repo2", "hero.kra"))
    assert not kvc.in_repo(root, root)  # the folder itself isn't a document in it
    assert not kvc.in_repo(root, None)
    assert not kvc.in_repo(None, "x.kra")


def test_stat_key_of_missing_file_is_none():
    # Used to decide "did the op rewrite this?" — a deleted file must answer, not raise.
    assert kvc.stat_key(os.path.join(os.path.dirname(__file__), "nope.kra")) is None
    assert kvc.stat_key(__file__) is not None


if __name__ == "__main__":
    real = (kvc._exec, kvc.get_binary_path, subprocess.run)
    for name, fn in sorted(globals().items()):
        if name.startswith("test_"):
            kvc._exec, kvc.get_binary_path, subprocess.run = real
            fn()
            print(f"ok {name}")
    print("all good")
