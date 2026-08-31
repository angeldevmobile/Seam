"""The shared conformance suite, run from Python.

Same files the Rust runner reads. Agreement between the two is what "no drift"
means; if this file and `seam-core/tests/conformance.rs` ever disagree, one of
the two bindings is wrong.

Python's `json` module is a good citizen here: its integers are arbitrary
precision, so `9007199254740993` survives `json.loads` exactly. JavaScript will
not be so lucky, which is why the suite tests it.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from seam import Limits, Schema, ValidationError

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"


def load_schema(name: str) -> Schema:
    return Schema.load(CONFORMANCE / "schemas" / f"{name}.seam")


def case_files() -> list[Path]:
    return sorted((CONFORMANCE / "cases").glob("*.json"))


def collect() -> list[tuple[str, str, str, dict, object, dict]]:
    out = []
    for path in case_files():
        doc = json.loads(path.read_text(encoding="utf-8"))
        base = doc.get("base", {})
        for case in doc["cases"]:
            payload = {**base, **case.get("input", {})}
            out.append(
                (
                    f"{path.name}::{case['name']}",
                    doc["schema"],
                    doc["type"],
                    payload,
                    case["expect"],
                    case.get("limits", {}),
                )
            )
    return out


# The case files spell limits the way the spec does; the binding takes them
# under the same names, so nothing has to be translated here.
LIMIT_KEYS = ("max_depth", "max_items", "max_string_bytes", "max_object_keys")


CASES = collect()


def found_issues(
    schema: Schema, type_name: str, payload: dict, limits: dict
) -> list[tuple[str, str]]:
    # Binding is inside the try on purpose: a binding may report an undeclared
    # type name when the validator is bound rather than when a payload
    # arrives, which is earlier and better, and it is still the same verdict.
    try:
        bound = schema.validator(
            type_name, Limits(**{k: limits[k] for k in LIMIT_KEYS if k in limits})
        )
        bound(payload)
    except ValidationError as e:
        return [(i.path, i.code) for i in e.issues]
    return []


def expected_issues(expect: object) -> list[tuple[str, str]]:
    if expect == "valid":
        return []
    assert isinstance(expect, dict), f"unexpected `expect`: {expect!r}"
    return [(i["path"], i["code"]) for i in expect.get("issues", [])]


@pytest.mark.parametrize(
    "name,schema_name,type_name,payload,expect,limits",
    CASES,
    ids=[c[0] for c in CASES],
)
def test_case(name, schema_name, type_name, payload, expect, limits):
    # Python's integers are exact and this runner always feeds host values, so
    # `host_value` needs nothing here and `expect_inexact_integers` describes a
    # host this is not.
    schema = load_schema(schema_name)
    found = found_issues(schema, type_name, payload, limits)

    # Which codes appeared, not where or how many times. A limit is caught
    # while parsing, before the value it belongs to exists: a binding fed bytes
    # stops at the first breach and has no path, while one fed host objects
    # finishes the walk and reports each. Both must agree on the code.
    if isinstance(expect, dict) and "codes" in expect:
        assert sorted({code for _, code in found}) == sorted(expect["codes"])
    else:
        assert found == expected_issues(expect)


def test_the_suite_is_not_empty():
    assert len(CASES) >= 78, f"expected a meaningful suite, collected {len(CASES)}"
