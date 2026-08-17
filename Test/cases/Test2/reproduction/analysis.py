"""Deterministic numerical checks derived from the Test2 paper.

The implementation intentionally uses only the main article. It does not fetch
or inspect the publisher's supplementary information.
"""

from __future__ import annotations

import hashlib
import math
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

EXPECTED_PDF_SHA256 = "23aa5423cc6af79247265dd75ba8af13137e2d9b76efa6b0358883b288019bd2"
EXPECTED_EXTRACTED_SHA256 = (
    "a3e368dc739dac6cf087d18cb9445eb579dd3be99e44dffb5914433beaa326cf"
)
STRAIGHT_BEAM_BENCHMARK = {
    # Independent closed-form benchmarks from Eqs. (52), (53), (60), and
    # (61), frozen as literals so a defect in the numerical observation path
    # cannot redefine its own expected answer.
    "critical_load_coefficient": 39.478417604357434,
    "u_app_second_order_coefficient": 2.4674011002723395,
    "normalized_maximum_strain": 6.283185307179586,
}
POLYNOMIAL_COMPLETION_BENCHMARK = {
    # Independently evaluated with adaptive Gauss-Kronrod quadrature directly
    # on Eqs. (54)-(55), rather than the uniform-grid implementation below.
    "sample_x": [-0.5, -0.25, 0.0, 0.25, 0.5],
    "kappa3": [
        0.0,
        -3.0399993234497518,
        0.0,
        3.0399993234497518,
        0.0,
    ],
    "phi": [
        0.0,
        -1.2465676632656348,
        -1.8886713243831512,
        -1.2465676632656348,
        0.0,
    ],
    "u2": [
        0.0,
        -2.5847251916185936,
        -0.9509435311329635,
        -2.5847251916185936,
        0.0,
    ],
}


@dataclass(frozen=True, slots=True)
class ValidationCheck:
    """One auditable reproduction check."""

    check_id: str
    title: str
    status: str
    evidence: tuple[str, ...]
    observed: Any
    expected: Any
    tolerance: Any
    interpretation: str

    def to_dict(self) -> dict[str, Any]:
        """Return a JSON-compatible representation."""

        return asdict(self)


def sha256_file(path: Path) -> str:
    """Return a lower-case SHA-256 digest for a local file."""

    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _linspace(start: float, stop: float, count: int) -> list[float]:
    if count < 2:
        raise ValueError("count must be at least two")
    step = (stop - start) / (count - 1)
    return [start + index * step for index in range(count)]


def _trapz(x_values: list[float], y_values: list[float]) -> float:
    if len(x_values) != len(y_values) or len(x_values) < 2:
        raise ValueError("trapezoidal inputs must have the same non-zero size")
    return sum(
        0.5
        * (x_values[index] - x_values[index - 1])
        * (y_values[index] + y_values[index - 1])
        for index in range(1, len(x_values))
    )


def _cumulative_from_zero(x_values: list[float], y_values: list[float]) -> list[float]:
    if len(x_values) % 2 == 0:
        raise ValueError("an odd grid is required so x=0 is represented")
    midpoint = len(x_values) // 2
    if abs(x_values[midpoint]) > 1e-14:
        raise ValueError("grid midpoint must be zero")
    result = [0.0] * len(x_values)
    for index in range(midpoint + 1, len(x_values)):
        result[index] = result[index - 1] + 0.5 * (
            x_values[index] - x_values[index - 1]
        ) * (y_values[index] + y_values[index - 1])
    for index in range(midpoint - 1, -1, -1):
        result[index] = result[index + 1] - 0.5 * (
            x_values[index + 1] - x_values[index]
        ) * (y_values[index + 1] + y_values[index])
    return result


def _cumulative_from_left(x_values: list[float], y_values: list[float]) -> list[float]:
    """Return trapezoidal integrals from the left endpoint."""

    if len(x_values) != len(y_values) or len(x_values) < 2:
        raise ValueError("cumulative inputs must have the same non-zero size")
    result = [0.0] * len(x_values)
    for index in range(1, len(x_values)):
        result[index] = result[index - 1] + 0.5 * (
            x_values[index] - x_values[index - 1]
        ) * (y_values[index] + y_values[index - 1])
    return result


def b_max_values() -> dict[str, float]:
    """Return the geometric single-valuedness limits from Eqs. (22) and (66)."""

    polynomial_derivative_max = 15.36 / math.sqrt(20.0)
    return {
        "sinusoidal": 1.0 / (2.0 * math.pi),
        "polynomial": 1.0 / polynomial_derivative_max,
        "arc": math.pi,
    }


