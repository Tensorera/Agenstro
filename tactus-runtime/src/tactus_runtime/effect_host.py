"""Reference ``workspace-paths`` observation effect plugin."""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import re
import secrets
import stat
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import TextIO, cast

from . import __version__
from .plugin_protocol import (
    API_VERSION,
    EventWriter,
    JsonObject,
    PluginError,
    PluginRequest,
    configure_utf8_standard_stream,
    run_plugin,
)

_EFFECT_NAME = "workspace-paths"
_EXCLUDED_RELATIVE_PATHS = frozenset(
    {
        ".git",
        ".tactus/path-effect",
        ".tactus/dist-newstyle",
    }
)
_TRANSPARENT_DIRECTORY_PATHS = frozenset({".tactus"})
_TOKEN_PATTERN = re.compile(r"\A[0-9a-f]{48}\Z")


def build_parser() -> argparse.ArgumentParser:
    """Build the standalone effect host parser."""
    parser = argparse.ArgumentParser(
        prog="tactus-effect-host",
        description="Run one Tactus workspace path observation request from stdin.",
    )
    parser.add_argument("effect", choices=[_EFFECT_NAME])
    return parser


def main(
    argv: Sequence[str] | None = None,
    *,
    stdin: TextIO | None = None,
    stdout: TextIO | None = None,
    stderr: TextIO | None = None,
) -> int:
    """Run one effect request and return a process exit code."""
    input_stream = configure_utf8_standard_stream(sys.stdin) if stdin is None else stdin
    output_stream = (
        configure_utf8_standard_stream(sys.stdout) if stdout is None else stdout
    )
    build_parser().parse_args(argv)
    error_stream = sys.stderr if stderr is None else stderr
    return run_plugin(
        handle_request,
        stdin=input_stream,
        stdout=output_stream,
        stderr=error_stream,
    )


def handle_request(request: PluginRequest, writer: EventWriter) -> object:
    """Dispatch one workspace path effect request."""
    del writer
    if request.method == "describe":
        return _describe()
    if request.method == "smoke":
        workspace = _optional_workspace(request.params) or Path.cwd().resolve()
        current = snapshot_workspace(workspace)
        return {
            "effect": _EFFECT_NAME,
            "text": "workspace-paths ok",
            "path_count": _path_count(current),
        }
    if request.method == "snapshot":
        return _persist_snapshot(request.params)
    if request.method == "diff":
        return _diff_persisted_snapshots(request.params)
    if request.method == "observe.begin":
        return _observe_begin(request.params)
    if request.method == "observe.end":
        return _observe_end(request.params)
    if request.method == "forget":
        return _forget(request.params)
    raise PluginError(
        "method_not_found",
        f"effect {_EFFECT_NAME!r} does not implement {request.method!r}",
        details={
            "methods": [
                "describe",
                "smoke",
                "observe.begin",
                "observe.end",
                "snapshot",
                "diff",
                "forget",
            ]
        },
    )


def snapshot_workspace(workspace: Path) -> JsonObject:
    """Recursively capture path metadata without following symbolic links."""
    root = workspace.resolve()
    if not root.is_dir():
        raise PluginError(
            "invalid_params",
            "workspace must name an existing directory",
            details={"workspace": str(root)},
        )
    paths: dict[str, object] = {}
    try:
        _scan_directory(root, root, paths, root.lstat())
    except OSError as exc:
        raise PluginError(
            "snapshot_failed",
            f"could not snapshot workspace: {exc}",
            details={"workspace": str(root), "errno": exc.errno},
        ) from exc
    return {
        "workspace": str(root),
        "paths": dict(sorted(paths.items())),
    }


def diff_snapshots(
    before: Mapping[str, object], after: Mapping[str, object]
) -> JsonObject:
    """Return deterministic path sets changed between two snapshots."""
    before_paths = _snapshot_paths(before)
    after_paths = _snapshot_paths(after)
    before_names = set(before_paths)
    after_names = set(after_paths)
    added = sorted(after_names - before_names)
    deleted = sorted(before_names - after_names)
    modified: list[str] = []
    type_changed: list[str] = []
    for name in sorted(before_names & after_names):
        old = before_paths[name]
        new = after_paths[name]
        if _entry_kind(old) != _entry_kind(new):
            type_changed.append(name)
        elif old != new:
            modified.append(name)
    return {
        "added": added,
        "modified": modified,
        "deleted": deleted,
        "type_changed": type_changed,
    }


