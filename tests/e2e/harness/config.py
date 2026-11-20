"""Builds RustStrom TOML configurations (v1.1 format, valid for v2 as well).

Only the v1.1 keys are emitted unless a test passes v2-only keys explicitly
(`upstream=` per pool or `extra_global=`). v1.1 ignores unknown keys, so a
config with v2 keys still loads on v1.1.
"""

import json


def q(value):
    """TOML basic string (JSON string escaping is a subset TOML accepts)."""
    return json.dumps(value)


def _value(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, str):
        return q(value)
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_value(v) for v in value) + "]"
    raise TypeError("unsupported TOML value %r" % (value,))


def _table(header, values):
    lines = ["[%s]" % header]
    for key, value in values.items():
        lines.append("%s = %s" % (key, _value(value)))
    return "\n".join(lines)


ROUND_ROBIN = "{ RoundRobin = {} }"
RANDOM = "{ Random = {} }"
IP_HASH = "{ IPHash = {} }"
LEAST_CONNECTION = "{ LeastConnection = {} }"


def sticky_cookie(cookie_name="RSID", http_only=True, secure=False, same_site="Lax",
                  inner=ROUND_ROBIN):
    return ("{ StickyCookie = { cookie_name = %s, http_only = %s, secure = %s, "
            "same_site = %s, inner = %s } }" % (q(cookie_name), _value(http_only),
                                                _value(secure), q(same_site), inner))


def pool(matcher, addresses, schemes=("HTTP", "HTTPS"), strategy=ROUND_ROBIN,
         health=None, middlewares=None, client=None, upstream=None):
    """One [[backend_pools]] entry.

    health: dict for [backend_pools.health_config] (defaults: fast checks on /__health)
    middlewares: {"RateLimiter": {"limit": 5, "window_sec": 60}, "Compression": {}}
    upstream: v2-only [backend_pools.upstream] keys
    """
    health_values = {"slow_threshold": 2000, "timeout": 1000, "path": "/__health"}
    health_values.update(health or {})
    parts = [
        "[[backend_pools]]",
        "matcher = %s" % q(matcher),
        "addresses = %s" % _value(list(addresses)),
        "schemes = %s" % _value(list(schemes)),
        "strategy = %s" % strategy,
        _table("backend_pools.health_config", health_values),
    ]
    if client:
        parts.append(_table("backend_pools.client", client))
    if upstream:
        parts.append(_table("backend_pools.upstream", upstream))
    for name, values in (middlewares or {}).items():
        parts.append(_table("backend_pools.middlewares.%s" % name, values or {}))
    return "\n".join(parts)


def config(http_port, https_port, metrics_port, pools, certificates=None, check_every=1,
           extra_global=None):
    """A full configuration. All listen addresses are explicit so the LB never
    binds a port outside the allowed test range.

    certificates: {sni_name: (cert_path, key_path)}
    extra_global: v2-only top-level keys, e.g. {"grace_period_secs": 10}
    """
    head = [
        "http_address = %s" % q("127.0.0.1:%d" % http_port),
        "https_address = %s" % q("127.0.0.1:%d" % https_port),
        "prometheus_metrics_address = %s" % q("127.0.0.1:%d" % metrics_port),
    ]
    for key, value in (extra_global or {}).items():
        head.append("%s = %s" % (key, _value(value)))
    parts = ["\n".join(head), "[health_interval]\ncheck_every = %d" % check_every]
    parts.extend(pools)
    for name, (cert, key) in (certificates or {}).items():
        parts.append(_table('certificates."%s"' % name,
                            {"type": "Local", "certificate_path": cert,
                             "private_key_path": key}))
    return "\n\n".join(parts) + "\n"
