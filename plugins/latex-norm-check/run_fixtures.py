#!/usr/bin/env python3
"""Run domain expectations and agenstro.plugin/v1 framing invariants."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
from typing import Any

HERE = pathlib.Path(__file__).parent
PLUGIN = HERE / "latex_norm_check.py"
FIXTURES = sorted((HERE / "fixtures").glob("*.json"))


def run_plugin(raw_request: str) -> tuple[list[dict[str, Any]], int, str]:
    completed = subprocess.run(
        [sys.executable, str(PLUGIN)],
        input=raw_request.rstrip("\n") + "\n",
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=30,
        check=False,
    )
    frames: list[dict[str, Any]] = []
    for line in completed.stdout.splitlines():
        if not line.strip():
            raise AssertionError("blank line on stdout is not a valid JSONL frame")
        frames.append(json.loads(line))
    return frames, completed.returncode, completed.stderr


def check_protocol(frames: list[dict[str, Any]], request_id: str | int) -> None:
    terminals = [index for index, frame in enumerate(frames) if frame.get("type") == "result"]
    assert len(terminals) == 1, f"expected exactly one terminal frame, got {len(terminals)}"
    assert terminals[0] == len(frames) - 1, "a frame followed the terminal result"
    for frame in frames:
        assert frame.get("id") == request_id, f"correlation id mismatch in {frame}"
        assert frame.get("type") in ("event", "result"), f"unknown frame type in {frame}"
    terminal = frames[-1]
    if terminal.get("ok"):
        assert "value" in terminal and "error" not in terminal
    else:
        assert "error" in terminal and "value" not in terminal


def expand_request_fields(fixture: dict[str, Any], request: dict[str, Any]) -> None:
    """Expand large boundary inputs without storing them verbatim in fixtures."""
    for expansion in fixture.get("requestRepeats", []):
        path = expansion["path"]
        if not isinstance(path, list) or not path:
            raise ValueError("requestRepeats.path must be a non-empty array")
        cursor: Any = request
        for segment in path[:-1]:
            cursor = cursor[segment]
        cursor[path[-1]] = expansion["text"] * expansion["count"]


def main() -> int:
    if not FIXTURES:
        print("no fixtures found", file=sys.stderr)
        return 1
    failures = 0
    for path in FIXTURES:
        fixture = json.loads(path.read_text(encoding="utf-8"))
        name, expect = fixture["name"], fixture["expect"]
        if "rawRequest" in fixture:
            raw_request = fixture["rawRequest"]
            request_id = fixture.get("expectedId", "unknown")
        else:
            request = fixture["request"]
            expand_request_fields(fixture, request)
            raw_request = json.dumps(request, ensure_ascii=False, allow_nan=False)
            request_id = request["id"]
        try:
            frames, code, _stderr = run_plugin(raw_request)
            check_protocol(frames, request_id)
            terminal = frames[-1]
            assert terminal["ok"] == expect["ok"], f"ok mismatch: {terminal}"
            if terminal["ok"]:
                value = terminal["value"]
                if "violationNorms" in expect:
                    actual = sorted(item["norm"] for item in value["violations"])
                    assert actual == sorted(expect["violationNorms"]), f"violations {actual}"
                for field in ("checked", "unchecked"):
                    if field in expect:
                        assert sorted(value[field]) == sorted(expect[field]), f"{field} {value[field]}"
                if "firstLocus" in expect:
                    locus = value["violations"][0]["locus"]
                    for key, wanted in expect["firstLocus"].items():
                        assert locus.get(key) == wanted, f"locus.{key} = {locus.get(key)!r}"
            else:
                assert terminal["error"]["code"] == expect["code"], f"code mismatch: {terminal}"
            if "exitCode" in expect:
                assert code == expect["exitCode"], f"exit {code} != {expect['exitCode']}"
            print(f"PASS  {name}")
        except (AssertionError, KeyError, ValueError, json.JSONDecodeError) as error:
            failures += 1
            print(f"FAIL  {name}: {error}")
    print(f"\n{len(FIXTURES) - failures}/{len(FIXTURES)} fixtures passed")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