def _describe() -> JsonObject:
    operations = [
        "describe",
        "smoke",
        "observe.begin",
        "observe.end",
        "snapshot",
        "diff",
        "forget",
    ]
    return {
        "api": API_VERSION,
        "kind": "effect",
        "name": _EFFECT_NAME,
        "implementation_version": __version__,
        "methods": operations,
        "operations": operations,
        "options_schema": {"type": "object", "additionalProperties": True},
        "observes": ["added", "modified", "deleted", "type_changed"],
        "excludes": [
            "/.git",
            "/.tactus/path-effect",
            "/.tactus/dist-newstyle",
        ],
        "transparent_directories": ["/.tactus"],
        "follows_symlinks": False,
        "enforcement": False,
    }


def _observe_begin(params: Mapping[str, object]) -> JsonObject:
    workspace = _required_workspace(params)
    invocation = _required_json_value(params, "invocation")
    before = snapshot_workspace(workspace)
    token = secrets.token_hex(24)
    state_directory = _state_directory(workspace, create=True)
    try:
        state_path = state_directory / f"{token}.json"
        record = {
            "api": API_VERSION,
            "effect": _EFFECT_NAME,
            "state_kind": "observation",
            "workspace": str(workspace),
            "invocation": invocation,
            "snapshot": before,
        }
        _write_state(state_path, record)
    except OSError as exc:
        raise PluginError(
            "state_write_failed",
            f"could not persist observation state: {exc}",
            details={"workspace": str(workspace), "errno": exc.errno},
        ) from exc
    return {
        "token": token,
        "path_count": _path_count(before),
    }


def _observe_end(params: Mapping[str, object]) -> JsonObject:
    workspace = _required_workspace(params)
    invocation = _required_json_value(params, "invocation")
    token = _observation_token(params)
    if "outcome" not in params:
        raise PluginError("invalid_params", "outcome is required")
    outcome = params["outcome"]
    state_path = _state_path(workspace, token)
    claimed_path = _claim_state(
        state_path,
        not_found_code="observation_not_found",
        not_found_message="observation token was not found",
    )
    if claimed_path is None:  # pragma: no cover - required claim cannot be absent
        raise PluginError("observation_not_found", "observation token was not found")
    try:
        record = _load_state(claimed_path)
        _validate_observation_state(record, workspace, invocation)
        before = _required_snapshot(record, "snapshot")
        after = snapshot_workspace(workspace)
        delta = diff_snapshots(before, after)
        result: JsonObject = {
            "invocation": invocation,
            "outcome": outcome,
            "delta": delta,
            "before_count": _path_count(before),
            "after_count": _path_count(after),
        }
        _delete_state(claimed_path)
    except BaseException:
        _restore_claim(claimed_path, state_path)
        raise
    return result


def _forget(params: Mapping[str, object]) -> JsonObject:
    workspace = _required_workspace(params)
    if "snapshot_id" in params:
        snapshot_id = _snapshot_id(params["snapshot_id"])
        state_path = _snapshot_state_path(workspace, snapshot_id)
        claimed_path = _claim_state(state_path, missing_ok=True)
        if claimed_path is None:
            return {"forgotten": False}
        try:
            _delete_state(claimed_path)
        except BaseException:
            _restore_claim(claimed_path, state_path)
            raise
        return {"forgotten": True}

    token = _observation_token(params)
    invocation = params.get("invocation")
    state_path = _state_path(workspace, token)
    claimed_path = _claim_state(state_path, missing_ok=True)
    if claimed_path is None:
        return {"forgotten": False}
    try:
        if "invocation" in params:
            record = _load_state(claimed_path)
            _validate_observation_state(record, workspace, invocation)
        _delete_state(claimed_path)
    except BaseException:
        _restore_claim(claimed_path, state_path)
        raise
    return {"forgotten": True}


def _persist_snapshot(params: Mapping[str, object]) -> JsonObject:
    workspace = _required_workspace(params)
    snapshot = snapshot_workspace(workspace)
    snapshot_id = secrets.token_hex(24)
    state_directory = _state_directory(workspace, create=True)
    try:
        state_path = state_directory / f"snapshot-{snapshot_id}.json"
        record = {
            "api": API_VERSION,
            "effect": _EFFECT_NAME,
            "state_kind": "snapshot",
            "workspace": str(workspace),
            "snapshot": snapshot,
        }
        _write_state(state_path, record)
    except OSError as exc:
        raise PluginError(
            "state_write_failed",
            f"could not persist workspace snapshot: {exc}",
            details={"workspace": str(workspace), "errno": exc.errno},
        ) from exc
    return {"snapshot_id": snapshot_id}


