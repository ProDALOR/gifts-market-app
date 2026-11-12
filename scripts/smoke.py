#!/usr/bin/env python3
"""Disposable token-authenticated core/gateway + fake-native Rust adapter integration; no accounts used.
Build the services independently; supply --hub-bin and --adapter-manifest.
"""
import argparse
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
parser=argparse.ArgumentParser(description="Black-box HTTP/2 integration between independently built services")
parser.add_argument("--hub-bin", required=True, type=Path)
parser.add_argument("--adapter-manifest", required=True, type=Path)
args=parser.parse_args()
hub_bin=args.hub_bin.resolve(strict=True)
adapter_manifest=args.adapter_manifest.resolve(strict=True)
adapters=adapter_manifest.parent
assert (app / "market-hub.proto").read_bytes()==(adapters / "market-hub.proto").read_bytes(), "contract copies differ"

with tempfile.TemporaryDirectory(prefix="market-split-smoke-") as temporary:
    directory = Path(temporary)
    # This test owns its signing secret and never reads local deployment files.
    clean = {k: v for k, v in os.environ.items() if not k.startswith("MARKET_")}
    token = subprocess.run([str(hub_bin), "key"], cwd=directory, env=clean, check=True, capture_output=True, text=True).stdout.strip()
    assert len(token) == 64 and all(c in "0123456789abcdef" for c in token)
    # Offline issuance must work before the hub starts and without reading a hub config.
    issue_env = dict(clean, MARKET_ACCESS_TOKEN=token)
    issued = {}
    for node_id in ["gui", "market-app", "telegram", "reader"]:
        result = subprocess.run([str(hub_bin), "token", "--id", node_id], cwd=directory, env=issue_env, check=True, capture_output=True, text=True)
        issued[node_id] = result.stdout.strip()
        assert issued[node_id].startswith(node_id + '.') and len(result.stdout.splitlines()) == 1
        assert token not in result.stdout, "signing secret appeared in token output"
    environment = dict(clean, MARKET_NODE_TOKEN=issued["market-app"])
    sockets = [socket.socket() for _ in range(3)]
    for handle in sockets:
        handle.bind(("127.0.0.1", 0))
    core_grpc, metrics, http = [handle.getsockname()[1] for handle in sockets]
    for handle in sockets:
        handle.close()
    processes = []
    logs = []

    def start(name, command, cwd, hub=False):
        log = (directory / (name + ".log")).open("w+")
        logs.append((name, log))
        process = subprocess.Popen(command, cwd=cwd, env=dict(environment, MARKET_ACCESS_TOKEN=token) if hub else environment, stdout=log, stderr=log)
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
        core_command = [str(hub_bin), "--data-dir", str(directory / "catalog"), "--grpc-addr", f"127.0.0.1:{core_grpc}", "--http-addr", f"127.0.0.1:{metrics}", "--feed-lease", "5s"]
        api_command = [str(app / "target/debug/market-app"), "--bind", f"127.0.0.1:{http}", "--core", f"http://127.0.0.1:{core_grpc}"]
        core = start("core", core_command, directory, hub=True)
        api = start("gateway", api_command, app)
        generation = wait_ready()
        environment.update(MARKET_RESALE_ENDPOINT=f"http://127.0.0.1:{core_grpc}", MARKET_RESALE_API=f"http://127.0.0.1:{http}", MARKET_RESALE_TOKEN=issued["telegram"], MARKET_RESALE_DATA_DIR=str(directory / "adapter-state"))
        adapter_command = ["cargo", "test", "--manifest-path", str(adapter_manifest), "--locked", "-p", "market-telegram", "--test", "deployment", "--", "--ignored", "--nocapture"]
        subprocess.run(adapter_command, cwd=adapters, env=environment, check=True, timeout=180)
        cursor_files = list((directory / "adapter-state/trade-cursors").glob("*.cursor"))
        assert len(cursor_files) == 1 and cursor_files[0].read_text() == "page-1", "first acknowledged trade cursor was not persisted"

        subprocess.run([str(app / "target/debug/examples/read_core"), "--endpoint", f"http://127.0.0.1:{core_grpc}", "--item-type", "telegram.gift.123"], cwd=app, env=dict(clean, MARKET_NODE_TOKEN=issued["reader"]), check=True, timeout=30)
        def trades():
            with urllib.request.urlopen(f"http://127.0.0.1:{http}/trades?feed_id=telegram", timeout=2) as response:
                return json.load(response)["data"]
        before = trades()
        assert len(before) == 1 and before[0]["trade_id"] == "test-execution-1"
        original_token = environment["MARKET_NODE_TOKEN"]
        # A hard kill leaves no opportunity for a shutdown flush. The acknowledged
        # execution must survive in core's WAL, independently of the volatile book.
        core.kill()
        core.wait(timeout=5)
        stopped.append(core)
        core = start("restarted-core", core_command, directory, hub=True)
        wait_ready(generation)
        assert trades() == before, "acknowledged trade disappeared after a core crash"
        # The restarted adapter backfills the same fact. Core deduplicates its identity
        # even though the source observation clock and core generation changed.
        subprocess.run(adapter_command, cwd=adapters, env=environment, check=True, timeout=180)
        assert list((directory / "adapter-state/trade-cursors").glob("*.cursor")) == cursor_files, "cursor scope changed across a normal core restart"
        assert cursor_files[0].read_text() == "page-2", "adapter did not resume and persist the overlapping next page"

        assert trades() == before, "backfilled trade was duplicated or replaced"
        stop_service(api)
        api = start("restarted-gateway", api_command, app)
        wait_ready()
        assert trades() == before
        assert environment["MARKET_NODE_TOKEN"] == original_token, "service token changed on restart"
        assert not list(directory.glob("*.json")), "credential JSON files must not be created"
        assert not (directory / "catalog/nodes.json").exists(), "credential registry must not be created"
        print("Core ownership verified: one token-authenticated HTTP/2 endpoint, indexed books, durable trade crash recovery and deduplication, persistent trade cursor resume, stateless gateway, offline signed credentials, no enrollment or credential registry")
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
