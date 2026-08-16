from __future__ import annotations

import io
import json
import os
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from tactus_runtime import effect_host
from tactus_runtime.plugin_protocol import API_VERSION


def _run_effect(
    method: str,
    params: dict[str, object],
    request_id: str = "effect-1",
) -> tuple[int, list[dict[str, object]], str]:
    request = json.dumps(
        {
            "api": API_VERSION,
            "id": request_id,
            "method": method,
            "params": params,
        },
        ensure_ascii=False,
    )
    stdout = io.StringIO()
    stderr = io.StringIO()
    exit_code = effect_host.main(
        ["workspace-paths"],
        stdin=io.StringIO(request),
        stdout=stdout,
        stderr=stderr,
    )
    lines = [json.loads(line) for line in stdout.getvalue().splitlines()]
    return exit_code, lines, stderr.getvalue()


def test_effect_host_uses_utf8_for_legacy_codepage_standard_streams(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    request_id = "路径-😀"
    request = json.dumps(
        {
            "api": API_VERSION,
            "id": request_id,
            "method": "describe",
            "params": {},
        },
        ensure_ascii=False,
    )
    raw_stdin = io.BytesIO(request.encode("utf-8"))
    raw_stdout = io.BytesIO()
    stdin = io.TextIOWrapper(raw_stdin, encoding="cp936", errors="strict")
    stdout = io.TextIOWrapper(raw_stdout, encoding="cp936", errors="strict")

    with monkeypatch.context() as scoped:
        scoped.setattr(effect_host.sys, "stdin", stdin)
        scoped.setattr(effect_host.sys, "stdout", stdout)
        exit_code = effect_host.main(
            ["workspace-paths"],
            stderr=io.StringIO(),
        )

    stdout.flush()
    [terminal] = [
        json.loads(line) for line in raw_stdout.getvalue().decode("utf-8").splitlines()
    ]

    assert exit_code == 0
    assert stdin.encoding == "utf-8"
    assert stdout.encoding == "utf-8"
    assert not stdin.closed
    assert not stdout.closed
    assert terminal["id"] == request_id
    assert terminal["ok"] is True


def test_snapshot_tracks_ignored_and_unicode_paths_but_excludes_control_dirs(
    tmp_path: Path,
) -> None:
    (tmp_path / ".git").mkdir()
    (tmp_path / ".git" / "config").write_text("secret", encoding="utf-8")
    (tmp_path / ".tactus").mkdir()
    (tmp_path / ".tactus" / "runtime.json").write_text("{}", encoding="utf-8")
    (tmp_path / ".tactus" / "scripts").mkdir()
    (tmp_path / ".tactus" / "scripts" / "010_plan.hs").write_text(
        "main = pure ()\n", encoding="utf-8"
    )
    (tmp_path / ".tactus" / "path-effect").mkdir()
    (tmp_path / ".tactus" / "path-effect" / "private.json").write_text(
        "{}", encoding="utf-8"
    )
    (tmp_path / ".tactus" / "dist-newstyle").mkdir()
    (tmp_path / ".tactus" / "dist-newstyle" / "cache").write_text(
        "build", encoding="utf-8"
    )
    (tmp_path / ".gitignore").write_text("ignored/\n", encoding="utf-8")
    ignored = tmp_path / "ignored"
    ignored.mkdir()
    (ignored / "still-tracked.txt").write_text("tracked", encoding="utf-8")
    unicode_directory = tmp_path / "目录"
    unicode_directory.mkdir()
    unicode_file = unicode_directory / "你好.txt"
    unicode_file.write_text("雪", encoding="utf-8")

    snapshot = effect_host.snapshot_workspace(tmp_path)
    paths = snapshot["paths"]
    assert ".gitignore" in paths
    assert "ignored/still-tracked.txt" in paths
    assert "目录/你好.txt" in paths
    assert not any(path == ".git" or path.startswith(".git/") for path in paths)
    assert ".tactus" not in paths
    assert ".tactus/runtime.json" in paths
    assert ".tactus/scripts/010_plan.hs" in paths
    assert not any(path.startswith(".tactus/path-effect") for path in paths)
    assert not any(path.startswith(".tactus/dist-newstyle") for path in paths)
    metadata = paths["目录/你好.txt"]
    assert metadata["kind"] == "file"
    assert metadata["size"] == len("雪".encode())
    assert len(metadata["sha256"]) == 64


def test_diff_classifies_add_modify_delete_and_type_change(tmp_path: Path) -> None:
    modified = tmp_path / "modified.txt"
    deleted = tmp_path / "deleted.txt"
    switched = tmp_path / "switched"
    modified.write_text("before", encoding="utf-8")
    deleted.write_text("remove me", encoding="utf-8")
    switched.write_text("file first", encoding="utf-8")
    before = effect_host.snapshot_workspace(tmp_path)

    modified.write_text("after and longer", encoding="utf-8")
    deleted.unlink()
    (tmp_path / "added.txt").write_text("new", encoding="utf-8")
    switched.unlink()
    switched.mkdir()
    (switched / "child.txt").write_text("child", encoding="utf-8")
    after = effect_host.snapshot_workspace(tmp_path)

    delta = effect_host.diff_snapshots(before, after)
    assert "added.txt" in delta["added"]
    assert "switched/child.txt" in delta["added"]
    assert delta["modified"] == ["modified.txt"]
    assert delta["deleted"] == ["deleted.txt"]
    assert delta["type_changed"] == ["switched"]


def test_explicit_snapshot_diff_and_forget_use_opaque_handles(tmp_path: Path) -> None:
    tracked = tmp_path / "tracked.txt"
    tracked.write_text("before", encoding="utf-8")

    exit_code, lines, diagnostics = _run_effect(
        "snapshot",
        {"workspace": str(tmp_path), "options": {"future": True}},
    )
    assert exit_code == 0
    assert diagnostics == ""
    before = lines[-1]["value"]
    assert set(before) == {"snapshot_id"}

    tracked.write_text("after", encoding="utf-8")
    (tmp_path / "added.txt").write_text("new", encoding="utf-8")
    exit_code, lines, diagnostics = _run_effect(
        "snapshot",
        {"workspace": str(tmp_path)},
    )
    assert exit_code == 0
    assert diagnostics == ""
    after = lines[-1]["value"]
    assert set(after) == {"snapshot_id"}

    exit_code, lines, diagnostics = _run_effect(
        "diff",
        {
            "workspace": str(tmp_path),
            "before": before,
            "after": after,
            "options": {"unknown": "accepted"},
        },
    )
    assert exit_code == 0
    assert diagnostics == ""
    delta = lines[-1]["value"]
    assert delta["added"] == ["added.txt"]
    assert delta["modified"] == ["tracked.txt"]
    state_directory = tmp_path / ".tactus" / "path-effect"
    assert len(list(state_directory.glob("snapshot-*.json"))) == 2

    for handle in (before, after):
        exit_code, lines, diagnostics = _run_effect(
            "forget",
            {"workspace": str(tmp_path), "snapshot_id": handle["snapshot_id"]},
        )
        assert exit_code == 0
        assert diagnostics == ""
        assert lines[-1]["value"] == {"forgotten": True}
    assert not state_directory.exists()

    exit_code, lines, diagnostics = _run_effect(
        "forget",
        {"workspace": str(tmp_path), "snapshot_id": before["snapshot_id"]},
    )
    assert exit_code == 0
    assert diagnostics == ""
    assert lines[-1]["value"] == {"forgotten": False}


def test_observe_begin_end_tracks_delta_and_cleans_opaque_state(tmp_path: Path) -> None:
    original = tmp_path / "原始.txt"
    original.write_text("before", encoding="utf-8")
    invocation = {
        "run": "run-你好",
        "context": {"attempt": 1, "labels": ["provider", "effect"]},
    }

    exit_code, lines, diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": invocation},
    )

    assert exit_code == 0
    assert diagnostics == ""
    begin = lines[-1]["value"]
    token = begin["token"]
    state_file = tmp_path / ".tactus" / "path-effect" / f"{token}.json"
    assert state_file.is_file()
    assert len(token) == 48

    original.write_text("after", encoding="utf-8")
    (tmp_path / "新增.txt").write_text("new", encoding="utf-8")
    outcome = {"status": "completed", "provider": "codex"}
    exit_code, lines, diagnostics = _run_effect(
        "observe.end",
        {
            "workspace": str(tmp_path),
            "invocation": invocation,
            "begin": begin,
            "outcome": outcome,
        },
    )

    assert exit_code == 0
    assert diagnostics == ""
    value = lines[-1]["value"]
    assert value["invocation"] == invocation
    assert value["outcome"] == outcome
    assert value["delta"]["added"] == ["新增.txt"]
    assert value["delta"]["modified"] == ["原始.txt"]
    assert not state_file.exists()
    assert not (tmp_path / ".tactus" / "path-effect").exists()
    assert not any(
        path.startswith(".tactus/path-effect") for path in value["delta"]["added"]
    )


