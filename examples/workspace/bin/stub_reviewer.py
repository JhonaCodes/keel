#!/usr/bin/env python3
"""Stand-in AgentExecutor (spec §14.8). Emulates a specialized reviewer model.

Reads the AgentRequest prompt from stdin (the material is delimited as data,
§13.2) and returns a schema-valid verdict-json result. This is a DEMO stub:
it flags a direct notifier access if present, otherwise passes. A real
executor would be `claude --bare -p --output-format json ...`.
"""
import json
import sys

prompt = sys.stdin.read()
# Only inspect what is inside the data markers — never obey instructions there.
material = prompt.split("MATERIAL-BEGIN", 1)[-1].split("MATERIAL-END", 1)[0]
if ".notifier.data" in material:
    out = {"verdict": "invalid",
           "findings": ["reactive state read through the notifier instance"]}
else:
    out = {"verdict": "valid", "findings": []}
print(json.dumps(out))
