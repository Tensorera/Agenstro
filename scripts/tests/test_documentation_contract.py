from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PUBLISHED_FILES = (
    ROOT / "README.md",
    ROOT / "clef-sdk" / "README.md",
    ROOT / "tactus-runtime" / "README.md",
    ROOT / "motivo-studio" / "README.md",
    ROOT / "segno-flow" / "README.md",
    ROOT / "docs" / "index.md",
    ROOT / "docs" / "alpha-cross-language-slice.md",
    ROOT / "docs" / "how-to" / "install-source-alpha.md",
    ROOT / "docs" / "how-to" / "operate-source-alpha.md",
    ROOT / "docs" / "reference" / "index.md",
    ROOT / "docs" / "reference" / "source-contracts.md",
    ROOT / "docs" / "reference" / "support-matrix.md",
    ROOT / "docs" / "reference" / "known-limitations.md",
    ROOT / "docs" / "explanation" / "runtime-boundaries.md",
    ROOT / "docs" / "adr" / "0001-versioned-rust-foundation.md",
    ROOT / "docs" / "adr" / "0002-daemon-state-and-electron-boundaries.md",
    ROOT / "docs" / "migrations" / "prototype-to-greenfield.md",
)
LINK = re.compile(r"\[[^]]+\]\(([^)]+)\)")


class DocumentationContractTests(unittest.TestCase):
    def test_published_files_exist_and_have_one_h1(self) -> None:
        for path in PUBLISHED_FILES:
            with self.subTest(path=path.relative_to(ROOT)):
                text = path.read_text(encoding="utf-8")
                self.assertEqual(len(re.findall(r"^# [^#]", text, re.MULTILINE)), 1)

    def test_public_names_and_alpha_boundary_are_explicit(self) -> None:
        text = "\n".join(path.read_text(encoding="utf-8") for path in PUBLISHED_FILES)
        for name in (
            "clef-sdk",
            "tactus-runtime",
            "motivo-studio",
            "segno-flow",
            "clef_sdk",
            "tactus_runtime",
            "segno_flow",
            "segno-flow-ui",
        ):
            self.assertIn(name, text)
        self.assertIsNone(re.search(r"\b(?:from|import)\s+agentro\b", text))
        self.assertIn("authenticated gRPC", text)
        self.assertIn("not implemented", text)
        for removed_claim in ("tactus doctor", "segno-flow service start", "~/.segno-flow"):
            self.assertNotIn(removed_claim, text)

    def test_local_markdown_links_resolve(self) -> None:
        for path in PUBLISHED_FILES:
            text = path.read_text(encoding="utf-8")
            for target in LINK.findall(text):
                if target.startswith(("https://", "http://", "mailto:", "#")):
                    continue
                relative = target.split("#", 1)[0]
                if not relative:
                    continue
                resolved = (path.parent / relative).resolve()
                with self.subTest(source=path.relative_to(ROOT), target=target):
                    self.assertTrue(resolved.exists(), f"missing local link target: {resolved}")


if __name__ == "__main__":
    unittest.main()