def test_observe_end_restores_state_when_invocation_does_not_match(
    tmp_path: Path,
) -> None:
    _exit_code, lines, _diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": {"id": "correct"}},
    )
    begin = lines[-1]["value"]
    token = begin["token"]
    state_file = tmp_path / ".tactus" / "path-effect" / f"{token}.json"

    exit_code, lines, diagnostics = _run_effect(
        "observe.end",
        {
            "workspace": str(tmp_path),
            "invocation": {"id": "wrong"},
            "begin": begin,
            "outcome": "failed",
        },
    )

    assert exit_code == 1
    assert lines[-1]["ok"] is False
    assert lines[-1]["error"]["code"] == "state_invalid"
    assert "invocation" in lines[-1]["error"]["message"]
    assert diagnostics
    assert state_file.exists()

    exit_code, lines, diagnostics = _run_effect(
        "observe.end",
        {
            "workspace": str(tmp_path),
            "invocation": {"id": "correct"},
            "begin": begin,
            "outcome": "retried",
        },
    )
    assert exit_code == 0
    assert diagnostics == ""
    assert lines[-1]["value"]["outcome"] == "retried"
    assert not state_file.exists()


def test_forget_is_idempotent_and_removes_stored_snapshot(tmp_path: Path) -> None:
    _exit_code, lines, _diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": ["forget-me", 1]},
    )
    token = lines[-1]["value"]["token"]
    params = {
        "workspace": str(tmp_path),
        "invocation": ["forget-me", 1],
        "token": token,
    }

    exit_code, lines, _diagnostics = _run_effect("forget", params)
    assert exit_code == 0
    assert lines[-1]["value"] == {"forgotten": True}

    exit_code, lines, _diagnostics = _run_effect("forget", params)
    assert exit_code == 0
    assert lines[-1]["value"] == {"forgotten": False}