def _diff_persisted_snapshots(params: Mapping[str, object]) -> JsonObject:
    workspace = _required_workspace(params)
    before_id = _snapshot_handle_id(params, "before")
    after_id = _snapshot_handle_id(params, "after")
    before = _load_persisted_snapshot(workspace, before_id)
    after = _load_persisted_snapshot(workspace, after_id)
    return diff_snapshots(before, after)


def _scan_directory(
    root: Path,
    directory: Path,
    paths: dict[str, object],
    expected: os.stat_result,
) -> None:
    current = directory.lstat()
    if not stat.S_ISDIR(current.st_mode) or not os.path.samestat(expected, current):
        raise OSError(f"directory changed while snapshotting: {directory}")
    with os.scandir(directory) as iterator:
        opened = directory.lstat()
        if not stat.S_ISDIR(opened.st_mode) or not os.path.samestat(expected, opened):
            raise OSError(f"directory changed while snapshotting: {directory}")
        entries = sorted(iterator, key=lambda item: item.name)
    for entry in entries:
        absolute = directory / entry.name
        relative = absolute.relative_to(root).as_posix()
        if _is_excluded(relative):
            continue
        try:
            metadata, identity = _entry_metadata(absolute)
        except FileNotFoundError:
            continue
        if not (
            metadata["kind"] == "directory"
            and _normalized_relative(relative) in _TRANSPARENT_DIRECTORY_PATHS
        ):
            paths[relative] = metadata
        if metadata["kind"] == "directory":
            _scan_directory(root, absolute, paths, identity)


def _is_excluded(relative: str) -> bool:
    return _normalized_relative(relative) in _EXCLUDED_RELATIVE_PATHS


def _normalized_relative(relative: str) -> str:
    return relative.casefold() if os.name == "nt" else relative


def _entry_metadata(path: Path) -> tuple[JsonObject, os.stat_result]:
    metadata = path.lstat()
    mode = metadata.st_mode
    if stat.S_ISLNK(mode):
        target = os.readlink(path)
        return (
            {
                "kind": "symlink",
                "size": metadata.st_size,
                "sha256": hashlib.sha256(os.fsencode(target)).hexdigest(),
            },
            metadata,
        )
    if stat.S_ISDIR(mode):
        return {"kind": "directory", "size": 0, "sha256": None}, metadata
    if stat.S_ISREG(mode):
        return (
            {
                "kind": "file",
                "size": metadata.st_size,
                "sha256": _file_sha256(path, metadata),
            },
            metadata,
        )
    return {"kind": "other", "size": metadata.st_size, "sha256": None}, metadata


def _file_sha256(path: Path, expected: os.stat_result) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        opened = os.fstat(stream.fileno())
        if not stat.S_ISREG(opened.st_mode) or not os.path.samestat(expected, opened):
            raise OSError(f"file identity changed while snapshotting: {path}")
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
        finished = os.fstat(stream.fileno())
    current = path.lstat()
    if (
        not stat.S_ISREG(current.st_mode)
        or not os.path.samestat(expected, current)
        or not os.path.samestat(opened, finished)
        or _mutation_identity(opened) != _mutation_identity(finished)
    ):
        raise OSError(f"file changed while snapshotting: {path}")
    return digest.hexdigest()


def _mutation_identity(metadata: os.stat_result) -> tuple[int, int, int]:
    return metadata.st_size, metadata.st_mtime_ns, metadata.st_ctime_ns


def _snapshot_paths(snapshot: Mapping[str, object]) -> dict[str, object]:
    value = _string_keyed_object(snapshot.get("paths"))
    if value is None:
        raise PluginError("invalid_params", "snapshot.paths must be a JSON object")
    converted: dict[str, object] = {}
    for name, entry in value.items():
        entry_object = _string_keyed_object(entry)
        if entry_object is None:
            raise PluginError(
                "invalid_params",
                "every snapshot path must contain a metadata object",
                details={"path": name},
            )
        _entry_kind(entry_object)
        converted[name] = entry_object
    return converted


def _entry_kind(entry: object) -> str:
    value = _string_keyed_object(entry)
    if value is None:
        raise PluginError("invalid_params", "snapshot entry must be a JSON object")
    kind = value.get("kind")
    if not isinstance(kind, str) or not kind:
        raise PluginError("invalid_params", "snapshot entry kind must be a string")
    return kind


def _required_snapshot(params: Mapping[str, object], key: str) -> JsonObject:
    value = _string_keyed_object(params.get(key))
    if value is None:
        raise PluginError("invalid_params", f"{key} must be a snapshot object")
    snapshot: JsonObject = dict(value)
    _snapshot_paths(snapshot)
    return snapshot