def sinusoidal_initial_errors(
    b_value: float,
    order: int,
    *,
    points: int = 5001,
) -> dict[str, float]:
    """Recompute the Eq. (68) errors for the sinusoidal initial geometry.

    Arc length is normalized to one and ``x=S/L_S``. The exact tangent is
    ``Z'=sqrt(1-(2*pi*b*cos(2*pi*x))**2)``. Expanding this tangent and the
    corresponding curvature gives the same even/odd truncation structure shown
    in Fig. 5.
    """

    if not 0.0 < b_value < 1.0 / (2.0 * math.pi):
        raise ValueError("b_value must be inside the sinusoidal geometry limit")
    if not 1 <= order <= 5:
        raise ValueError("order must be between one and five")
    if points % 2 == 0:
        raise ValueError("points must be odd")

    x_values = _linspace(-0.5, 0.5, points)
    sine = [math.sin(2.0 * math.pi * value) for value in x_values]
    cosine = [math.cos(2.0 * math.pi * value) for value in x_values]
    q_values = [(2.0 * math.pi * b_value * value) ** 2 for value in cosine]

    exact_y = [b_value * value for value in sine]
    approximate_y = list(exact_y)
    exact_z_tangent = [math.sqrt(1.0 - value) for value in q_values]
    approximate_z_tangent: list[float] = []
    for q_value in q_values:
        value = 1.0
        if order >= 2:
            value -= 0.5 * q_value
        if order >= 4:
            value -= 0.125 * q_value**2
        approximate_z_tangent.append(value)
    exact_z = _cumulative_from_zero(x_values, exact_z_tangent)
    approximate_z = _cumulative_from_zero(x_values, approximate_z_tangent)

    exact_curvature = [
        4.0 * math.pi**2 * b_value * sin_value / math.sqrt(1.0 - q_value)
        for sin_value, q_value in zip(sine, q_values, strict=True)
    ]
    approximate_curvature: list[float] = []
    for sin_value, q_value in zip(sine, q_values, strict=True):
        multiplier = 1.0
        if order >= 3:
            multiplier += 0.5 * q_value
        if order >= 5:
            multiplier += 0.375 * q_value**2
        approximate_curvature.append(
            4.0 * math.pi**2 * b_value * sin_value * multiplier
        )

    y_error = _trapz(
        x_values,
        [
            (approximate - exact) ** 2
            for approximate, exact in zip(approximate_y, exact_y, strict=True)
        ],
    ) / _trapz(x_values, [value**2 for value in exact_y])
    z_error = _trapz(
        x_values,
        [
            (approximate - exact) ** 2
            for approximate, exact in zip(approximate_z, exact_z, strict=True)
        ],
    ) / _trapz(x_values, [value**2 for value in exact_z])
    curvature_error = math.sqrt(
        _trapz(
            x_values,
            [
                (approximate - exact) ** 2
                for approximate, exact in zip(
                    approximate_curvature,
                    exact_curvature,
                    strict=True,
                )
            ],
        )
        / _trapz(x_values, [value**2 for value in exact_curvature])
    )
    return {
        "initial_shape": math.sqrt(y_error + z_error),
        "initial_curvature": curvature_error,
    }


def _shape_terms(
    shape: str, x_values: list[float]
) -> tuple[list[float], list[float], list[float]]:
    if shape == "sinusoidal":
        function = [math.sin(2.0 * math.pi * value) for value in x_values]
        derivative = [
            2.0 * math.pi * math.cos(2.0 * math.pi * value) for value in x_values
        ]
        second = [
            -4.0 * math.pi**2 * math.sin(2.0 * math.pi * value) for value in x_values
        ]
        return function, derivative, second
    if shape == "polynomial":
        function = [64.0 * (0.25 - value**2) ** 3 for value in x_values]
        derivative = [-384.0 * value * (0.25 - value**2) ** 2 for value in x_values]
        second = [
            -384.0 * (0.25 - value**2) * (0.25 - 5.0 * value**2) for value in x_values
        ]
        return function, derivative, second
    if shape == "arc":
        function = [0.125 - 0.5 * value**2 for value in x_values]
        derivative = [-value for value in x_values]
        second = [-1.0 for _value in x_values]
        return function, derivative, second
    raise ValueError(f"unsupported shape: {shape}")


