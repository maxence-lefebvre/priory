#!/usr/bin/env python3
"""Answers Priory's typed questions with TypeSafe's Jev.

Priory writes {"state": ..., "questions": ...} on stdin. This adds the model, posts it to Jev's
System One API and prints Jev's answer, which has the shape Priory reads. It names a versioned
model: a calibration holds only for the model that answered it.
"""

import json
import os
import sys
import urllib.error
import urllib.request

MODEL = "jev-1.14.0"
URL = "https://api.typesafe.ai/v1/systemone"

request = json.load(sys.stdin)
request["model"] = MODEL
post = urllib.request.Request(
    URL,
    data=json.dumps(request).encode(),
    headers={
        "Authorization": f"Bearer {os.environ['TYPESAFE_API_KEY']}",
        "Content-Type": "application/json",
    },
)
try:
    with urllib.request.urlopen(post, timeout=30) as response:
        sys.stdout.write(response.read().decode())
except urllib.error.HTTPError as error:
    sys.exit(f"TypeSafe answered {error.code}: {error.read().decode()[:200]}")
except urllib.error.URLError as error:
    sys.exit(f"cannot reach TypeSafe: {error.reason}")
