"""Thin RPC client and offline task-package authoring surface for Segno."""

from segno_flow.client import SegnoClient
from segno_flow.manifest import PackageManifest, load_manifest
from segno_flow.package import ArchiveBudget, build_task_package
from segno_flow.reports import read_import_report, read_migration_report

__all__ = [
    "ArchiveBudget",
    "PackageManifest",
    "SegnoClient",
    "build_task_package",
    "load_manifest",
    "read_import_report",
    "read_migration_report",
]
__version__ = "0.2.0"
