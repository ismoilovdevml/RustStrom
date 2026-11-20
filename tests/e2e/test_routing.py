"""Request routing: matcher language and HTTP/2 :authority over TLS."""

import pytest

from conftest import make_module_stack
from harness import client
from harness import config as cfg

# (matcher, backend name, schemes); pools are matched in this order
POOLS = [
    ("Host('and.test') && Path('/x')", "and", ("HTTP", "HTTPS")),
    ("Host('or1.test') || Host('or2.test')", "or", ("HTTP", "HTTPS")),
    ("Host('grp.test') && (Path('/a') || Path('/b'))", "grp", ("HTTP", "HTTPS")),
    ("Host('host.test')", "host", ("HTTP", "HTTPS")),
    ("HostRegexp('^[a-z0-9]+\\.wild\\.test$')", "hostre", ("HTTP", "HTTPS")),
    ("Path('/exact')", "path", ("HTTP", "HTTPS")),
    ("PathRegexp('^/api/')", "pathre", ("HTTP", "HTTPS")),
    ("Method('DELETE')", "method", ("HTTP", "HTTPS")),
    ("Query('k', 'v')", "query", ("HTTP", "HTTPS")),
    ("Host('h2.test')", "h2", ("HTTPS",)),
]


@pytest.fixture(scope="module")
def routing(request, binary, pki, tmp_path_factory):
    stack = make_module_stack(request, binary, pki, tmp_path_factory, "routing")
    backends = {name: stack.backend(name) for _, name, _ in POOLS}
    pools = [cfg.pool(matcher, [backends[name].address], schemes=schemes)
             for matcher, name, schemes in POOLS]
    lb = stack.lb(pools)
    return lb


@pytest.fixture
def lb(routing, request):
    request.node._e2e_stacks = [type("S", (), {"lbs": [routing]})()]
    yield routing
    assert routing.alive(), "rust-strom exited during the test"


CASES = [
    # id, method, host, path, expected backend (None = 404)
    ("host", "GET", "host.test", "/", "host"),
    ("host-other-path", "GET", "host.test", "/anything/else", "host"),
    ("host-unknown", "GET", "nomatch.test", "/", None),
    ("hostregexp", "GET", "foo.wild.test", "/", "hostre"),
    ("hostregexp-no-subdomain", "GET", "wild.test", "/", None),
    ("hostregexp-anchored", "GET", "foo.wild.test.evil", "/", None),
    ("path-exact", "GET", "nomatch.test", "/exact", "path"),
    ("path-is-not-prefix", "GET", "nomatch.test", "/exact/more", None),
    ("pathregexp", "GET", "nomatch.test", "/api/users/1", "pathre"),
    ("pathregexp-miss", "GET", "nomatch.test", "/apix", None),
    ("method", "DELETE", "nomatch.test", "/zzz", "method"),
    ("method-miss", "PUT", "nomatch.test", "/zzz", None),
    ("query", "GET", "nomatch.test", "/q?a=1&k=v", "query"),
    ("query-wrong-value", "GET", "nomatch.test", "/q?k=w", None),
    ("and", "GET", "and.test", "/x", "and"),
    ("and-partial", "GET", "and.test", "/y", None),
    ("or-left", "GET", "or1.test", "/", "or"),
    ("or-right", "GET", "or2.test", "/", "or"),
    ("group-a", "GET", "grp.test", "/a", "grp"),
    ("group-b", "GET", "grp.test", "/b", "grp"),
    ("group-miss", "GET", "grp.test", "/c", None),
    ("https-only-pool-over-http", "GET", "h2.test", "/", None),
]


@pytest.mark.parametrize("method,host,path,expected", [c[1:] for c in CASES],
                         ids=[c[0] for c in CASES])
def test_matcher(lb, method, host, path, expected):
    r = client.request(lb.http_port, path, method=method, host=host)
    if expected is None:
        assert r.status == 404, r
    else:
        assert r.status == 200, r
        assert r.backend == expected
        assert r.json()["path"] == path, "path and query must be forwarded unchanged"


def test_routing_over_https_http1(lb, pki):
    r = client.request(lb.https_port, "/", host="h2.test", tls=pki.ca_cert)
    assert r.status == 200 and r.backend == "h2", r
    r = client.request(lb.https_port, "/api/v1", host="nomatch.test", sni="a.test",
                       tls=pki.ca_cert)
    assert r.status == 200 and r.backend == "pathre", r


def test_http2_authority_routing_over_tls(lb, pki):
    """HTTP/2 has no Host header: routing must use :authority."""
    r = client.h2_request(lb.https_port, "h2.test", "/h2path", ca=pki.ca_cert)
    assert r.status == 200, r
    assert r.backend == "h2"
    got = r.json()
    assert got["path"] == "/h2path"
    assert dict(got["headers"]).get("host", "").split(":")[0] == "h2.test", \
        "backend must receive a Host header derived from :authority"


def test_http2_authority_no_match_is_404(lb, pki):
    r = client.h2_request(lb.https_port, "nomatch.test", "/", ca=pki.ca_cert, sni="a.test")
    assert r.status == 404, r
