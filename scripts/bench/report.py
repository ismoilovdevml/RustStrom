#!/usr/bin/env python3
"""Builds report.md from a run directory written by run.sh.

usage: report.py RUN_DIR > RUN_DIR/report.md

Every number comes from the raw files next to it: <scenario>/<lb>-<step>.txt (tool output),
.csv (CPU/RSS samples from lib/sample.py) and .meta (command, load average, events).
"""
import csv
import os
import re
import sys

LBS = ["haproxy", "ruststrom"]
LB_TITLE = {"haproxy": "HAProxy", "ruststrom": "RustStrom"}
SAME_BAND_PCT = 3.0  # differences inside +-3 % are reported as "same"

UNIT_TIME = {"us": 1e-3, "ms": 1.0, "s": 1e3, "m": 60e3, "h": 3600e3}  # -> milliseconds
UNIT_BYTES = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3, "TB": 1024**4}


# ------------------------------------------------------------------ parsing

def read(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return None


def read_kv(path):
    out = {}
    for line in (read(path) or "").splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            out.setdefault(k.strip(), v.strip())
    return out


def to_ms(value, unit):
    return float(value) * UNIT_TIME[unit]


def parse_wrk(text):
    """wrk and wrk2 share the summary format; wrk2 adds the HdrHistogram percentiles."""
    if not text:
        return None
    r = {}
    m = re.search(r"^Requests/sec:\s+([\d.]+)", text, re.M)
    r["rps"] = float(m.group(1)) if m else None
    m = re.search(r"(\d+) requests in ([\d.]+)(\w+), ([\d.]+)(\w+) read", text)
    if m:
        r["requests"] = int(m.group(1))
        r["seconds"] = to_ms(m.group(2), m.group(3)) / 1000
        r["bytes"] = float(m.group(4)) * UNIT_BYTES.get(m.group(5), 1)
    m = re.search(r"Socket errors: connect (\d+), read (\d+), write (\d+), timeout (\d+)", text)
    r["err_connect"], r["err_read"], r["err_write"], r["err_timeout"] = (
        [int(x) for x in m.groups()] if m else [0, 0, 0, 0])
    m = re.search(r"Non-2xx or 3xx responses: (\d+)", text)
    r["non2xx"] = int(m.group(1)) if m else 0
    r["errors"] = r["err_connect"] + r["err_read"] + r["err_write"] + r["err_timeout"] + r["non2xx"]
    pct = {}
    # wrk2: " 99.900%    1.23ms"; wrk: "     99%    1.23ms"
    for m in re.finditer(r"^\s+(\d+(?:\.\d+)?)%\s+([\d.]+)(us|ms|s|m)\s*$", text, re.M):
        key = float(m.group(1))
        pct.setdefault(key, to_ms(m.group(2), m.group(3)))
    r["pct"] = pct
    if r["rps"] is None:
        r["failed_output"] = text.strip().splitlines()[-1:] or ["empty output"]
    return r


def parse_h2load(text):
    if not text:
        return None
    r = {}
    m = re.search(r"finished in [\d.]+\w+, ([\d.]+) req/s, ([\d.]+)(\w+)/s", text)
    r["rps"] = float(m.group(1)) if m else None
    m = re.search(r"requests: (\d+) total, (\d+) started, (\d+) done, (\d+) succeeded, "
                  r"(\d+) failed, (\d+) errored, (\d+) timeout", text)
    if m:
        total, _, done, ok, failed, errored, timeout = (int(x) for x in m.groups())
        r["requests"] = done
        r["errors"] = failed + errored + timeout
    m = re.search(r"status codes: (\d+) 2xx, (\d+) 3xx, (\d+) 4xx, (\d+) 5xx", text)
    if m:
        r["errors"] = r.get("errors", 0) + int(m.group(3)) + int(m.group(4))
    m = re.search(r"time for request:\s+([\d.]+)(us|ms|s)\s+([\d.]+)(us|ms|s)\s+([\d.]+)(us|ms|s)", text)
    if m:
        r["lat_min"] = to_ms(m.group(1), m.group(2))
        r["lat_max"] = to_ms(m.group(3), m.group(4))
        r["lat_mean"] = to_ms(m.group(5), m.group(6))
    for key, pat in (("tls", r"TLS Protocol: (\S+)"), ("cipher", r"Cipher: (\S+)"),
                     ("kx", r"Server Temp Key: (.+)"), ("alpn", r"Application protocol: (\S+)")):
        m = re.search(pat, text)
        r[key] = m.group(1).strip() if m else None
    return r


def parse_samples(path):
    """Per group: average CPU % (100 = one core), peak and last RSS in KiB."""
    groups = {}
    try:
        with open(path) as f:
            for row in csv.DictReader(f):
                g = groups.setdefault(row["group"], {"cpu": [], "rss": []})
                g["cpu"].append(float(row["cpu_pct"]))
                g["rss"].append(int(row["rss_kb"]))
    except OSError:
        return {}
    out = {}
    for name, g in groups.items():
        if not g["cpu"]:
            continue
        # Drop the last sample: it may cover the tool exiting part-way through the interval.
        cpu = g["cpu"][:-1] or g["cpu"]
        out[name] = {"cpu_avg": sum(cpu) / len(cpu), "cpu_max": max(cpu),
                     "rss_peak_kb": max(g["rss"]), "rss_avg_kb": sum(g["rss"]) / len(g["rss"])}
    return out


def error_breakdown(r):
    if not r:
        return None
    return ("non-2xx {non2xx}, connect {err_connect}, read {err_read}, write {err_write}, "
            "timeout {err_timeout}".format(**r))


def cpu_count(spec):
    n = 0
    for part in (spec or "").split(","):
        if "-" in part:
            a, b = part.split("-")
            n += int(b) - int(a) + 1
        elif part:
            n += 1
    return n


# ------------------------------------------------------------------ formatting

def fmt_num(v, digits=0):
    if v is None:
        return "—"
    return f"{v:,.{digits}f}"


def compare(hp, rs, higher_is_better, worse_word="worse"):
    if hp is None or rs is None:
        return "—"
    if hp == 0:
        return "same" if rs == 0 else "n/a (HAProxy 0)"
    pct = (rs - hp) / abs(hp) * 100
    if not VALID:
        return f"{pct:+.1f}%"
    if abs(pct) <= SAME_BAND_PCT:
        verdict = "same"
    elif (pct > 0) == higher_is_better:
        verdict = "better"
    else:
        verdict = worse_word
    return f"{pct:+.1f}% ({verdict})"


class Table:
    def __init__(self, lbs):
        self.lbs = lbs
        self.rows = []

    def add(self, label, values, fmt, higher_is_better=None, worse_word="worse"):
        cells = [fmt(values.get(lb)) for lb in self.lbs]
        if len(self.lbs) == 2 and higher_is_better is not None:
            cells.append(compare(values.get("haproxy"), values.get("ruststrom"),
                                 higher_is_better, worse_word))
        elif len(self.lbs) == 2:
            cells.append("")
        self.rows.append([label] + cells)

    def render(self):
        head = ["Metric"] + [LB_TITLE[lb] for lb in self.lbs]
        if len(self.lbs) == 2:
            head.append("RustStrom vs HAProxy" + ("" if VALID else " (invalid run, no verdict)"))
        lines = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
        lines += ["| " + " | ".join(str(c) for c in row) + " |" for row in self.rows]
        return "\n".join(lines)


# ------------------------------------------------------------------ report

def step(run, scen, lb, name, parser):
    base = os.path.join(run, scen, f"{lb}-{name}")
    return (parser(read(base + ".txt")), parse_samples(base + ".csv"), read_kv(base + ".meta"))


def main():
    global VALID
    run = sys.argv[1]
    env = read_kv(os.path.join(run, "env.txt"))
    env_text = read(os.path.join(run, "env.txt")) or ""
    VALID = env.get("valid") == "1"
    lbs = [lb for lb in LBS if any(
        os.path.exists(os.path.join(run, d, f)) for d in os.listdir(run)
        if os.path.isdir(os.path.join(run, d)) for f in os.listdir(os.path.join(run, d))
        if f.startswith(lb + "-"))]
    n_gen = cpu_count(env.get("cpus_loadgen"))
    n_be = cpu_count(env.get("cpus_backends"))
    n_lb = cpu_count(env.get("cpus_lb"))
    warnings = []

    def check_saturation(scen, lb, name, samples, meta, wrk=None):
        gen = samples.get("loadgen", {}).get("cpu_avg")
        be = samples.get("backends", {}).get("cpu_avg")
        if gen is not None and gen > 0.85 * n_gen * 100:
            warnings.append(f"{scen}/{lb}/{name}: load generator CPU averaged {gen:.0f}% of "
                            f"{n_gen * 100}%; the client may be the bottleneck.")
        if be is not None and be > 0.85 * n_be * 100:
            warnings.append(f"{scen}/{lb}/{name}: backends CPU averaged {be:.0f}% of "
                            f"{n_be * 100}%; the backends may be the bottleneck.")
        if meta.get("exit_code", "0") != "0":
            warnings.append(f"{scen}/{lb}/{name}: tool exited with {meta.get('exit_code')}.")
        if wrk is not None and wrk.get("rps") is None:
            warnings.append(f"{scen}/{lb}/{name}: no result parsed ({wrk.get('failed_output')}).")
        la = meta.get("loadavg_before", "")
        if la and float(la.split()[0]) >= 1.0:
            warnings.append(f"{scen}/{lb}/{name}: load average before the step was {la.split()[0]}.")

    out = []
    w = out.append
    w(f"# RustStrom vs HAProxy benchmark: run {env.get('run_id', '?')}\n")
    if VALID:
        w("> Valid run: 1-minute load average was below 1.0 at the start and every step ran with "
          "the full duration. Check the warnings section before quoting numbers.\n")
    else:
        w("> **NOT VALID. Do not quote or compare these numbers.** This run only validates the "
          f"harness. Reasons: {env.get('invalid_reasons') or 'unknown'}.\n")

    # Environment
    w("## Environment\n")
    w("| Item | Value |\n|---|---|")
    for key, label in [
        ("date", "Date (UTC)"), ("host", "Host"), ("os", "OS"), ("kernel", "Kernel"),
        ("cpu_model", "CPU"), ("cpus", "vCPUs"), ("numa", "NUMA nodes (CPUs)"),
        ("mem_total", "Memory"), ("loadavg_start", "Load average at start (1/5/15 min)"),
        ("loadavg_end", "Load average at end"), ("cpus_loadgen", "Load generator cores"),
        ("cpus_lb", "Load balancer cores"), ("cpus_backends", "Backend cores"),
        ("lb_threads", "LB threads (HAProxy nbthread / RustStrom workers)"),
        ("backends", "Backends"), ("target", "Client target"),
        ("version_haproxy", "HAProxy"), ("haproxy_image", "HAProxy image"),
        ("version_ruststrom", "RustStrom"), ("ruststrom_bin", "RustStrom binary"),
        ("ruststrom_sha256", "RustStrom sha256 (prefix)"), ("nginx_image", "Backend image"),
        ("cert", "Certificate"), ("podman", "Podman"), ("duration_s", "Seconds per step"),
        ("event_duration_s", "Seconds per failover/reload step"), ("warmup_s", "Warm-up seconds"),
        ("conns", "Concurrency"), ("sysctl_tuning", "Sysctl tuning"), ("label", "Label"),
        ("harness_rev", "Harness revision"),
    ]:
        if env.get(key):
            w(f"| {label} | {env[key].replace('|', '/')} |")
    for line in env_text.splitlines():
        if line.startswith("tool: "):
            w(f"| Tool | {line[6:]} |")
        elif line.startswith("sysctl: "):
            w(f"| sysctl during run | `{line[8:]}` |")
    w("")

    # 1. plain HTTP
    scen = "s1_http_1k"
    if os.path.isdir(os.path.join(run, scen)):
        w("## 1. Plain HTTP/1.1 keep-alive, 1 KB responses\n")
        t = Table(lbs)
        mx = {lb: step(run, scen, lb, "max", parse_wrk) for lb in lbs}
        for lb, (r, s, m) in mx.items():
            check_saturation(scen, lb, "max", s, m, r)
        rps = {lb: (mx[lb][0] or {}).get("rps") for lb in lbs}
        lbcpu = {lb: mx[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}
        t.add("Max throughput, wrk (req/s)", rps, fmt_num, True, "slower")
        t.add(f"LB CPU at max (avg %, {n_lb * 100}% = all LB cores)", lbcpu, fmt_num, False)
        t.add("Requests per LB core-second at max",
              {lb: rps[lb] / (lbcpu[lb] / 100) if rps[lb] and lbcpu[lb] else None for lb in lbs},
              fmt_num, True)
        t.add("LB peak RSS at max (MiB)",
              {lb: mx[lb][1].get("lb", {}).get("rss_peak_kb", 0) / 1024 or None for lb in lbs},
              lambda v: fmt_num(v, 1), False)
        t.add("Errors at max (socket + non-2xx)",
              {lb: (mx[lb][0] or {}).get("errors") for lb in lbs}, fmt_num, False)
        ref = env.get("s1_reference_max_rps")
        for frac in (50, 80):
            st = {lb: step(run, scen, lb, f"rate{frac}", parse_wrk) for lb in lbs}
            if not any(st[lb][0] for lb in lbs):
                continue
            target = re.search(r"-R (\d+)", st[lbs[0]][2].get("cmd", ""))
            target = int(target.group(1)) if target else None
            for lb, (r, s, m) in st.items():
                check_saturation(scen, lb, f"rate{frac}", s, m, r)
                if r and r.get("rps") and target and r["rps"] < 0.95 * target:
                    warnings.append(f"{scen}/{lb}/rate{frac}: achieved {r['rps']:.0f} req/s, "
                                    f"below the {target} req/s target.")
            label = f"{frac}% load ({fmt_num(target)} req/s, wrk2)"
            for p, pname in ((50.0, "p50"), (99.0, "p99"), (99.9, "p99.9")):
                t.add(f"{label}: {pname} latency (ms)",
                      {lb: (st[lb][0] or {}).get("pct", {}).get(p) for lb in lbs},
                      lambda v: fmt_num(v, 3), False, "slower")
            t.add(f"{label}: LB CPU (avg %)",
                  {lb: st[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}, fmt_num, False)
            t.add(f"{label}: errors", {lb: (st[lb][0] or {}).get("errors") for lb in lbs},
                  fmt_num, False)
        w(t.render())
        w(f"\nLatency steps use the same offered load for both load balancers: a fraction of the "
          f"lower of the two max results ({fmt_num(float(ref)) if ref else '—'} req/s). wrk2 "
          f"latencies are corrected for coordinated omission.\n")

    # 2. TLS handshakes
    scen = "s2_tls_handshake"
    if os.path.isdir(os.path.join(run, scen)):
        w("## 2. TLS 1.3 full handshakes (new connection per request)\n")
        t = Table(lbs)
        st = {lb: step(run, scen, lb, "handshake", parse_wrk) for lb in lbs}
        for lb, (r, s, m) in st.items():
            check_saturation(scen, lb, "handshake", s, m, r)
        t.add("New TLS connections per second", {lb: (st[lb][0] or {}).get("rps") for lb in lbs},
              fmt_num, True, "slower")
        t.add(f"LB CPU (avg %, {n_lb * 100}% = all LB cores)",
              {lb: st[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}, fmt_num, False)
        t.add("Load generator CPU (avg %)",
              {lb: st[lb][1].get("loadgen", {}).get("cpu_avg") for lb in lbs}, fmt_num, None)
        t.add("p99 latency incl. handshake (ms)",
              {lb: (st[lb][0] or {}).get("pct", {}).get(99.0) for lb in lbs},
              lambda v: fmt_num(v, 3), False, "slower")
        t.add("Errors (socket + non-2xx)", {lb: (st[lb][0] or {}).get("errors") for lb in lbs},
              fmt_num, False)
        t.add("Error breakdown", {lb: error_breakdown(st[lb][0]) for lb in lbs},
              lambda v: v or "—", None)
        w(t.render())
        hm = st.get("haproxy", (None, None, {}))[2]
        if hm.get("haproxy_new_tls_sessions"):
            w(f"\nHAProxy counted {hm['haproxy_new_tls_sessions']} new TLS sessions during the step; "
              f"mid-run it reported SslRate {hm.get('haproxy_mid_SslRate', '?')}/s, "
              f"SslFrontendKeyRate {hm.get('haproxy_mid_SslFrontendKeyRate', '?')}/s, session reuse "
              f"{hm.get('haproxy_mid_SslFrontendSessionReuse_pct', '?')}% (0% = every handshake was "
              f"a full handshake).")
            reuse = hm.get("haproxy_mid_SslFrontendSessionReuse_pct", "")
            if reuse.isdigit() and int(reuse) > 5:
                warnings.append(f"{scen}: HAProxy reported {reuse}% TLS session reuse mid-run; the "
                                f"step measured resumed, not full, handshakes and is invalid.")
        for lb in lbs:
            r = st[lb][0]
            hdrs = read(os.path.join(run, scen, f"{lb}-close-headers.txt")) or ""
            conn_hdr = re.search(r"^connection:\s*(.+)$", hdrs, re.I | re.M)
            if r and r.get("requests") and r.get("err_read", 0) >= 0.9 * r["requests"]:
                w(f"\n{LB_TITLE[lb]}: almost every connection ended in a wrk read error. The "
                  f"response to a `Connection: close` request carried `connection: "
                  f"{conn_hdr.group(1) if conn_hdr else '(none)'}`, so wrk expected keep-alive and "
                  f"then saw the server close. Each request still completed and counts as one "
                  f"handshake; the errors are a protocol finding, not lost requests.")
        probe = read(os.path.join(run, "logs", "tls-probe.txt"))
        if probe:
            w("\nNegotiated parameters (`openssl s_client` probe):\n\n```\n" + probe.strip() + "\n```")
        w("")

    # 3. h2
    scen = "s3_h2"
    if os.path.isdir(os.path.join(run, scen)):
        w("## 3. HTTPS with HTTP/2 (ALPN h2), keep-alive, 1 KB\n")
        t = Table(lbs)
        st = {lb: step(run, scen, lb, "h2", parse_h2load) for lb in lbs}
        for lb, (r, s, m) in st.items():
            check_saturation(scen, lb, "h2", s, m)
            if r and r.get("alpn") != "h2":
                warnings.append(f"{scen}/{lb}: negotiated '{r.get('alpn')}' instead of h2.")
        t.add("Throughput, h2load (req/s)", {lb: (st[lb][0] or {}).get("rps") for lb in lbs},
              fmt_num, True, "slower")
        t.add("Mean request time (ms)", {lb: (st[lb][0] or {}).get("lat_mean") for lb in lbs},
              lambda v: fmt_num(v, 3), False, "slower")
        t.add("Max request time (ms)", {lb: (st[lb][0] or {}).get("lat_max") for lb in lbs},
              lambda v: fmt_num(v, 3), False, "slower")
        t.add(f"LB CPU (avg %, {n_lb * 100}% = all LB cores)",
              {lb: st[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}, fmt_num, False)
        t.add("LB peak RSS (MiB)",
              {lb: st[lb][1].get("lb", {}).get("rss_peak_kb", 0) / 1024 or None for lb in lbs},
              lambda v: fmt_num(v, 1), False)
        t.add("Errors (failed/errored/timeout + 4xx/5xx)",
              {lb: (st[lb][0] or {}).get("errors") for lb in lbs}, fmt_num, False)
        t.add("Negotiated", {lb: " / ".join(str((st[lb][0] or {}).get(k)) for k in ("tls", "alpn"))
                             for lb in lbs}, str, None)
        w(t.render())
        w("\nh2load reports mean and max only, no percentiles.\n")

    # 4. large bodies
    scen = "s4_large"
    if os.path.isdir(os.path.join(run, scen)):
        w("## 4. Large responses, plain HTTP/1.1 keep-alive\n")
        t = Table(lbs)
        for size in ("100k", "10m"):
            st = {lb: step(run, scen, lb, size, parse_wrk) for lb in lbs}
            if not any(st[lb][0] for lb in lbs):
                continue
            for lb, (r, s, m) in st.items():
                check_saturation(scen, lb, size, s, m, r)
            label = {"100k": "100 KB", "10m": "10 MB"}[size]
            t.add(f"{label}: req/s", {lb: (st[lb][0] or {}).get("rps") for lb in lbs},
                  lambda v: fmt_num(v, 1), True, "slower")
            t.add(f"{label}: throughput (Gbit/s)",
                  {lb: (st[lb][0]["bytes"] * 8 / st[lb][0]["seconds"] / 1e9)
                   if st[lb][0] and st[lb][0].get("bytes") and st[lb][0].get("seconds") else None
                   for lb in lbs}, lambda v: fmt_num(v, 2), True, "slower")
            t.add(f"{label}: p99 latency (ms)",
                  {lb: (st[lb][0] or {}).get("pct", {}).get(99.0) for lb in lbs},
                  lambda v: fmt_num(v, 2), False, "slower")
            t.add(f"{label}: LB CPU (avg %)",
                  {lb: st[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}, fmt_num, False)
            t.add(f"{label}: LB peak RSS (MiB)",
                  {lb: st[lb][1].get("lb", {}).get("rss_peak_kb", 0) / 1024 or None for lb in lbs},
                  lambda v: fmt_num(v, 1), False)
            t.add(f"{label}: errors", {lb: (st[lb][0] or {}).get("errors") for lb in lbs},
                  fmt_num, False)
        w(t.render())
        w("")

    # 5. idle connections
    scen = "s5_idle_conns"
    if os.path.isdir(os.path.join(run, scen)):
        w("## 5. Many mostly-idle keep-alive connections\n")
        t = Table(lbs)
        st = {lb: step(run, scen, lb, "conns", parse_wrk) for lb in lbs}
        base = {lb: parse_samples(os.path.join(run, scen, f"{lb}-baseline.csv")) for lb in lbs}
        conns = {}
        for lb, (r, s, m) in st.items():
            check_saturation(scen, lb, "conns", s, m, r)
            c = re.search(r"-c (\d+)", m.get("cmd", ""))
            conns[lb] = int(c.group(1)) if c else None
        rate = re.search(r"-R (\d+)", st[lbs[0]][2].get("cmd", ""))
        idle = {lb: base[lb].get("lb", {}).get("rss_avg_kb") for lb in lbs}
        peak = {lb: st[lb][1].get("lb", {}).get("rss_peak_kb") for lb in lbs}
        t.add("Connections opened by the client", conns, fmt_num, None)
        est = {lb: int(st[lb][2]["established_mid_run"]) if st[lb][2].get("established_mid_run", "").isdigit()
               else None for lb in lbs}
        t.add("Established at the LB port at 70% of the step (ss)", est, fmt_num, None)
        for lb in lbs:
            if est[lb] is not None and conns[lb] and est[lb] < 0.98 * conns[lb]:
                warnings.append(f"{scen}/{lb}: only {est[lb]} of {conns[lb]} connections were "
                                f"established mid-run; memory per connection is understated.")
        t.add("LB RSS idle, before the step (MiB)",
              {lb: idle[lb] / 1024 if idle[lb] else None for lb in lbs}, lambda v: fmt_num(v, 1), False)
        t.add("LB peak RSS with the connections (MiB)",
              {lb: peak[lb] / 1024 if peak[lb] else None for lb in lbs}, lambda v: fmt_num(v, 1), False)
        t.add("Memory per connection (KiB)",
              {lb: (peak[lb] - idle[lb]) / conns[lb] if peak[lb] and idle[lb] and conns[lb] else None
               for lb in lbs}, lambda v: fmt_num(v, 1), False)
        t.add(f"LB CPU at {rate.group(1) if rate else '?'} req/s (avg %)",
              {lb: st[lb][1].get("lb", {}).get("cpu_avg") for lb in lbs}, fmt_num, False)
        t.add("p99 latency (ms)", {lb: (st[lb][0] or {}).get("pct", {}).get(99.0) for lb in lbs},
              lambda v: fmt_num(v, 3), False, "slower")
        t.add("Connect errors", {lb: (st[lb][0] or {}).get("err_connect") for lb in lbs}, fmt_num, False)
        t.add("Other errors (read/write/timeout + non-2xx)",
              {lb: ((st[lb][0] or {}).get("errors") or 0) - ((st[lb][0] or {}).get("err_connect") or 0)
               if st[lb][0] else None for lb in lbs}, fmt_num, False)
        w(t.render())
        w("\nRSS is the sum over all LB processes (HAProxy master + worker; RustStrom single "
          "process). Memory per connection = (peak RSS - idle RSS) / connections; it includes the "
          "backend-side connections the LB opens.\n")

    # 6. failover and reload
    if os.path.isdir(os.path.join(run, "s6_failover")) or os.path.isdir(os.path.join(run, "s6_reload")):
        w("## 6. Backend failure and configuration reload under load\n")
        t = Table(lbs)
        for scen, name, title in (("s6_failover", "failover", "Backend killed"),
                                  ("s6_reload", "reload", "Config reload")):
            if not os.path.isdir(os.path.join(run, scen)):
                continue
            st = {lb: step(run, scen, lb, name, parse_wrk) for lb in lbs}
            for lb, (r, s, m) in st.items():
                check_saturation(scen, lb, name, s, m, r)
            t.add(f"{title}: requests completed", {lb: (st[lb][0] or {}).get("requests") for lb in lbs},
                  fmt_num, None)
            t.add(f"{title}: errors (socket + non-2xx)", {lb: (st[lb][0] or {}).get("errors") for lb in lbs},
                  fmt_num, False)
            t.add(f"{title}: error breakdown", {lb: error_breakdown(st[lb][0]) for lb in lbs},
                  lambda v: v or "—", None)
            t.add(f"{title}: p99.9 latency (ms)",
                  {lb: (st[lb][0] or {}).get("pct", {}).get(99.9) for lb in lbs},
                  lambda v: fmt_num(v, 2), False, "slower")
            if name == "reload":
                t.add("Config reload: reloads applied / requested",
                      {lb: f"{st[lb][2].get('reloads_applied', '?')} / {st[lb][2].get('reloads_requested', '?')}"
                       for lb in lbs}, str, None)
        w(t.render())
        w("\nBackend failure: the last backend gets SIGKILL a third into the step and is started "
          "again at two thirds. Reload: HAProxy master-worker reload (SIGUSR2, listening sockets "
          "passed to the new worker); RustStrom config file replaced atomically (file watcher). "
          "Health checks every 1 s, rise 2, fall 3 on both (RustStrom v1.1 has no rise/fall).\n")

    w("## Warnings\n")
    if warnings:
        for x in warnings:
            w(f"- {x}")
    else:
        w("None: no load generator or backend saturation, no tool failures, load average below 1.0 "
          "before every step.")
    w("\n## Method\n")
    w("One host, cores split with taskset/cpuset: load generator, load balancer and backends never "
      "share a core. Each scenario starts a fresh LB process and warms it up before measuring. "
      "Raw outputs are next to this file: `<scenario>/<lb>-<step>.txt` (tool output), `.csv` (CPU "
      "and RSS samples every 0.5 s), `.meta` (command, load average, injected events); generated "
      "configs in `conf/`, LB logs in `logs/`. See scripts/bench/README.md for the limitations.")
    print("\n".join(out))


VALID = False

if __name__ == "__main__":
    main()
