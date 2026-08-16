from __future__ import annotations

import hashlib
import json
import os
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch


TEST2_ROOT = Path(__file__).resolve().parents[1]
REPOSITORY_ROOT = TEST2_ROOT.parent
sys.path.insert(0, str(REPOSITORY_ROOT / "clef-sdk" / "src"))
sys.path.insert(0, str(TEST2_ROOT))

from script.Mineru import (  # noqa: E402
    MinerUClient,
    MinerUJob,
    extract_pdf,
    load_mineru_token,
    prepare_manuscript,
    reuse_extraction,
)
from script.Mineru.extractor import _extract_archive  # noqa: E402


class _FakeMinerUClient(MinerUClient):
    def __init__(self, archive: Path) -> None:
        self.archive = archive

    def submit_and_wait(
        self,
        pdf_path: Path,
        *,
        data_id: str,
        model_version: str = "vlm",
        language: str = "en",
        is_ocr: bool = True,
        enable_formula: bool = True,
        enable_table: bool = True,
        timeout_seconds: float = 1800.0,
        poll_interval_seconds: float = 5.0,
    ) -> MinerUJob:
        del (
            data_id,
            model_version,
            language,
            is_ocr,
            enable_formula,
            enable_table,
            timeout_seconds,
            poll_interval_seconds,
        )
        return MinerUJob(
            batch_id="batch-offline",
            file_name=pdf_path.name,
            data_id="manuscript-offline",
            full_zip_url="https://invalid.example/offline.zip",
            submit_trace_id="submit-offline",
            result_trace_id="result-offline",
        )

    def download(
        self,
        url: str,
        target: Path,
        *,
        maximum_bytes: int = 1_073_741_824,
    ) -> None:
        del url, maximum_bytes
        shutil.copyfile(self.archive, target)


class MinerUOfflineTests(unittest.TestCase):
    def test_env_reader_does_not_require_python_dotenv(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-env-", dir=TEST2_ROOT
        ) as temporary:
            env = Path(temporary) / ".env"
            env.write_text("Mineru_Api='offline-token'\n", encoding="utf-8")
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(load_mineru_token(env), "offline-token")

    def test_safe_archive_rejects_parent_traversal(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-zip-", dir=TEST2_ROOT
        ) as temporary:
            root = Path(temporary)
            archive = root / "unsafe.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("../escape.txt", "forbidden")
            with self.assertRaises(ValueError):
                _extract_archive(archive, root / "out")
            self.assertFalse((root / "escape.txt").exists())

    def test_fake_download_is_installed_and_reusable(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="test2-extract-", dir=TEST2_ROOT
        ) as temporary:
            root = Path(temporary).resolve()
            source = root / "source.pdf"
            source.write_bytes(b"%PDF-1.7\noffline\n%%EOF\n")
            workfolder = root / "work"
            manuscript = prepare_manuscript(source, workfolder)
            archive = root / "mineru.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr(
                    "result/full.md",
                    "# Extracted\n\nMinerU offline fixture.",
                )
                bundle.writestr("result/images/figure.png", b"\x89PNG\r\n")
            result = extract_pdf(
                manuscript,
                workfolder,
                env_file=root / "unused.env",
                client=_FakeMinerUClient(archive),
                timeout_seconds=1,
                poll_interval_seconds=0.1,
            )
            self.assertFalse(result.reused)
            self.assertEqual(result.full_markdown, workfolder / "Extractedmd" / "full.md")
            self.assertTrue(result.full_markdown.is_file())
            self.assertTrue(
                (workfolder / "Extractedmd" / "images" / "figure.png").is_file()
            )
            manifest = json.loads(
                (
                    workfolder
                    / "Extractedmd"
                    / "extraction-manifest.json"
                ).read_text(encoding="utf-8")
            )
            self.assertEqual(manifest["batch_id"], "batch-offline")
            self.assertEqual(
                manifest["full_markdown_sha256"],
                hashlib.sha256(result.full_markdown.read_bytes()).hexdigest(),
            )
            reused = reuse_extraction(manuscript, workfolder)
            self.assertTrue(reused.reused)
            self.assertEqual(
                reused.full_markdown_sha256, result.full_markdown_sha256
            )


if __name__ == "__main__":
    unittest.main()
