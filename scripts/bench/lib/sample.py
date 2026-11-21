#!/usr/bin/env python3
"""Samples CPU% and RSS of process trees from /proc until SIGTERM.

usage: sample.py --out FILE.csv [--interval 0.5] --group NAME=PID[,PID...] [--group ...]

Each group is the union of the given PIDs and all their descendants, re-resolved at every
sample (HAProxy starts a new worker on reload, nginx has several workers). Output columns:
  t_s, group, cpu_pct (100 = one core), rss_kb (sum of VmRSS over the tree), nprocs
"""
import argparse
import os
import signal
import sys
import time

CLK_TCK = os.sysconf("SC_CLK_TCK")


def children(pid):
    out = []
    try:
        for tid in os.listdir(f"/proc/{pid}/task"):
            with open(f"/proc/{pid}/task/{tid}/children") as f:
                out.extend(int(x) for x in f.read().split())
    except OSError:
        pass
    return out


def tree(roots):
    seen, stack = set(), list(roots)
    while stack:
        p = stack.pop()
        if p in seen:
            continue
        seen.add(p)
        stack.extend(children(p))
    return seen


def ticks(pid):
    try:
        with open(f"/proc/{pid}/stat") as f:
            data = f.read()
        fields = data[data.rindex(")") + 2:].split()
        return int(fields[11]) + int(fields[12])  # utime + stime
    except (OSError, ValueError, IndexError):
        return None


def rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--interval", type=float, default=0.5)
    ap.add_argument("--group", action="append", required=True)
    args = ap.parse_args()

    groups = {}
    for g in args.group:
        name, pids = g.split("=", 1)
        groups[name] = [int(p) for p in pids.split(",") if p]

    stop = False

    def on_signal(*_):
        nonlocal stop
        stop = True

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)

    prev = {name: {p: ticks(p) or 0 for p in tree(roots)} for name, roots in groups.items()}
    t0 = last = time.monotonic()
    with open(args.out, "w") as out:
        out.write("t_s,group,cpu_pct,rss_kb,nprocs\n")
        while not stop:
            time.sleep(args.interval)
            now = time.monotonic()
            dt = now - last
            last = now
            for name, roots in groups.items():
                pids = tree(roots)
                cur, delta, rss = {}, 0, 0
                for p in pids:
                    t = ticks(p)
                    if t is None:
                        continue
                    cur[p] = t
                    # A process that appeared since the last sample counts from zero.
                    delta += max(0, t - prev[name].get(p, 0))
                    rss += rss_kb(p)
                prev[name] = cur
                cpu = 100.0 * delta / CLK_TCK / dt if dt > 0 else 0.0
                out.write(f"{now - t0:.2f},{name},{cpu:.1f},{rss},{len(cur)}\n")
            out.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
