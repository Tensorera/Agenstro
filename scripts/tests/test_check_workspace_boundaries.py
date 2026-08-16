"""Regression tests for the retained Rust workspace boundary gate."""

from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Any

import scripts.check_workspace_boundaries as boundaries
from scripts.check_workspace_boundaries import (
    REQUIRED_WORKSPACE_PACKAGES,
    check_crate_direction,
)


def dependency(package_id: str) -> dict[str, Any]:
    """Create one normal Cargo metadata dependency edge."""
    return {"pkg": package_id, "dep_kinds": [{"kind": None}]}


def metadata(
    *,
    segno_dependencies: tuple[str, ...],
    extra_edges: dict[str, tuple[str, ...]] | None = None,
) -> dict[str, Any]:
    """Build the minimal Cargo metadata shape consumed by the gate."""
    edges = {
        **{package: () for package in REQUIRED_WORKSPACE_PACKAGES},
        "agentro-store": ("rusqlite",),
        "segno-core": segno_dependencies,
        **(extra_edges or {}),
    }
    package_names = set(edges)
    package_names.update(item for values in edges.values() for item in values)
    return {
        "packages": [
            {"id": package_name, "name": package_name}
            for package_name in sorted(package_names)
        ],
        "workspace_members": sorted(REQUIRED_WORKSPACE_PACKAGES),
        "resolve": {
            "nodes": [
                {
                    "id": package_name,
                    "deps": [dependency(item) for item in dependencies],
                }
                for package_name, dependencies in edges.items()
            ]
        },
    }


class CrateDirectionTests(unittest.TestCase):
    """Exercise the retained Segno domain dependency invariants."""

    def test_segno_core_may_add_another_pure_domain_dependency(self) -> None:
        graph = metadata(
            segno_dependencies=("agentro-contracts", "schedule-model"),
            extra_edges={"schedule-model": ()},
        )

        check_crate_direction(graph)

    def test_segno_core_must_depend_on_agentro_contracts(self) -> None:
        graph = metadata(
            segno_dependencies=("schedule-model",),
            extra_edges={"schedule-model": ()},
        )

        with self.assertRaisesRegex(
            RuntimeError,
            "segno-core must depend directly on agentro-contracts",
        ):
            check_crate_direction(graph)

    def test_transitive_transport_dependency_remains_forbidden(self) -> None:
        graph = metadata(
            segno_dependencies=("agentro-contracts", "schedule-model"),
            extra_edges={"schedule-model": ("tonic",), "tonic": ()},
        )

        with self.assertRaisesRegex(
            RuntimeError,
            "segno-core reaches forbidden domain dependencies: \\['tonic'\\]",
        ):
            check_crate_direction(graph)

    def test_segnod_may_keep_its_private_rusqlite_dependency(self) -> None:
        graph = metadata(
            segno_dependencies=("agentro-contracts",),
            extra_edges={"segnod": ("rusqlite",), "rusqlite": ()},
        )

        check_crate_direction(graph)


class AgentroStoreSourceBoundaryTests(unittest.TestCase):
    """Keep executable SQL in repository and migration modules."""

    def test_store_source_rejects_sql_in_actor_or_model_modules(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "crates" / "agentro-store" / "src"
            source.mkdir(parents=True)
            (source / "actor.rs").write_text(
                'const QUERY: &str = "SELECT run_id FROM runs";\n',
                encoding="utf-8",
            )
            (source / "model.rs").write_text("", encoding="utf-8")

            with self.assertRaisesRegex(
                RuntimeError, "SQL outside repository/migration"
            ):
                boundaries.check_agentro_store_sql_source(root)

    def test_store_source_allows_sql_in_repository_and_migration_modules(self) -> None:
        with TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "crates" / "agentro-store" / "src"
            source.mkdir(parents=True)
            (source / "repository.rs").write_text(
                'const QUERY: &str = "SELECT version FROM schema_migrations";\n',
                encoding="utf-8",
            )
            (source / "migration.rs").write_text(
                'const SCHEMA: &str = "CREATE TABLE runs (id TEXT)";\n',
                encoding="utf-8",
            )

            boundaries.check_agentro_store_sql_source(root)


if __name__ == "__main__":
    unittest.main()
