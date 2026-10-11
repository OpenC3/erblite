# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

"""Command line interface: ``python -m erblite template.erb [--vars vars.json] [--base-dir DIR]``."""

from __future__ import annotations

import argparse
import json
import os
import sys

from . import Error, render_file


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="erblite", description="Render an ERBLite template.")
    ap.add_argument("template", help="template file")
    ap.add_argument("--vars", help="JSON file with an object of variables")
    ap.add_argument("--base-dir", help="sandbox directory for File/Dir/render (default: template's directory)")
    ap.add_argument("--max-steps", type=int, default=None, help="abort after this many evaluation steps")
    args = ap.parse_args(argv)
    variables = {}
    if args.vars:
        with open(args.vars, encoding="utf-8") as f:
            variables = json.load(f)
        if not isinstance(variables, dict):
            print("erblite: --vars must contain a JSON object", file=sys.stderr)
            return 2
    try:
        out = render_file(
            args.template,
            variables,
            base_dir=args.base_dir or os.path.dirname(os.path.abspath(args.template)),
            filename=args.template,
            max_steps=args.max_steps,
        )
    except Error as e:
        print(str(e), file=sys.stderr)
        return 1
    except OSError as e:
        print(f"erblite: {e}", file=sys.stderr)
        return 2
    sys.stdout.write(out)
    sys.stdout.flush()
    return 0
