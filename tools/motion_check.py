import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PIECES = ["tarnation", "grief", "tomandjerry", "theentertainer", "mountainking", "faroffisle"]
DOWNLOADS = Path.home() / "Downloads"
OUT = ROOT / "out" / "motion"

PATTERNS = {
    "strays": r"stray touches \(a finger pushing a key nobody is playing\): (\d+)",
    "steps": r"places a hand steps rather than moves: (\d+)",
    "wrist_twitches": r"wrist twitches\s+(\d+)",
    "tip_twitches": r"tips  twitches\s+(\d+)",
    "contact_p95": r"fingertip to its key as it sounds \(mm\):\s+p50\s+\S+\s+p95\s+(\S+)",
}


def measure(exe, midi, model):
    env = dict(os.environ, ON_MODEL=str(model))
    text = subprocess.run([str(exe), str(midi)], env=env, capture_output=True, text=True).stdout
    found = {}
    for key, pattern in PATTERNS.items():
        values = [float(v) for v in re.findall(pattern, text)]
        found[key] = sum(values) if values else None
    return found, text


def main():
    exe = ROOT / "target" / "release" / "examples" / "jitter.exe"
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "on-viz", "--example", "jitter"], cwd=ROOT, check=True)
    model = ROOT / "models" / (sys.argv[1] if len(sys.argv) > 1 else "helios-beta1.5.json")
    OUT.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y-%m-%d_%H-%M")
    report = {"when": stamp, "model": model.name, "pieces": {}}
    with open(OUT / f"{stamp}.txt", "w", encoding="utf-8") as log:
        for piece in PIECES:
            midi = DOWNLOADS / ("faroffisle.mid" if piece == "faroffisle" else f"{piece}.mid")
            if not midi.exists():
                continue
            print(f"measuring {piece}", flush=True)
            found, text = measure(exe, midi, model)
            report["pieces"][piece] = found
            log.write(text)
    totals = {key: sum(p[key] or 0 for p in report["pieces"].values()) for key in PATTERNS if key != "contact_p95"}
    contacts = [p["contact_p95"] for p in report["pieces"].values() if p["contact_p95"] is not None]
    totals["contact_p95_worst"] = max(contacts) if contacts else None
    report["totals"] = totals
    (OUT / "latest.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(json.dumps(totals))


if __name__ == "__main__":
    main()
