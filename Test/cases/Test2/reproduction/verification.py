"""Domain verifiers for the Test2 blind reproduction bundle."""

from __future__ import annotations

import json
import math
import re
import unicodedata
from collections.abc import Mapping
from pathlib import Path, PurePosixPath
from typing import Any

from clef_sdk.model import (
    ArtifactRef,
    CheckResult,
    CheckStatus,
    FrozenDict,
    VerifierSpec,
)
from clef_sdk.verification import (
    VerificationContext,
    default_registry,
    digest_path,
    uri_to_path,
)

from .analysis import build_validation_report

_MISSING = object()


def _strict_json(path: Path) -> Mapping[str, Any]:
    def reject_constant(value: str) -> None:
        raise ValueError(f"non-finite JSON number: {value}")

    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    with path.open("r", encoding="utf-8") as stream:
        value = json.load(
            stream,
            parse_constant=reject_constant,
            object_pairs_hook=unique_object,
        )
    if not isinstance(value, dict):
        raise ValueError("JSON root must be an object")
    return value


def _output(context: VerificationContext, name: object) -> tuple[ArtifactRef, Path]:
    if not isinstance(name, str) or not name:
        raise ValueError("output parameter must be a non-empty string")
    artifact = context.outputs.get(name)
    if artifact is None:
        raise ValueError(f"missing output: {name}")
    path = uri_to_path(artifact.uri).resolve(strict=True)
    if not path.is_relative_to(context.workspace):
        raise ValueError(f"output escapes task workspace: {path}")
    return artifact, path


def _input_path(context: VerificationContext, name: object) -> Path:
    if not isinstance(name, str) or not name:
        raise ValueError("input parameter must be a non-empty string")
    value = context.task.inputs.get(name)
    if not isinstance(value, ArtifactRef):
        raise ValueError(f"input {name!r} is not a bound ArtifactRef")
    return uri_to_path(value.uri).resolve(strict=True)


def _result(
    name: str,
    problems: list[str],
    evidence: tuple[ArtifactRef, ...],
    required: bool,
) -> CheckResult:
    return CheckResult(
        name=name,
        status=CheckStatus.PASSED if not problems else CheckStatus.FAILED,
        message=(
            "reproduction consistency passed"
            if not problems
            else "; ".join(problems[:8])
        ),
        required=required,
        score=1.0 if not problems else 0.0,
        evidence=evidence,
        details=FrozenDict[Any]({"problems": problems[:100]}),
    )


def _public_contract(
    context: VerificationContext,
    name: str,
) -> Mapping[str, Any] | None:
    benchmark = _strict_json(_input_path(context, "benchmark_spec"))
    value = benchmark.get(name)
    return value if isinstance(value, Mapping) else None


def _nonempty_strings(value: object) -> list[str] | None:
    if not isinstance(value, list) or not all(
        isinstance(item, str) and item for item in value
    ):
        return None
    return value


def _text_satisfies_alternatives(
    text: object,
    raw_groups: object,
    *,
    label: str,
) -> list[str]:
    """Require at least one case-insensitive token from every public group."""

    folded = _normalized_contract_text(text)
    if not isinstance(raw_groups, list):
        return [f"public contract has invalid alternatives for {label}"]
    problems: list[str] = []
    for index, raw_group in enumerate(raw_groups):
        group = _nonempty_strings(raw_group)
        if group is None:
            problems.append(
                f"public contract has invalid alternative group {label}[{index}]"
            )
        elif not any(
            _normalized_contract_text(token) in folded for token in group
        ):
            problems.append(
                f"{label} omits one of the public alternatives {group}"
            )
    return problems


def _normalized_contract_text(value: object) -> str:
    """Normalize harmless scientific typography and spacing differences."""

    text = unicodedata.normalize("NFKC", str(value)).casefold()
    text = text.translate(str.maketrans({"ν": "nu", "μ": "u", "π": "pi"}))
    text = re.sub(r"\s+", " ", text)
    text = re.sub(r"\s*([=:])\s*", r"\1", text)
    text = re.sub(r"\s+%", "%", text)
    return text.strip()


def _has_nonempty_content(value: object) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, (list, dict)):
        return bool(value)
    return value is not None


def _assessment_claim_ids(
    value: object,
    policy: Mapping[str, Any],
) -> tuple[set[str], list[str]]:
    """Parse the public claim entry format without constraining prose."""
    problems: list[str] = []
    entries = _nonempty_strings(value)
    if entries is None:
        return set(), ["assessment validated_claims must be non-empty strings"]
    raw_format = policy.get("entry_format")
    if not isinstance(raw_format, Mapping):
        return set(), ["public validated claim entry_format is invalid"]
    plain_allowed = raw_format.get("plain_claim_id_allowed")
    separator = raw_format.get("described_claim_separator")
    description_required = raw_format.get(
        "described_claim_requires_nonempty_description"
    )
    if (
        plain_allowed is not True
        or not isinstance(separator, str)
        or not separator
        or description_required is not True
    ):
        return set(), ["public validated claim entry_format is unsupported"]

    claim_ids: set[str] = set()
    for index, raw_entry in enumerate(entries):
        entry = raw_entry.strip()
        if separator in entry:
            raw_claim_id, description = entry.split(separator, 1)
            claim_id = raw_claim_id.strip()
            if not description.strip():
                problems.append(
                    f"validated_claims[{index}] has an empty description"
                )
        else:
            claim_id = entry
        if not claim_id:
            problems.append(f"validated_claims[{index}] has no claim ID")
            continue
        if claim_id in claim_ids:
            problems.append(f"duplicate validated claim ID: {claim_id}")
        claim_ids.add(claim_id)
    return claim_ids, problems


def _additional_claim_evidence_problems(
    context: VerificationContext,
    claim_contract: Mapping[str, Any],
) -> list[str]:
    """Verify that an optional final claim exists in a bound upstream artifact."""
    claim_id = claim_contract.get("claim_id")
    source_input = claim_contract.get("evidence_input")
    collection_name = claim_contract.get("evidence_collection")
    id_field = claim_contract.get("evidence_id_field")
    if not all(
        isinstance(value, str) and value
        for value in (claim_id, source_input, collection_name, id_field)
    ):
        return ["public additional validated claim evidence selector is invalid"]
    try:
        source = _strict_json(_input_path(context, source_input))
    except (OSError, ValueError) as error:
        return [f"evidence for additional claim {claim_id} is unavailable: {error}"]
    collection = source.get(collection_name)
    if not isinstance(collection, list):
        return [
            f"evidence for additional claim {claim_id} has no "
            f"{collection_name} collection"
        ]
    matches = [
        item
        for item in collection
        if isinstance(item, Mapping) and item.get(id_field) == claim_id
    ]
    if len(matches) != 1:
        return [
            f"additional claim {claim_id} has {len(matches)} upstream "
            "evidence records; expected one"
        ]
    record = matches[0]
    problems: list[str] = []

    required_values = claim_contract.get("required_field_values", {})
    if not isinstance(required_values, Mapping):
        problems.append(
            f"public required_field_values for {claim_id} is invalid"
        )
    else:
        for field, expected in required_values.items():
            if not isinstance(field, str) or not field:
                problems.append(
                    f"public required field name for {claim_id} is invalid"
                )
            elif record.get(field, _MISSING) != expected:
                problems.append(
                    f"additional claim {claim_id} lacks evidence "
                    f"{field}={expected!r}"
                )

    required_members = claim_contract.get("required_list_members", {})
    if not isinstance(required_members, Mapping):
        problems.append(
            f"public required_list_members for {claim_id} is invalid"
        )
    else:
        for field, raw_expected in required_members.items():
            expected = _nonempty_strings(raw_expected)
            actual = record.get(field)
            if (
                not isinstance(field, str)
                or not field
                or expected is None
                or not isinstance(actual, list)
                or not all(item in actual for item in expected)
            ):
                problems.append(
                    f"additional claim {claim_id} lacks required "
                    f"evidence members in {field}"
                )

    required_nonempty = _nonempty_strings(
        claim_contract.get("required_nonempty_fields", [])
    )
    if required_nonempty is None:
        problems.append(
            f"public required_nonempty_fields for {claim_id} is invalid"
        )
    else:
        for field in required_nonempty:
            if not _has_nonempty_content(record.get(field)):
                problems.append(
                    f"additional claim {claim_id} has no evidence in {field}"
                )
    return problems


