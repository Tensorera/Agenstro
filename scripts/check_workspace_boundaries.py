"""Check greenfield crate direction and unchanged public product identities."""

from __future__ import annotations

import ast
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

import tomllib

ROOT = Path(__file__).resolve().parents[1]
RELEASE_VERSION = "0.2.0"
PURE_DOMAIN_PACKAGES = {
    "agentro-contracts",
    "clef-core",
    "segno-core",
}
CONTRACT_CONSUMERS = {"clef-core", "segno-core", "tactus-core"}
REQUIRED_WORKSPACE_PACKAGES = {
    "agentro-cas",
    "agentro-contracts",
    "agentro-process",
    "agentro-proto",
    "agentro-store",
    "agentro-workspace",
    "agentrod",
    "clef-agent",
    "clef-core",
    "segno-core",
    "segnod",
    "tactus-core",
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
TACTUS_SQL_LITERAL_PATTERN = re.compile(
    r'(?:br|rb|r|b)?#*"\s*(?:SELECT\b[^\"]*\bFROM\b|INSERT\s+INTO\b|'
    r"REPLACE\s+INTO\b|UPDATE\s+[A-Z_][A-Z0-9_.]*\s+SET\b|DELETE\s+FROM\b|"
    r"CREATE\s+(?:TABLE|INDEX|TRIGGER|VIEW)\b|ALTER\s+TABLE\b|"
    r"DROP\s+(?:TABLE|INDEX|TRIGGER|VIEW)\b|PRAGMA\b|"
    r"WITH\b[^\"]*\b(?:SELECT|INSERT|UPDATE|DELETE)\b|"
    r"BEGIN\s+(?:IMMEDIATE|EXCLUSIVE|DEFERRED|TRANSACTION)\b|"
    r"COMMIT\s+TRANSACTION\b|ROLLBACK\s+(?:TRANSACTION|TO)\b|VACUUM\b|"
    r"ATTACH\b|DETACH\b|EXPLAIN\b)",
    re.IGNORECASE,
)


def fail(message: str) -> None:
    """Raise one concise boundary failure."""
    raise RuntimeError(message)


def load_metadata() -> dict[str, Any]:
    """Load the locked Cargo graph without invoking a shell."""
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
    """Build the name-level graph needed by this gate from no-deps metadata."""
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

    tactus_id = workspace_packages["tactus-core"]
    tactus_direct = {
        packages_by_id[item]["name"]
        for item in normal_dependencies(nodes_by_id[tactus_id])
    }
    if "agentro-store" not in tactus_direct:
        fail(
            "tactus-core must depend directly on agentro-store, "
            f"found {sorted(tactus_direct)}"
        )
    if "rusqlite" in tactus_direct:
        fail(
            "tactus-core must not depend directly on rusqlite, "
            f"found {sorted(tactus_direct)}"
        )


def check_tactus_storage_source(root: Path = ROOT) -> None:
    """Reject database implementation ownership in the Tactus application crate."""
    source_root = root / "tactus-runtime" / "rust" / "tactus-core" / "src"
    if not source_root.is_dir():
        fail(f"tactus-core source directory is missing: {source_root}")

    violations: list[str] = []
    for source_path in sorted(source_root.rglob("*.rs")):
        source = source_path.read_text(encoding="utf-8")
        if TACTUS_SQL_LITERAL_PATTERN.search(source) or re.search(
            r"\brusqlite\b", source, re.IGNORECASE
        ):
            violations.append(source_path.relative_to(root).as_posix())
    if violations:
        fail(f"tactus-core source must not contain SQL or rusqlite: {violations}")


def check_agentro_store_sql_source(root: Path = ROOT) -> None:
    """Reject executable SQL outside agentro-store repository/migration modules."""
    source_root = root / "crates" / "agentro-store" / "src"
    if not source_root.is_dir():
        fail(f"agentro-store source directory is missing: {source_root}")

    violations: list[str] = []
    for source_path in sorted(source_root.rglob("*.rs")):
        if source_path.name in {"repository.rs", "migration.rs"}:
            continue
        source = source_path.read_text(encoding="utf-8")
        if TACTUS_SQL_LITERAL_PATTERN.search(source):
            violations.append(source_path.relative_to(root).as_posix())
    if violations:
        fail(f"agentro-store SQL outside repository/migration modules: {violations}")


def load_toml(path: Path) -> dict[str, Any]:
    """Load one UTF-8 TOML manifest."""
    with path.open("rb") as stream:
        return tomllib.load(stream)


def module_version(path: Path) -> str:
    """Read a literal module ``__version__`` without importing package code."""
    module = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for statement in module.body:
        if not isinstance(statement, ast.Assign) or len(statement.targets) != 1:
            continue
        target = statement.targets[0]
        if (
            isinstance(target, ast.Name)
            and target.id == "__version__"
            and isinstance(statement.value, ast.Constant)
            and isinstance(statement.value.value, str)
        ):
            return statement.value.value
    fail(f"literal __version__ is missing from {path}")


def check_public_identities() -> None:
    """Lock the existing directory, distribution, import, and entry-point names."""
    clef = load_toml(ROOT / "clef-sdk" / "pyproject.toml")
    tactus = load_toml(ROOT / "tactus-runtime" / "pyproject.toml")
    segno = load_toml(ROOT / "segno-flow" / "pyproject.toml")
    workspace = load_toml(ROOT / "Cargo.toml")
    with (ROOT / "motivo-studio" / "package.json").open(encoding="utf-8") as stream:
        motivo = json.load(stream)

    checks = {
        "clef-sdk distribution": clef["project"]["name"] == "clef-sdk",
        "clef_sdk import": (
            ROOT / "clef-sdk" / "src" / "clef_sdk" / "__init__.py"
        ).is_file(),
        "tactus-runtime distribution": tactus["project"]["name"] == "tactus-runtime",
        "tactus_runtime import": (
            ROOT / "tactus-runtime" / "src" / "tactus_runtime" / "__init__.py"
        ).is_file(),
        "tactus CLI": tactus["project"]["scripts"].get("tactus")
        == "tactus_runtime.cli:main",
        "motivo-studio GUI": tactus["project"]["gui-scripts"].get("motivo-studio")
        == "tactus_runtime.studio:main",
        "motivo-studio package": motivo["name"] == "motivo-studio",
        "segno-flow distribution": segno["project"]["name"] == "segno-flow",
        "segno_flow import": (
            ROOT / "segno-flow" / "src" / "segno_flow" / "__init__.py"
        ).is_file(),
        "segno-flow CLI": segno["project"]["scripts"].get("segno-flow")
        == "segno_flow.cli:main",
        "segno-flow-ui GUI": segno["project"]["scripts"].get("segno-flow-ui")
        == "segno_flow.desktop:main",
        "Rust release version": workspace["workspace"]["package"]["version"]
        == RELEASE_VERSION,
        "clef-sdk release version": module_version(
            ROOT / "clef-sdk" / "src" / "clef_sdk" / "__init__.py"
        )
        == RELEASE_VERSION,
        "tactus-runtime release version": module_version(
            ROOT / "tactus-runtime" / "src" / "tactus_runtime" / "__init__.py"
        )
        == RELEASE_VERSION,
        "segno-flow release version": segno["project"]["version"]
        == module_version(ROOT / "segno-flow" / "src" / "segno_flow" / "__init__.py")
        == RELEASE_VERSION,
        "motivo-studio release version": motivo["version"] == RELEASE_VERSION,
        "no clef agentro import": not (ROOT / "clef-sdk" / "src" / "agentro").exists(),
        "no tactus agentro import": not (
            ROOT / "tactus-runtime" / "src" / "agentro"
        ).exists(),
        "no segno agentro import": not (
            ROOT / "segno-flow" / "src" / "agentro"
        ).exists(),
    }
    failed = sorted(name for name, passed in checks.items() if not passed)
    if failed:
        fail(f"public identity checks failed: {failed}")


def main() -> int:
    """Run all foundation boundary checks."""
    try:
        check_crate_direction(load_metadata())
        check_tactus_storage_source()
        check_agentro_store_sql_source()
        check_public_identities()
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        print(f"foundation boundary check failed: {error}", file=sys.stderr)
        return 1

    print("foundation boundary check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