def first_order_mode_profile(
    shape: str,
    poisson_ratio: float = 0.39,
    *,
    points: int = 10001,
) -> tuple[list[float], list[float]]:
    """Reconstruct the first-order twisting curvature from Eqs. (34) and (55).

    The returned curvature is the coefficient of ``b`` for normalized
    ``L_S=EI_2=1``. This quadrature reconstructs the polynomial result that the
    main article delegates to the unavailable supplementary information.
    """

    if not -1.0 < poisson_ratio < 0.5:
        raise ValueError("poisson_ratio must be physically admissible")
    if points % 2 == 0:
        raise ValueError("points must be odd")
    x_values = _linspace(-0.5, 0.5, points)
    function, _derivative, second = _shape_terms(shape, x_values)
    integral_f = _trapz(x_values, function)
    integral_xf = _trapz(
        x_values,
        [
            x_value * f_value
            for x_value, f_value in zip(x_values, function, strict=True)
        ],
    )

    critical_load = 4.0 * math.pi**2
    curvature_b1 = [-value for value in second]
    moment_b1 = [
        critical_load * (-integral_f - 12.0 * x_value * integral_xf + f_value)
        for x_value, f_value in zip(x_values, function, strict=True)
    ]
    forcing = [
        2.0 * math.pi**2 * math.cos(2.0 * math.pi * x_value) * (curvature - moment)
        for x_value, curvature, moment in zip(
            x_values,
            curvature_b1,
            moment_b1,
            strict=True,
        )
    ]
    integral_forcing = _cumulative_from_zero(x_values, forcing)
    ratio_ei_to_gj = (1.0 + poisson_ratio) / 2.0
    provisional = [ratio_ei_to_gj * value for value in integral_forcing]
    tilde_f1 = [
        value + math.pi * curvature * math.sin(2.0 * math.pi * x_value)
        for value, curvature, x_value in zip(
            provisional,
            curvature_b1,
            x_values,
            strict=True,
        )
    ]
    integration_constant = -_trapz(x_values, tilde_f1)
    twisting_curvature = [value + integration_constant for value in provisional]
    return x_values, twisting_curvature


def polynomial_supplement_profiles(
    poisson_ratio: float = 0.39,
    *,
    points: int = 10001,
) -> dict[str, list[float]]:
    """Reconstruct all three polynomial ``b^1`` quantities omitted to SI.

    The main article states that the missing polynomial result comprises
    ``phi_(1)b(1)``, ``U_2(2)b(1)``, and ``kappa_3_(1)b(1)``.  This routine
    evaluates the general Eqs. (52)-(55) for the polynomial shape in Eq. (66),
    with ``L_S=EI_2=1``.  Twist uses the endpoint convention in Eq. (69), and
    the in-plane displacement uses the boundary-corrected double integral in
    Eq. (54).
    """

    if not -1.0 < poisson_ratio < 0.5:
        raise ValueError("poisson_ratio must be physically admissible")
    if points % 2 == 0:
        raise ValueError("points must be odd")
    x_values = _linspace(-0.5, 0.5, points)
    function, derivative, second = _shape_terms("polynomial", x_values)
    curvature_b1 = [-value for value in second]
    curvature_b1_prime = [-1152.0 * value + 7680.0 * value**3 for value in x_values]
    integral_f = _trapz(x_values, function)
    critical_load = 4.0 * math.pi**2
    moment_b1 = [critical_load * (f_value - integral_f) for f_value in function]
    forcing = [
        2.0 * math.pi**2 * math.cos(2.0 * math.pi * x_value) * (curvature - moment)
        for x_value, curvature, moment in zip(
            x_values,
            curvature_b1,
            moment_b1,
            strict=True,
        )
    ]
    inverse_gj = (1.0 + poisson_ratio) / 2.0
    integrated_forcing = _cumulative_from_zero(x_values, forcing)
    provisional_kappa = [inverse_gj * value for value in integrated_forcing]
    tilde_f1 = [
        kappa + math.pi * curvature * math.sin(2.0 * math.pi * x_value)
        for kappa, curvature, x_value in zip(
            provisional_kappa,
            curvature_b1,
            x_values,
            strict=True,
        )
    ]
    kappa_constant = -_trapz(x_values, tilde_f1)
    kappa3 = [value + kappa_constant for value in provisional_kappa]

    cumulative_kappa = _cumulative_from_left(x_values, kappa3)
    total_kappa = cumulative_kappa[-1]
    midpoint = len(x_values) // 2
    phi = [
        (
            cumulative_kappa[index]
            if index <= midpoint
            else cumulative_kappa[index] - total_kappa
        )
        for index in range(len(x_values))
    ]

    cumulative_tilde = _cumulative_from_left(x_values, tilde_f1)
    total_tilde = cumulative_tilde[-1]
    f2 = []
    for (
        x_value,
        curvature,
        curvature_prime,
        integral_to_x,
    ) in zip(
        x_values,
        curvature_b1,
        curvature_b1_prime,
        cumulative_tilde,
        strict=True,
    ):
        two_sided_integral = (
            total_tilde - 2.0 * integral_to_x + 2.0 * x_value * total_tilde
        )
        f2.append(
            8.0 * math.pi * (math.cos(4.0 * math.pi * x_value) - 1.0) * curvature
            + (-4.0 * math.pi * x_value + math.sin(4.0 * math.pi * x_value))
            * curvature_prime
            + 16.0 * math.pi * math.cos(2.0 * math.pi * x_value) * two_sided_integral
        )
    first_integral = _cumulative_from_zero(x_values, f2)
    second_integral = _cumulative_from_zero(x_values, first_integral)
    at_left = second_integral[0]
    at_right = second_integral[-1]
    # For Eq. (66)'s polynomial, sin(Theta)_{b(1)} = F'(x) vanishes at
    # both endpoints, so Eq. (54)'s endpoint-angle terms are exactly zero.
    u2 = [
        math.pi
        / 32.0
        * (
            (-1.0 + 2.0 * x_value) * at_left
            - (1.0 + 2.0 * x_value) * at_right
            + 2.0 * integral
        )
        for x_value, integral in zip(x_values, second_integral, strict=True)
    ]
    return {
        "x": x_values,
        "kappa3": kappa3,
        "phi": phi,
        "u2": u2,
        "f2": f2,
        "shape_derivative": derivative,
    }


