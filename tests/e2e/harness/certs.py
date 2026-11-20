"""Self-signed test PKI generated with the openssl CLI: one CA and one leaf
certificate whose SAN covers every test host name."""

import os
import subprocess
from dataclasses import dataclass

TEST_HOSTS = [
    "localhost",
    "a.test",
    "b.test",
    "host.test",
    "h2.test",
    "ws.test",
    "backend.test",
]


@dataclass
class TestPki:
    ca_cert: str
    cert: str
    key: str
    hosts: list


def _run(args, cwd):
    subprocess.run(args, cwd=cwd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def generate(directory, hosts=None):
    hosts = list(hosts or TEST_HOSTS)
    os.makedirs(directory, exist_ok=True)
    ca_key = os.path.join(directory, "ca.key")
    ca_cert = os.path.join(directory, "ca.crt")
    key = os.path.join(directory, "server.key")
    csr = os.path.join(directory, "server.csr")
    cert = os.path.join(directory, "server.crt")
    ext = os.path.join(directory, "server.ext")

    _run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
          "-out", ca_key], directory)
    _run(["openssl", "req", "-x509", "-new", "-key", ca_key, "-sha256", "-days", "3",
          "-subj", "/CN=RustStrom e2e CA", "-out", ca_cert], directory)
    # PKCS#8 key: supported by v1 (rustls-pemfile) and by Pingora (BoringSSL/OpenSSL)
    _run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
          "-out", key], directory)
    _run(["openssl", "req", "-new", "-key", key, "-subj", "/CN=%s" % hosts[0], "-out", csr],
         directory)
    san = ",".join(["DNS:%s" % h for h in hosts] + ["IP:127.0.0.1"])
    with open(ext, "w") as f:
        f.write("basicConstraints=CA:FALSE\n"
                "keyUsage=digitalSignature,keyEncipherment\n"
                "extendedKeyUsage=serverAuth\n"
                "subjectAltName=%s\n" % san)
    _run(["openssl", "x509", "-req", "-in", csr, "-CA", ca_cert, "-CAkey", ca_key,
          "-CAcreateserial", "-days", "3", "-sha256", "-extfile", ext, "-out", cert], directory)
    return TestPki(ca_cert=ca_cert, cert=cert, key=key, hosts=hosts)
