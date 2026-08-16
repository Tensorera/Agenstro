"""Minimal standard-library client for the MinerU precision extract API v4."""

from __future__ import annotations

import http.client
import json
import socket
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, urlunsplit
from urllib.request import Request, urlopen


class MinerUError(RuntimeError):
    """A bounded, token-safe MinerU API or transport failure."""


@dataclass(frozen=True, slots=True)
class MinerUJob:
    batch_id: str
    file_name: str
    data_id: str
    full_zip_url: str
    submit_trace_id: str | None = None
    result_trace_id: str | None = None


class MinerUClient:
    """Upload one local document through `/api/v4/file-urls/batch`."""

    BASE_URL = "https://mineru.net"
    MAX_ERROR_BODY = 8 * 1024

    def __init__(
        self,
        token: str,
        *,
        request_timeout_seconds: float = 60.0,
        base_url: str = BASE_URL,
    ) -> None:
        token = token.strip()
        if not token:
            raise ValueError("MinerU token is empty")
        if request_timeout_seconds <= 0:
            raise ValueError("request timeout must be positive")
        self._token = token
        self.request_timeout_seconds = float(request_timeout_seconds)
        self.base_url = base_url.rstrip("/")

    def _api_request(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        body = None
        headers = {
            "Accept": "application/json",
            "Authorization": f"Bearer {self._token}",
        }
        if payload is not None:
            body = json.dumps(
                payload,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
                allow_nan=False,
            ).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = Request(
            f"{self.base_url}{path}",
            data=body,
            method=method,
            headers=headers,
        )
        try:
            with urlopen(
                request, timeout=self.request_timeout_seconds
            ) as response:
                raw = response.read()
                status = response.status
        except HTTPError as error:
            detail = error.read(self.MAX_ERROR_BODY).decode(
                "utf-8", errors="replace"
            )
            raise MinerUError(
                f"MinerU HTTP {error.code} for {path}: {detail}"
            ) from error
        except (URLError, TimeoutError, socket.timeout, OSError) as error:
            raise MinerUError(
                f"MinerU transport failure for {path}: "
                f"{type(error).__name__}: {error}"
            ) from error
        if not 200 <= status < 300:
            raise MinerUError(f"MinerU HTTP {status} for {path}")
        try:
            value = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise MinerUError(f"MinerU returned invalid JSON for {path}") from error
        if not isinstance(value, dict):
            raise MinerUError(f"MinerU returned a non-object JSON response for {path}")
        if value.get("code") != 0:
            raise MinerUError(
                f"MinerU API error for {path}: code={value.get('code')!r}, "
                f"message={value.get('msg')!r}"
            )
        if not isinstance(value.get("data"), dict):
            raise MinerUError(f"MinerU response has no data object for {path}")
        return value

    def request_upload(
        self,
        pdf_path: Path,
        *,
        data_id: str,
        model_version: str = "vlm",
        language: str = "en",
        is_ocr: bool = True,
        enable_formula: bool = True,
        enable_table: bool = True,
    ) -> tuple[str, str, str | None]:
        if not pdf_path.is_file():
            raise FileNotFoundError(pdf_path)
        payload = {
            "files": [
                {
                    "name": pdf_path.name,
                    "data_id": data_id,
                    "is_ocr": is_ocr,
                }
            ],
            "model_version": model_version,
            "language": language,
            "enable_formula": enable_formula,
            "enable_table": enable_table,
        }
        value = self._api_request("POST", "/api/v4/file-urls/batch", payload)
        data = value["data"]
        batch_id = data.get("batch_id")
        file_urls = data.get("file_urls")
        if not isinstance(batch_id, str) or not batch_id:
            raise MinerUError("MinerU upload response has no batch_id")
        if (
            not isinstance(file_urls, list)
            or len(file_urls) != 1
            or not isinstance(file_urls[0], str)
            or not file_urls[0]
        ):
            raise MinerUError("MinerU upload response has no unique signed file URL")
        trace_id = value.get("trace_id")
        return batch_id, file_urls[0], trace_id if isinstance(trace_id, str) else None

    def upload_file(self, signed_url: str, source: Path) -> None:
        """Stream a PUT without a Content-Type header, as required by MinerU."""

        parsed = urlsplit(signed_url)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise MinerUError("MinerU returned an invalid signed upload URL")
        connection_type = (
            http.client.HTTPSConnection
            if parsed.scheme == "https"
            else http.client.HTTPConnection
        )
        port = parsed.port
        connection = connection_type(
            parsed.hostname,
            port=port,
            timeout=self.request_timeout_seconds,
        )
        target = urlunsplit(("", "", parsed.path or "/", parsed.query, ""))
        try:
            connection.putrequest("PUT", target)
            connection.putheader("Content-Length", str(source.stat().st_size))
            connection.putheader("Accept", "*/*")
            connection.endheaders()
            with source.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    connection.send(chunk)
            response = connection.getresponse()
            detail = response.read(self.MAX_ERROR_BODY)
            if not 200 <= response.status < 300:
                message = detail.decode("utf-8", errors="replace")
                raise MinerUError(
                    f"MinerU signed upload failed with HTTP "
                    f"{response.status}: {message}"
                )
        except (OSError, HTTPError, socket.timeout) as error:
            if isinstance(error, MinerUError):
                raise
            raise MinerUError(
                f"MinerU signed upload transport failure: "
                f"{type(error).__name__}: {error}"
            ) from error
        finally:
            connection.close()

    def wait_for_result(
        self,
        batch_id: str,
        *,
        file_name: str,
        data_id: str,
        timeout_seconds: float = 1800.0,
        poll_interval_seconds: float = 5.0,
        submit_trace_id: str | None = None,
    ) -> MinerUJob:
        if timeout_seconds <= 0 or poll_interval_seconds <= 0:
            raise ValueError("poll timeout and interval must be positive")
        deadline = time.monotonic() + timeout_seconds
        last_state = "unknown"
        last_trace: str | None = None
        while True:
            value = self._api_request(
                "GET", f"/api/v4/extract-results/batch/{batch_id}"
            )
            trace = value.get("trace_id")
            last_trace = trace if isinstance(trace, str) else last_trace
            data = value["data"]
            results = data.get("extract_result")
            if not isinstance(results, list):
                raise MinerUError("MinerU result response has no extract_result array")
            matching = [
                item
                for item in results
                if isinstance(item, dict)
                and (
                    item.get("data_id") == data_id
                    or item.get("file_name") == file_name
                )
            ]
            if len(matching) != 1:
                raise MinerUError(
                    "MinerU result does not uniquely identify the uploaded file"
                )
            result = matching[0]
            state = result.get("state")
            if not isinstance(state, str):
                raise MinerUError("MinerU result has no state")
            last_state = state
            if state == "done":
                full_zip_url = result.get("full_zip_url")
                if not isinstance(full_zip_url, str) or not full_zip_url:
                    raise MinerUError("completed MinerU result has no full_zip_url")
                return MinerUJob(
                    batch_id=batch_id,
                    file_name=file_name,
                    data_id=data_id,
                    full_zip_url=full_zip_url,
                    submit_trace_id=submit_trace_id,
                    result_trace_id=last_trace,
                )
            if state == "failed":
                raise MinerUError(
                    f"MinerU extraction failed: {result.get('err_msg')!r}"
                )
            if state not in {
                "waiting-file",
                "pending",
                "running",
                "converting",
                "uploading",
            }:
                raise MinerUError(f"unknown MinerU extraction state: {state!r}")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise MinerUError(
                    f"MinerU polling timed out after state {last_state!r}; "
                    f"batch_id={batch_id}"
                )
            time.sleep(min(poll_interval_seconds, remaining))

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
        batch_id, signed_url, trace_id = self.request_upload(
            pdf_path,
            data_id=data_id,
            model_version=model_version,
            language=language,
            is_ocr=is_ocr,
            enable_formula=enable_formula,
            enable_table=enable_table,
        )
        self.upload_file(signed_url, pdf_path)
        return self.wait_for_result(
            batch_id,
            file_name=pdf_path.name,
            data_id=data_id,
            timeout_seconds=timeout_seconds,
            poll_interval_seconds=poll_interval_seconds,
            submit_trace_id=trace_id,
        )

    def download(
        self,
        url: str,
        target: Path,
        *,
        maximum_bytes: int = 1_073_741_824,
    ) -> None:
        if maximum_bytes < 1:
            raise ValueError("maximum_bytes must be positive")
        parsed = urlsplit(url)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise MinerUError("MinerU returned an invalid archive download URL")
        request = Request(url, method="GET", headers={"Accept": "application/zip"})
        try:
            with urlopen(
                request, timeout=self.request_timeout_seconds
            ) as response, target.open("xb") as stream:
                declared = response.headers.get("Content-Length")
                if declared is not None and int(declared) > maximum_bytes:
                    raise MinerUError("MinerU archive exceeds configured size limit")
                written = 0
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    written += len(chunk)
                    if written > maximum_bytes:
                        raise MinerUError(
                            "MinerU archive exceeds configured size limit"
                        )
                    stream.write(chunk)
        except HTTPError as error:
            detail = error.read(self.MAX_ERROR_BODY).decode(
                "utf-8", errors="replace"
            )
            raise MinerUError(
                f"MinerU archive download HTTP {error.code}: {detail}"
            ) from error
        except (URLError, TimeoutError, socket.timeout, OSError) as error:
            if isinstance(error, FileExistsError):
                raise
            raise MinerUError(
                f"MinerU archive download failed: {type(error).__name__}: {error}"
            ) from error


__all__ = ["MinerUClient", "MinerUError", "MinerUJob"]
