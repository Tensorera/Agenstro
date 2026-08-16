from __future__ import annotations

import json
import tempfile
import tomllib
import zipfile
from pathlib import Path

import pytest

from segno_flow.cli import main as cli_main
from segno_flow.client import (
    DaemonUnavailableError,
    InvalidRequestError,
    SegnoClient,
)
from segno_flow.desktop import main as desktop_main
from segno_flow.manifest import ManifestError, parse_manifest
from segno_flow.package import ArchiveBudget, PackageBuildError, build_task_package
from segno_flow.reports import read_import_report, read_migration_report


def local_test_directory() -> tempfile.TemporaryDirectory[str]:
    return tempfile.TemporaryDirectory(prefix="python-surface-", dir=Path(__file__).parent)


def manifest() -> dict[str, object]:
    return {
        "schema_version": 1,
        "id": "test-task",
        "name": "Test task",
        "schedule": {
            "cron_dialect": "unix5",
            "cron": "0 8 * * *",
            "timezone": "America/New_York",
            "dst_gap": "skip",
            "dst_fold": "first",
            "misfire": {"kind": "coalesce"},
            "overlap": {"kind": "forbid"},
            "retry": {"kind": "none"},
            "jitter_seconds": 0,
        },
        "scripts": {
            "pre": "scripts/pre.py",
            "main": "scripts/main.py",
            "post": "scripts/post.py",
        },
    }


def write_source(root: Path, *, main_source: str = "pass\n") -> None:
    (root / "scripts").mkdir(parents=True)
    (root / "segno-flow.json").write_text(json.dumps(manifest()), encoding="utf-8")
    (root / "scripts" / "pre.py").write_text("pass\n", encoding="utf-8")
    (root / "scripts" / "main.py").write_text(main_source, encoding="utf-8")
    (root / "scripts" / "post.py").write_text("pass\n", encoding="utf-8")


def test_distribution_import_and_entrypoint_names_are_preserved() -> None:
    project = Path(__file__).parents[1]
    with (project / "pyproject.toml").open("rb") as source:
        metadata = tomllib.load(source)

    assert metadata["project"]["name"] == "segno-flow"
    assert metadata["project"]["scripts"] == {
        "segno-flow": "segno_flow.cli:main",
        "segno-flow-ui": "segno_flow.desktop:main",
    }
    assert __import__("segno_flow").__name__ == "segno_flow"


def test_cli_version_matches_the_aligned_release(
    capsys: pytest.CaptureFixture[str],
) -> None:
    with pytest.raises(SystemExit) as captured:
        cli_main(["--version"])
    assert captured.value.code == 0
    assert capsys.readouterr().out.strip() == "segno-flow 0.2.0"


def test_manifest_rejects_unknown_and_duplicate_fields() -> None:
    unknown = manifest()
    unknown["description"] = "not in schema v1"
    with pytest.raises(ManifestError, match="unknown fields"):
        parse_manifest(json.dumps(unknown).encode())

    raw = json.dumps(manifest())
    duplicate = raw[:-1] + ', "id": "other"}'
    with pytest.raises(ManifestError, match="duplicate JSON key"):
        parse_manifest(duplicate.encode())


def test_builder_is_deterministic_and_never_executes_imported_script() -> None:
    with local_test_directory() as directory:
        root = Path(directory)
        source = root / "source"
        sentinel = root / "must-not-exist.txt"
        write_source(
            source,
            main_source=(
                f"from pathlib import Path\nPath({str(sentinel)!r}).write_text('executed')\n"
            ),
        )
        first = build_task_package(source, root / "first.zip")
        second = build_task_package(source, root / "second.zip")

        assert first.manifest.id == "test-task"
        assert first.path.read_bytes() == second.path.read_bytes()
        assert not sentinel.exists()
        with zipfile.ZipFile(first.path) as archive:
            assert archive.namelist() == [
                "scripts/main.py",
                "scripts/post.py",
                "scripts/pre.py",
                "segno-flow.json",
            ]


