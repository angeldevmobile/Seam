"""The generated types are only worth anything if a type checker acts on them,
so these tests run mypy rather than inspecting strings."""

from __future__ import annotations

import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from seam_schema import Schema
from seam_schema.cli import main
from seam_schema.typegen import generate

CONFORMANCE = Path(__file__).resolve().parents[2] / "conformance"
USER_SEAM = CONFORMANCE / "schemas" / "user.seam"

SCHEMA = """
schema Address {
  city:    String
  zip:     optional String
}

schema Person {
  name:     String
  age:      u32               @range(0..=130)
  plan:     enum { free, pro }
  home:     Address
  nickname: String?
  bio:      optional String
  avatar:   optional String?
  born:     optional Date
  seen:     optional DateTime
  tags:     [String?]
}
"""


def render(tmp_path: Path) -> Path:
    seam_file = tmp_path / "person.seam"
    seam_file.write_text(SCHEMA, encoding="utf-8")
    out = tmp_path / "person_types.py"
    out.write_text(generate(seam_file), encoding="utf-8")
    return out


# --- the mapping ------------------------------------------------------------


def test_the_four_presence_states_render_distinctly(tmp_path):
    body = render(tmp_path).read_text(encoding="utf-8")
    assert "nickname: str | None" in body
    assert "bio: NotRequired[str]" in body
    assert "avatar: NotRequired[str | None]" in body
    assert "name: str\n" in body


def test_types_render_as_their_python_counterparts(tmp_path):
    body = render(tmp_path).read_text(encoding="utf-8")
    assert "age: int" in body
    assert 'plan: Literal["free", "pro"]' in body
    assert "born: NotRequired[datetime.date]" in body
    assert "seen: NotRequired[datetime.datetime]" in body
    assert "home: Address" in body
    assert "tags: list[str | None]" in body


def test_a_reference_may_point_at_a_type_defined_later(tmp_path):
    """`Address` is declared after `Person` uses it in the sorted output."""
    body = render(tmp_path).read_text(encoding="utf-8")
    assert "from __future__ import annotations" in body


# --- the point: does a type checker act on it -------------------------------


def mypy(path: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-m", "mypy", "--no-incremental", str(path)],
        capture_output=True,
        text=True,
    )


needs_mypy = pytest.mark.skipif(
    shutil.which("mypy") is None
    and subprocess.run(
        [sys.executable, "-c", "import mypy"], capture_output=True
    ).returncode
    != 0,
    reason="mypy is not installed",
)


@needs_mypy
def test_the_generated_module_type_checks(tmp_path):
    result = mypy(render(tmp_path))
    assert result.returncode == 0, result.stdout + result.stderr


@needs_mypy
def test_mypy_catches_a_misspelled_key(tmp_path):
    render(tmp_path)
    user = tmp_path / "use_it.py"
    user.write_text(
        textwrap.dedent(
            """
            from person_types import validate_person

            person = validate_person({})
            print(person["nmae"])
            """
        ),
        encoding="utf-8",
    )
    result = mypy(user)
    assert result.returncode != 0
    assert "nmae" in result.stdout


@needs_mypy
def test_mypy_catches_a_wrong_type(tmp_path):
    render(tmp_path)
    user = tmp_path / "use_it.py"
    user.write_text(
        textwrap.dedent(
            """
            from person_types import validate_person

            person = validate_person({})
            age: str = person["age"]
            """
        ),
        encoding="utf-8",
    )
    result = mypy(user)
    assert result.returncode != 0


# --- the CLI ----------------------------------------------------------------


def test_typegen_writes_next_to_the_schema(tmp_path):
    seam_file = tmp_path / "person.seam"
    seam_file.write_text(SCHEMA, encoding="utf-8")

    assert main(["typegen", str(seam_file)]) == 0
    assert (tmp_path / "person_types.py").exists()


def test_check_fails_when_the_generated_file_is_stale(tmp_path):
    seam_file = tmp_path / "person.seam"
    seam_file.write_text(SCHEMA, encoding="utf-8")

    # Missing.
    assert main(["typegen", "--check", str(seam_file)]) == 1

    # Present and current.
    assert main(["typegen", str(seam_file)]) == 0
    assert main(["typegen", "--check", str(seam_file)]) == 0

    # The schema moves on and the generated file does not.
    seam_file.write_text(SCHEMA + "\nschema Extra { x: u8 }\n", encoding="utf-8")
    assert main(["typegen", "--check", str(seam_file)]) == 1


