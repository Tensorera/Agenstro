"""Model-free acceptance of the installed Segno -> Tactus -> Haskell path."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tactus", required=True, type=Path)
    parser.add_argument("--segno", required=True, type=Path)
    args = parser.parse_args()
    tactus = args.tactus.resolve(strict=True)
    segno = args.segno.resolve(strict=True)
    repository = Path(__file__).resolve().parents[2]
    build = repository / "Build"
    build.mkdir(exist_ok=True)
    environment = os.environ.copy()
    environment["PATH"] = os.pathsep.join(
        [str(tactus.parent), str(segno.parent), environment.get("PATH", "")]
    )
    # Do not inherit a caller's live task context or default machine registry.
    for key in list(environment):
        if key.startswith(("SEGNO_", "TACTUS_")):
            environment.pop(key)

    def run(command):
        result = subprocess.run(
            [str(part) for part in command], env=environment, cwd=repository,
            capture_output=True, text=True, encoding="utf-8", timeout=600,
        )
        if result.returncode:
            raise AssertionError(
                f"Command failed: {command}\n{result.stdout}\n{result.stderr}"
            )
        return result.stdout

    with tempfile.TemporaryDirectory(prefix="segno-service-acceptance-", dir=build) as tmp:
        root = Path(tmp)
        registry = root / "registry.sqlite3"
        service = [segno, "service", "--registry", registry]
        projects = []
        for name in ("first", "second"):
            project = root / name
            projects.append(project)
            run([tactus, "init", project, "--sdk", repository / "clef-sdk"])
            run([segno, "init", "--root", project, "--sdk", repository / "segno-flow"])
            shutil.copyfile(project / ".tactus/skills/segno/assets/segno.hs",
                            project / ".tactus/segno.hs")
            shutil.copyfile(project / ".tactus/skills/segno/assets/010_business.hs",
                            project / ".tactus/scripts/010_business.hs")
            run([*service, "register", name, "--root", project,
                 "--every-seconds", "86400", "--task-timeout-seconds", "120"])

        first = json.loads(run([*service, "once"]))
        assert first["executed"] == 2, first
        status = json.loads(run([*service, "status", "--json"]))
        assert len(status["jobs"]) == len(status["runs"]) == 2, status
        for invocation in status["runs"]:
            assert invocation["state"] == "succeeded", invocation
            assert invocation["status_code"] == 0, invocation
        for project in projects:
            reports = list((project / ".tactus/segno-work").glob("*/report.txt"))
            assert len(reports) == 1, reports
            assert reports[0].read_text() == "Business workflow completed.\n"
            assert (project / ".tactus/segno-service-state/latest.json").is_file()

        # A new CLI process sees the same history and does not repeat completed work.
        second = json.loads(run([*service, "once"]))
        assert second["executed"] == 0, second
        assert len(json.loads(run([*service, "status", "--json"]))["runs"]) == 2
    print("Segno service acceptance passed: two workspaces, actual Haskell organizers, durable history, no duplicate invocation.")


if __name__ == "__main__":
    main()