def polynomial_completion_diagnostics(
    poisson_ratio: float = 0.39,
    *,
    points: int = 10001,
) -> dict[str, Any]:
    """Return independent identities and samples for the omitted SI fields."""

    profiles = polynomial_supplement_profiles(poisson_ratio, points=points)
    x_values = profiles["x"]
    kappa3 = profiles["kappa3"]
    phi = profiles["phi"]
    u2 = profiles["u2"]
    f2 = profiles["f2"]
    step = x_values[1] - x_values[0]
    phi_derivative_residual = max(
        abs((phi[index + 1] - phi[index - 1]) / (2.0 * step) - kappa3[index])
        for index in range(1, len(x_values) - 1)
        if index != len(x_values) // 2
    )
    # Differentiating Eq. (54) twice eliminates its linear boundary
    # corrections, yielding U2''=(pi/16)*F2 for L_S=1.
    u2_equation_residual = max(
        abs(
            (u2[index + 1] - 2.0 * u2[index] + u2[index - 1]) / step**2
            - math.pi / 16.0 * f2[index]
        )
        for index in range(1, len(x_values) - 1)
    )
    sample_indices = (
        0,
        len(x_values) // 4,
        len(x_values) // 2,
        3 * len(x_values) // 4,
        len(x_values) - 1,
    )
    kappa_odd_residual = max(
        abs(left + right) for left, right in zip(kappa3, reversed(kappa3), strict=True)
    )
    phi_even_residual = max(
        abs(left - right) for left, right in zip(phi, reversed(phi), strict=True)
    )
    u2_even_residual = max(
        abs(left - right) for left, right in zip(u2, reversed(u2), strict=True)
    )
    arc_x, arc_numerical = first_order_mode_profile("arc", poisson_ratio, points=points)
    arc_published = [
        math.pi**2
        * (1.0 + poisson_ratio)
        / 12.0
        * (
            12.0 * value * math.cos(2.0 * math.pi * value)
            + math.pi * (-1.0 + 12.0 * value**2) * math.sin(2.0 * math.pi * value)
        )
        for value in arc_x
    ]
    return {
        "sample_x": [x_values[index] for index in sample_indices],
        "kappa3": [kappa3[index] for index in sample_indices],
        "phi": [phi[index] for index in sample_indices],
        "u2": [u2[index] for index in sample_indices],
        "phi_endpoint_residual": max(abs(phi[0]), abs(phi[-1])),
        "phi_derivative_residual": phi_derivative_residual,
        "u2_endpoint_residual": max(abs(u2[0]), abs(u2[-1])),
        "u2_equation_residual": u2_equation_residual,
        "kappa3_odd_residual": kappa_odd_residual,
        "phi_even_residual": phi_even_residual,
        "u2_even_residual": u2_even_residual,
        "arc_closed_form_residual": max(
            abs(actual - expected)
            for actual, expected in zip(arc_numerical, arc_published, strict=True)
        ),
    }


