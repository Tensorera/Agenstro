#!/usr/bin/env python3
"""Offline behavior checks for installed Motivo templates and explicit calls.

No model or sandbox workload is started: the provider/effect are protocol
fixtures. The motivo-test crate separately checks OS sandbox guarantees.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
METHODS = ["clarify", "investigate", "analyze", "research", "probe", "organize", "retrospect", "handoff"]


def execute(argv: list[str], *, cwd: Path, success: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(argv, cwd=cwd, text=True, encoding="utf-8", stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
    if (completed.returncode == 0) != success:
        raise AssertionError(f"Unexpected exit {completed.returncode}: {argv}\n{completed.stdout[-12000:]}")
    return completed


def digest_tree(path: Path) -> str:
    digest = hashlib.sha256()
    for file in sorted(path.rglob("*")):
        if file.is_file():
            digest.update(str(file.relative_to(path)).encode())
            digest.update(file.read_bytes())
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tactus", default=os.environ.get("TACTUS_BIN", str(ROOT / "Build/cargo/debug/tactus")))
    parser.add_argument("--installed-only", action="store_true", help="Use only init-embedded assets, without refreshing from source")
    args = parser.parse_args()
    tactus = str(Path(args.tactus).resolve())
    with tempfile.TemporaryDirectory(prefix="motivo-methods-") as temporary:
        project = Path(temporary) / "project"
        execute([tactus, "init", str(project), "--sdk", str(ROOT / "clef-sdk")], cwd=ROOT)
        if not args.installed_only:
            shutil.copytree(ROOT / "motivo/templates", project / ".tactus/motivoscript", dirs_exist_ok=True)
            asset = project / ".tactus/skills/motivo/assets/report-template.html"
            asset.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / "motivo-studio/report-template.html", asset)
        fixture = Path(temporary) / "plugin.py"
        fixture.write_text('''import json, pathlib, sys
request=json.loads(sys.stdin.buffer.readline().decode("utf-8"))
params=request["params"]
marker=pathlib.Path(__file__).with_name("calls.jsonl")
with marker.open("a", encoding="utf-8") as output:
    output.write(json.dumps({"mode":sys.argv[1], "params":params})+"\\n")
if sys.argv[1]=="forbidden":
    raise SystemExit("The implicit default provider was invoked")
if sys.argv[1]=="provider":
    value={"text":"Independent evidence was returned without mandatory headings. <script>motivo_attack()</script>"}
else:
    assert request["method"]=="run"
    assert params["run_id"]=="probe-case"
    assert params["sample_id"]=="sample-001"
    assert params["argv"]==["fixture-command", "argument with spaces"]
    assert params["timeout_seconds"]==7
    value={"status":"exited", "exit_code":3, "signal":None, "duration_ms":4,
           "timeout_seconds":7, "workspace_dir":".tactus/motivotest/probe-case/sample-001",
           "stdout_path":"fixture-output.txt", "stderr_path":"fixture-error.txt",
           "stdout_truncated":False, "stderr_truncated":False, "logs_may_be_partial":False}
print(json.dumps({"type":"result", "id":request["id"], "ok":True, "value":value}))
''', encoding="utf-8")
        python = json.dumps(sys.executable)
        fixture_literal = json.dumps(str(fixture))
        (project / ".tactus/tactus.toml").write_text(f'''api = "clef.runtime/v1"
default_provider = "forbidden-default"
instructions = ".tactus/PROMPT.md"
[providers.forbidden-default]
command = [{python}, {fixture_literal}, "forbidden"]
[providers.independent]
command = [{python}, {fixture_literal}, "provider"]
[effects."motivo.test"]
command = [{python}, {fixture_literal}, "effect"]
''', encoding="utf-8")
        note = project / "notes.md"
        note.write_text("A concrete observation without fixed headings. 中文证据 — “quoted evidence”.\n<script>motivo_attack()</script>\n", encoding="utf-8")
        marker = fixture.with_name("calls.jsonl")
        runs = project / ".tactus/motivo/runs"

        def run_method(method: str, run_id: str, extra: list[str] | None = None, *, success: bool = True):
            number = (METHODS.index(method) + 1) * 10
            command = [tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                       "--script", f".tactus/motivoscript/{number:03}_{method}.hs", "--",
                       "--input", "notes.md", "--run-id", run_id]
            return execute(command + (extra or []), cwd=project, success=success)

        # Compile all real installed entrypoints/helper modules; no additional packages.
        execute([tactus, "check", "--root", str(project), "--scripts-dir", ".tactus/motivoscript", "--all"], cwd=project)
        for method in METHODS:
            if method == "probe":
                continue
            run_id = f"record-{method}"
            run_method(method, run_id)
            directory = runs / run_id
            record = json.loads((directory / "run.json").read_text(encoding="utf-8"))
            assert record["method"] == method and record["status"] == "recorded"
            assert record["requested_provider"] is None
            assert record["model"] == "not reported"
            assert record["missing_sections"]
            assert (directory / "request.md").read_text(encoding="utf-8") == note.read_text(encoding="utf-8")
            assert (directory / "artifacts/submitted-notes.md").read_text(encoding="utf-8") == note.read_text(encoding="utf-8")
            html = (directory / "report.html").read_text(encoding="utf-8")
            assert "中文证据 — “quoted evidence”" in html
            assert "&lt;script&gt;motivo_attack()&lt;/script&gt;" in html
            assert "<script>motivo_attack()</script>" not in html
            events = [json.loads(line) for line in (directory / "samples.jsonl").read_text(encoding="utf-8").splitlines()]
            assert [event["seq"] for event in events] == list(range(1, len(events) + 1))
        assert not marker.exists(), "record-only methods must never call a provider/effect"
        print("PASS: all seven recording methods compile, accept ordinary prose, escape HTML and invoke no model")

        review = runs / "record-retrospect/artifacts"
        assert (review / "retrospective.md").read_text(encoding="utf-8") == note.read_text(encoding="utf-8")
        history = json.loads((review / "decision-history.json").read_text(encoding="utf-8"))
        assert history["temporal_order_known"] is False
        assert all(row["decided_at"] is None for row in history["entries"])
        assert "Unknown" in (review / "handoff.md").read_text(encoding="utf-8")
        print("PASS: retrospective publishes review/handoff/history without inventing missing facts or dates")

        before = digest_tree(runs / "record-investigate")
        run_method("investigate", "record-investigate", success=False)
        assert digest_tree(runs / "record-investigate") == before
        run_method("clarify", "../outside", success=False)
        assert not (project / ".tactus/motivo/outside").exists()
        print("PASS: existing runs are preserved and traversal identities are rejected")

        run_method("investigate", "explicit-provider", ["--provider", "independent", "--model", "requested-fixture-model"])
        calls = [json.loads(line) for line in marker.read_text(encoding="utf-8").splitlines()]
        assert len(calls) == 1 and calls[0]["mode"] == "provider"
        assert calls[0]["params"]["model"] == "requested-fixture-model"
        assert "Motivo method: investigate" in calls[0]["params"]["prompt"]
        assert "中文证据 — “quoted evidence”" in calls[0]["params"]["prompt"]
        returned = json.loads((runs / "explicit-provider/run.json").read_text(encoding="utf-8"))
        assert returned["status"] == "responded"
        assert returned["requested_provider"] == "independent"
        assert returned["actual_provider_model"] is None
        assert (runs / "explicit-provider/artifacts/provider-response.md").is_file()
        assert "artifacts/provider-response.md" in returned["artifacts"]
        assert "Independent evidence" in (runs / "explicit-provider/report.html").read_text(encoding="utf-8")
        print("PASS: independent investigation calls only the explicit provider once and distinguishes requested/unknown actual model")

        run_method("investigate", "unavailable-provider", ["--provider", "missing-provider"], success=False)
        unavailable = runs / "unavailable-provider"
        unavailable_record = json.loads((unavailable / "run.json").read_text(encoding="utf-8"))
        assert unavailable_record["status"] == "needs-check"
        assert "artifacts/provider-error.json" in unavailable_record["artifacts"]
        assert (unavailable / "artifacts/provider-error.json").is_file()
        assert (unavailable / "artifacts/submitted-notes.md").read_text(encoding="utf-8") == note.read_text(encoding="utf-8")
        assert len(marker.read_text(encoding="utf-8").splitlines()) == 1
        print("PASS: an unavailable explicit provider preserves the attempt and never falls back or retries")

        run_method("probe", "probe-case", ["--timeout-seconds", "7", "--", "fixture-command", "argument with spaces"])
        probe = json.loads((runs / "probe-case/run.json").read_text(encoding="utf-8"))
        assert probe["status"] == "experiment-failed"
        assert probe["experiment"]["exit_code"] == 3
        assert "artifacts/experiment.json" in probe["artifacts"]
        calls = [json.loads(line) for line in marker.read_text(encoding="utf-8").splitlines()]
        assert [call["mode"] for call in calls] == ["provider", "effect"]
        for timeout in ["0", "601"]:
            run_method("probe", f"invalid-timeout-{timeout}", ["--timeout-seconds", timeout, "--", "fixture-command"], success=False)
        run_method("probe", "invalid-provider", ["--provider", "independent", "--", "fixture-command"], success=False)
        assert len(marker.read_text(encoding="utf-8").splitlines()) == 2
        print("PASS: probe uses perform, preserves negative observations and rejects timeout/provider bypasses before dispatch")

        (project / "notes.md").write_text("", encoding="utf-8")
        run_method("analyze", "empty-input", success=False)
        (project / "notes.md").write_text("## Evidence\none\n## Evidence\ntwo\n", encoding="utf-8")
        run_method("analyze", "ambiguous-input", success=False)
        (project / "notes.md").write_text("x" * (512 * 1024 + 1), encoding="utf-8")
        run_method("analyze", "oversized-input", success=False)
        index = (project / ".tactus/motivo/index.html").read_text(encoding="utf-8")
        assert "runs/probe-case/report.html" in index
        assert "runs/explicit-provider/report.html" in index
        print("PASS: invalid input does not create runs; generated offline index links actual records")

        # Custom IDs have no temporal meaning. More than 2,000 lexically later
        # old directories must not hide the fresh aaa-current method run.
        for number in range(2000):
            old_id = f"zz-old-{number:04}"
            old_directory = runs / old_id
            old_directory.mkdir()
            timestamp = {
                0: "2099-01-01T00:00:00.1Z",
                1: "2099-01-01T00:00:00.11Z",
                2: "2099-01-01T00:00:00.11Z",
                3: "invalid-time",
            }.get(number, "2000-01-01T00:00:00Z")
            (old_directory / "run.json").write_text(json.dumps({
                "run_id": old_id, "method": "investigate", "title": "Old observation",
                "started_at": timestamp, "status": "recorded",
                "agent": "fixture", "model": "not reported",
            }), encoding="utf-8")
        note.write_text("""## Question
