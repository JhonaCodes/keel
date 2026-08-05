#!/usr/bin/env python3
"""Stand-in for tool:awsume.session-active (spec §11.4, ADR-022).

"Live credential RIGHT NOW" precondition: in replay (Phase 0) the state of
the world comes captured in the event (`env`); a live adapter (Phase 1)
would probe the real awsume session.

  valid   → a session exists (AWSUME_EXPIRATION present in the event env)
  invalid → it does not (the precondition fails → fail-closed)
"""
import json
import sys


def main() -> None:
    try:
        event = json.load(sys.stdin)
    except json.JSONDecodeError:
        print(json.dumps({"verdict": "unknown",
                          "findings": [{"message": "unparseable event"}]}))
        return

    env = event.get("env") or {}
    if "AWSUME_EXPIRATION" in env:
        print(json.dumps({"verdict": "valid", "findings": []}))
    else:
        print(json.dumps({"verdict": "invalid",
                          "findings": [{"message": "no active credential session"}]}))


if __name__ == "__main__":
    main()