def _path_count(snapshot: Mapping[str, object]) -> int:
    return len(_snapshot_paths(snapshot))


def _state_directory(workspace: Path, *, create: bool = False) -> Path:
    tactus_directory = workspace / ".tactus"
    state_directory = tactus_directory / "path-effect"
    for directory in (tactus_directory, state_directory):
        try:
            metadata = directory.lstat()
        except FileNotFoundError:
            if not create:
                return state_directory
            try:
                directory.mkdir()
                metadata = directory.lstat()
            except OSError as exc:
                raise PluginError(
                    "state_path_invalid",
                    f"could not create effect state directory: {exc}",
                    details={"path": str(directory), "errno": exc.errno},
                ) from exc
        except OSError as exc:
            raise PluginError(
                "state_path_invalid",
                f"could not inspect effect state directory: {exc}",
                details={"path": str(directory), "errno": exc.errno},
            ) from exc
        if not stat.S_ISDIR(metadata.st_mode):
            raise PluginError(
                "state_path_invalid",
                "effect state path must contain only real directories",
                details={"path": str(directory)},
            )
    return state_directory


def _state_path(workspace: Path, token: str) -> Path:
    return _state_directory(workspace) / f"{token}.json"


def _snapshot_state_path(workspace: Path, snapshot_id: str) -> Path:
    return _state_directory(workspace) / f"snapshot-{snapshot_id}.json"


def _load_state(
    path: Path,
    *,
    not_found_code: str = "observation_not_found",
    not_found_message: str = "observation token was not found",
) -> JsonObject:
    try:
        with path.open("r", encoding="utf-8") as stream:
            value: object = json.load(stream)
    except FileNotFoundError as exc:
        raise PluginError(
            not_found_code,
            not_found_message,
        ) from exc
    except (OSError, json.JSONDecodeError) as exc:
        raise PluginError(
            "state_read_failed",
            f"could not read observation state: {exc}",
        ) from exc
    record = _string_keyed_object(value)
    if record is None:
        raise PluginError("state_invalid", "observation state is not a JSON object")
    return record


def _load_persisted_snapshot(workspace: Path, snapshot_id: str) -> JsonObject:
    record = _load_state(
        _snapshot_state_path(workspace, snapshot_id),
        not_found_code="snapshot_not_found",
        not_found_message="workspace snapshot was not found",
    )
    _validate_snapshot_state(record, workspace)
    return _required_snapshot(record, "snapshot")


def _validate_observation_state(
    record: Mapping[str, object],
    workspace: Path,
    invocation: object,
) -> None:
    _validate_common_state(record, workspace, "observation")
    if not _json_equal(record.get("invocation"), invocation):
        raise PluginError("state_invalid", "observation invocation does not match")


def _validate_snapshot_state(record: Mapping[str, object], workspace: Path) -> None:
    _validate_common_state(record, workspace, "snapshot")


def _validate_common_state(
    record: Mapping[str, object],
    workspace: Path,
    expected_kind: str,
) -> None:
    if record.get("api") != API_VERSION or record.get("effect") != _EFFECT_NAME:
        raise PluginError("state_invalid", "observation state has an invalid protocol")
    state_kind = record.get("state_kind")
    if state_kind != expected_kind and not (
        expected_kind == "observation" and state_kind is None
    ):
        raise PluginError("state_invalid", "stored effect state has the wrong kind")
    if record.get("workspace") != str(workspace):
        raise PluginError("state_invalid", "observation workspace does not match")


def _json_equal(left: object, right: object) -> bool:
    try:
        left_encoded = json.dumps(
            left,
            allow_nan=False,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        )
        right_encoded = json.dumps(
            right,
            allow_nan=False,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        )
    except (TypeError, ValueError) as exc:
        raise PluginError(
            "invalid_params",
            "invocation must be a JSON value",
        ) from exc
    return left_encoded == right_encoded


def _write_state(path: Path, record: Mapping[str, object]) -> None:
    """Publish one complete state file without exposing a partial JSON record."""
    temporary = path.with_name(f".{path.name}.{secrets.token_hex(12)}.tmp")
    try:
        with temporary.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(record, stream, ensure_ascii=False, separators=(",", ":"))
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        with contextlib.suppress(OSError):
            temporary.unlink()