def first_order_mode_ratio_coefficient(
    shape: str,
    poisson_ratio: float = 0.39,
    *,
    points: int = 10001,
) -> float:
    """Return the coefficient in ``R_mode = coefficient*b + O(b^3)``."""

    x_values, twisting = first_order_mode_profile(shape, poisson_ratio, points=points)
    bending = [
        -2.0 * math.pi**2 * math.cos(2.0 * math.pi * value) for value in x_values
    ]
    return _trapz(x_values, [abs(value) for value in twisting]) / _trapz(
        x_values, [abs(value) for value in bending]
    )


def closed_form_mode_ratio_coefficient(
    shape: str, poisson_ratio: float = 0.39
) -> float:
    """Return the coefficients printed below Eq. (71)."""

    if shape == "sinusoidal":
        return math.pi**2 / 2.0
    if shape == "polynomial":
        numerator = 3.0 * (
            5040.0 * math.pi**2 - 33600.0 - 140.0 * math.pi**4 - math.pi**6
        )
        return numerator * (1.0 + poisson_ratio) / (70.0 * math.pi**5)
    if shape == "arc":
        return (24.0 - math.pi**2) * (1.0 + poisson_ratio) / (48.0 * math.pi)
    raise ValueError(f"unsupported shape: {shape}")


def compression_from_prestrain(prestrain: float) -> float:
    """Convert engineering prestrain to release-induced compression."""

    if prestrain < 0.0:
        raise ValueError("prestrain must be non-negative")
    return prestrain / (1.0 + prestrain)


def inverse_design_strain(diameter_to_pitch: float) -> float:
    """Invert Eq. (73) for the applied compression."""

    if diameter_to_pitch < 0.0:
        raise ValueError("diameter_to_pitch must be non-negative")
    if diameter_to_pitch == 0.0:
        return 0.0
    scaled = math.pi * diameter_to_pitch
    root_epsilon = (math.sqrt(1.0 + scaled**2) - 1.0) / scaled
    return root_epsilon**2


def _source_checks(pdf_path: Path, extracted_markdown: Path) -> list[ValidationCheck]:
    pdf_digest = sha256_file(pdf_path)
    markdown_digest = sha256_file(extracted_markdown)
    return [
        ValidationCheck(
            check_id="SRC-001",
            title="固定论文 PDF 身份",
            status=("PASS" if pdf_digest == EXPECTED_PDF_SHA256 else "FAIL"),
            evidence=("Testarticle.pdf SHA-256",),
            observed=pdf_digest,
            expected=EXPECTED_PDF_SHA256,
            tolerance="exact",
            interpretation="防止在错误论文上得到表面一致的数值结果。",
        ),
        ValidationCheck(
            check_id="SRC-002",
            title="固定 OCR 全文身份",
            status=("PASS" if markdown_digest == EXPECTED_EXTRACTED_SHA256 else "FAIL"),
            evidence=("review-work/Extractedmd/full.md SHA-256",),
            observed=markdown_digest,
            expected=EXPECTED_EXTRACTED_SHA256,
            tolerance="exact",
            interpretation="数值与证据锚点均绑定到已审查的提取版本。",
        ),
    ]


def _supplement_scope_check(markdown: str) -> ValidationCheck:
    anchors = {
        "oblique_basis": (
            "T _ {\\mathrm{I} (0) b (0)}",
            "are given in Supporting Information",
        ),
        "polynomial_results": (
            "The results for polynomial-shaped ribbons are given in "
            "Supporting Information.",
        ),
        "experiment_fea_details": (
            "Both the experiments and FEA (See Supporting Information "
            "for more details)",
        ),
        "figure_s1": ("Fig. S1 (Supporting Information)",),
    }
    found = {
        name: all(fragment in markdown for fragment in fragments)
        for name, fragments in anchors.items()
    }
    return ValidationCheck(
        check_id="SI-SCOPE-001",
        title="确认正文显式指向的四类补充材料依赖",
        status="PASS" if all(found.values()) else "FAIL",
        evidence=(
            "Eq. (48)",
            "Section 3.2",
            "Section 3.3",
            "PDF page 231",
        ),
        observed=found,
        expected={name: True for name in anchors},
        tolerance="all anchors present",
        interpretation=(
            "正文可确认 SI 至少覆盖斜压基函数、多项式结果、实验/FEA 方法和 Fig. S1。"
        ),
    )


