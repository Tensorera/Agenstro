from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PUBLISHED_FILES = (
    ROOT / "README.md",
    ROOT / "CHANGELOG.md",
    ROOT / "SECURITY.md",
    ROOT / "CONTRIBUTING.md",
    ROOT / "clef-sdk" / "README.md",
    ROOT / "tactus-runtime" / "README.md",
    ROOT / "motivo-studio" / "README.md",
    ROOT / "segno-flow" / "README.md",
    ROOT / "docs" / "index.md",
    ROOT / "docs" / "getting-started.md",
    ROOT / "docs" / "architecture.md",
    ROOT / "docs" / "segno.md",
    ROOT / "docs" / "motivo-studio.md",
    ROOT / "docs" / "troubleshooting.md",
    ROOT / "docs" / "roadmap.md",
    ROOT / "docs" / "reference" / "cli-v0.3.md",
    ROOT / "docs" / "reference" / "plugin-protocol-v1.md",
    ROOT / "docs" / "reference" / "segno-plugin-wire-v1.md",
    ROOT / "docs" / "reference" / "studio-control-v1.md",
    ROOT / "docs" / "reference" / "support-matrix.md",
    ROOT / "docs" / "adr" / "0003-haskell-dsl-and-local-plugins.md",
    ROOT / "docs" / "adr" / "0004-haskell-segno-persistent-tasks.md",
    ROOT / "docs" / "migrations" / "0.2-to-haskell-0.3.md",
    ROOT / "docs" / "how-to" / "write-documentation.md",
)
LINK = re.compile(r"\[[^]]+\]\(([^)]+)\)")


def markdown_h1_count(text: str) -> int:
    """Count ATX H1 headings while ignoring fenced code examples."""
    in_fence = False
    count = 0
    for line in text.splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            in_fence = not in_fence
        elif not in_fence and re.match(r"^# [^#]", line):
            count += 1
    return count


class DocumentationContractTests(unittest.TestCase):
    def test_published_files_exist_and_have_one_h1(self) -> None:
        for path in PUBLISHED_FILES:
            with self.subTest(path=path.relative_to(ROOT)):
                text = path.read_text(encoding="utf-8")
                self.assertEqual(markdown_h1_count(text), 1)

    def test_current_implementation_and_boundaries_are_explicit(self) -> None:
        text = "\n".join(path.read_text(encoding="utf-8") for path in PUBLISHED_FILES)
        for name in ("clef-sdk", "tactus-runtime", "motivo-studio", "segno-flow"):
            self.assertIn(name, text)
        for current_contract in (
            "agenstro.plugin/v1",
            "OutcomeUnknown",
            "at least once",
            "TypeScript",
        ):
            self.assertIn(current_contract, text)
        for removed_claim in (
            "segno-flow service start",
            "segno-flow-ui",
            "~/.segno-flow",
        ):
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