def _numbers_close(left: Any, right: Any) -> bool:
    if isinstance(left, bool) or isinstance(right, bool):
        return left is right
    if isinstance(left, (int, float)) and isinstance(right, (int, float)):
        return math.isclose(
            float(left),
            float(right),
            rel_tol=1e-10,
            abs_tol=1e-10,
        )
    if isinstance(left, dict) and isinstance(right, dict):
        return left.keys() == right.keys() and all(
            _numbers_close(left[key], right[key]) for key in left
        )
    if isinstance(left, list) and isinstance(right, list):
        return len(left) == len(right) and all(
            _numbers_close(a, b) for a, b in zip(left, right, strict=True)
        )
    return left == right


def _path_value(value: Any, path: object) -> Any:
    """Resolve a public dot path, allowing dots inside mapping keys."""
    if path == "$":
        return value
    if not isinstance(path, str) or not path:
        return _MISSING
    current = value
    remaining = path
    while remaining:
        if isinstance(current, Mapping):
            # Keys such as the Fig. 10 ratio ``"0.1"`` contain a dot. Prefer
            # the complete remaining key before treating the dot as a
            # separator.
            if remaining in current:
                return current[remaining]
            head, separator, tail = remaining.partition(".")
            if head not in current:
                return _MISSING
            current = current[head]
            remaining = tail if separator else ""
            continue
        if isinstance(current, list):
            head, separator, tail = remaining.partition(".")
            try:
                index = int(head)
            except ValueError:
                return _MISSING
            if index < 0 or index >= len(current):
                return _MISSING
            current = current[index]
            remaining = tail if separator else ""
            continue
        return _MISSING
    return current


def _is_comparison_rule(value: object) -> bool:
    if isinstance(value, str) and value in {"exact", "nonempty"}:
        return True
    return (
        isinstance(value, Mapping)
        and set(value) == {"absolute"}
        and isinstance(value.get("absolute"), (int, float))
        and not isinstance(value.get("absolute"), bool)
        and math.isfinite(float(value["absolute"]))
        and float(value["absolute"]) >= 0.0
    )


def _comparison_rule(comparison: object, path: str) -> object:
    """Return the comparison rule for one path in the public contract."""
    if _is_comparison_rule(comparison):
        return comparison
    if not isinstance(comparison, Mapping):
        return _MISSING
    direct = comparison.get(path, _MISSING)
    if _is_comparison_rule(direct):
        return direct
    # ``$`` is useful when all required paths share one explicit fallback.
    fallback = comparison.get("$", _MISSING)
    if _is_comparison_rule(fallback):
        return fallback
    return _MISSING


def _value_semantic(
    check_contract: Mapping[str, Any],
    path: str,
) -> Mapping[str, Any] | object | None:
    raw_semantics = check_contract.get("value_semantics")
    if raw_semantics is None:
        semantic = check_contract.get("default_value_semantic")
    elif not isinstance(raw_semantics, Mapping):
        return _MISSING
    else:
        semantic = raw_semantics.get(
            path,
            check_contract.get("default_value_semantic"),
        )
    if semantic is None:
        return None
    return semantic if isinstance(semantic, Mapping) else _MISSING


