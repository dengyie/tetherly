#!/usr/bin/env python3
"""Real-device acceptance check for the Windows -> Mac crossing (§111/§112).

Records a start time, lets the operator perform the gesture for N seconds,
then pulls both nodes' /api/input/trace and judges:

  * Mac: every `cursor_reveal controller entered` must end visible
    (`visible=..->true`). An invisible pointer on Enter was the v0.2.39
    root cause of the "can't get across for 10-20 s" symptom.
  * Windows: every controller stint into the Mac is listed with how it
    ended. A return within --bounce-ms whose stroke is balanced
    (away and toward both large) is a "search wiggle" — the user could
    not see the pointer. At least one stint must stay on the Mac longer
    than --bounce-ms.

Read-only: it only issues GET requests and an ssh curl; nothing on either
node is changed.

Usage:
  scripts/verify_crossing.py                 # 30 s window, live
  scripts/verify_crossing.py --seconds 45
  scripts/verify_crossing.py --mac-trace m.json --win-trace w.json --since 0
                                             # offline, from saved traces

Exit code: 0 PASS, 1 FAIL, 2 not enough data / collection error.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.request

MAC_UI = "http://127.0.0.1:45716"
WIN_HOST = "home-win"
WIN_TRACE_CMD = "curl.exe -s http://127.0.0.1:45716/api/input/trace"
WIN_HEALTH_CMD = "curl.exe -s http://127.0.0.1:45716/api/health"

REVEAL_RE = re.compile(r"visible=(true|false)->(true|false)")
EXTRA_RE = re.compile(r"extra_shows=(\d+)")
RETURN_RE = re.compile(r"return elapsed_ms=(\d+)")
KV_RE = re.compile(r"(\w+)=([\w./-]+)")


# ---------------------------------------------------------------- collection


def fetch_mac(path: str) -> dict:
    with urllib.request.urlopen(MAC_UI + path, timeout=5) as r:
        return json.loads(r.read().decode())


def fetch_win(cmd: str) -> dict:
    out = subprocess.run(
        ["ssh", "-o", "ConnectTimeout=8", WIN_HOST, cmd],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if out.returncode != 0:
        raise RuntimeError(f"ssh {WIN_HOST} failed ({out.returncode}): {out.stderr.strip()}")
    return json.loads(out.stdout)


def load_file(path: str) -> dict:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------ analysis


def mac_reveals(entries: list[dict], since: int) -> list[dict]:
    out = []
    for e in entries:
        if e["ts_ms"] < since or e["kind"] != "cursor_reveal":
            continue
        m = REVEAL_RE.search(e["detail"])
        if not m:
            continue
        x = EXTRA_RE.search(e["detail"])
        out.append(
            {
                "ts": e["ts_ms"],
                "before": m.group(1) == "true",
                "after": m.group(2) == "true",
                "idle_hidden": "idle_hidden=true" in e["detail"],
                "extra_shows": int(x.group(1)) if x else None,
            }
        )
    return out


def win_stints(entries: list[dict], since: int) -> list[dict]:
    """Controller stints into the peer: `seat local -> remote` opens one,
    `seat remote -> local` closes it; the crossing trace right before the
    close carries the return diagnostics."""
    stints: list[dict] = []
    cur = None
    last_return = None
    for e in entries:
        if e["ts_ms"] < since:
            continue
        k, d = e["kind"], e["detail"]
        if k == "crossing" and "return elapsed_ms=" in d:
            last_return = dict(KV_RE.findall(d))
        elif k == "seat" and "local -> remote" in d:
            cur = {"start": e["ts_ms"], "end": None, "diag": None}
            last_return = None
        elif k == "seat" and "remote -> local" in d and cur is not None:
            cur["end"] = e["ts_ms"]
            cur["diag"] = last_return
            stints.append(cur)
            cur = None
    if cur is not None:
        stints.append(cur)  # still on the peer at collection time
    return stints


def classify(stint: dict, bounce_ms: int) -> str:
    if stint["end"] is None:
        return "STAYED"
    dur = stint["end"] - stint["start"]
    if dur >= bounce_ms:
        return "OK"
    d = stint["diag"] or {}
    away, toward = int(d.get("away", 0)), int(d.get("toward", 0))
    if away >= 150 and toward >= 150:
        return "BOUNCE(wiggle)"
    return "BOUNCE"


def judge(mac_entries, win_entries, since, bounce_ms) -> tuple[int, list[str]]:
    lines: list[str] = []
    reveals = mac_reveals(mac_entries, since)
    stints = win_stints(win_entries, since)
    fail = False

    lines.append(f"Mac cursor_reveal ({len(reveals)}):")
    for r in reveals:
        tag = "OK " if r["after"] else "BAD"
        extra = "" if r["extra_shows"] is None else f" extra_shows={r['extra_shows']}"
        lines.append(
            f"  [{tag}] {r['ts']} visible={r['before']}->{r['after']}"
            f" idle_hidden={r['idle_hidden']}{extra}"
        )
        fail |= not r["after"]

    lines.append(f"Windows controller stints ({len(stints)}):")
    long_ok = False
    for s in stints:
        c = classify(s, bounce_ms)
        dur = "open" if s["end"] is None else f"{s['end'] - s['start']}ms"
        d = s["diag"] or {}
        extra = " ".join(
            f"{k}={d[k]}" for k in ("peak_depth", "push", "away", "toward", "max_toward", "relay") if k in d
        )
        lines.append(f"  [{c}] {s['start']} dur={dur} {extra}".rstrip())
        long_ok |= c in ("OK", "STAYED")

    if not stints and not reveals:
        lines.append("NO DATA: no crossing happened in the window (or the trace ring rolled over).")
        return 2, lines
    if stints and not long_ok:
        lines.append(f"FAIL: every stint returned within {bounce_ms} ms.")
        fail = True
    if any(not r["after"] for r in reveals):
        lines.append("FAIL: the Mac pointer was still invisible after a controller Enter.")
    lines.append("RESULT: " + ("FAIL" if fail else "PASS"))
    return (1 if fail else 0), lines


# ---------------------------------------------------------------------- main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=int, default=30, help="gesture window (live mode)")
    ap.add_argument("--bounce-ms", type=int, default=1500, help="a stint shorter than this is a bounce")
    ap.add_argument("--mac-trace", help="offline: saved Mac trace JSON")
    ap.add_argument("--win-trace", help="offline: saved Windows trace JSON")
    ap.add_argument("--since", type=int, help="offline: only entries at/after this ts_ms")
    a = ap.parse_args()

    offline = bool(a.mac_trace or a.win_trace)
    try:
        if offline:
            mac = load_file(a.mac_trace) if a.mac_trace else {"entries": []}
            win = load_file(a.win_trace) if a.win_trace else {"entries": []}
            since = a.since or 0
        else:
            mh = fetch_mac("/api/health")
            wh = fetch_win(WIN_HEALTH_CMD)
            print(f"Mac {mh.get('version')} pid {mh.get('pid')} | Windows {wh.get('version')} pid {wh.get('pid')}")
            since = int(time.time() * 1000)
            print(
                f"开始：先用 Mac 鼠标进 Windows 控制几秒，再拿 Windows 鼠标接管并划向 Mac（底边），"
                f"在 Mac 上停留移动 2 s 以上。窗口 {a.seconds}s。"
            )
            for left in range(a.seconds, 0, -5):
                print(f"  ...{left}s", flush=True)
                time.sleep(min(5, left))
            mac = fetch_mac("/api/input/trace")
            win = fetch_win(WIN_TRACE_CMD)
    except Exception as exc:  # collection only; analysis errors should surface
        print(f"COLLECT ERROR: {exc}", file=sys.stderr)
        return 2

    for name, t in (("Mac", mac), ("Windows", win)):
        es = t.get("entries", [])
        if es and not offline and es[0]["ts_ms"] > since:
            print(f"WARN: {name} trace ring rolled over inside the window; shorten --seconds.")

    code, lines = judge(mac.get("entries", []), win.get("entries", []), since, a.bounce_ms)
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    sys.exit(main())