def test_forget_reports_state_delete_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _exit_code, lines, _diagnostics = _run_effect(
        "snapshot",
        {"workspace": str(tmp_path)},
    )
    snapshot_id = lines[-1]["value"]["snapshot_id"]
    state_file = tmp_path / ".tactus" / "path-effect" / f"snapshot-{snapshot_id}.json"
    original_unlink = Path.unlink

    def fail_state_unlink(path: Path, *args: object, **kwargs: object) -> None:
        if path.parent == state_file.parent and path.name.endswith(".claimed"):
            raise PermissionError("state is locked")
        original_unlink(path, *args, **kwargs)

    monkeypatch.setattr(Path, "unlink", fail_state_unlink)
    exit_code, result, diagnostics = _run_effect(
        "forget",
        {"workspace": str(tmp_path), "snapshot_id": snapshot_id},
    )

    assert exit_code == 1
    assert result[-1]["ok"] is False
    assert result[-1]["error"]["code"] == "state_delete_failed"
    assert state_file.exists()
    assert not any(state_file.parent.glob("*.claimed"))
    assert diagnostics

    monkeypatch.undo()
    exit_code, result, diagnostics = _run_effect(
        "forget",
        {"workspace": str(tmp_path), "snapshot_id": snapshot_id},
    )
    assert exit_code == 0
    assert result[-1]["value"] == {"forgotten": True}
    assert diagnostics == ""


def test_observation_token_is_consumed_once_under_concurrent_end(
    tmp_path: Path,
) -> None:
    invocation = {"id": "single-consumer"}
    _exit_code, lines, _diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": invocation},
    )
    begin = lines[-1]["value"]
    params = {
        "workspace": str(tmp_path),
        "invocation": invocation,
        "begin": begin,
        "outcome": "ok",
    }

    with ThreadPoolExecutor(max_workers=2) as pool:
        results = list(
            pool.map(lambda _index: _run_effect("observe.end", params), range(2))
        )

    assert sorted(result[0] for result in results) == [0, 1]
    failure = next(result for result in results if result[0] == 1)
    assert failure[1][-1]["error"]["code"] == "observation_not_found"
    state_directory = tmp_path / ".tactus" / "path-effect"
    assert not state_directory.exists()


def test_observation_end_and_forget_cannot_both_consume_one_token(
    tmp_path: Path,
) -> None:
    invocation = {"id": "end-forget-race"}
    _exit_code, lines, _diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": invocation},
    )
    begin = lines[-1]["value"]
    end_params = {
        "workspace": str(tmp_path),
        "invocation": invocation,
        "begin": begin,
        "outcome": "ok",
    }
    forget_params = {
        "workspace": str(tmp_path),
        "invocation": invocation,
        "token": begin["token"],
    }

    with ThreadPoolExecutor(max_workers=2) as pool:
        end_future = pool.submit(_run_effect, "observe.end", end_params)
        forget_future = pool.submit(_run_effect, "forget", forget_params)
        end_result = end_future.result()
        forget_result = forget_future.result()

    end_succeeded = end_result[0] == 0
    forget_succeeded = forget_result[0] == 0 and forget_result[1][-1].get("value") == {
        "forgotten": True
    }
    assert end_succeeded is not forget_succeeded
    assert not (tmp_path / ".tactus" / "path-effect").exists()


