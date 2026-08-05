#!/usr/bin/env python3
"""Stand-in for tool:reactive-notifier.validate-access (spec §11.3).

A real implementation is AST analysis of the edited Dart (0 tokens,
deterministic). The stand-in demonstrates the three-state contract with
findings located by line:

  invalid → direct `.notifier.data` access outside a comment
  valid   → no direct accesses
  unknown → no content to analyze
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

    content = event.get("content")
    if not content:
        print(json.dumps({"verdict": "unknown",
                          "findings": [{"message": "no content to analyze"}]}))
        return

    findings = []
    for line_no, line in enumerate(content.splitlines(), start=1):
        code = line.split("//")[0]  # ignore line comments
        if re.search(r"\.notifier\.data\b", code):
            findings.append({
                "message": "direct access to notifier data",
                "file": event.get("file"),
                "line": line_no,
            })

    verdict = "invalid" if findings else "valid"
    print(json.dumps({"verdict": verdict, "findings": findings}))


if __name__ == "__main__":
    main()