def _b_max_check() -> ValidationCheck:
    observed = b_max_values()
    published = {
        "sinusoidal": 0.159,
        "polynomial": 0.291,
        "arc": math.pi,
    }
    errors = {name: abs(observed[name] - published[name]) for name in observed}
    return ValidationCheck(
        check_id="NUM-BMAX-001",
        title="复现三类初始形状的 b_max",
        status="PASS" if max(errors.values()) < 5e-4 else "FAIL",
        evidence=("Eqs. (22), (66)", "Section 3.2"),
        observed=observed,
        expected=published,
        tolerance={"absolute": 5e-4},
        interpretation=("由单值性约束直接得到 1/(2π)、约 0.291 和 π。"),
    )


def _truncation_check() -> ValidationCheck:
    observed = sinusoidal_initial_errors(0.1, 3)
    published = {
        "initial_shape": 0.00906,
        "initial_curvature": 0.0173,
    }
    errors = {name: abs(observed[name] - published[name]) for name in observed}
    return ValidationCheck(
        check_id="NUM-FIG5-001",
        title="复现 Fig. 5 的三阶截断误差",
        status="PASS" if max(errors.values()) < 5e-4 else "FAIL",
        evidence=("Eq. (68)", "Fig. 5", "Section 3.4"),
        observed=observed,
        expected=published,
        tolerance={"absolute_fraction": 5e-4},
        interpretation=("b=0.1、l=3 时得到约 0.906% 与 1.73%，与正文逐字报告一致。"),
    )


def _mode_ratio_check() -> ValidationCheck:
    poisson_ratio = 0.39
    observed_ratios = {
        shape: first_order_mode_ratio_coefficient(shape, poisson_ratio)
        for shape in ("sinusoidal", "polynomial", "arc")
    }
    expected_ratios = {
        shape: closed_form_mode_ratio_coefficient(shape, poisson_ratio)
        for shape in observed_ratios
    }
    ratio_errors = {
        shape: abs(observed_ratios[shape] - expected_ratios[shape])
        for shape in observed_ratios
    }
    completion = polynomial_completion_diagnostics(poisson_ratio)
    benchmark_errors = {
        name: max(
            abs(actual - expected)
            for actual, expected in zip(
                completion[name],
                POLYNOMIAL_COMPLETION_BENCHMARK[name],
                strict=True,
            )
        )
        for name in ("kappa3", "phi", "u2")
    }
    passed = (
        max(ratio_errors.values()) < 2e-7
        and benchmark_errors["kappa3"] < 5e-6
        and benchmark_errors["phi"] < 1e-6
        and benchmark_errors["u2"] < 2e-6
        and completion["phi_endpoint_residual"] < 1e-10
        and completion["phi_derivative_residual"] < 2e-5
        and completion["u2_endpoint_residual"] < 1e-10
        and completion["u2_equation_residual"] < 3e-4
        and completion["kappa3_odd_residual"] < 1e-10
        and completion["phi_even_residual"] < 1e-10
        and completion["u2_even_residual"] < 1e-10
        and completion["arc_closed_form_residual"] < 5e-6
    )
    return ValidationCheck(
        check_id="NUM-FIG8-001",
        title="重建 polynomial SI 的三个 b^1 量与 Fig. 8 模态比",
        status="PASS" if passed else "FAIL",
        evidence=(
            "Eqs. (34), (52)-(55), (65), (66), (71)",
            "Section 3.2 polynomial SI delegation",
            "Eq. (69) endpoint twist convention",
        ),
        observed={
            "mode_ratio": observed_ratios,
            "polynomial_completion": completion,
        },
        expected={
            "mode_ratio": expected_ratios,
            "polynomial_completion": POLYNOMIAL_COMPLETION_BENCHMARK,
        },
        tolerance={
            "mode_ratio_absolute": 2e-7,
            "kappa3_sample_absolute": 5e-6,
            "phi_sample_absolute": 1e-6,
            "u2_sample_absolute": 2e-6,
            "identity_residual": {
                "phi_prime_equals_kappa3": 2e-5,
                "u2_second_derivative_equals_pi_over_16_f2": 3e-4,
                "endpoint_conditions": 1e-10,
                "parity": 1e-10,
                "arc_published_closed_form": 5e-6,
            },
        },
        interpretation=(
            "正文委托给 SI 的 polynomial φ_(1)b(1)、U2_(2)b(1) 与"
            " κ3_(1)b(1) 均由一般式重建；独立自适应积分样点、微分恒等式、"
            "端点条件和 0.216252789(1+ν) 模态比共同闭环。"
        ),
    )