def test_a_broken_schema_reports_rather_than_writing(tmp_path):
    seam_file = tmp_path / "bad.seam"
    seam_file.write_text("schema A { x: Nope }", encoding="utf-8")

    assert main(["typegen", str(seam_file)]) == 1
    assert not (tmp_path / "bad_types.py").exists()


# --- the generated module is not a second source of truth -------------------


def test_deleting_the_generated_file_costs_only_static_checking(tmp_path):
    out = render(tmp_path)
    schema = Schema.load(tmp_path / "person.seam")
    payload = {
        "name": "Gabriel",
        "age": 30,
        "plan": "pro",
        "home": {"city": "Lima"},
        "nickname": None,
        "tags": ["a", None],
    }
    before = schema.validate("Person", payload)

    out.unlink()
    after = Schema.load(tmp_path / "person.seam").validate("Person", payload)
    assert before == after


# --- tagged unions ----------------------------------------------------------

UNION_SCHEMA = """
schema Created {
  who:    String
  amount: u64
}

schema Deleted {
  who:    String
  reason: optional String
}

union Event @tag("type") {
  created: Created
  deleted: Deleted
}

schema Feed {
  id:     u64
  latest: Event
  log:    [Event]
}
"""


def render_union(tmp_path: Path) -> Path:
    seam_file = tmp_path / "feed.seam"
    seam_file.write_text(UNION_SCHEMA, encoding="utf-8")
    out = tmp_path / "feed_types.py"
    out.write_text(generate(seam_file), encoding="utf-8")
    return out


def test_a_union_renders_as_a_union_of_tag_pinned_typed_dicts(tmp_path):
    body = render_union(tmp_path).read_text(encoding="utf-8")
    assert "class EventCreated(Created):" in body
    assert '    type: Literal["created"]' in body
    assert "Event = Union[EventCreated, EventDeleted]" in body


def test_the_tag_is_added_by_the_union_not_the_variant(tmp_path):
    # `Created` carries who and amount and nothing else; the tag is the
    # union's, and the .seam parser refuses a variant that declares it.
    body = render_union(tmp_path).read_text(encoding="utf-8")
    created = body[body.index("class Created"):body.index("class Deleted")]
    assert "type" not in created


def test_a_union_is_a_type_like_any_other(tmp_path):
    body = render_union(tmp_path).read_text(encoding="utf-8")
    assert "    latest: Event" in body
    assert "    log: list[Event]" in body
    assert "def validate_event(" in body


@needs_mypy
def test_the_generated_union_module_type_checks(tmp_path):
    result = mypy(render_union(tmp_path))
    assert result.returncode == 0, result.stdout + result.stderr


@needs_mypy
def test_mypy_narrows_on_the_tag(tmp_path):
    # The point of pinning the tag to a Literal: checking it tells the type
    # checker which variant this is.
    render_union(tmp_path)
    user = tmp_path / "use_it.py"
    user.write_text(
        textwrap.dedent(
            """
            from feed_types import validate_event

            event = validate_event({})
            if event["type"] == "created":
                amount: int = event["amount"]
                print(amount)
            """
        ),
        encoding="utf-8",
    )
    result = mypy(user)
    assert result.returncode == 0, result.stdout + result.stderr


@needs_mypy
def test_mypy_refuses_a_variant_field_without_narrowing(tmp_path):
    render_union(tmp_path)
    user = tmp_path / "use_it.py"
    user.write_text(
        textwrap.dedent(
            """
            from feed_types import validate_event

            event = validate_event({})
            amount: int = event["amount"]
            """
        ),
        encoding="utf-8",
    )
    assert mypy(user).returncode != 0


@needs_mypy
def test_mypy_refuses_the_wrong_variant_after_narrowing(tmp_path):
    render_union(tmp_path)
    user = tmp_path / "use_it.py"
    user.write_text(
        textwrap.dedent(
            """
            from feed_types import validate_event

            event = validate_event({})
            if event["type"] == "deleted":
                amount: int = event["amount"]
                print(amount)
            """
        ),
        encoding="utf-8",
    )
    assert mypy(user).returncode != 0
