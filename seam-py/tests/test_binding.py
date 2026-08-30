"""The traps that are specific to Python, and the shape of what comes back."""

from __future__ import annotations

import datetime as dt
from pathlib import Path

import pytest

from seam import Limits, ParseError, Schema, ValidationError

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"


@pytest.fixture(scope="module")
def user() -> Schema:
    return Schema.load(CONFORMANCE / "schemas" / "user.seam")


def base() -> dict:
    return {"id": 1, "name": "Gabriel", "plan": "pro", "nickname": None}


def codes(schema: Schema, payload: dict) -> list[tuple[str, str]]:
    try:
        schema.validate("User", payload)
    except ValidationError as e:
        return [(i.path, i.code) for i in e.issues]
    return []


# --- the four presence states, as a caller reads them -----------------------


def test_absence_is_a_missing_key_not_a_none(user):
    out = user.validate("User", base())
    assert "bio" not in out, "an absent field must not appear in the result"
    assert "nickname" in out and out["nickname"] is None


def test_null_and_absent_are_judged_separately(user):
    assert codes(user, {**base(), "bio": None}) == [("bio", "null_not_allowed")]
    assert codes(user, {**base(), "avatar": None}) == []

    without_nickname = {k: v for k, v in base().items() if k != "nickname"}
    assert codes(user, without_nickname) == [("nickname", "required")]


# --- Python's own type traps ------------------------------------------------


def test_a_bool_is_not_an_integer(user):
    """`isinstance(True, int)` is True in Python. It must not be here."""
    assert codes(user, {**base(), "id": True}) == [("id", "type_mismatch")]


def test_a_datetime_is_not_a_date(user):
    """`isinstance(datetime.now(), date)` is True in Python. Not here."""
    aware = dt.datetime(2026, 8, 29, 14, 30, tzinfo=dt.timezone.utc)
    assert codes(user, {**base(), "signup_date": aware}) == [
        ("signup_date", "invalid_date")
    ]
    assert codes(user, {**base(), "signup_date": dt.date(2026, 8, 29)}) == []


def test_a_naive_datetime_is_rejected(user):
    naive = dt.datetime(2026, 8, 29, 14, 30)
    assert codes(user, {**base(), "last_seen": naive}) == [
        ("last_seen", "missing_timezone")
    ]


def test_an_integer_wider_than_64_bits_does_not_truncate(user):
    """Python ints are unbounded; the model stops at 64 bits."""
    assert codes(user, {**base(), "id": 2**64}) == [("id", "integer_too_wide")]
    assert codes(user, {**base(), "id": 2**64 - 1}) == []


def test_the_max_safe_integer_boundary_survives(user):
    """The value JavaScript corrupts to ...992."""
    out = user.validate("User", {**base(), "id": 9007199254740993})
    assert out["id"] == 9007199254740993


# --- normalisation ----------------------------------------------------------


def test_a_date_comes_back_as_a_date_not_a_datetime(user):
    out = user.validate("User", {**base(), "signup_date": "2026-08-29"})
    assert out["signup_date"] == dt.date(2026, 8, 29)
    assert type(out["signup_date"]) is dt.date


def test_a_datetime_comes_back_aware(user):
    out = user.validate("User", {**base(), "last_seen": "2026-08-29T14:30:00Z"})
    got = out["last_seen"]
    assert isinstance(got, dt.datetime)
    assert got.tzinfo is not None
    assert got.utcoffset() == dt.timedelta(0)


def test_fractional_seconds_and_offsets_round_trip(user):
    out = user.validate("User", {**base(), "last_seen": "2026-08-29T14:30:00.5-05:00"})
    got = out["last_seen"]
    assert got.microsecond == 500000
    assert got.utcoffset() == dt.timedelta(hours=-5)


# --- errors -----------------------------------------------------------------


def test_every_issue_is_reported_not_just_the_first(user):
    found = codes(user, {"id": "x", "name": "ab", "plan": "platinum", "nickname": None})
    assert set(found) == {
        ("id", "type_mismatch"),
        ("name", "too_short"),
        ("plan", "not_in_enum"),
    }


def test_the_error_exposes_the_first_issue_directly(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "name": "ab"})
    err = excinfo.value
    assert err.path == "name"
    assert err.code == "too_short"
    assert isinstance(err.message, str)