def _normalized_sha256(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    folded = value.casefold()
    if folded.startswith("sha256:"):
        folded = folded.removeprefix("sha256:")
    return folded if re.fullmatch(r"[0-9a-f]{64}", folded) else None


def _semantic_value(
    value: object,
    semantic: Mapping[str, Any],
) -> tuple[object, str | None]:
    """Normalize one explicitly published alternate value representation."""
    kind = semantic.get("kind")
    representations = semantic.get("accepted_representations")
    if not isinstance(representations, list) or not representations:
        return _MISSING, "public accepted_representations is invalid"

    if kind == "sha256_digest":
        for representation in representations:
            if not isinstance(representation, Mapping):
                return _MISSING, "public sha256 representation is invalid"
            value_type = representation.get("type")
            encoding = representation.get("encoding")
            if encoding != "hex64_or_sha256_prefixed":
                return _MISSING, "public sha256 encoding is unsupported"
            candidate: object = _MISSING
            if value_type == "string" and isinstance(value, str):
                candidate = value
            elif value_type == "object" and isinstance(value, Mapping):
                value_path = representation.get("value_path")
                if not isinstance(value_path, str) or not value_path:
                    return _MISSING, "public sha256 object value_path is invalid"
                candidate = _path_value(value, value_path)
            if candidate is not _MISSING:
                digest = _normalized_sha256(candidate)
                if digest is None:
                    return _MISSING, "does not contain a valid SHA-256 digest"
                return digest, None
        return _MISSING, "does not match public SHA-256 representations"

    if kind == "scope_present":
        for representation in representations:
            if not isinstance(representation, Mapping):
                return _MISSING, "public scope representation is invalid"
            value_type = representation.get("type")
            if value_type == "boolean" and isinstance(value, bool):
                required = representation.get("const")
                if required is not True:
                    return _MISSING, "public scope boolean const is invalid"
                if value is required:
                    return True, None
            elif value_type == "object" and isinstance(value, Mapping):
                minimum = representation.get("minimum_properties")
                if (
                    not isinstance(minimum, int)
                    or isinstance(minimum, bool)
                    or minimum < 1
                ):
                    return _MISSING, "public scope minimum_properties is invalid"
                if len(value) >= minimum:
                    return True, None
        return _MISSING, "does not match public scope-present representations"

    if kind == "finite_number":
        if (
            len(representations) != 1
            or not isinstance(representations[0], Mapping)
            or representations[0].get("type") != "number"
        ):
            return _MISSING, "public finite-number representation is invalid"
        if not _is_json_type(value, "number"):
            return _MISSING, "is not a finite number"
        return float(value), None

    if kind == "blocked_explanation":
        for representation in representations:
            if not isinstance(representation, Mapping):
                return _MISSING, "public blocked representation is invalid"
            value_type = representation.get("type")
            if value_type == "string" and isinstance(value, str):
                minimum = representation.get("minimum_length")
                if (
                    not isinstance(minimum, int)
                    or isinstance(minimum, bool)
                    or minimum < 1
                ):
                    return _MISSING, "public blocked minimum_length is invalid"
                if len(value.strip()) >= minimum:
                    return True, None
            elif value_type == "object" and isinstance(value, Mapping):
                minimum = representation.get("minimum_properties")
                if (
                    not isinstance(minimum, int)
                    or isinstance(minimum, bool)
                    or minimum < 1
                ):
                    return _MISSING, "public blocked minimum_properties is invalid"
                if len(value) >= minimum:
                    return True, None
        return _MISSING, "does not match public blocked representations"

    return _MISSING, f"public value semantic kind is unsupported: {kind!r}"


def _is_json_type(value: object, type_name: object) -> bool:
    if type_name == "array":
        return isinstance(value, list)
    if type_name == "number":
        return (
            isinstance(value, (int, float))
            and not isinstance(value, bool)
            and math.isfinite(float(value))
        )
    return False


def _representation_group_declares_path(
    check_contract: Mapping[str, Any],
    path: str,
) -> bool:
    raw_groups = check_contract.get("representation_groups")
    if not isinstance(raw_groups, list):
        return False
    for group in raw_groups:
        if not isinstance(group, Mapping):
            continue
        base_path = group.get("base_path")
        coordinate_field = group.get("coordinate_field")
        value_fields = group.get("value_fields")
        if (
            isinstance(base_path, str)
            and isinstance(coordinate_field, str)
            and isinstance(value_fields, list)
            and path
            in {
                f"{base_path}.{coordinate_field}",
                *(
                    f"{base_path}.{value_field}"
                    for value_field in value_fields
                    if isinstance(value_field, str)
                ),
            }
        ):
            return True
    return False


def _sampled_profile_contexts(
    actual_check: Mapping[str, Any],
    host_check: Mapping[str, Any],
    check_contract: Mapping[str, Any],
    comparison: object,
    field: str,
) -> tuple[dict[str, dict[str, Any]], list[str]]:
    """Select a publicly declared array or scalar sampled-profile form."""
    raw_groups = check_contract.get("representation_groups")
    if raw_groups is None:
        return {}, []
    if not isinstance(raw_groups, list):
        return {}, ["public representation_groups is invalid"]

    contexts: dict[str, dict[str, Any]] = {}
    problems: list[str] = []
    for index, group in enumerate(raw_groups):
        label = f"public representation_groups[{index}]"
        if not isinstance(group, Mapping):
            problems.append(f"{label} is not an object")
            continue
        base_path = group.get("base_path")
        coordinate_field = group.get("coordinate_field")
        value_fields = _nonempty_strings(group.get("value_fields"))
        sample_coordinates = group.get("sample_coordinates")
        raw_representations = group.get("accepted_representations")
        if (
            group.get("semantic") != "sampled_profile"
            or not isinstance(base_path, str)
            or not base_path
            or not isinstance(coordinate_field, str)
            or not coordinate_field
            or value_fields is None
            or group.get("observed_expected_alignment")
            != "same_representation_and_coordinates"
            or group.get("additional_properties_allowed") is not True
            or not isinstance(sample_coordinates, list)
            or not sample_coordinates
            or not all(
                isinstance(value, (int, float))
                and not isinstance(value, bool)
                and math.isfinite(float(value))
                for value in sample_coordinates
            )
            or not isinstance(raw_representations, list)
            or not raw_representations
        ):
            problems.append(f"{label} is invalid")
            continue
        actual_container = _path_value(actual_check.get(field), base_path)
        host_container = _path_value(host_check.get(field), base_path)
        if not isinstance(actual_container, Mapping):
            problems.append(f"{field}.{base_path} is not an object")
            continue
        if not isinstance(host_container, Mapping):
            problems.append(f"{field}.{base_path} has no host profile")
            continue
        coordinate = actual_container.get(coordinate_field, _MISSING)
        values = {
            value_field: actual_container.get(value_field, _MISSING)
            for value_field in value_fields
        }
        matching: list[Mapping[str, Any]] = []
        for representation in raw_representations:
            if not isinstance(representation, Mapping):
                problems.append(f"{label} has an invalid representation")
                continue
            representation_name = representation.get("name")
            expected_coordinate_scope = {
                "profile": "all_sample_coordinates",
                "single_point": "interior_sample_coordinates",
            }.get(representation_name)
            if (
                expected_coordinate_scope is None
                or representation.get("coordinate_scope")
                != expected_coordinate_scope
            ):
                problems.append(
                    f"{label} has an invalid public coordinate scope"
                )
                continue
            if representation.get("phi_reference") != "endpoint_zero_eq69":
                problems.append(
                    f"{label} must publish the Eq. (69) endpoint-zero convention"
                )
                continue
            if _is_json_type(
                coordinate, representation.get("coordinate_type")
            ) and all(
                _is_json_type(value, representation.get("value_type"))
                for value in values.values()
            ):
                matching.append(representation)
        if len(matching) != 1:
            problems.append(
                f"{field}.{base_path} does not match exactly one public "
                "accepted representation"
            )
            continue

        representation = matching[0]
        representation_name = representation.get("name")
        coordinate_rule = _comparison_rule(
            comparison,
            f"{base_path}.{coordinate_field}",
        )
        if coordinate_rule is _MISSING:
            problems.append(
                f"{label} has no comparison for {base_path}.{coordinate_field}"
            )
            continue
        if representation_name == "profile":
            if not all(
                isinstance(value, list) and len(value) == len(coordinate)
                for value in values.values()
            ):
                problems.append(
                    f"{field}.{base_path} profile arrays have unequal lengths"
                )
                continue
            differences = _semantic_differences(
                coordinate,
                sample_coordinates,
                coordinate_rule,
            )
            if differences:
                problems.extend(
                    f"{field}.{base_path}.{coordinate_field}: {difference}"
                    for difference in differences
                )
                continue
            contexts[base_path] = {
                "name": "profile",
                "host_container": host_container,
            }
            continue
        if representation_name != "single_point":
            problems.append(f"{label} has unsupported representation name")
            continue

        absolute = (
            float(coordinate_rule["absolute"])
            if isinstance(coordinate_rule, Mapping)
            else 0.0
        )
        public_matches = [
            position
            for position, public_x in enumerate(sample_coordinates)
            if abs(float(coordinate) - float(public_x)) <= absolute
        ]
        host_coordinates = host_container.get(coordinate_field)
        if len(public_matches) != 1 or not isinstance(host_coordinates, list):
            problems.append(
                f"{field}.{base_path}.{coordinate_field} is not one public "
                "sample coordinate"
            )
            continue
        public_index = public_matches[0]
        if public_index in {0, len(sample_coordinates) - 1}:
            problems.append(
                f"{field}.{base_path}.{coordinate_field} must be an interior "
                "sample so the Eq. (69) endpoint-zero gauge is nontrivial"
            )
            continue
        host_matches = [
            position
            for position, host_x in enumerate(host_coordinates)
            if isinstance(host_x, (int, float))
            and not isinstance(host_x, bool)
            and abs(float(coordinate) - float(host_x)) <= absolute
        ]
        if len(host_matches) != 1:
            problems.append(
                f"{field}.{base_path}.{coordinate_field} has no unique host sample"
            )
            continue
        contexts[base_path] = {
            "name": "single_point",
            "host_container": host_container,
            "host_index": host_matches[0],
            "public_index": public_index,
            "coordinate_field": coordinate_field,
            "value_fields": value_fields,
        }
    return contexts, problems


def _sampled_profile_alignment_problems(
    check_contract: Mapping[str, Any],
    contexts_by_field: Mapping[str, Mapping[str, Mapping[str, Any]]],
) -> list[str]:
    """Require observed and expected sampled profiles to be comparable."""

    raw_groups = check_contract.get("representation_groups")
    if raw_groups is None:
        return []
    if not isinstance(raw_groups, list):
        return ["public representation_groups is invalid"]

    problems: list[str] = []
    observed_contexts = contexts_by_field.get("observed", {})
    expected_contexts = contexts_by_field.get("expected", {})
    for index, group in enumerate(raw_groups):
        if not isinstance(group, Mapping):
            continue
        base_path = group.get("base_path")
        if not isinstance(base_path, str):
            continue
        observed = observed_contexts.get(base_path)
        expected = expected_contexts.get(base_path)
        if observed is None or expected is None:
            continue
        if observed.get("name") != expected.get("name"):
            problems.append(
                f"observed/expected {base_path} must use the same public "
                "representation"
            )
            continue
        if (
            observed.get("name") == "single_point"
            and observed.get("public_index") != expected.get("public_index")
        ):
            problems.append(
                f"observed/expected {base_path} single points must use the "
                "same public coordinate"
            )
        if (
            group.get("observed_expected_alignment")
            != "same_representation_and_coordinates"
        ):
            problems.append(
                f"public representation_groups[{index}] has an invalid "
                "observed/expected alignment"
            )
    return problems


def _represented_host_value(
    host_value: object,
    path: str,
    contexts: Mapping[str, Mapping[str, Any]],
) -> object:
    for base_path, representation in contexts.items():
        prefix = f"{base_path}."
        if not path.startswith(prefix) or representation.get("name") != "single_point":
            continue
        relative_path = path.removeprefix(prefix)
        fields = {
            representation.get("coordinate_field"),
            *representation.get("value_fields", []),
        }
        if relative_path not in fields or not isinstance(host_value, list):
            continue
        host_index = representation.get("host_index")
        if isinstance(host_index, int) and 0 <= host_index < len(host_value):
            return host_value[host_index]
    return host_value


def _is_nonempty(value: object) -> bool:
    if value is None:
        return False
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, (Mapping, list, tuple, set)):
        return bool(value)
    return True


def _evidence_has_anchor(evidence_text: str, anchor: str) -> bool:
    folded_anchor = anchor.casefold()
    if folded_anchor in evidence_text:
        return True
    if folded_anchor.startswith("eq"):
        equation_numbers = re.findall(r"\(\s*[\d-]+\s*\)", folded_anchor)
        return "eq" in evidence_text and all(
            number.replace(" ", "") in evidence_text.replace(" ", "")
            for number in equation_numbers
        )
    if folded_anchor.startswith("fig"):
        figure_ids = re.findall(r"\b(?:s\d+|\d+)\b", folded_anchor)
        return "fig" in evidence_text and all(
            figure_id in evidence_text for figure_id in figure_ids
        )
    return False


