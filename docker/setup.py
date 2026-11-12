#!/usr/bin/env python3
"""Configure API gateway using a hub-issued join bundle and an API-owned TLS identity."""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import tempfile
from urllib.parse import urlsplit


def env_values(path):
    result = {}
    if path.exists():
        for line in path.read_text().splitlines():
            key, separator, value = line.removeprefix('export ').partition('=')
            if separator and re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*', key.strip()):
                value = value.strip()
                if value.startswith("'") and value.endswith("'"):
                    value = value[1:-1].replace("\\'", "'")
                elif value.startswith('"'):
                    value = json.loads(value)
                result[key.strip()] = value
    return result


def atomic(path, data, mode=0o600):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(dir=path.parent, prefix='.setup-')
    try:
        with os.fdopen(handle, 'wb') as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def credential(value):
    return isinstance(value, str) and 16 <= len(value) <= 512 and all(33 <= ord(c) <= 126 for c in value)


def core_url(value):
    if not isinstance(value, str) or not all(33 <= ord(c) <= 126 for c in value):
        return False
    try:
        uri = urlsplit(value)
        return (uri.scheme == 'https' and bool(uri.hostname) and uri.port != 0
                and uri.username is None and uri.password is None
                and uri.path in ('', '/') and not uri.query and not uri.fragment)
    except ValueError:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('join', nargs='?', type=Path, help='Hub join bundle directory; required on first setup')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    env = Path(os.environ.get('MARKET_ENV_FILE', root / '.env'))
    values = env_values(env)
    certs = Path(os.environ.get('MARKET_CERT_DIR', values.get('MARKET_CERT_DIR', root / '.docker/certs')))
    trust = Path(os.environ.get('MARKET_HUB_CA', values.get('MARKET_HUB_CA', root / '.docker/hub-ca.crt')))
    if not certs.is_absolute():
        certs = root / certs
    if not trust.is_absolute():
        trust = root / trust
    updates = {}
    if args.join:
        bundle = json.loads((args.join / 'join.json').read_text())
        token, network, endpoint = bundle.get('join_token'), bundle.get('network'), bundle.get('core_url')
        if not credential(token) or not core_url(endpoint) or not isinstance(network, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_.-]{0,62}', network):
            parser.error('Invalid hub join bundle')
        subprocess.run(['openssl', 'x509', '-noout', '-in', str(args.join / 'ca.crt')], check=True, stdout=subprocess.DEVNULL)
        trust.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        atomic(trust, (args.join / 'ca.crt').read_bytes(), 0o644)
        updates.update(MARKET_JOIN_TOKEN=token, MARKET_NETWORK=network, MARKET_CORE_URL=endpoint)
    elif not trust.is_file():
        parser.error('First setup requires the hub join bundle: sh docker/setup.sh /path/to/join')
    effective_join = updates.get('MARKET_JOIN_TOKEN', values.get('MARKET_JOIN_TOKEN'))
    if effective_join not in (None, '') and not credential(effective_join):
        parser.error('Existing MARKET_JOIN_TOKEN is invalid; it was preserved')
    if not credential(values.get('MARKET_API_TOKEN')):
        if 'MARKET_API_TOKEN' in values:
            parser.error('Existing MARKET_API_TOKEN is invalid; it was preserved')
        updates['MARKET_API_TOKEN'] = secrets.token_hex(32)
    # Preserve unrelated settings and existing identities; quote values as Compose literals.
    lines = env.read_text().splitlines() if env.exists() else []
    lines = [line for line in lines if line.removeprefix('export ').partition('=')[0].strip() not in updates]
    lines.extend(f"{key}='" + value.replace("'", "\\'") + "'" for key, value in updates.items())
    atomic(env, ('\n'.join(lines) + '\n').encode())
    certs.mkdir(parents=True, exist_ok=True, mode=0o700)
    certs.chmod(0o700)

    def openssl(*arguments):
        subprocess.run(['openssl', *map(str, arguments)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    if (certs / 'ca.key').exists() != (certs / 'ca.crt').exists():
        parser.error('API CA is incomplete; existing files were preserved')
    if not (certs / 'ca.key').exists():
        ca_config = certs / 'ca.cnf'
        atomic(ca_config, b'[req]\nprompt=no\ndistinguished_name=name\nx509_extensions=ca\n[name]\nCN=Market API local CA\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n')
        openssl('genpkey', '-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256', '-out', certs / 'ca.key')
        openssl('req', '-new', '-x509', '-sha256', '-days', '3650', '-key', certs / 'ca.key', '-config', ca_config, '-out', certs / 'ca.crt')
    reuse = (certs / 'api.key').is_file() and (certs / 'api.crt').is_file()
    if reuse:
        try:
            openssl('x509', '-checkend', '86400', '-noout', '-in', certs / 'api.crt')
            openssl('x509', '-checkhost', 'market-app', '-noout', '-in', certs / 'api.crt')
            openssl('verify', '-CAfile', certs / 'ca.crt', certs / 'api.crt')
        except subprocess.CalledProcessError:
            reuse = False
    if not reuse:
        configuration = certs / 'api.cnf'
        atomic(configuration, b'[req]\nprompt=no\ndistinguished_name=name\nreq_extensions=server\n[name]\nCN=market-app\n[server]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:market-app,DNS:localhost,IP:127.0.0.1,IP:::1\n')
        openssl('genpkey', '-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:P-256', '-out', certs / 'api.key')
        openssl('req', '-new', '-sha256', '-key', certs / 'api.key', '-config', configuration, '-out', certs / 'api.csr')
        openssl('x509', '-req', '-sha256', '-days', '365', '-in', certs / 'api.csr', '-CA', certs / 'ca.crt', '-CAkey', certs / 'ca.key', '-CAcreateserial', '-extfile', configuration, '-extensions', 'server', '-out', certs / 'api.crt')
    # The host directory stays private; file-backed container secrets must be readable by UID 10001.
    for name in ['ca.crt', 'api.crt', 'api.key']:
        (certs / name).chmod(0o644)
    (certs / 'ca.key').chmod(0o600)
    print('Prepared gateway hub trust and independent API TLS credentials.')


if __name__ == '__main__':
    main()