def test_errors_carry_the_path_into_arrays(user):
    assert codes(user, {**base(), "tags": ["ok", 7]}) == [("tags[1]", "type_mismatch")]


def test_a_parse_error_says_where(user):
    with pytest.raises(ParseError) as excinfo:
        Schema.parse("schema A { x: Nope }")
    assert "unknown type" in str(excinfo.value)


# --- limits -----------------------------------------------------------------


def test_limits_are_reachable_from_python(user):
    tight = Limits(max_items=2)
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "tags": ["a", "b", "c"]}, tight)
    assert excinfo.value.code == "size_exceeded"

    assert user.validate("User", {**base(), "tags": ["a", "b"]}, tight) is not None


def test_a_payload_that_is_not_a_dict_is_rejected(user):
    assert codes(user, [1, 2, 3]) == [("<root>", "type_mismatch")]


# --- schema surface ---------------------------------------------------------


def test_type_names(user):
    assert user.type_names() == ["User"]


def test_an_unsupported_python_type_is_a_validation_issue(user):
    """A `set` in a payload is reported like any other bad value, with a path.

    It used to raise TypeError, which aborted on the first one and carried the
    location only inside a message. As an issue it joins the rest of the report
    and its path is structured, which is worth more than matching Python's
    convention of TypeError for a wrong type.
    """
    found = codes(user, {**base(), "name": {1, 2}, "plan": "platinum"})
    assert ("name", "type_mismatch") in found
    assert ("plan", "not_in_enum") in found, "reporting must not stop at the set"


# --- bound validator --------------------------------------------------------


def test_a_bound_validator_agrees_with_the_convenience_path(user):
    bound = user.validator("User")
    payload = {**base(), "signup_date": "2026-08-29"}
    assert bound(payload) == user.validate("User", payload)
    assert bound.validate(payload) == bound(payload)


def test_a_bound_validator_reports_the_same_issues(user):
    bound = user.validator("User")
    with pytest.raises(ValidationError) as excinfo:
        bound({**base(), "name": "ab"})
    assert excinfo.value.code == "too_short"


def test_binding_an_unknown_type_fails_at_bind_time(user):
    """Not on the first call: a typo should surface where it was written."""
    with pytest.raises(ValidationError):
        user.validator("Nope")


def test_a_bound_validator_carries_its_limits(user):
    strict = user.validator("User", Limits(max_items=1))
    with pytest.raises(ValidationError) as excinfo:
        strict({**base(), "tags": ["a", "b"]})
    assert excinfo.value.code == "size_exceeded"

    assert strict({**base(), "tags": ["a"]}) is not None


def test_a_bound_validator_exposes_its_type(user):
    bound = user.validator("User")
    assert bound.type_name == "User"
    assert "User" in repr(bound)


def test_the_generated_module_binds_once(tmp_path):
    from seam.typegen import generate

    seam_file = tmp_path / "u.seam"
    seam_file.write_text("schema U { x: String }\n", encoding="utf-8")
    body = generate(seam_file)
    assert '.validator("U")' in body
    assert ".validate(" not in body


# --- the error object -------------------------------------------------------


def test_the_error_is_a_normal_exception(user):
    """It is declared in Python and raised from Rust; it must still behave."""
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "name": "ab"})
    err = excinfo.value
    assert isinstance(err, Exception)
    assert isinstance(err, ValidationError)


def test_str_gives_the_summary_without_touching_issues(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "name": "ab"})
    text = str(excinfo.value)
    assert "name" in text and "too_short" in text


def test_the_summary_says_how_many_more_there_are(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {"id": "x", "name": "ab", "plan": "no", "nickname": None})
    assert "and 2 more" in str(excinfo.value)


def test_len_counts_the_issues(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {"id": "x", "name": "ab", "plan": "no", "nickname": None})
    assert len(excinfo.value) == 3
    assert len(excinfo.value.issues) == 3


def test_issues_are_built_on_every_read_and_are_equal(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "name": "ab"})
    err = excinfo.value
    first, second = err.issues, err.issues
    assert [(i.path, i.code) for i in first] == [(i.path, i.code) for i in second]


def test_repr_is_readable(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validate("User", {**base(), "name": "ab"})
    assert repr(excinfo.value).startswith("ValidationError(")


def test_an_unknown_type_reports_as_an_issue(user):
    with pytest.raises(ValidationError) as excinfo:
        user.validator("Nope")
    assert excinfo.value.code == "unknown_type"