def _semantic_differences(
    actual: Any,
    host_value: Any,
    rule: object,
    *,
    path: str = "",
) -> list[str]:
    """Compare one contracted value while ignoring mapping key order."""
    if rule == "nonempty":
        return [] if _is_nonempty(actual) else [f"{path or '$'} is empty"]
    absolute = float(rule["absolute"]) if isinstance(rule, Mapping) else None

    if isinstance(host_value, bool) or isinstance(actual, bool):
        if actual is host_value:
            return []
        return [f"{path or '$'} differs (expected {host_value!r}, got {actual!r})"]
    if isinstance(host_value, (int, float)):
        if not isinstance(actual, (int, float)) or isinstance(actual, bool):
            return [f"{path or '$'} is not numeric"]
        actual_number = float(actual)
        host_number = float(host_value)
        if not math.isfinite(actual_number):
            return [f"{path or '$'} is not finite"]
        if absolute is None:
            if actual == host_value:
                return []
            return [
                f"{path or '$'} differs "
                f"(expected {host_value!r}, got {actual!r})"
            ]
        error = abs(actual_number - host_number)
        if error <= absolute:
            return []
        return [
            f"{path or '$'} differs "
            f"(absolute error {error:.12g} > {absolute:.12g})"
        ]
    if isinstance(host_value, Mapping):
        if not isinstance(actual, Mapping):
            return [f"{path or '$'} is not an object"]
        differences: list[str] = []
        for key, expected_item in host_value.items():
            child_path = f"{path}.{key}" if path else str(key)
            if key not in actual:
                differences.append(f"{child_path} is missing")
                continue
            differences.extend(
                _semantic_differences(
                    actual[key],
                    expected_item,
                    rule,
                    path=child_path,
                )
            )
        return differences
    if isinstance(host_value, list):
        if not isinstance(actual, list):
            return [f"{path or '$'} is not an array"]
        if len(actual) != len(host_value):
            return [
                f"{path or '$'} length differs "
                f"(expected {len(host_value)}, got {len(actual)})"
            ]
        differences = []
        for index, (actual_item, expected_item) in enumerate(
            zip(actual, host_value, strict=True)
        ):
            child_path = f"{path}.{index}" if path else str(index)
            differences.extend(
                _semantic_differences(
                    actual_item,
                    expected_item,
                    rule,
                    path=child_path,
                )
            )
        return differences
    if actual == host_value:
        return []
    return [f"{path or '$'} differs (expected {host_value!r}, got {actual!r})"]