Does the report preserve wrapped reflection answers?
## Local reflection
- First answer keeps its context.
  Wrapped continuation belongs to that first answer.
- Second answer remains the second observation.
- Third answer must remain visible.
""", encoding="utf-8")
        run_method("investigate", "aaa-current")
        current_html = (runs / "aaa-current/report.html").read_text(encoding="utf-8")
        reflection = current_html.split('<section class="reflection">', 1)[1].split("</section>", 1)[0]
        markers = ["First answer keeps its context.", "Wrapped continuation belongs to that first answer.",
                   "Second answer remains the second observation.", "Third answer must remain visible."]
        positions = [reflection.index(marker) for marker in markers]
        assert positions == sorted(positions)
        assert all(position < reflection.index('class="reflection-prompts"') for position in positions)
        current_index = (project / ".tactus/motivo/index.html").read_text(encoding="utf-8")
        assert 'runs/aaa-current/report.html' in current_index
        assert current_index.index('runs/zz-old-0001/report.html') < current_index.index('runs/zz-old-0002/report.html') < current_index.index('runs/zz-old-0000/report.html')
        assert 'runs/zz-old-0003/report.html' not in current_index
        print("PASS: multiline reflections retain every answer and current custom IDs remain visible beyond 2,000 older names")
        print("PASS: index sorts actual fractional times, uses stable ID ties and places invalid timestamps last")
    print("Motivo template checks passed (no live model calls).")


if __name__ == "__main__":
    main()
