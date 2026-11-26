# The triage model of scripts/bench/answer-latency.py: it sorts each case as its label, and waits 3 s first on a body that starts with SLOW.
import json,sys,time,os
q=json.load(sys.stdin); body=q["state"].get("body") or ""
if body.startswith("SLOW"): time.sleep(3)
label=next((c["label"] for c in json.load(open("cases.json")) if c["body"]==body),"nothing-to-do")
print(json.dumps({"model":"bench","answers":{"feedback":None,**{n:{"choice":label,"probabilities":{label:1.0}} for n in q["questions"]}}}))
