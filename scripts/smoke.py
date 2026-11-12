#!/usr/bin/env python3
"""Disposable TLS core/gateway + fake-native Rust adapter integration; no accounts used.
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
    (join / "join.json").write_text(json.dumps({"core_url": "https://market-hub:50051", "join_token": token, "network": "market-smoke"}))
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
    sockets = [socket.socket() for _ in range(4)]
    for handle in sockets:
        handle.bind(("127.0.0.1", 0))
    core_grpc, metrics, http, grpc = [handle.getsockname()[1] for handle in sockets]
    config=f'''data_dir = "{directory}/catalog"
grpc_addr = "127.0.0.1:{core_grpc}"
http_addr = "127.0.0.1:{metrics}"
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

    def status():
        with urllib.request.urlopen(f"http://127.0.0.1:{http}/status", timeout=1) as response:
            return json.load(response)

    def wait_ready(previous_generation=None):
        deadline = time.monotonic() + 20
        while True:
            assert all(process.poll() is None for process in processes if process not in stopped), "a service exited"
            try:
                value = status()
                if value["data"]["ready"] and value["meta"]["cursor"]["generation"] != previous_generation:
                    return value["meta"]["cursor"]["generation"]
            except OSError:
                pass
            assert time.monotonic() < deadline, "API synchronization timed out"
            time.sleep(.05)

    stopped = []
    def stop_service(process):
        stop(process)
        stopped.append(process)

    try:
        core_command = [str(hub_bin), "--config", str(directory / "core.toml")]
        api_command = [str(app / "target/debug/market-app"), "--bind", f"127.0.0.1:{http}", "--core", f"https://127.0.0.1:{core_grpc}", "--core-ca", str(directory / "hub-ca.crt"), "--credentials", str(directory / "api.json"), "--grpc-bind", f"127.0.0.1:{grpc}", "--grpc-cert", str(directory / "certs/api.crt"), "--grpc-key", str(directory / "certs/api.key")]
        core = start("core", core_command, directory)
        api = start("gateway", api_command, app)
        generation = wait_ready()
        environment.update(MARKET_RESALE_ENDPOINT=f"https://127.0.0.1:{core_grpc}", MARKET_RESALE_API=f"http://127.0.0.1:{http}", MARKET_RESALE_CA=str(directory / "hub-ca.crt"), MARKET_RESALE_CREDENTIALS=str(directory / "adapter.json"))
        adapter_command = ["cargo", "test", "--manifest-path", str(adapter_manifest), "--locked", "-p", "market-telegram", "--test", "deployment", "--", "--ignored", "--nocapture"]
        subprocess.run(adapter_command, cwd=adapters, env=environment, check=True, timeout=180)
        subprocess.run([str(app / "target/debug/examples/read_api"), "--endpoint", f"https://127.0.0.1:{grpc}", "--ca", str(directory / "certs/ca.crt"), "--item-type", "telegram.gift.123"], cwd=app, env=environment, check=True, timeout=30)
        def trades():
            with urllib.request.urlopen(f"http://127.0.0.1:{http}/trades?feed_id=telegram", timeout=2) as response:
                return json.load(response)["data"]
        before = trades()
        assert len(before) == 1 and before[0]["trade_id"] == "test-execution-1"
        identities = {name: (directory / name).read_bytes() for name in ("api.json", "adapter.json")}
        # A hard kill leaves no opportunity for a shutdown flush. The acknowledged
        # execution must survive in core's WAL, independently of the volatile book.
        core.kill()
        core.wait(timeout=5)
        stopped.append(core)
        environment["MARKET_JOIN_TOKEN"] = secrets.token_hex(32)
        core = start("restarted-core", core_command, directory)
        environment.pop("MARKET_JOIN_TOKEN")
        wait_ready(generation)
        assert trades() == before, "acknowledged trade disappeared after a core crash"
        # The restarted adapter backfills the same fact. Core deduplicates its identity
        # even though the source observation clock and core generation changed.
        subprocess.run(adapter_command, cwd=adapters, env=environment, check=True, timeout=180)
        assert trades() == before, "backfilled trade was duplicated or replaced"
        stop_service(api)
        environment["MARKET_JOIN_TOKEN"] = ""  # Compose omits enrollment authority this way.
        api = start("restarted-gateway", api_command, app)
        wait_ready()
        assert trades() == before
        assert all((directory / name).read_bytes() == value for name, value in identities.items()), "node credentials changed on restart"
        print("Core ownership verified: one TLS endpoint, indexed books, durable trade crash recovery and deduplication, stateless gateway, token-free reconnects")
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