def _prestrain_check() -> ValidationCheck:
    observed = {
        "66.67_percent": compression_from_prestrain(2.0 / 3.0),
        "70_percent": compression_from_prestrain(0.7),
    }
    expected = {"66.67_percent": 0.4, "70_percent": 0.412}
    errors = {name: abs(observed[name] - expected[name]) for name in observed}
    return ValidationCheck(
        check_id="NUM-PRESTRAIN-001",
        title="复现预拉伸到压缩应变的换算",
        status="PASS" if max(errors.values()) < 5e-4 else "FAIL",
        evidence=("Section 3.3", "Section 3.4", "Fig. S1 description"),
        observed=observed,
        expected=expected,
        tolerance={"absolute_fraction": 5e-4},
        interpretation=(
            "释放后压缩为 p/(1+p)：若 66.7% 表示四舍五入的 2/3，"
            "则对应约 40%；70% 对应约 41.2%。"
        ),
    )


def _inverse_design_check() -> ValidationCheck:
    ratios = (0.1, 0.3, 0.5)
    observed = {str(value): inverse_design_strain(value) for value in ratios}
    figure_values = {"0.1": 0.0235, "0.3": 0.157, "0.5": 0.297}
    errors = {name: abs(observed[name] - figure_values[name]) for name in observed}
    return ValidationCheck(
        check_id="NUM-FIG10-001",
        title="复现 Fig. 10 圆/椭圆截面反设计应变",
        status="PASS" if max(errors.values()) < 0.005 else "FAIL",
        evidence=("Eq. (73)", "Fig. 10(c)"),
        observed=observed,
        expected=figure_values,
        tolerance={"absolute_fraction": 0.005},
        interpretation=(
            "直梁近似给出 2.353%、15.759%、30.121%；与有限 b 图值的最大差"
            " 0.421 个百分点，符合正文所称近似。"
        ),
    )


def _straight_beam_check() -> ValidationCheck:
    x_values = _linspace(-0.5, 0.5, 20001)
    first_derivative = [
        -math.pi * math.sin(2.0 * math.pi * value) for value in x_values
    ]
    second_derivative = [
        -2.0 * math.pi**2 * math.cos(2.0 * math.pi * value) for value in x_values
    ]
    slope_energy = _trapz(x_values, [value**2 for value in first_derivative])
    curvature_energy = _trapz(x_values, [value**2 for value in second_derivative])
    u_app = 0.5 * slope_energy
    observed = {
        "critical_load_coefficient": curvature_energy / slope_energy,
        "u_app_second_order_coefficient": u_app,
        "normalized_maximum_strain": (
            0.5 * max(abs(value) for value in second_derivative) / math.sqrt(u_app)
        ),
    }
    errors = {
        name: abs(observed[name] - STRAIGHT_BEAM_BENCHMARK[name]) for name in observed
    }
    return ValidationCheck(
        check_id="NUM-STRAIGHT-001",
        title="复现 b→0 的直梁退化极限",
        status="PASS" if max(errors.values()) < 1e-10 else "FAIL",
        evidence=("Eqs. (52), (53), (60), (61)", "Fig. 8"),
        observed=observed,
        expected=STRAIGHT_BEAM_BENCHMARK,
        tolerance={"absolute": 1e-10},
        interpretation=(
            "对 cos²(πS/L) 模态独立做 Rayleigh 商、端缩短积分和 Eq. (60)"
            " 应变归一化，分别回到冻结的 4π²、π²/4 与 2π 基准。"
        ),
    )