def evidence_ledger_consistency(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate source identity and the public SI dependency inventory."""

    output_artifact, output_path = _output(context, spec.parameters.get("output"))
    ledger = _strict_json(output_path)
    contract = _public_contract(context, "evidence_output_contract")
    if contract is None:
        return _result(
            "evidence_ledger_consistency",
            ["benchmark_spec omits evidence_output_contract"],
            (output_artifact,),
            spec.required,
        )

    problems: list[str] = []
    source_contract = contract.get("source_identity")
    source_identity = ledger.get("source_identity")
    if not isinstance(source_contract, Mapping) or not isinstance(
        source_identity, Mapping
    ):
        problems.append("source_identity is missing or contract is invalid")
    else:
        if source_identity.get("doi") != source_contract.get("doi"):
            problems.append("source_identity.doi differs from public contract")
        digest_inputs = source_contract.get("digest_inputs")
        if not isinstance(digest_inputs, Mapping):
            problems.append("public source_identity.digest_inputs is invalid")
        else:
            for digest_field, input_name in digest_inputs.items():
                if not isinstance(digest_field, str) or not isinstance(
                    input_name, str
                ):
                    problems.append("public digest input mapping is invalid")
                    continue
                expected_digest = digest_path(_input_path(context, input_name))
                if source_identity.get(digest_field) != expected_digest:
                    problems.append(
                        f"source_identity.{digest_field} differs from bound input"
                    )

    policy_contract = contract.get("policy")
    policy = ledger.get("policy")
    if not isinstance(policy_contract, Mapping) or not isinstance(policy, Mapping):
        problems.append("evidence policy is missing or contract is invalid")
    else:
        for key, expected_value in policy_contract.items():
            if policy.get(key) != expected_value:
                problems.append(f"evidence policy.{key} differs from public contract")

    raw_contract_dependencies = contract.get("dependencies")
    if not isinstance(raw_contract_dependencies, list) or not all(
        isinstance(item, Mapping)
        and isinstance(item.get("dependency_id"), str)
        and item.get("dependency_id")
        for item in raw_contract_dependencies
    ):
        problems.append("public evidence dependencies are invalid")
        raw_contract_dependencies = []
    raw_dependencies = ledger.get("si_dependencies")
    if not isinstance(raw_dependencies, list):
        problems.append("si_dependencies must be an array")
        raw_dependencies = []
    dependencies = {
        item.get("dependency_id"): item
        for item in raw_dependencies
        if isinstance(item, Mapping)
        and isinstance(item.get("dependency_id"), str)
    }
    expected_dependencies = {
        item["dependency_id"]: item for item in raw_contract_dependencies
    }
    if (
        set(dependencies) != set(expected_dependencies)
        or len(raw_dependencies) != len(expected_dependencies)
    ):
        problems.append(
            "evidence dependency IDs differ from public contract: "
            f"expected {sorted(expected_dependencies)}"
        )
    for dependency_id, expected in expected_dependencies.items():
        dependency = dependencies.get(dependency_id, {})
        if dependency.get("recoverability") != expected.get("recoverability"):
            problems.append(f"{dependency_id} recoverability differs from contract")
        for field, tokens_key in (
            ("main_text_anchor", "required_anchor_tokens"),
            ("confirmed_scope", "required_scope_tokens"),
        ):
            raw_tokens = _nonempty_strings(expected.get(tokens_key))
            if raw_tokens is None:
                problems.append(
                    f"public {dependency_id}.{tokens_key} is invalid"
                )
                continue
            folded = _normalized_contract_text(dependency.get(field, ""))
            for token in raw_tokens:
                if _normalized_contract_text(token) not in folded:
                    problems.append(
                        f"{dependency_id} {field} omits public token {token}"
                    )
    return _result(
        "evidence_ledger_consistency",
        problems,
        (output_artifact,),
        spec.required,
    )


def _legacy_numeric_comparison(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> list[str]:
    """Preserve the pre-contract verifier for old benchmark bundles."""
    problems: list[str] = []
    if not _numbers_close(dict(actual), dict(expected)):
        problems.append(
            "validation-report.json differs from independent verifier recomputation"
        )
    expected_blocked = {
        "LIMIT-OBLIQUE-001",
        "LIMIT-FEA-001",
        "LIMIT-EXPERIMENT-001",
    }
    actual_blocked = {
        item.get("check_id")
        for item in actual.get("checks", [])
        if isinstance(item, dict) and item.get("status") == "BLOCKED"
    }
    if actual_blocked != expected_blocked:
        problems.append("blocked check set differs from the frozen benchmark")
    return problems


def _numeric_summary_from_contract(
    contract: Mapping[str, Any],
    checks: Mapping[str, Mapping[str, Any]],
) -> tuple[dict[str, int], str, bool, bool, list[str]]:
    """Apply the summary rules published in the blind benchmark."""

    raw = contract.get("summary_derivation")
    if not isinstance(raw, Mapping):
        return {}, "", False, False, [
            "public numeric summary_derivation is invalid"
        ]
    statuses = _nonempty_strings(raw.get("counted_statuses"))
    if (
        statuses is None
        or len(statuses) != len(set(statuses))
        or raw.get("additional_count_keys_ignored") is not True
    ):
        return {}, "", False, False, [
            "public numeric summary_derivation counts are invalid"
        ]
    counts = {
        status: sum(check.get("status") == status for check in checks.values())
        for status in statuses
    }

    raw_precedence = raw.get("status_precedence")
    derived_status: str | None = None
    if not isinstance(raw_precedence, list) or not raw_precedence:
        problems = ["public numeric summary status_precedence is invalid"]
    else:
        problems = []
        for index, rule in enumerate(raw_precedence):
            if not isinstance(rule, Mapping):
                problems.append(
                    f"public numeric summary status rule {index} is invalid"
                )
                continue
            if set(rule) == {"positive_count", "result"}:
                count_name = rule.get("positive_count")
                result = rule.get("result")
                if (
                    not isinstance(count_name, str)
                    or count_name not in counts
                    or not isinstance(result, str)
                    or not result
                ):
                    problems.append(
                        f"public numeric summary status rule {index} is invalid"
                    )
                elif derived_status is None and counts[count_name] > 0:
                    derived_status = result
                continue
            if (
                set(rule) == {"otherwise"}
                and index == len(raw_precedence) - 1
                and isinstance(rule.get("otherwise"), str)
                and rule.get("otherwise")
            ):
                if derived_status is None:
                    derived_status = str(rule["otherwise"])
                continue
            problems.append(
                f"public numeric summary status rule {index} is invalid"
            )
    if derived_status is None:
        problems.append("public numeric summary has no applicable status rule")
        derived_status = ""

    def true_when_zero(label: str) -> bool:
        rule = raw.get(label)
        names = (
            _nonempty_strings(rule.get("true_when_zero"))
            if isinstance(rule, Mapping) and set(rule) == {"true_when_zero"}
            else None
        )
        if names is None or any(name not in counts for name in names):
            problems.append(
                f"public numeric summary {label} rule is invalid"
            )
            return False
        return all(counts[name] == 0 for name in names)

    return (
        counts,
        derived_status,
        true_when_zero("passed"),
        true_when_zero("fully_reproduced"),
        problems,
    )


def numeric_reproduction_consistency(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Validate public numeric semantics against an independent host run."""
    output_artifact, output_path = _output(context, spec.parameters.get("output"))
    pdf_path = _input_path(context, spec.parameters.get("pdf_input"))
    markdown_path = _input_path(context, spec.parameters.get("markdown_input"))
    actual = _strict_json(output_path)
    expected = json.loads(json.dumps(build_validation_report(pdf_path, markdown_path)))
    benchmark = _strict_json(_input_path(context, "benchmark_spec"))
    contract = benchmark.get("numeric_output_contract")
    problems: list[str] = []
    raw_actual_checks = actual.get("checks")
    if not isinstance(raw_actual_checks, list):
        raw_actual_checks = []
        problems.append("checks must be an array")
    actual_checks: dict[str, Mapping[str, Any]] = {}
    actual_order: list[str] = []
    for index, item in enumerate(raw_actual_checks):
        if not isinstance(item, Mapping):
            problems.append(f"check at index {index} is not an object")
            continue
        check_id = item.get("check_id")
        if not isinstance(check_id, str) or not check_id:
            problems.append(f"check at index {index} has no check_id")
            continue
        actual_order.append(check_id)
        if check_id in actual_checks:
            problems.append(f"{check_id} check_id is duplicated")
            continue
        actual_checks[check_id] = item

    if not isinstance(contract, Mapping):
        problems.extend(_legacy_numeric_comparison(actual, expected))
        return _result(
            "numeric_reproduction_consistency",
            problems,
            (output_artifact,),
            spec.required,
        )

    raw_contract_checks = contract.get("checks")
    if not isinstance(raw_contract_checks, list):
        problems.append("benchmark numeric_output_contract.checks is invalid")
        raw_contract_checks = []
    contract_checks: dict[str, Mapping[str, Any]] = {}
    contract_order: list[str] = []
    for index, item in enumerate(raw_contract_checks):
        if not isinstance(item, Mapping):
            problems.append(f"numeric contract check {index} is not an object")
            continue
        check_id = item.get("check_id")
        if not isinstance(check_id, str) or not check_id:
            problems.append(f"numeric contract check {index} has no check_id")
            continue
        contract_order.append(check_id)
        if check_id in contract_checks:
            problems.append(f"numeric contract duplicates {check_id}")
            continue
        contract_checks[check_id] = item

    expected_ids = set(contract_checks)
    actual_ids = set(actual_checks)
    if actual_ids != expected_ids:
        problems.append(
            "numeric check IDs differ from public contract: "
            f"missing={sorted(expected_ids - actual_ids)}, "
            f"extra={sorted(actual_ids - expected_ids)}"
        )
    order_significant = contract.get("check_order_significant", False)
    if not isinstance(order_significant, bool):
        problems.append(
            "benchmark numeric_output_contract.check_order_significant is invalid"
        )
    elif order_significant and actual_order != contract_order:
        problems.append("numeric check order differs from public contract")

    raw_host_checks = expected.get("checks", [])
    host_checks = {
        item.get("check_id"): item
        for item in raw_host_checks
        if isinstance(item, Mapping) and isinstance(item.get("check_id"), str)
    }
    for check_id, check_contract in contract_checks.items():
        actual_check = actual_checks.get(check_id)
        if actual_check is None:
            continue
        host_check = host_checks.get(check_id)
        if host_check is None:
            problems.append(
                f"{check_id} has no independent host recomputation"
            )
            continue
        contract_status = check_contract.get("status")
        if actual_check.get("status") != contract_status:
            problems.append(
                f"{check_id} status differs from public contract "
                f"(expected {contract_status!r}, "
                f"got {actual_check.get('status')!r})"
            )
        if host_check.get("status") != contract_status:
            problems.append(
                f"{check_id} public status disagrees with host recomputation"
            )

        comparison = check_contract.get("comparison")
        representation_contexts_by_field: dict[
            str, dict[str, dict[str, Any]]
        ] = {}
        for field, paths_key in (
            ("observed", "required_observed_paths"),
            ("expected", "required_expected_paths"),
        ):
            representation_contexts, representation_problems = (
                _sampled_profile_contexts(
                    actual_check,
                    host_check,
                    check_contract,
                    comparison,
                    field,
                )
            )
            representation_contexts_by_field[field] = representation_contexts
            problems.extend(
                f"{check_id} {problem}"
                for problem in representation_problems
            )
            raw_paths = check_contract.get(paths_key)
            if not isinstance(raw_paths, list) or not all(
                isinstance(path, str) and path for path in raw_paths
            ):
                problems.append(
                    f"{check_id} public {paths_key} is invalid"
                )
                continue
            for path in raw_paths:
                actual_value = _path_value(actual_check.get(field), path)
                if actual_value is _MISSING:
                    problems.append(f"{check_id} {field}.{path} is missing")
                    continue
                host_value = _path_value(host_check.get(field), path)
                if host_value is _MISSING:
                    problems.append(
                        f"{check_id} {field}.{path} has no host recomputation"
                    )
                    continue
                host_value = _represented_host_value(
                    host_value,
                    path,
                    representation_contexts,
                )
                semantic = _value_semantic(check_contract, path)
                if semantic is _MISSING:
                    problems.append(
                        f"{check_id} public value_semantics.{path} is invalid"
                    )
                    continue
                if semantic is None and not _representation_group_declares_path(
                    check_contract,
                    path,
                ):
                    problems.append(
                        f"{check_id} public contract omits accepted shapes "
                        f"for {field}.{path}"
                    )
                    continue
                if isinstance(semantic, Mapping):
                    actual_value, actual_semantic_error = _semantic_value(
                        actual_value,
                        semantic,
                    )
                    if actual_semantic_error is not None:
                        problems.append(
                            f"{check_id} {field}.{path} "
                            f"{actual_semantic_error}"
                        )
                        continue
                    host_value, host_semantic_error = _semantic_value(
                        host_value,
                        semantic,
                    )
                    if host_semantic_error is not None:
                        problems.append(
                            f"{check_id} public value_semantics.{path} "
                            f"rejects host recomputation: {host_semantic_error}"
                        )
                        continue
                rule = _comparison_rule(comparison, path)
                if rule is _MISSING:
                    problems.append(
                        f"{check_id} public comparison omits {field}.{path}"
                    )
                    continue
                for difference in _semantic_differences(
                    actual_value,
                    host_value,
                    rule,
                ):
                    problems.append(
                        f"{check_id} {field}.{path}: {difference}"
                    )
        problems.extend(
            f"{check_id} {problem}"
            for problem in _sampled_profile_alignment_problems(
                check_contract,
                representation_contexts_by_field,
            )
        )

        raw_anchors = check_contract.get("required_evidence_anchors")
        if not isinstance(raw_anchors, list) or not all(
            isinstance(anchor, str) and anchor for anchor in raw_anchors
        ):
            problems.append(
                f"{check_id} public required_evidence_anchors is invalid"
            )
        else:
            evidence = actual_check.get("evidence")
            evidence_text = (
                " ".join(str(item) for item in evidence)
                if isinstance(evidence, list)
                else ""
            ).casefold()
            for anchor in raw_anchors:
                if not _evidence_has_anchor(evidence_text, anchor):
                    problems.append(
                        f"{check_id} evidence omits public anchor {anchor}"
                    )

    policy = actual.get("policy")
    if (
        not isinstance(policy, Mapping)
        or policy.get("external_supplement_used") is not False
    ):
        problems.append("policy.external_supplement_used must be false")

    source_identity_policy = contract.get("source_identity_policy")
    if not isinstance(source_identity_policy, Mapping):
        problems.append("public numeric source_identity_policy is invalid")
    else:
        authoritative_checks = source_identity_policy.get(
            "authoritative_checks"
        )
        if (
            not isinstance(authoritative_checks, Mapping)
            or authoritative_checks.get("pdf") != "SRC-001"
            or authoritative_checks.get("markdown") != "SRC-002"
            or source_identity_policy.get("paper_object_is_presentation_only")
            is not True
        ):
            problems.append("public numeric source identity policy is invalid")

    (
        counts,
        derived_status,
        derived_passed,
        derived_fully_reproduced,
        summary_contract_problems,
    ) = _numeric_summary_from_contract(
        contract,
        actual_checks,
    )
    problems.extend(summary_contract_problems)
    summary = actual.get("summary")
    if not isinstance(summary, Mapping):
        problems.append("summary is missing")
    else:
        summary_counts = summary.get("counts")
        if not isinstance(summary_counts, Mapping) or any(
            summary_counts.get(status) != count
            for status, count in counts.items()
        ):
            problems.append(
                "summary.counts is inconsistent with numeric check statuses"
            )
        if summary.get("status") != derived_status:
            problems.append(
                "summary.status is inconsistent with numeric check statuses"
            )
        if summary.get("passed") is not derived_passed:
            problems.append(
                "summary.passed is inconsistent with numeric check statuses"
            )
        if (
            summary.get("fully_reproduced")
            is not derived_fully_reproduced
        ):
            problems.append(
                "summary.fully_reproduced is inconsistent with check statuses"
            )
    public_summary_status = contract.get("summary_status")
    if public_summary_status != derived_status:
        problems.append(
            "summary status differs from public numeric_output_contract"
        )
    return _result(
        "numeric_reproduction_consistency",
        problems,
        (output_artifact,),
        spec.required,
    )


def theory_inference_consistency(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Require complete, evidence-anchored polynomial and oblique theory."""

    json_artifact, json_path = _output(context, spec.parameters.get("json_output"))
    report_artifact, report_path = _output(
        context, spec.parameters.get("report_output")
    )
    theory = _strict_json(json_path)
    report = report_path.read_text(encoding="utf-8")
    benchmark = _strict_json(_input_path(context, "benchmark_spec"))
    contract = benchmark.get("theory_output_contract")
    if not isinstance(contract, dict):
        return _result(
            "theory_inference_consistency",
            ["benchmark_spec omits theory_output_contract"],
            (json_artifact, report_artifact),
            spec.required,
        )
    problems: list[str] = []
    expected_external = contract.get("external_supplement_used")
    if theory.get("external_supplement_used") is not expected_external:
        problems.append(
            "theory external_supplement_used differs from the public contract"
        )
    raw_sections = theory.get("sections", [])
    sections = {
        item.get("section_id"): item
        for item in raw_sections
        if isinstance(item, dict) and isinstance(item.get("section_id"), str)
    }
    raw_expected_sections = contract.get("sections")
    if not isinstance(raw_expected_sections, list) or not all(
        isinstance(item, dict)
        and isinstance(item.get("section_id"), str)
        and item.get("section_id")
        for item in raw_expected_sections
    ):
        return _result(
            "theory_inference_consistency",
            ["benchmark theory_output_contract.sections is invalid"],
            (json_artifact, report_artifact),
            spec.required,
        )
    expected_sections = {
        item["section_id"]: item for item in raw_expected_sections
    }
    expected_ids = set(expected_sections)
    if set(sections) != expected_ids or len(raw_sections) != len(expected_ids):
        problems.append(
            "theory section IDs differ from public contract: "
            f"expected {sorted(expected_ids)}"
        )

    for section_id, expected in expected_sections.items():
        section = sections.get(section_id, {})
        if section.get("status") != expected.get("status"):
            problems.append(f"{section_id} has the wrong status")
        if set(section.get("validation_ids", [])) != set(
            expected.get("validation_ids", [])
        ):
            problems.append(
                f"{section_id} validation IDs differ from public contract"
            )
        evidence = " ".join(str(item) for item in section.get("evidence", []))
        for anchor in expected.get("required_evidence_anchors", []):
            if str(anchor).casefold() not in evidence.casefold():
                problems.append(f"{section_id} omits evidence anchor {anchor}")
        reconstruction = str(section.get("reconstruction", ""))
        for token in expected.get("required_reconstruction_tokens", []):
            if str(token).casefold() not in reconstruction.casefold():
                problems.append(
                    f"{section_id} reconstruction omits {token}"
                )
        contract_text = " ".join(
            (
                reconstruction,
                *(
                    str(item)
                    for item in section.get("residual_unknowns", [])
                ),
            )
        ).casefold()
        for token in expected.get("required_contract_tokens", []):
            if str(token).casefold() not in contract_text:
                problems.append(f"{section_id} contract omits {token}")

    report_folded = report.casefold()
    for token in contract.get("required_report_anchors", []):
        if str(token).casefold() not in report_folded:
            problems.append(f"theory report omits semantic anchor {token}")
    for forbidden in contract.get("forbidden_claims", []):
        forbidden = str(forbidden).casefold()
        if (
            forbidden in report_folded
            or forbidden in json.dumps(theory, ensure_ascii=False).casefold()
        ):
            problems.append(f"theory promotes an unknown claim: {forbidden}")
    return _result(
        "theory_inference_consistency",
        problems,
        (json_artifact, report_artifact),
        spec.required,
    )


def methods_inference_consistency(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Verify known methods facts while preserving every missing field."""

    json_artifact, json_path = _output(context, spec.parameters.get("json_output"))
    report_artifact, report_path = _output(
        context, spec.parameters.get("report_output")
    )
    methods = _strict_json(json_path)
    report = report_path.read_text(encoding="utf-8")
    contract = _public_contract(context, "methods_output_contract")
    if contract is None:
        return _result(
            "methods_inference_consistency",
            ["benchmark_spec omits methods_output_contract"],
            (json_artifact, report_artifact),
            spec.required,
        )
    problems: list[str] = []
    raw_facts = methods.get("confirmed_facts", [])
    if not isinstance(raw_facts, list):
        raw_facts = []
        problems.append("confirmed_facts must be an array")
    facts = {
        item.get("fact_id"): item
        for item in raw_facts
        if isinstance(item, Mapping) and isinstance(item.get("fact_id"), str)
    }
    raw_contract_facts = contract.get("confirmed_facts")
    if not isinstance(raw_contract_facts, list) or not all(
        isinstance(item, Mapping)
        and isinstance(item.get("fact_id"), str)
        and item.get("fact_id")
        for item in raw_contract_facts
    ):
        problems.append("public methods confirmed_facts contract is invalid")
        raw_contract_facts = []
    expected_facts = {item["fact_id"]: item for item in raw_contract_facts}
    if set(facts) != set(expected_facts) or len(raw_facts) != len(expected_facts):
        problems.append(
            "confirmed method fact IDs differ from public contract: "
            f"expected {sorted(expected_facts)}"
        )
    for fact_id, expected in expected_facts.items():
        fact = facts.get(fact_id, {})
        problems.extend(
            _text_satisfies_alternatives(
                fact.get("value", ""),
                expected.get("required_value_alternatives"),
                label=f"method fact {fact_id} value",
            )
        )
        anchor_alternatives = _nonempty_strings(
            expected.get("required_anchor_alternatives")
        )
        if anchor_alternatives is None:
            problems.append(
                f"public method fact {fact_id} anchor alternatives are invalid"
            )
        elif not any(
            _normalized_contract_text(token)
            in _normalized_contract_text(fact.get("anchor", ""))
            for token in anchor_alternatives
        ):
            problems.append(
                f"method fact {fact_id} anchor omits one of "
                f"{anchor_alternatives}"
            )

    figure = methods.get("figure_s1", {})
    if not isinstance(figure, Mapping):
        figure = {}
        problems.append("figure_s1 must be an object")
    figure_contract = contract.get("figure_s1")
    if not isinstance(figure_contract, Mapping):
        figure_contract = {}
        problems.append("public methods figure_s1 contract is invalid")
    if figure.get("quantitative_reproduction") != figure_contract.get(
        "quantitative_reproduction"
    ):
        problems.append("Fig. S1 quantitative reproduction was promoted")
    figure_text = " ".join(
        (
            str(figure.get("comparison", "")),
            *(str(item) for item in figure.get("parameters", [])),
            str(figure.get("reported_outcome", "")),
        )
    )
    problems.extend(
        _text_satisfies_alternatives(
            figure_text,
            figure_contract.get("required_text_alternatives"),
            label="Fig. S1 contract",
        )
    )

    missing_text = " ".join(
        str(item) for item in methods.get("missing_fields", [])
    )
    problems.extend(
        _text_satisfies_alternatives(
            missing_text,
            contract.get("missing_field_categories"),
            label="missing methods inventory",
        )
    )
    if methods.get("replication_status") != contract.get("replication_status"):
        problems.append("methods replication status is not blocked")
    problems.extend(
        _text_satisfies_alternatives(
            report,
            contract.get("required_report_token_alternatives"),
            label="methods report",
        )
    )
    forbidden_claims = _nonempty_strings(contract.get("forbidden_claims"))
    if forbidden_claims is None:
        problems.append("public methods forbidden_claims contract is invalid")
        forbidden_claims = []
    report_folded = report.casefold()
    methods_folded = json.dumps(methods, ensure_ascii=False).casefold()
    for forbidden in forbidden_claims:
        forbidden = forbidden.casefold()
        if (
            forbidden in report_folded
            or forbidden in methods_folded
        ):
            problems.append(f"methods invents a blocked result: {forbidden}")
    return _result(
        "methods_inference_consistency",
        problems,
        (json_artifact, report_artifact),
        spec.required,
    )


def reproduction_bundle_consistency(
    spec: VerifierSpec,
    context: VerificationContext,
) -> CheckResult:
    """Cross-check the final supplement, assessment, and digest manifest."""

    supplement_artifact, supplement_path = _output(
        context, spec.parameters.get("supplement_output")
    )
    assessment_artifact, assessment_path = _output(
        context, spec.parameters.get("assessment_output")
    )
    manifest_artifact, manifest_path = _output(
        context, spec.parameters.get("manifest_output")
    )
    supplement = supplement_path.read_text(encoding="utf-8")
    assessment = _strict_json(assessment_path)
    manifest = _strict_json(manifest_path)
    validation = _strict_json(
        _input_path(
            context,
            "validate_paper_numerics_validation_report",
        )
    )
    contract = _public_contract(context, "final_output_contract")
    if contract is None:
        return _result(
            "reproduction_bundle_consistency",
            ["benchmark_spec omits final_output_contract"],
            (
                supplement_artifact,
                assessment_artifact,
                manifest_artifact,
            ),
            spec.required,
        )
    problems: list[str] = []

    problems.extend(
        _text_satisfies_alternatives(
            supplement,
            contract.get("required_section_token_alternatives"),
            label="supplement sections",
        )
    )
    declarations = _nonempty_strings(
        contract.get("blind_source_declaration_alternatives")
    )
    if declarations is None:
        problems.append("public blind-source declaration contract is invalid")
    elif not any(
        _normalized_contract_text(token)
        in _normalized_contract_text(supplement)
        for token in declarations
    ):
        problems.append("supplement omits the blind-source declaration")

    validation_checks = [
        item
        for item in validation.get("checks", [])
        if isinstance(item, Mapping)
    ]

    raw_claim_policy = contract.get("validated_claims_policy")
    if not isinstance(raw_claim_policy, Mapping):
        problems.append("public validated_claims_policy is invalid")
        claim_policy: Mapping[str, Any] = {}
    else:
        claim_policy = raw_claim_policy
    required_numeric_status = claim_policy.get("required_numeric_status")
    blocked_numeric_status = claim_policy.get("blocked_numeric_status")
    if (
        not isinstance(required_numeric_status, str)
        or not required_numeric_status
        or not isinstance(blocked_numeric_status, str)
        or not blocked_numeric_status
        or claim_policy.get("numeric_coverage") != "all"
        or claim_policy.get("blocked_claim_promotion") != "forbidden"
        or claim_policy.get("unlisted_claims") != "forbidden"
        or claim_policy.get("duplicate_claim_ids") != "forbidden"
    ):
        problems.append("public validated_claims_policy controls are invalid")
        required_numeric_status = "PASS"
        blocked_numeric_status = "BLOCKED"
    passed_ids = {
        item["check_id"]
        for item in validation_checks
        if item.get("status") == required_numeric_status
        and isinstance(item.get("check_id"), str)
        and item["check_id"]
    }
    blocked_ids = {
        item["check_id"]
        for item in validation_checks
        if item.get("status") == blocked_numeric_status
        and isinstance(item.get("check_id"), str)
        and item["check_id"]
    }
    validated_ids, validated_problems = _assessment_claim_ids(
        assessment.get("validated_claims"),
        claim_policy,
    )
    problems.extend(validated_problems)

    raw_additional_claims = claim_policy.get("permitted_additional_claims")
    if not isinstance(raw_additional_claims, list) or not all(
        isinstance(item, Mapping)
        and isinstance(item.get("claim_id"), str)
        and item.get("claim_id")
        for item in raw_additional_claims
    ):
        problems.append(
            "public permitted_additional_claims contract is invalid"
        )
        raw_additional_claims = []
    additional_contracts = {
        item["claim_id"]: item for item in raw_additional_claims
    }
    if len(additional_contracts) != len(raw_additional_claims):
        problems.append(
            "public permitted_additional_claims contains duplicate IDs"
        )

    missing_numeric_claims = passed_ids - validated_ids
    if missing_numeric_claims:
        problems.append(
            "assessment omits numeric PASS claims: "
            f"{sorted(missing_numeric_claims)}"
        )
    promoted_blocked_claims = blocked_ids & validated_ids
    if promoted_blocked_claims:
        problems.append(
            "assessment promotes numeric BLOCKED claims: "
            f"{sorted(promoted_blocked_claims)}"
        )
    unknown_claims = (
        validated_ids - passed_ids - blocked_ids - set(additional_contracts)
    )
    if unknown_claims:
        problems.append(
            "assessment includes unlisted validated claims: "
            f"{sorted(unknown_claims)}"
        )
    for claim_id in validated_ids & set(additional_contracts):
        problems.extend(
            _additional_claim_evidence_problems(
                context,
                additional_contracts[claim_id],
            )
        )

    derived_counts = {
        status: sum(item.get("status") == status for item in validation_checks)
        for status in ("PASS", "FAIL", "BLOCKED")
    }
    validation_policy = contract.get("validation_report_policy")
    if (
        not isinstance(validation_policy, Mapping)
        or validation_policy.get("summary_counts_are_derived_from_checks")
        is not True
        or not isinstance(validation_policy.get("maximum_fail_count"), int)
        or isinstance(validation_policy.get("maximum_fail_count"), bool)
        or validation_policy["maximum_fail_count"] < 0
    ):
        problems.append("public validation_report_policy is invalid")
        maximum_fail_count = 0
    else:
        maximum_fail_count = validation_policy["maximum_fail_count"]
    summary = validation.get("summary")
    summary_counts = summary.get("counts") if isinstance(summary, Mapping) else None
    if not isinstance(summary_counts, Mapping) or any(
        summary_counts.get(status) != count
        for status, count in derived_counts.items()
    ):
        problems.append("numeric summary counts differ from submitted checks")
    if derived_counts["FAIL"] > maximum_fail_count:
        problems.append(
            "final bundle exceeds the public numeric failure limit: "
            f"{derived_counts['FAIL']} > {maximum_fail_count}"
        )
    if assessment.get("reproduction_status") != contract.get(
        "reproduction_status"
    ):
        problems.append("assessment does not remain a partial reproduction")
    if assessment.get("external_supplement_used") is not contract.get(
        "external_supplement_used"
    ):
        problems.append("assessment claims external supplementary use")
    if assessment.get("historical_identity_verified") is not contract.get(
        "historical_identity_verified"
    ):
        problems.append("assessment overstates historical identity with publisher SI")

    raw_blocked_contracts = contract.get("blocked_claims")
    if not isinstance(raw_blocked_contracts, list) or not all(
        isinstance(item, Mapping)
        and isinstance(item.get("claim_id"), str)
        and item.get("claim_id")
        for item in raw_blocked_contracts
    ):
        problems.append("public final blocked_claims contract is invalid")
        raw_blocked_contracts = []
    blocked_contracts = {
        item["claim_id"]: item for item in raw_blocked_contracts
    }
    blocked_policy = contract.get("blocked_claims_policy")
    if (
        not isinstance(blocked_policy, Mapping)
        or blocked_policy.get("required_numeric_status")
        != blocked_numeric_status
        or blocked_policy.get("numeric_coverage") != "exact"
        or blocked_policy.get("public_contract_coverage") != "exact"
        or blocked_policy.get("duplicate_claim_ids") != "forbidden"
    ):
        problems.append("public blocked_claims_policy is invalid")
    raw_assessment_blocked = assessment.get("blocked_claims", [])
    if not isinstance(raw_assessment_blocked, list):
        problems.append("assessment blocked_claims must be an array")
        raw_assessment_blocked = []
    blocked_by_id = {
        item.get("claim_id"): item
        for item in raw_assessment_blocked
        if isinstance(item, Mapping)
        and isinstance(item.get("claim_id"), str)
        and item.get("claim_id")
    }
    if len(blocked_by_id) != len(raw_assessment_blocked):
        problems.append("assessment contains duplicate or invalid blocked claims")
    if set(blocked_by_id) != blocked_ids:
        problems.append(
            "assessment blocked claim IDs differ from numeric BLOCKED checks"
        )
    if (
        set(blocked_by_id) != set(blocked_contracts)
        or len(raw_assessment_blocked) != len(blocked_contracts)
    ):
        problems.append(
            "assessment blocked claim IDs differ from public final contract"
        )
    for claim_id, blocked_contract in blocked_contracts.items():
        claim = blocked_by_id.get(claim_id, {})
        claim_text = " ".join(
            (
                str(claim.get("reason", "")),
                *(str(value) for value in claim.get("required_inputs", [])),
            )
        )
        problems.extend(
            _text_satisfies_alternatives(
                claim_text,
                blocked_contract.get("required_input_token_alternatives"),
                label=f"assessment blocker {claim_id}",
            )
        )

    supplement_folded = supplement.casefold()
    supplement_tokens = _nonempty_strings(
        contract.get("required_supplement_tokens")
    )
    if supplement_tokens is None:
        problems.append("public required_supplement_tokens contract is invalid")
        supplement_tokens = []
    for token in supplement_tokens:
        if token.casefold() not in supplement_folded:
            problems.append(f"supplement omits cross-checked claim {token}")
    forbidden_claims = _nonempty_strings(contract.get("forbidden_claims"))
    if forbidden_claims is None:
        problems.append("public final forbidden_claims contract is invalid")
        forbidden_claims = []
    for forbidden in forbidden_claims:
        if forbidden.casefold() in supplement_folded:
            problems.append(f"supplement promotes a blocked claim: {forbidden}")

    manifest_policy = contract.get("manifest_policy")
    if (
        not isinstance(manifest_policy, Mapping)
        or manifest_policy.get("entry_order_significant") is not False
        or manifest_policy.get("roles") != "exactly_manifest_slots"
        or manifest_policy.get("paths") != "exactly_slot_path"
        or manifest_policy.get("digest")
        != "sha256_of_bound_input_or_self_output"
        or manifest_policy.get("additional_entries") != "forbidden"
    ):
        problems.append("public manifest_policy is invalid")
    entries = manifest.get("entries", [])
    if not isinstance(entries, list):
        entries = []
        problems.append("manifest entries must be an array")
    roles: set[str] = set()
    self_sources = {
        "self:inferred_supplement": supplement_path,
        "self:assessment": assessment_path,
    }
    raw_slots = contract.get("manifest_slots")
    if not isinstance(raw_slots, list) or not all(
        isinstance(item, Mapping)
        and all(
            isinstance(item.get(field), str) and item.get(field)
            for field in ("role", "path", "input_name")
        )
        for item in raw_slots
    ):
        problems.append("public final manifest_slots contract is invalid")
        raw_slots = []
    slots = {item["role"]: item for item in raw_slots}
    if len(slots) != len(raw_slots):
        problems.append("public final manifest_slots contains duplicate roles")
    sources: dict[str, Path] = {}
    for role, slot in slots.items():
        input_name = slot["input_name"]
        if input_name in self_sources:
            sources[role] = self_sources[input_name]
            continue
        try:
            sources[role] = _input_path(context, input_name)
        except (OSError, ValueError) as error:
            problems.append(f"manifest source is unavailable for {role}: {error}")
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            problems.append(f"manifest entry {index} is not an object")
            continue
        role = entry.get("role")
        if not isinstance(role, str):
            problems.append(f"manifest entry {index} has no role")
            continue
        if role in roles:
            problems.append(f"duplicate manifest role: {role}")
        roles.add(role)
        expected_slot = slots.get(role)
        if expected_slot is None:
            problems.append(f"manifest role is outside stable slots: {role}")
            continue
        raw_path = entry.get("path")
        if not isinstance(raw_path, str):
            problems.append(f"manifest entry {role} has no path")
            continue
        parsed = PurePosixPath(raw_path)
        if (
            parsed.is_absolute()
            or "\\" in raw_path
            or ".." in parsed.parts
            or "." in parsed.parts
            or raw_path != expected_slot["path"]
        ):
            problems.append(f"manifest path does not match stable slot for {role}")
            continue
        digest_source = sources.get(role)
        if digest_source is None or not digest_source.is_file():
            problems.append(f"manifest source does not exist for {role}")
            continue
        if entry.get("digest") != digest_path(digest_source):
            problems.append(f"manifest digest mismatch for {role}")
    required_roles = set(slots)
    if roles != required_roles:
        problems.append(
            "manifest roles differ from stable slots: "
            f"missing={sorted(required_roles - roles)}, "
            f"extra={sorted(roles - required_roles)}"
        )
    return _result(
        "reproduction_bundle_consistency",
        problems,
        (
            supplement_artifact,
            assessment_artifact,
            manifest_artifact,
        ),
        spec.required,
    )


def build_reproduction_registry():
    """Return built-ins plus Test2 domain verifiers."""

    registry = default_registry()
    registry.register(
        "evidence_ledger_consistency",
        evidence_ledger_consistency,
    )
    registry.register(
        "numeric_reproduction_consistency",
        numeric_reproduction_consistency,
    )
    registry.register(
        "theory_inference_consistency",
        theory_inference_consistency,
    )
    registry.register(
        "methods_inference_consistency",
        methods_inference_consistency,
    )
    registry.register(
        "reproduction_bundle_consistency",
        reproduction_bundle_consistency,
    )
    return registry


__all__ = [
    "build_reproduction_registry",
    "evidence_ledger_consistency",
    "methods_inference_consistency",
    "numeric_reproduction_consistency",
    "reproduction_bundle_consistency",
    "theory_inference_consistency",
]
