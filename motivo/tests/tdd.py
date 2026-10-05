#!/usr/bin/env python3
"""Offline Motivo.Test acceptance: real entrypoints, fake facts, real Linux red/green.

Providers are protocol fixtures. No live model calls or dependency installation.
All source edits occur in TemporaryDirectory, including the real sandbox example.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def execute(argv: list[str], cwd: Path, expected: int = 0) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(argv, cwd=cwd, text=True, encoding="utf-8", stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=180)
    if result.returncode != expected:
        raise AssertionError(f"Expected {expected}, got {result.returncode}: {argv}\n{result.stdout[-14000:]}")
    return result


def digest(path: Path) -> str:
    value = hashlib.sha256()
    for file in sorted(path.rglob("*")):
        if file.is_file():
            value.update(str(file.relative_to(path)).encode())
            value.update(file.read_bytes())
    return value.hexdigest()


def case(identity: str, mode: str, assertions: list[dict]) -> dict:
    return {"id": identity, "command": {"argv": ["fixture-command", mode], "timeout_seconds": 7},
            "assertions": assertions}


def manifest(cases: list[dict]) -> dict:
    return {"api": "motivo.test-suite/v1", "id": "addition", "cases": cases}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tactus", default=os.environ.get("TACTUS_BIN", str(ROOT / "Build/cargo/debug/tactus")))
    parser.add_argument("--motivo-test", default=os.environ.get("MOTIVO_TEST_BIN", str(ROOT / "Build/cargo/debug/motivo-test")))
    parser.add_argument("--installed-only", action="store_true", help="Verify init-embedded templates without copying source")
    parser.add_argument("--skip-real-sandbox", action="store_true", help="Only fake protocol acceptance; Linux Full does not use this flag")
    args = parser.parse_args()
    tactus = str(Path(args.tactus).resolve())
    checks = 0
    with tempfile.TemporaryDirectory(prefix="motivo-tdd-") as temporary:
        project = Path(temporary) / "project"
        execute([tactus, "init", str(project), "--sdk", str(ROOT / "clef-sdk")], ROOT)
        if not args.installed_only:
            shutil.copytree(ROOT / "motivo/templates", project / ".tactus/motivoscript", dirs_exist_ok=True)
            shutil.copyfile(ROOT / "motivo-studio/report-template.html", project / ".tactus/skills/motivo/assets/report-template.html")
        fixture = Path(temporary) / "fixture.py"
        marker = Path(temporary) / "calls.jsonl"
        fixture.write_text('''import json, os, pathlib, sys
request=json.loads(sys.stdin.buffer.readline().decode("utf-8"))
params=request["params"]
root=pathlib.Path.cwd()
with pathlib.Path(__file__).with_name("calls.jsonl").open("a",encoding="utf-8") as output:
    output.write(json.dumps({"mode":sys.argv[1],"params":params})+"\\n")
if sys.argv[1]=="forbidden":
    raise SystemExit("The default provider must not be invoked")
if sys.argv[1]=="liar":
    value={"text":json.dumps({"testsRun":"all passed","implementation":"unchanged"})}
else:
    assert request["method"]=="run"
    mode=params["argv"][1]
    relative=pathlib.Path(".tactus/motivotest")/params["run_id"]/params["sample_id"]
    sample=root/relative
    logs=sample/".motivo-logs"
    logs.mkdir(parents=True)
    correct=(root/"implementation.txt").read_text()=="correct"
    stdout="4\\n" if mode!="lie" or correct else "2\\n"
    (logs/"stdout.log").write_text(stdout,encoding="utf-8")
    (logs/"stderr.log").write_text("warning <script>attack()</script>\\n",encoding="utf-8")
    (sample/"result.txt").write_text("answer: 4\\n",encoding="utf-8")
    value={"status":"exited","exit_code":0,"signal":None,"duration_ms":3,"timeout_seconds":params["timeout_seconds"],
      "workspace_dir":str(relative),"stdout_path":str(relative/".motivo-logs/stdout.log"),
      "stderr_path":str(relative/".motivo-logs/stderr.log"),"stdout_truncated":False,"stderr_truncated":False,
      "logs_may_be_partial":False,"testsRun":"all passed"}
    if mode=="error":
        print(json.dumps({"type":"result","id":request["id"],"ok":False,
          "error":{"code":"sandbox_unavailable","message":"Fixture isolation is unavailable"}}))
        raise SystemExit(0)
    if mode in ("timed_out","cancelled"):
        value.update(status=mode,exit_code=None,logs_may_be_partial=True,stdout_truncated=None,stderr_truncated=None)
    if mode=="signal": value.update(exit_code=None,signal=9)
    if mode=="truncated": value["stdout_truncated"]=True
    if mode=="partial": value["logs_may_be_partial"]=True
    if mode=="bad-path": value["stdout_path"]="/etc/passwd"
    if mode=="bad-timeout": value["timeout_seconds"]=8
    if mode=="missing-log": (logs/"stdout.log").unlink()
    if mode=="binary-log": (logs/"stdout.log").write_bytes(b"\\xff")
    if mode=="oversized-log": (logs/"stdout.log").write_bytes(b"x"*(1024*1024+1))
    if mode=="symlink-log":
        (logs/"stdout.log").unlink(); (logs/"stdout.log").symlink_to(root/"implementation.txt")
    if mode=="hardlink-log":
        (logs/"stdout.log").unlink(); os.link(root/"implementation.txt",logs/"stdout.log")
    if mode=="fifo-log":
        (logs/"stdout.log").unlink(); os.mkfifo(logs/"stdout.log")
    if mode=="malformed": value={"testsRun":"all passed"}
print(json.dumps({"type":"result","id":request["id"],"ok":True,"value":value}))
''', encoding="utf-8")
        python = json.dumps(sys.executable)
        plugin_path = json.dumps(str(fixture))
        config = f'''api = "clef.runtime/v1"
default_provider = "forbidden"
instructions = ".tactus/PROMPT.md"
[providers.forbidden]
command = [{python}, {plugin_path}, "forbidden"]
[providers.liar]
command = [{python}, {plugin_path}, "liar"]
[effects."motivo.test"]
command = [{python}, {plugin_path}, "effect"]
'''
        (project / ".tactus/tactus.toml").write_text(config, encoding="utf-8")
        (project / "implementation.txt").write_text("wrong", encoding="utf-8")
        suite_file = project / "suite.json"
        runs = project / ".tactus/motivo/runs"

        def write_suite(value: dict) -> None:
            suite_file.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")

        def run(identity: str, value: dict, *, phase: str = "normal", baseline: str | None = None,
                expected: int = 0, script: str = "090_test.hs") -> dict:
            write_suite(value)
            command = [tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                       "--script", f".tactus/motivoscript/{script}", "--", "--run-id", identity,
                       "--phase", phase]
            if script == "090_test.hs":
                command += ["--suite", "suite.json"]
            if baseline:
                command += ["--baseline", baseline]
            execute(command, project, expected)
            return json.loads((runs / identity / "run.json").read_text(encoding="utf-8"))

        def calls() -> list[dict]:
            return [json.loads(line) for line in marker.read_text(encoding="utf-8").splitlines()] if marker.exists() else []

        execute([tactus, "check", "--root", str(project), "--scripts-dir", ".tactus/motivoscript", "--all"], project)
        if sys.platform != "linux":
            print("PASS: all installed Motivo entrypoints, including Test, compile")
            print("SKIP: command/evidence acceptance requires the Linux motivo.test backend; no host fallback")
            return
        oracle = manifest([case("addition", "lie", [{"kind": "exit_code", "expected": 0},
                                                     {"kind": "stdout_equals", "expected": "4\n"}])])
        red = run("red", oracle, phase="red", expected=1)
        assert red["status"] == "test-failed" and red["test_suite"]["red_observed"]
        assert red["test_suite"]["counts"] == {"total": 1, "passed": 0, "failed": 1, "errors": 0}
        evidence = json.loads((runs / "red/artifacts/test-suite.json").read_text(encoding="utf-8"))
        assert evidence["cases"][0]["outcome"]["testsRun"] == "all passed"
        assert evidence["cases"][0]["assertions"][1]["status"] == "failed"
        checks += 1
        print("PASS: provider-like testsRun claim cannot establish red/green; independent stdout assertion observes red")

        before = digest(runs / "red")
        (project / "implementation.txt").write_text("correct", encoding="utf-8")
        green = run("green", oracle, phase="green", baseline="red")
        assert green["status"] == "test-passed" and green["test_suite"]["oracle_matched"]
        assert digest(runs / "red") == before
        assert green["test_suite"]["manifest"] == red["test_suite"]["manifest"]
        checks += 1
        print("PASS: green reuses the frozen red oracle and preserves all baseline evidence")

        for field in ["expected", "argv", "timeout", "case-id", "suite-id", "assertion-order", "removed-assertion"]:
            changed = copy.deepcopy(oracle)
            if field == "expected": changed["cases"][0]["assertions"][1]["expected"] = "2\n"
            if field == "argv": changed["cases"][0]["command"]["argv"].append("extra")
            if field == "timeout": changed["cases"][0]["command"]["timeout_seconds"] = 8
            if field == "case-id": changed["cases"][0]["id"] = "renamed"
            if field == "suite-id": changed["id"] = "renamed"
            if field == "assertion-order": changed["cases"][0]["assertions"].reverse()
            if field == "removed-assertion": changed["cases"][0]["assertions"].pop()
            count = len(calls())
            blocked = run("drift-" + field, changed, phase="green", baseline="red", expected=2)
            assert blocked["status"] == "needs-check" and not blocked["test_suite"]["oracle_matched"]
            assert "Oracle changed" in blocked["test_suite"]["suite_error"] and len(calls()) == count
            checks += 1
        print("PASS: changed/removed/reordered assertions, argv, timeout or suite/case identity are blocked before any effect call")

        count = len(calls())
        blocked = run("invalid-baseline", oracle, phase="green", baseline="green", expected=2)
        assert blocked["status"] == "needs-check" and len(calls()) == count
        missing = run("missing-baseline", oracle, phase="green", baseline="absent", expected=2)
        assert missing["status"] == "needs-check" and len(calls()) == count
        not_red = run("not-red", oracle, phase="red", expected=2)
        assert not not_red["test_suite"]["red_observed"] and not_red["test_suite"]["suite_error"]
        checks += 3
        print("PASS: missing/non-red baselines and unexpectedly passing red phases remain errors")

        duplicate = runs / "duplicate-baseline"
        duplicate.mkdir()
        duplicated = copy.deepcopy(red)
        duplicated["run_id"] = "duplicate-baseline"
        content = json.dumps(duplicated)
        (duplicate / "run.json").write_text('{"api":"motivo.run/v1",' + content[1:], encoding="utf-8")
        count = len(calls())
        invalid = run("duplicate-baseline-blocked", oracle, phase="green", baseline="duplicate-baseline", expected=2)
        assert invalid["test_suite"]["oracle_matched"] is False and len(calls()) == count
        checks += 1

        config_error = project / ".tactus/motivoscript/120_configerror.hs"
        config_error.write_text('''{-# LANGUAGE OverloadedStrings #-}
import Motivo.Test
import System.Environment (setEnv)
main :: IO ()
main = do
  setEnv "TACTUS_RUNTIME_CONFIG" "invalid-runtime.json"
  runTestSuite (suite "addition" [testCase "addition" (withTimeout 7 (command ["fixture-command", "lie"])) [assertExit 0, assertStdout "4\\n"]])
''', encoding="utf-8")
        (project / "invalid-runtime.json").write_text("{}", encoding="utf-8")
        failed_after_match = run("matched-config-error", oracle, phase="green", baseline="red", expected=2, script="120_configerror.hs")
        assert failed_after_match["test_suite"]["oracle_matched"] is True
        assert failed_after_match["test_suite"]["suite_error"] and len(calls()) == count
        checks += 1
        print("PASS: duplicate-key baseline is refused; matched oracle remains known when later runtime configuration fails")

        assertions = [{"kind": "exit_code", "expected": 0}, {"kind": "stdout_equals", "expected": "4\n"},
                      {"kind": "stdout_contains", "expected": "4"},
                      {"kind": "stderr_equals", "expected": "warning <script>attack()</script>\n"},
                      {"kind": "stderr_contains", "expected": "warning"},
                      {"kind": "file_exists", "path": "result.txt"},
                      {"kind": "file_equals", "path": "result.txt", "expected": "answer: 4\n"},
                      {"kind": "file_contains", "path": "result.txt", "expected": "4"}]
        all_assertions = run("all-assertions", manifest([case("all", "pass", assertions)]))
        assert all_assertions["status"] == "test-passed"
        html = (runs / "all-assertions/report.html").read_text(encoding="utf-8")
        assert "<script>attack()</script>" not in html and "&lt;script&gt;attack()&lt;/script&gt;" in html
        events = [json.loads(line) for line in (runs / "all-assertions/samples.jsonl").read_text(encoding="utf-8").splitlines()]
        assert [item["seq"] for item in events] == list(range(1, len(events) + 1))
        assert len([item for item in events if item["kind"] == "assertion.checked"]) == 8
        assert "all-assertions/report.html" in (project / ".tactus/motivo/index.html").read_text(encoding="utf-8")
        checks += 1
        print("PASS: all eight assertion kinds execute, escape HTML, publish ordered evidence and join the method index")

        modes = ["error", "timed_out", "cancelled", "signal", "truncated", "partial", "bad-path", "bad-timeout",
                 "missing-log", "binary-log", "oversized-log", "malformed"]
        if sys.platform == "linux":
            modes += ["symlink-log", "hardlink-log", "fifo-log"]
        for mode in modes:
            count = len(calls())
            record = run("unknown-" + mode, manifest([case("unknown", mode, [{"kind": "exit_code", "expected": 1}])]), phase="red", expected=2)
            assert record["status"] == "needs-check" and not record["test_suite"]["red_observed"]
            assert record["test_suite"]["counts"]["errors"] == 1 and len(calls()) == count + 1
            checks += 1
        print(f"PASS: {len(modes)} unavailable/incomplete/unsafe evidence outcomes are errors, never red and never retried")

        mixed = run("mixed", manifest([case("failure", "pass", [{"kind": "exit_code", "expected": 3}]),
                                        case("success", "pass", [{"kind": "exit_code", "expected": 0}])]), expected=1)
        assert mixed["test_suite"]["counts"] == {"total": 2, "passed": 1, "failed": 1, "errors": 0}
        checks += 1
        print("PASS: ordinary assertion failure preserves other case results without retries")

        invalids = [manifest([]), manifest([case("same", "pass", []), case("same", "pass", assertions)]),
                    manifest([case("escape", "pass", [{"kind": "file_exists", "path": "../implementation.txt"}])]),
                    manifest([case("vacuous", "pass", [{"kind": "stdout_contains", "expected": ""}])])]
        invalids.append({**oracle, "silently_ignored": True})
        for number, invalid in enumerate(invalids):
            count = len(calls())
            write_suite(invalid)
            execute([tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                     "--script", ".tactus/motivoscript/090_test.hs", "--", "--suite", "suite.json",
                     "--run-id", f"invalid-{number}"], project, 2)
            assert len(calls()) == count and not (runs / f"invalid-{number}").exists()
            checks += 1
        print("PASS: empty/ambiguous/vacuous/traversing and unknown-field suites are rejected before execution")

        count = len(calls())
        suite_file.write_text('{"api":"motivo.test-suite/v1","id":"addition","id":"lowered","cases":[]}', encoding="utf-8")
        execute([tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                 "--script", ".tactus/motivoscript/090_test.hs", "--", "--suite", "suite.json",
                 "--run-id", "duplicate-key"], project, 2)
        assert len(calls()) == count and not (runs / "duplicate-key").exists()
        checks += 1
        print("PASS: duplicate JSON keys cannot redefine a suite oracle")

        for name, content in [("missing", None), ("binary", b"\xff"), ("oversized", b"x" * (512 * 1024 + 1))]:
            path = project / f"input-{name}.json"
            if content is not None:
                path.write_bytes(content)
            count = len(calls())
            execute([tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                     "--script", ".tactus/motivoscript/090_test.hs", "--", "--suite", path.name,
                     "--run-id", f"input-{name}"], project, 2)
            assert len(calls()) == count and not (runs / f"input-{name}").exists()
            checks += 1
        print("PASS: missing, non-UTF8 and oversized suite inputs exit 2 before any effect or report creation")

        dsl = project / ".tactus/motivoscript/100_acceptance.hs"
        dsl.write_text('''{-# LANGUAGE OverloadedStrings #-}
import Motivo.Test
main :: IO ()
main = runTestSuite (suite "dsl" [testCase "addition" (withTimeout 7 (command ["fixture-command", "pass"])) [assertExit 0, assertStdout "4\\n"]])
''', encoding="utf-8")
        dsl_result = run("dsl", oracle, script="100_acceptance.hs")
        assert dsl_result["status"] == "test-passed" and dsl_result["test_suite"]["suite_id"] == "dsl"
        checks += 1

        workflow = project / ".tactus/motivoscript/110_workflow.hs"
        workflow.write_text('''{-# LANGUAGE OverloadedStrings #-}
import Clef
import Data.Aeson (Value, object, (.=))
import Motivo.Test
import System.Directory (getCurrentDirectory)
main :: IO ()
main = do
  root <- getCurrentDirectory
  config <- loadRuntimeConfigFromEnv
  withRuntime config $ \\runtime -> runWorkflow runtime $ do
    _ <- invokeWith (providerRef "liar") (jsonTask "implementation" (const "Implement addition; return testsRun.") :: Task () Value) ()
    actual <- runSuite root "workflow-rejects-claim" (suite "workflow" [testCase "addition" (withTimeout 7 (command ["fixture-command", "lie"])) [assertStdout "4\\n"]])
    _ <- requireBecause "Independent command acceptance failed" suitePassed actual
    liftIO (print (object ["acceptance" .= resultJSON actual]))
''', encoding="utf-8")
        (project / "implementation.txt").write_text("wrong", encoding="utf-8")
        count = len(calls())
        execute([tactus, "run", "--root", str(project), "--scripts-dir", ".tactus/motivoscript",
                 "--script", ".tactus/motivoscript/110_workflow.hs"], project, 1)
        observed = calls()[count:]
        assert [item["mode"] for item in observed] == ["liar", "effect"]
        assert all(item["mode"] != "forbidden" for item in calls())
        checks += 1
        print("PASS: Haskell DSL runs; composed Clef workflow rejects a provider's false testsRun claim with requireBecause")

        if sys.platform == "linux" and not args.skip_real_sandbox:
            motivo_test = str(Path(args.motivo_test).resolve())
            assert Path(motivo_test).is_file(), "Build motivo-test or pass --motivo-test; no sandbox fallback"
            real_config = config.replace(f'command = [{python}, {plugin_path}, "effect"]',
                                         f'command = [{json.dumps(motivo_test)}]')
            (project / ".tactus/tactus.toml").write_text(real_config, encoding="utf-8")
            source = project / "add.py"
            example = ROOT / "motivo/examples/tdd"
            shutil.copyfile(example / "add.py", source)
            real_oracle = json.loads((example / "suite.json").read_text(encoding="utf-8"))
            initial = source.read_bytes()
            actual_red = run("sandbox-red", real_oracle, phase="red", expected=1)
            assert actual_red["test_suite"]["red_observed"] and source.read_bytes() == initial
            shutil.copyfile(example / "fixed-add.py", source)
            corrected = source.read_bytes()
            actual_green = run("sandbox-green", real_oracle, phase="green", baseline="sandbox-red")
            assert actual_green["status"] == "test-passed" and source.read_bytes() == corrected
            protected = run("sandbox-read-only", manifest([{"id": "write", "command": {"argv": ["/bin/sh", "-c", "printf changed > /project/add.py"], "timeout_seconds": 15},
                                                           "assertions": [{"kind": "exit_code", "expected": 0}]}]), expected=1)
            assert protected["status"] == "test-failed" and source.read_bytes() == corrected
            checks += 3
            print("PASS: real Bubblewrap red → edit only temporary implementation → matching green; sandbox cannot write the project")
        else:
            print("SKIP: real sandbox acceptance (non-Linux or explicit --skip-real-sandbox); no host fallback")

        # Recreate a workspace from before the Test/Record entries existed.
        # The stub exposes precisely the old public Run API, without relying on
        # git history, a checkout, or an export added by the new implementation.
        helpers = project / ".tactus/motivoscript/Motivo"
        (helpers / "Run.hs").write_text('''{-# LANGUAGE OverloadedStrings #-}
module Motivo.Run (runMethod, parseSections, validateNotes) where
import Data.Text (Text)
import Motivo.Method (Method)
runMethod :: Method -> IO ()
runMethod _ = pure ()
parseSections :: Text -> [(Text, Text)]
parseSections _ = []
validateNotes :: Method -> Text -> Either String ()
validateNotes _ _ = Right ()
''', encoding="utf-8")
        old_modules = {name: (helpers / name).read_bytes() for name in ["Run.hs", "Method.hs", "Html.hs", "Markdown.hs"]}
        for path in [helpers / "Test.hs", helpers / "Record.hs", project / ".tactus/motivoscript/090_test.hs"]:
            path.unlink()
        (project / ".tactus/tactus.toml").write_text(config, encoding="utf-8")
        execute([tactus, "init", str(project), "--sdk", str(ROOT / "clef-sdk")], ROOT)
        assert all((helpers / name).read_bytes() == content for name, content in old_modules.items())
        execute([tactus, "check", "--root", str(project), "--scripts-dir", ".tactus/motivoscript", "--all"], project)
        upgraded = run("upgraded-old-workspace", manifest([case("addition", "pass", [
            {"kind": "exit_code", "expected": 0}, {"kind": "stdout_equals", "expected": "4\n"}])]))
        assert upgraded["status"] == "test-passed"
        assert all((helpers / name).read_bytes() == content for name, content in old_modules.items())
        checks += 1
        print("PASS: init adds Test/Record to a preserved old Run API; installed TDD compiles/runs without changing old module bytes")
    print(f"Motivo TDD acceptance passed: {checks} checks")


if __name__ == "__main__":
    main()