def build_validation_report(pdf_path: Path, extracted_markdown: Path) -> dict[str, Any]:
    """Run the deterministic, blind reproduction suite."""

    pdf_path = pdf_path.expanduser().resolve(strict=True)
    extracted_markdown = extracted_markdown.expanduser().resolve(strict=True)
    markdown = extracted_markdown.read_text(encoding="utf-8")
    checks = [
        *_source_checks(pdf_path, extracted_markdown),
        _supplement_scope_check(markdown),
        _b_max_check(),
        _truncation_check(),
        _mode_ratio_check(),
        _prestrain_check(),
        _inverse_design_check(),
        _straight_beam_check(),
        ValidationCheck(
            check_id="LIMIT-OBLIQUE-001",
            title="斜压缩 T_I...T_IV 的历史闭式表达",
            status="BLOCKED",
            evidence=("Eqs. (47), (48)",),
            observed=(
                "可唯一推得三阶算子核与特解职责，但正文不足以恢复作者选择的"
                "基函数命名、归一化和完整闭式排版。"
            ),
            expected="publisher supplementary or an equivalent derivation",
            tolerance="not applicable",
            interpretation=("可验证的是算子契约，不应把任意等价基误报成作者原始 SI。"),
        ),
        ValidationCheck(
            check_id="LIMIT-FEA-001",
            title="Fig. S1 含/不含基底的 ABAQUS 对照",
            status="BLOCKED",
            evidence=("Section 3.3", "Fig. S1 description"),
            observed=(
                "正文仅给 C3D8R、S4R、b=0.15、ε_app=40% 与定性结论；"
                "没有 inp、网格、基底本构或节点数据。"
            ),
            expected="ABAQUS model or complete equivalent model definition",
            tolerance="not applicable",
            interpretation="不能用文字描述替代全场 FEA 数值复现。",
        ),
        ValidationCheck(
            check_id="LIMIT-EXPERIMENT-001",
            title="实验重复性与不确定度",
            status="BLOCKED",
            evidence=("Section 3.3", "Figs. 3 and 4"),
            observed=(
                "正文给出 PET 厚度、E、ν 和几何比，但没有样本量、误差棒、"
                "原始坐标、基底完整参数与测量协议。"
            ),
            expected="raw measurements and complete experimental protocol",
            tolerance="not applicable",
            interpretation="只能核查量纲与几何关系，不能重算统计一致性。",
        ),
    ]
    serialized = [check.to_dict() for check in checks]
    counts = {
        status: sum(check.status == status for check in checks)
        for status in ("PASS", "FAIL", "BLOCKED")
    }
    return {
        "schema_version": "1.0",
        "benchmark_id": "test2-blind-supplement-reproduction",
        "paper": {
            "title": (
                "A double perturbation method of postbuckling analysis in "
                "2D curved beams for assembly of 3D ribbon-shaped structures"
            ),
            "doi": "10.1016/j.jmps.2017.10.012",
            "pdf_sha256": sha256_file(pdf_path),
            "extracted_markdown_sha256": sha256_file(extracted_markdown),
        },
        "policy": {
            "external_supplement_used": False,
            "external_network_required": False,
            "epistemic_rule": (
                "derived facts must pass executable checks; unavailable raw "
                "data must remain explicitly blocked"
            ),
        },
        "summary": {
            "status": (
                "FAILED"
                if counts["FAIL"]
                else "PARTIAL_REPRODUCTION"
                if counts["BLOCKED"]
                else "FULL_REPRODUCTION"
            ),
            "passed": counts["FAIL"] == 0,
            "fully_reproduced": counts["FAIL"] == 0 and counts["BLOCKED"] == 0,
            "counts": counts,
        },
        "checks": serialized,
    }


def render_validation_markdown(report: dict[str, Any]) -> str:
    """Render the machine report as a compact human-readable document."""

    summary = report["summary"]
    lines = [
        "# Test2 盲复现数值验证报告",
        "",
        f"- 状态：`{summary['status']}`",
        f"- 通过：{summary['counts']['PASS']}",
        f"- 失败：{summary['counts']['FAIL']}",
        f"- 受阻：{summary['counts']['BLOCKED']}",
        "- 外部 supplementary：未使用",
        "- PASS 构成：2 项输入身份、1 项 SI 依赖盘点、6 项数学/数值检查",
        "- 验证语义：端点/奇偶性为构造不变量；方程残差为数值一致性；"
        "正文闭式与冻结高精度样点为交叉检查",
        "",
        "| ID | 状态 | 核查 | 结论 |",
        "| --- | --- | --- | --- |",
    ]
    for check in report["checks"]:
        lines.append(
            f"| {check['check_id']} | {check['status']} | "
            f"{check['title']} | {check['interpretation']} |"
        )
    lines.extend(
        [
            "",
            "## 判定边界",
            "",
            "PASS 表示可由正文方程和本地论文独立重算。BLOCKED 表示缺少的"
            " supplementary、原始实验数据或 ABAQUS 模型使强复现不可识别；"
            "它不是失败，也不得被填入猜测值。任何 FAIL 都会使 Clef 数值节点"
            "验证失败并阻止最终综合节点发布。",
            "",
        ]
    )
    return "\n".join(lines)


__all__ = [
    "EXPECTED_EXTRACTED_SHA256",
    "EXPECTED_PDF_SHA256",
    "ValidationCheck",
    "b_max_values",
    "build_validation_report",
    "closed_form_mode_ratio_coefficient",
    "compression_from_prestrain",
    "first_order_mode_profile",
    "first_order_mode_ratio_coefficient",
    "inverse_design_strain",
    "polynomial_completion_diagnostics",
    "polynomial_supplement_profiles",
    "render_validation_markdown",
    "sha256_file",
    "sinusoidal_initial_errors",
]
