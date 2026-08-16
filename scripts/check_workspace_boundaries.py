"""Check dependency boundaries for the retained Rust foundation and Segno."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
PURE_DOMAIN_PACKAGES = {"agentro-contracts", "segno-core"}
CONTRACT_CONSUMERS = {"segno-core"}
REQUIRED_WORKSPACE_PACKAGES = {
    "agentro-cas",
    "agentro-contracts",
    "agentro-process",
    "agentro-proto",
    "agentro-store",
    "agentro-workspace",
    "segno-core",
    "segnod",
}
FORBIDDEN_CORE_DEPENDENCIES = {
    "agentro-proto",
    "prost",
    "prost-types",
    "rusqlite",
    "tokio",
    "tonic",
    "tonic-prost",
}
SQL_LITERAL_PATTERN = re.compile(
    r'(?:br|rb|r|b)?#*"\s*(?:SELECT\b[^"]*\bFROM\b|INSERT\s+INTO\b|'
    r"REPLACE\s+INTO\b|UPDATE\s+[A-Z_][A-Z0-9_.]*\s+SET\b|DELETE\s+FROM\b|"
    r"CREATE\s+(?:TABLE|INDEX|TRIGGER|VIEW)\b|ALTER\s+TABLE\b|"
    r"DROP\s+(?:TABLE|INDEX|TRIGGER|VIEW)\b|PRAGMA\b|"
    r'WITH\b[^"]*\b(?:SELECT|INSERT|UPDATE|DELETE)\b|'
    r"BEGIN\s+(?:IMMEDIATE|EXCLUSIVE|DEFERRED|TRANSACTION)\b|"
    r"COMMIT\s+TRANSACTION\b|ROLLBACK\s+(?:TRANSACTION|TO)\b|VACUUM\b|"
    r"ATTACH\b|DETACH\b|EXPLAIN\b)",
    re.IGNORECASE,
)


def fail(message: str) -> None:
    """Raise one concise boundary failure."""
    raise RuntimeError(message)


def load_metadata() -> dict[str, Any]:
    """Load the locked Cargo graph without resolving external packages."""
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        fail(f"cargo metadata failed:\n{result.stderr.strip()}")
    metadata = json.loads(result.stdout)
    if metadata.get("resolve") is None:
        add_no_deps_resolve(metadata)
    return metadata


def add_no_deps_resolve(metadata: dict[str, Any]) -> None:
    """Build the name-level graph required by this gate."""
    packages = metadata["packages"]
    workspace_by_name = {package["name"]: package["id"] for package in packages}
    external_ids: dict[str, str] = {}
    nodes: list[dict[str, Any]] = []
    for package in packages:
        dependencies: dict[str, list[dict[str, str | None]]] = {}
        for dependency in package.get("dependencies", []):
            name = dependency["name"]
            package_id = workspace_by_name.get(name)
            if package_id is None:
                package_id = external_ids.setdefault(name, f"external:{name}")
            dependencies.setdefault(package_id, []).append({"kind": dependency["kind"]})
        nodes.append(
            {
                "id": package["id"],
                "deps": [
                    {"pkg": package_id, "dep_kinds": kinds}
                    for package_id, kinds in dependencies.items()
                ],
            }
        )
    packages.extend(
        {"id": package_id, "name": name} for name, package_id in external_ids.items()
    )
    metadata["resolve"] = {"nodes": nodes}


def normal_dependencies(node: dict[str, Any]) -> set[str]:
    """Return package IDs reached by normal, non-build dependency edges."""
    return {
        dependency["pkg"]
        for dependency in node["deps"]
        if any(kind["kind"] is None for kind in dependency["dep_kinds"])
    }


def transitive_normal_dependencies(
    package_id: str,
    nodes_by_id: dict[str, dict[str, Any]],
) -> set[str]:
    """Walk normal dependency edges from one workspace package."""
    pending = list(normal_dependencies(nodes_by_id[package_id]))
    visited: set[str] = set()
    while pending:
        dependency_id = pending.pop()
        if dependency_id in visited:
            continue
        visited.add(dependency_id)
        dependency_node = nodes_by_id.get(dependency_id)
        if dependency_node is not None:
            pending.extend(normal_dependencies(dependency_node) - visited)
    return visited


def check_crate_direction(metadata: dict[str, Any]) -> None:
    """Reject transport, runtime, or storage dependencies in domain crates."""
    packages_by_id = {package["id"]: package for package in metadata["packages"]}
    workspace_packages = {
        packages_by_id[package_id]["name"]: package_id
        for package_id in metadata["workspace_members"]
    }
    nodes_by_id = {node["id"]: node for node in metadata["resolve"]["nodes"]}

    missing = REQUIRED_WORKSPACE_PACKAGES - workspace_packages.keys()
    if missing:
        fail(f"missing root workspace packages: {sorted(missing)}")

    for package_name in sorted(PURE_DOMAIN_PACKAGES):
        package_id = workspace_packages[package_name]
        dependency_ids = transitive_normal_dependencies(package_id, nodes_by_id)
        dependency_names = {packages_by_id[item]["name"] for item in dependency_ids}
        forbidden = dependency_names & FORBIDDEN_CORE_DEPENDENCIES
        if forbidden:
            fail(
                f"{package_name} reaches forbidden domain dependencies: "
                f"{sorted(forbidden)}"
            )

    for package_name in sorted(CONTRACT_CONSUMERS):
        package_id = workspace_packages[package_name]
        direct_names = {
            packages_by_id[item]["name"]
            for item in normal_dependencies(nodes_by_id[package_id])
        }
        if "agentro-contracts" not in direct_names:
            fail(
                f"{package_name} must depend directly on agentro-contracts, "
                f"found {sorted(direct_names)}"
            )


def check_agentro_store_sql_source(root: Path = ROOT) -> None:
    """Reject executable SQL outside store repository/migration modules."""
    source_root = root / "crates" / "agentro-store" / "src"
    if not source_root.is_dir():
        fail(f"agentro-store source directory is missing: {source_root}")

    violations: list[str] = []
    for source_path in sorted(source_root.rglob("*.rs")):
        if source_path.name in {"repository.rs", "migration.rs"}:
            continue
        source = source_path.read_text(encoding="utf-8")
        if SQL_LITERAL_PATTERN.search(source):
            violations.append(source_path.relative_to(root).as_posix())
    if violations:
        fail(f"agentro-store SQL outside repository/migration modules: {violations}")


def main() -> int:
    """Run all retained Rust foundation boundary checks."""
    try:
        check_crate_direction(load_metadata())
        check_agentro_store_sql_source()
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        print(f"foundation boundary check failed: {error}", file=sys.stderr)
        return 1

    print("retained Rust foundation boundary check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