def _claim_state(
    path: Path,
    *,
    missing_ok: bool = False,
    not_found_code: str = "state_not_found",
    not_found_message: str = "effect state was not found",
) -> Path | None:
    """Reserve and move a state token so exactly one consumer can own it."""
    claimed = path.with_name(f".{path.name}.claimed")
    try:
        with claimed.open("x", encoding="utf-8"):
            pass
    except FileExistsError as exc:
        if missing_ok:
            return None
        raise PluginError(not_found_code, not_found_message) from exc
    except FileNotFoundError as exc:
        if missing_ok:
            return None
        raise PluginError(not_found_code, not_found_message) from exc
    except OSError as exc:
        raise PluginError(
            "state_claim_failed",
            f"could not reserve effect state: {exc}",
            details={"path": str(path), "errno": exc.errno},
        ) from exc
    try:
        os.replace(path, claimed)
    except FileNotFoundError as exc:
        with contextlib.suppress(OSError):
            claimed.unlink()
        if missing_ok:
            return None
        raise PluginError(not_found_code, not_found_message) from exc
    except OSError as exc:
        with contextlib.suppress(OSError):
            claimed.unlink()
        raise PluginError(
            "state_claim_failed",
            f"could not claim effect state: {exc}",
            details={"path": str(path), "errno": exc.errno},
        ) from exc
    return claimed


def _restore_claim(claimed: Path, original: Path) -> None:
    """Restore a claim that failed validation so its owner can retry."""
    try:
        os.replace(claimed, original)
    except OSError as exc:
        raise PluginError(
            "state_restore_failed",
            f"could not restore claimed effect state: {exc}",
            details={"path": str(original), "errno": exc.errno},
        ) from exc


def _delete_state(path: Path) -> bool:
    """Delete explicit state or fail honestly when the state remains on disk."""
    try:
        path.unlink()
    except FileNotFoundError:
        return False
    except OSError as exc:
        raise PluginError(
            "state_delete_failed",
            f"could not delete effect state: {exc}",
            details={"path": str(path), "errno": exc.errno},
        ) from exc
    with contextlib.suppress(OSError):
        path.parent.rmdir()
    return True


def _observation_token(params: Mapping[str, object]) -> str:
    nested_token: object = None
    if "begin" in params:
        begin = _string_keyed_object(params["begin"])
        if begin is None:
            raise PluginError("invalid_params", "begin must be a JSON object")
        nested_token = begin.get("token")
    top_level_token = params.get("token")
    if (
        nested_token is not None
        and top_level_token is not None
        and nested_token != top_level_token
    ):
        raise PluginError("invalid_params", "begin.token and token do not match")
    token = nested_token if nested_token is not None else top_level_token
    return _token_value(token)


def _token_value(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise PluginError("invalid_params", "token must be a non-empty string")
    token = value
    if _TOKEN_PATTERN.fullmatch(token) is None:
        raise PluginError("invalid_params", "token has an invalid shape")
    return token


def _snapshot_handle_id(params: Mapping[str, object], key: str) -> str:
    handle = _string_keyed_object(params.get(key))
    if handle is None:
        raise PluginError("invalid_params", f"{key} must be a snapshot handle object")
    return _snapshot_id(handle.get("snapshot_id"))


def _snapshot_id(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise PluginError("invalid_params", "snapshot_id must be a non-empty string")
    if _TOKEN_PATTERN.fullmatch(value) is None:
        raise PluginError("invalid_params", "snapshot_id has an invalid shape")
    return value


def _required_workspace(params: Mapping[str, object]) -> Path:
    workspace = _optional_workspace(params)
    if workspace is None:
        raise PluginError("invalid_params", "workspace must be a non-empty string")
    return workspace


def _optional_workspace(params: Mapping[str, object]) -> Path | None:
    value = params.get("workspace")
    if value is None:
        return None
    if not isinstance(value, str) or not value:
        raise PluginError("invalid_params", "workspace must be a non-empty string")
    path = Path(value).resolve()
    if not path.is_dir():
        raise PluginError(
            "invalid_params",
            "workspace must name an existing directory",
            details={"workspace": str(path)},
        )
    return path


def _required_json_value(params: Mapping[str, object], key: str) -> object:
    if key not in params:
        raise PluginError("invalid_params", f"{key} is required")
    return params[key]


def _string_keyed_object(value: object) -> dict[str, object] | None:
    if not isinstance(value, dict):
        return None
    raw = cast(dict[object, object], value)
    converted: dict[str, object] = {}
    for key, item in raw.items():
        if not isinstance(key, str):
            return None
        converted[key] = item
    return converted


if __name__ == "__main__":  # pragma: no cover - exercised through subprocess tests
    raise SystemExit(main())


__all__ = [
    "build_parser",
    "diff_snapshots",
    "handle_request",
    "main",
    "snapshot_workspace",
]
