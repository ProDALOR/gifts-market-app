#!/usr/bin/env python3
"""Disposable TLS core/API/history + fake-native Rust adapter integration; no accounts used.
Build the services independently; supply --hub-bin and --adapter-manifest.
"""
import argparse
import importlib.util
import secrets
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

app = Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description="Black-box TLS integration between independently built services")
parser.add_argument("--hub-bin", required=True, type=Path)
parser.add_argument("--adapter-manifest", required=True, type=Path)
args=parser.parse_args()
hub_bin=args.hub_bin.resolve(strict=True)
adapter_manifest=args.adapter_manifest.resolve(strict=True)
adapters=adapter_manifest.parent
assert (app / "market-hub.proto").read_bytes()==(adapters / "market-hub.proto").read_bytes(), "contract copies differ"

with tempfile.TemporaryDirectory(prefix="market-split-smoke-") as temporary:
    directory = Path(temporary)
    # This test owns its hub issuer. Production client setup only imports the join bundle.
    core_certs = directory / "hub-certs"
    core_certs.mkdir()
    join = directory / "join"
    join.mkdir()
    token = secrets.token_hex(32)
    (join / "join.json").write_text(json.dumps({"registration_token": token, "network": "market-smoke"}))
    (core_certs / "ca.cnf").write_text("[req]\nprompt=no\ndistinguished_name=name\nx509_extensions=ca\n[name]\nCN=Disposable hub CA\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n")
    (core_certs / "core.cnf").write_text("[req]\nprompt=no\ndistinguished_name=name\nreq_extensions=server\n[name]\nCN=market-hub\n[server]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:market-hub,DNS:localhost,IP:127.0.0.1,IP:::1\n")
    def openssl(*arguments):
        subprocess.run(["openssl", *map(str, arguments)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", core_certs / "ca.key")
    openssl("req", "-new", "-x509", "-sha256", "-days", "1", "-key", core_certs / "ca.key", "-config", core_certs / "ca.cnf", "-out", core_certs / "ca.crt")
    openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", core_certs / "core.key")
    openssl("req", "-new", "-key", core_certs / "core.key", "-config", core_certs / "core.cnf", "-out", core_certs / "core.csr")
    openssl("x509", "-req", "-days", "1", "-in", core_certs / "core.csr", "-CA", core_certs / "ca.crt", "-CAkey", core_certs / "ca.key", "-CAcreateserial", "-extfile", core_certs / "core.cnf", "-extensions", "server", "-out", core_certs / "core.crt")
    (join / "ca.crt").write_bytes((core_certs / "ca.crt").read_bytes())
    environment = dict(os.environ, MARKET_ENV_FILE=str(directory / "env"), MARKET_CERT_DIR=str(directory / "certs"), MARKET_HUB_CA=str(directory / "hub-ca.crt"))
    subprocess.run(["sh", "docker/setup.sh", str(join)], cwd=app, env=environment, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    spec = importlib.util.spec_from_file_location("deployment_setup", app / "docker/setup.py")
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    environment.update(setup.env_values(directory / "env"))
    sockets = [socket.socket() for _ in range(5)]
    for handle in sockets:
        handle.bind(("127.0.0.1", 0))
    ingest, read, metrics, http, grpc = [handle.getsockname()[1] for handle in sockets]
    config=f'''catalog_dir = "{directory}/catalog"
ingest_addr = "127.0.0.1:{ingest}"
read_addr = "127.0.0.1:{read}"
http_addr = "127.0.0.1:{metrics}"
registration_token_env = "MARKET_REGISTRATION_TOKEN"
[tls]
cert = "{core_certs}/core.crt"
key = "{core_certs}/core.key"
[limits]
feed_lease = "5s"
'''
    (directory / "core.toml").write_text(config)
    for handle in sockets:
        handle.close()
    processes = []
    logs = []

    def start(name, command, cwd):
        log = (directory / (name + ".log")).open("w+")
        logs.append((name, log))
        process = subprocess.Popen(command, cwd=cwd, env=environment, stdout=log, stderr=log)
        processes.append(process)
        return process

    def stop(process):
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    try:
        start("core", [str(hub_bin), "--config", str(directory / "core.toml")], directory)
        start("api", [str(app / "target/debug/market-app"), "--bind", f"127.0.0.1:{http}", "--core", f"https://127.0.0.1:{read}", "--core-ca", str(directory / "hub-ca.crt"), "--credentials", str(directory / "api.json"), "--grpc-bind", f"127.0.0.1:{grpc}", "--grpc-cert", str(directory / "certs/api.crt"), "--grpc-key", str(directory / "certs/api.key")], app)
        history = start("history", [str(app / "target/debug/history-node"), "--endpoint", f"https://127.0.0.1:{read}", "--core-ca", str(directory / "hub-ca.crt"), "--credentials", str(directory / "history.json"), "--output", str(directory / "archive")], app)
        deadline = time.monotonic() + 20
        while True:
            assert all(process.poll() is None for process in processes), "a service exited"
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{http}/status", timeout=1) as response:
                    if json.load(response)["data"]["ready"]:
                        break
            except OSError:
                pass
            assert time.monotonic() < deadline, "API bootstrap timed out"
            time.sleep(.05)
        # The initial empty catalog and explicit archive boundary are two complete frames.
        deadline = time.monotonic() + 10
        active = directory / "archive/segments/00000000000000000001.binpb"
        while True:
            count = 0
            if active.exists():
                data = active.read_bytes()
                position = 8
                while position + 40 <= len(data):
                    length = int.from_bytes(data[position:position+4], "little")
                    if position + 40 + length > len(data):
                        break
                    count += 1
                    position += 40 + length
            if count >= 2:
                break
            assert time.monotonic() < deadline, "history baseline timed out"
            time.sleep(.05)
        environment.update(MARKET_RESALE_ENDPOINT=f"https://127.0.0.1:{ingest}", MARKET_RESALE_API=f"http://127.0.0.1:{http}", MARKET_RESALE_CA=str(directory / "hub-ca.crt"))
        subprocess.run(["cargo", "test", "--manifest-path", str(adapter_manifest), "--locked", "-p", "market-telegram", "--test", "deployment", "--", "--ignored", "--nocapture"], cwd=adapters, env=environment, check=True, timeout=180)
        subprocess.run([str(app / "target/debug/examples/read_api"), "--endpoint", f"https://127.0.0.1:{grpc}", "--ca", str(directory / "certs/ca.crt"), "--item-type", "telegram.gift.123"], cwd=app, env=environment, check=True, timeout=30)
        # The history consumer should have followed all revisions before shutdown.
        time.sleep(1)
        stop(history)
        dump = subprocess.check_output([str(app / "target/debug/history-node"), "--output", str(directory / "archive"), "--dump"], cwd=app, env=environment, text=True)
        rows = [json.loads(line) for line in dump.splitlines()]
        events = [event for row in rows if row["type"] == "batch" for event in row["events"]]
        assert any(event["type"] == "order_removed" for event in events), "history missed quote removal"
        assert any(event["type"] == "upsert" for event in events), "history missed order observations"
        print(f"Split deployment verified: real TLS services, separate credentials, {len(events)} archived facts, recovered JSONL export")
    except Exception:
        for name, log in logs:
            log.flush()
            log.seek(0)
            print(name + ":\n" + log.read()[-5000:])
        raise
    finally:
        for process in reversed(processes):
            stop(process)
        for _, log in logs:
            log.close()