def test_builder_enforces_archive_budget_before_publication() -> None:
    with local_test_directory() as directory:
        root = Path(directory)
        source = root / "source"
        write_source(source)
        output = root / "task.zip"

        with pytest.raises(PackageBuildError, match="file exceeds"):
            build_task_package(
                source,
                output,
                budget=ArchiveBudget(max_file_bytes=4),
            )
        assert not output.exists()


def test_builder_rejects_configuration_above_hard_archive_ceiling() -> None:
    with pytest.raises(PackageBuildError, match="hard package ceiling"):
        ArchiveBudget(max_entries=1_001).validate()


@pytest.mark.parametrize("value", [True, 1.5])
def test_archive_budget_requires_runtime_integers(value: object) -> None:
    with pytest.raises(PackageBuildError, match="must be integers"):
        ArchiveBudget(max_entries=value).validate()  # type: ignore[arg-type]


class FakeRpcError(RuntimeError):
    def code(self) -> str:
        return "INVALID_ARGUMENT"

    def details(self) -> str:
        return "task ID is invalid"


class FakeTransport:
    def __init__(self, *, failure: bool = False) -> None:
        self.failure = failure
        self.calls: list[tuple[str, dict[str, object], float]] = []

    def unary(
        self,
        method: str,
        request: dict[str, object],
        *,
        timeout: float,
    ) -> dict[str, object]:
        self.calls.append((method, request, timeout))
        if self.failure:
            raise FakeRpcError
        return {
            "tasks": [
                {
                    "task_id": "test-task",
                    "revision": 1,
                    "enabled": False,
                    "package_digest": "sha256:" + "a" * 64,
                    "plan_digest": None,
                }
            ],
            "next_after": None,
        }

    def upload(
        self,
        method: str,
        package: Path,
        request: dict[str, object],
        *,
        timeout: float,
        max_bytes: int,
    ) -> dict[str, object]:
        raise AssertionError((method, package, request, timeout, max_bytes))


def test_client_is_typed_bounded_and_maps_rpc_code() -> None:
    transport = FakeTransport()
    page = SegnoClient(transport).list_tasks(limit=1, timeout=3)
    assert page.tasks[0].task_id == "test-task"
    assert transport.calls == [("ListTasks", {"after": None, "limit": 1}, 3)]

    with pytest.raises(InvalidRequestError, match="task ID is invalid"):
        SegnoClient(FakeTransport(failure=True)).list_tasks(limit=1)


def test_report_readers_are_typed_and_bounded() -> None:
    import_value = {
        "task_id": "test-task",
        "revision": 1,
        "package_digest": "sha256:" + "a" * 64,
        "workflow_spec_digest": "sha256:" + "b" * 64,
        "enabled": False,
    }
    with local_test_directory() as directory:
        root = Path(directory)
        import_path = root / "import.json"
        import_path.write_text(json.dumps(import_value), encoding="utf-8")
        migration_path = root / "migration.json"
        migration_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "source": "legacy-segno-v0",
                    "imports": [import_value],
                    "warnings": ["disabled until compiled"],
                }
            ),
            encoding="utf-8",
        )

        assert read_import_report(import_path).revision == 1
        assert read_migration_report(migration_path).imports[0].task_id == "test-task"


def test_cli_uses_client_boundary_and_ui_launches_motivo_surface(
    capsys: pytest.CaptureFixture[str],
) -> None:
    def unavailable() -> SegnoClient:
        raise DaemonUnavailableError("no generated binding")

    assert cli_main(["list"], client_factory=unavailable) == 2
    assert "UNAVAILABLE: no generated binding" in capsys.readouterr().err
    calls: list[tuple[str, list[str]]] = []

    def executor(executable: str, arguments: list[str]) -> object:
        calls.append((executable, arguments))
        return object()

    assert desktop_main(["--project", "example"], executor=executor) == 0
    assert calls == [
        (
            "motivo-studio",
            ["motivo-studio", "--surface", "scheduler", "--project", "example"],
        )
    ]
