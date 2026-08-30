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

from seam import Schema, ValidationError

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"


def load_schema(name: str) -> Schema:
    return Schema.load(CONFORMANCE / "schemas" / f"{name}.seam")


def case_files() -> list[Path]:
    return sorted((CONFORMANCE / "cases").glob("*.json"))


def collect() -> list[tuple[str, str, str, dict, object]]:
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
                )
            )
    return out


CASES = collect()


def found_issues(schema: Schema, type_name: str, payload: dict) -> list[tuple[str, str]]:
    try:
        schema.validate(type_name, payload)
    except ValidationError as e:
        return [(i.path, i.code) for i in e.issues]
    return []


def expected_issues(expect: object) -> list[tuple[str, str]]:
    if expect == "valid":
        return []
    assert isinstance(expect, dict), f"unexpected `expect`: {expect!r}"
    return [(i["path"], i["code"]) for i in expect.get("issues", [])]


@pytest.mark.parametrize(
    "name,schema_name,type_name,payload,expect",
    CASES,
    ids=[c[0] for c in CASES],
)
def test_case(name, schema_name, type_name, payload, expect):
    schema = load_schema(schema_name)
    assert found_issues(schema, type_name, payload) == expected_issues(expect)


def test_the_suite_is_not_empty():
    assert len(CASES) >= 50, f"expected a meaningful suite, collected {len(CASES)}"
