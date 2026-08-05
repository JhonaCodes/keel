#!/usr/bin/env python3
"""Stand-in for tool:sqlglot.classify-statement (spec §11.4).

NAIVE classifier of SQL statements extracted from a command. A real
implementation wraps sqlglot (AST); an honest three-state classifier is
enough here to demonstrate the contract:

  invalid  → DROP/TRUNCATE, or DELETE/UPDATE without WHERE (certain violation)
  valid    → SELECT, or scoped DML with WHERE (certain conformance)
  unknown  → the SQL could not be extracted/classified (undecidable → the
             runtime applies the unknown branch: deny-pending-approval on
             irreversibles)

Contract: JSON event via stdin → verdict-json via stdout.
"""
import json
import re
import sys


def main() -> None:
    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        print(json.dumps({"verdict": "unknown",
                          "findings": [{"message": "unparseable event"}]}))
        return

    command = event.get("command") or ""
    match = re.search(
        r"(?i)\b(select|insert|update|delete|drop|truncate|alter|create)\b",
        command,
    )
    if not match:
        # Runtime-built SQL / not visible in the command: undecidable.
        print(json.dumps({"verdict": "unknown",
                          "findings": [{"message": "could not extract a SQL statement from the command"}]}))
        return

    kind = match.group(1).lower()
    has_where = re.search(r"(?i)\bwhere\b", command) is not None

    if kind in ("drop", "truncate"):
        verdict, msg = "invalid", f"{kind.upper()} is destructive"
    elif kind in ("delete", "update") and not has_where:
        verdict, msg = "invalid", f"{kind.upper()} without WHERE"
    else:
        verdict, msg = "valid", f"{kind.upper()} scoped"

    findings = [] if verdict == "valid" else [{"message": msg}]
    print(json.dumps({"verdict": verdict, "findings": findings}))


if __name__ == "__main__":
    main()
