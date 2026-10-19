"""Compares per-function Ir across all runs of all jobs and prints a
Markdown report (also appended to the GitHub step summary).

Usage: compare.py <results.jsonl>...
"""

import json
import os
import statistics
import sys
from collections import defaultdict

records = []
for path in sys.argv[1:]:
    records += [json.loads(l) for l in open(path) if l.strip()]
records.sort(key=lambda r: (r["job"], r["run"]))

out = []
out.append(f"## Callgrind stability: {len(records)} runs on "
           f"{len({r['job'] for r in records})} runners\n")

out.append("### Runners\n")
out.append("| job | CPU | features seen under Valgrind |")
out.append("| --- | --- | --- |")
for job in sorted({r["job"] for r in records}):
    r = next(r for r in records if r["job"] == job)
    out.append(f"| {job} | {r['cpu']} | {r['valgrind_cpu_features']} |")

out.append("\n### Inclusive Ir per function\n")
out.append("| function | distinct values | min | max | spread | within-runner spread (max) | by CPU |")
out.append("| --- | --- | --- | --- | --- | --- | --- |")
names = list(records[0]["counts"].keys()) + ["program_total"]
all_identical = []
for name in names:
    vals = [r["program_total"] if name == "program_total" else r["counts"][name] for r in records]
    if any(v is None for v in vals):
        out.append(f"| {name} | symbol not found | | | | | |")
        continue
    spread = (max(vals) - min(vals)) / statistics.median(vals) * 100
    per_job = defaultdict(list)
    per_cpu = defaultdict(set)
    for r, v in zip(records, vals):
        per_job[r["job"]].append(v)
        per_cpu[r["cpu"]].add(v)
    within = max((max(v) - min(v)) / statistics.median(v) * 100 for v in per_job.values())
    by_cpu = "<br>".join(
        f"{cpu.split('@')[0].strip()}: {', '.join(f'{x:,}' for x in sorted(v))}"
        for cpu, v in sorted(per_cpu.items())
    ) if len(set(vals)) > 1 else "identical"
    if len(set(vals)) == 1:
        all_identical.append(name)
    out.append(f"| {name} | {len(set(vals))} | {min(vals):,} | {max(vals):,} | "
               f"{spread:.4f}% | {within:.4f}% | {by_cpu} |")

out.append(f"\nIdentical on every run and runner: {', '.join(all_identical) or 'none'}")

report = "\n".join(out)
print(report)
summary = os.environ.get("GITHUB_STEP_SUMMARY")
if summary:
    with open(summary, "a") as f:
        f.write(report + "\n")
