"""Public ``segno-flow`` RPC and offline authoring facade."""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Callable, Sequence
from dataclasses import asdict
from pathlib import Path

from segno_flow import __version__
from segno_flow.client import SegnoClient, SegnoClientError, connect_local
from segno_flow.package import PackageBuildError, build_task_package
from segno_flow.reports import ReportError, read_import_report, read_migration_report

ClientFactory = Callable[[], SegnoClient]


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="segno-flow")
    parser.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    commands = parser.add_subparsers(dest="command", required=True)

    package = commands.add_parser("package", help="offline task-package authoring")
    package_commands = package.add_subparsers(dest="package_command", required=True)
    build = package_commands.add_parser("build", help="build a strict task ZIP")
    build.add_argument("source", type=Path)
    build.add_argument("output", type=Path)

    report = commands.add_parser("report", help="read bounded JSON evidence")
    report_commands = report.add_subparsers(dest="report_command", required=True)
    import_report = report_commands.add_parser("import", help="read an import report")
    import_report.add_argument("path", type=Path)
    migration_report = report_commands.add_parser("migration", help="read a migration report")
    migration_report.add_argument("path", type=Path)

    import_package = commands.add_parser("import", help="stream a package to segnod")
    import_package.add_argument("package", type=Path)
    list_tasks = commands.add_parser("list", help="list a bounded task page")
    list_tasks.add_argument("--after")
    list_tasks.add_argument("--limit", type=int, default=100)
    run = commands.add_parser("run", help="create a manual occurrence")
    run.add_argument("task_id")
    status = commands.add_parser("status", help="read an occurrence snapshot")
    status.add_argument("occurrence_id")
    return parser


def _print_json(value: object) -> None:
    print(json.dumps(value, ensure_ascii=False, sort_keys=True))


def main(
    argv: Sequence[str] | None = None,
    *,
    client_factory: ClientFactory = connect_local,
) -> int:
    """Run one bounded offline command or one thin daemon-client call."""

    arguments = _parser().parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if arguments.command == "package":
            result = build_task_package(arguments.source, arguments.output)
            _print_json(
                {
                    "archive_bytes": result.archive_bytes,
                    "entries": result.entries,
                    "expanded_bytes": result.expanded_bytes,
                    "path": str(result.path),
                    "task_id": result.manifest.id,
                }
            )
            return 0
        if arguments.command == "report":
            result = (
                read_import_report(arguments.path)
                if arguments.report_command == "import"
                else read_migration_report(arguments.path)
            )
            _print_json(asdict(result))
            return 0

        client = client_factory()
        if arguments.command == "import":
            _print_json(asdict(client.import_package(arguments.package)))
        elif arguments.command == "list":
            page = client.list_tasks(after=arguments.after, limit=arguments.limit)
            _print_json(
                {"tasks": [asdict(task) for task in page.tasks], "next_after": page.next_after}
            )
        elif arguments.command == "run":
            _print_json(asdict(client.run_now(arguments.task_id)))
        else:
            _print_json(asdict(client.status(arguments.occurrence_id)))
        return 0
    except (OSError, PackageBuildError, ReportError, SegnoClientError, ValueError) as error:
        code = error.code if isinstance(error, SegnoClientError) else "INVALID_ARGUMENT"
        print(f"{code}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
