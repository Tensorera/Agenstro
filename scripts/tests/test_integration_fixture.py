"""Contract tests for the generated cross-language alpha fixture."""

from __future__ import annotations

import json
import unittest

from scripts.generate_integration_fixture import OUTPUT, build_fixture, render_fixture


class IntegrationFixtureTests(unittest.TestCase):
    """Keep the checked-in DTO tied to the public Python builder."""

    def test_checked_in_fixture_is_current(self) -> None:
        self.assertEqual(OUTPUT.read_text(encoding="utf-8"), render_fixture())

    def test_public_products_and_entrypoints_are_unchanged(self) -> None:
        fixture = build_fixture()
        products = json.loads(json.dumps(fixture["products"]))
        self.assertEqual(
            [(item["distribution"], item["entry_points"]) for item in products],
            [
                ("clef-sdk", []),
                ("tactus-runtime", ["tactus"]),
                ("motivo-studio", ["motivo-studio"]),
                ("segno-flow", ["segno-flow", "segno-flow-ui"]),
            ],
        )


if __name__ == "__main__":
    unittest.main()