def test_partial_state_write_is_never_published(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def interrupted_dump(
        _record: object,
        stream: object,
        **_kwargs: object,
    ) -> None:
        stream.write("{")  # type: ignore[attr-defined]
        raise OSError("disk interrupted")

    monkeypatch.setattr(effect_host.json, "dump", interrupted_dump)
    exit_code, lines, _diagnostics = _run_effect(
        "snapshot",
        {"workspace": str(tmp_path)},
    )

    assert exit_code == 1
    assert lines[-1]["error"]["code"] == "state_write_failed"
    state_directory = tmp_path / ".tactus" / "path-effect"
    assert not state_directory.exists() or not any(state_directory.iterdir())


def test_snapshot_fails_when_file_identity_changes_before_open(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target = tmp_path / "tracked.txt"
    target.write_text("original", encoding="utf-8")
    excluded = tmp_path / ".git"
    excluded.mkdir()
    replacement = excluded / "replacement.txt"
    replacement.write_text("must not be attributed", encoding="utf-8")
    original_open = Path.open
    swapped = False

    def swapping_open(path: Path, *args: object, **kwargs: object) -> object:
        nonlocal swapped
        if path == target and not swapped:
            swapped = True
            os.replace(replacement, target)
        return original_open(path, *args, **kwargs)

    monkeypatch.setattr(Path, "open", swapping_open)

    with pytest.raises(effect_host.PluginError) as failure:
        effect_host.snapshot_workspace(tmp_path)

    assert failure.value.code == "snapshot_failed"


def test_snapshot_fails_when_directory_identity_changes_before_scan(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target = tmp_path / "tracked"
    target.mkdir()
    (target / "original.txt").write_text("original", encoding="utf-8")
    excluded = tmp_path / ".git"
    excluded.mkdir()
    replacement = excluded / "replacement"
    replacement.mkdir()
    (replacement / "outside.txt").write_text("outside", encoding="utf-8")
    saved = tmp_path / "saved-original"
    original_scandir = os.scandir
    swapped = False

    def swapping_scandir(path: object) -> object:
        nonlocal swapped
        if Path(path) == target and not swapped:
            swapped = True
            target.rename(saved)
            replacement.rename(target)
        return original_scandir(path)

    monkeypatch.setattr(effect_host.os, "scandir", swapping_scandir)

    with pytest.raises(effect_host.PluginError) as failure:
        effect_host.snapshot_workspace(tmp_path)

    assert failure.value.code == "snapshot_failed"


def test_snapshot_does_not_follow_directory_symlink(tmp_path: Path) -> None:
    target = tmp_path / "target"
    target.mkdir()
    (target / "inside.txt").write_text("content", encoding="utf-8")
    link = tmp_path / "link"
    try:
        link.symlink_to(target, target_is_directory=True)
    except OSError as exc:
        pytest.skip(f"symlinks unavailable: {exc}")

    snapshot = effect_host.snapshot_workspace(tmp_path)
    paths = snapshot["paths"]

    assert paths["link"]["kind"] == "symlink"
    assert "link/inside.txt" not in paths
    assert "target/inside.txt" in paths


def test_effect_state_refuses_symlinked_control_directory(tmp_path: Path) -> None:
    outside = tmp_path.parent / f"{tmp_path.name}-outside-state"
    outside.mkdir()
    tactus = tmp_path / ".tactus"
    tactus.mkdir()
    try:
        (tactus / "path-effect").symlink_to(outside, target_is_directory=True)
    except OSError as exc:
        pytest.skip(f"directory symlinks unavailable: {exc}")

    exit_code, lines, _diagnostics = _run_effect(
        "observe.begin",
        {"workspace": str(tmp_path), "invocation": {"id": "no-follow"}},
    )

    assert exit_code == 1
    assert lines[-1]["error"]["code"] == "state_path_invalid"
    assert list(outside.iterdir()) == []


def test_smoke_and_describe_are_observational(tmp_path: Path) -> None:
    exit_code, lines, diagnostics = _run_effect("describe", {})
    assert exit_code == 0
    assert diagnostics == ""
    assert lines[-1]["value"]["name"] == "workspace-paths"
    assert lines[-1]["value"]["implementation_version"] == "0.3.0"
    assert lines[-1]["value"]["operations"] == lines[-1]["value"]["methods"]
    assert lines[-1]["value"]["options_schema"]["additionalProperties"] is True
    assert lines[-1]["value"]["enforcement"] is False

    exit_code, lines, diagnostics = _run_effect(
        "smoke",
        {"workspace": str(tmp_path), "live": False},
    )
    assert exit_code == 0
    assert diagnostics == ""
    assert lines[-1]["value"]["text"] == "workspace-paths ok"
    assert not (tmp_path / ".tactus").exists()
