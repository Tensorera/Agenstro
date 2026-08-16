"""Bind a reusable TOML profile to a caller-supplied workfolder."""

from __future__ import annotations

import hashlib
from dataclasses import replace
from pathlib import Path

from clef_sdk.profiles import (
    Profile,
    StorageConfig,
    WorkspaceConfig,
    load_profile,
)


def bind_profile(profile_path: Path, workfolder: Path) -> Profile:
    """Load adapter/runtime policy, then inject isolated workspace/state paths."""

    workfolder = workfolder.expanduser().resolve(strict=True)
    template = load_profile(
        profile_path.expanduser().resolve(strict=True),
        require_workspace=False,
        require_read_roots=False,
    )
    identity = hashlib.sha256(
        str(workfolder).casefold().encode("utf-8")
    ).hexdigest()[:16]
    state_root = (
        workfolder.parent
        / ".clef-state"
        / f"{workfolder.name}-{identity}"
    ).resolve(strict=False)
    state_root.mkdir(parents=True, exist_ok=True)
    profile = replace(
        template,
        workspace=WorkspaceConfig(
            root=workfolder,
            read_roots=(),
        ),
        storage=StorageConfig(
            state_root=state_root,
            cas_dir=template.storage.cas_dir,
            traces_dir=template.storage.traces_dir,
            cache_dir=template.storage.cache_dir,
            manifests_dir=template.storage.manifests_dir,
            cache_enabled=template.storage.cache_enabled,
            fsync=template.storage.fsync,
        ),
    )
    profile.validate_filesystem()
    return profile


__all__ = ["bind_profile"]
