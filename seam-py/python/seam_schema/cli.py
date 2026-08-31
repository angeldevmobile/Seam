"""The `seam` command."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from ._seam import ParseError
from .typegen import generate, output_path


def _typegen(args: argparse.Namespace) -> int:
    status = 0
    for schema in args.schemas:
        try:
            rendered = generate(schema)
        except ParseError as e:
            print(f"error: {e}", file=sys.stderr)
            status = 1
            continue

        target = Path(args.output) if args.output else output_path(schema)

        if args.check:
            current = target.read_text(encoding="utf-8") if target.exists() else None
            if current != rendered:
                what = "is out of date" if current is not None else "is missing"
                print(f"{target} {what}; run `seam typegen {schema}`", file=sys.stderr)
                status = 1
            continue

        target.parent.mkdir(parents=True, exist_ok=True)
        # `open`, not `write_text`: the latter only grew a `newline` argument
        # in 3.10, and the generated file has to carry the same bytes on every
        # platform or `--check` would fail over a line ending.
        with target.open("w", encoding="utf-8", newline="\n") as f:
            f.write(rendered)
        print(f"wrote {target}")

    return status


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="seam", description="Seam tooling.")
    sub = parser.add_subparsers(dest="command", required=True)

    typegen = sub.add_parser(
        "typegen",
        help="generate TypedDicts from .seam files",
        description=(
            "Generates TypedDicts so a type checker can see the shape of a "
            "validated payload. The generated file holds no rules."
        ),
    )
    typegen.add_argument("schemas", nargs="+", help=".seam files")
    typegen.add_argument("-o", "--output", help="output path (single schema only)")
    typegen.add_argument(
        "--check",
        action="store_true",
        help="fail if the generated file is missing or out of date, for CI",
    )
    typegen.set_defaults(func=_typegen)

    args = parser.parse_args(argv)
    if args.output and len(args.schemas) > 1:
        parser.error("--output takes a single schema")
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
