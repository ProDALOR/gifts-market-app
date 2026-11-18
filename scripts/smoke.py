#!/usr/bin/env python3
"""Disposable current-hub integration; never opens account exports or contacts markets."""

import argparse, json, os, signal, socket, subprocess, tempfile, time, urllib.request, urllib.error
from decimal import Decimal
from pathlib import Path


def clean_environment(source):
    """Never let a disposable run inherit service credentials or storage settings."""
    environment = {
        key: value
        for key, value in source.items()
        if not key.startswith(("MARKET_", "TELEGRAM_", "VITE_MARKET_"))
    }
    environment["NO_PROXY"] = environment["no_proxy"] = "127.0.0.1,localhost"
    return environment


def fixture_executable(build_output):
    for line in build_output.splitlines():
        try:
            artifact = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            artifact.get("reason") == "compiler-artifact"
            and artifact.get("target", {}).get("name") == "smoke_fixture"
            and artifact.get("executable")
        ):
            return Path(artifact["executable"])
    raise RuntimeError("Cargo did not report the smoke fixture executable")


def stop_group(process, sig):
    try:
        os.killpg(process.pid, sig)
    except ProcessLookupError:
        pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--hub-bin", type=Path, required=True)
    parser.add_argument("--app-bin", type=Path)
    parser.add_argument(
        "--keep",
        type=Path,
        help="Keep local preview processes and logs at this ignored path",
    )
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    hub = args.hub_bin.resolve()
    app = (args.app_bin or root / "target/debug/market-app").resolve()
    assert hub.is_file() and app.is_file(), (
        "Build the hub and public app binaries first"
    )
    workspace = (
        Path(tempfile.mkdtemp(prefix="market-public-"))
        if not args.keep
        else args.keep.resolve()
    )
    workspace.mkdir(parents=True, exist_ok=True)

    def port():
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            return s.getsockname()[1]

    grpc, admin, http = port(), port(), port()
    hub_url = f"http://127.0.0.1:{grpc}"
    base = f"http://127.0.0.1:{http}"
    base_env = clean_environment(os.environ)
    key = subprocess.check_output([str(hub), "key"], env=base_env, text=True).strip()
    env = dict(base_env, MARKET_SIGNING_KEY=key)
    token = subprocess.check_output(
        [str(hub), "token", "--id", "public-app-test"], env=env, text=True
    ).strip()
    processes = []

    def start(name, command, environment, cwd):
        with (workspace / (name + ".log")).open("ab") as stream:
            process = subprocess.Popen(
                command,
                env=environment,
                cwd=cwd,
                stdout=stream,
                stderr=stream,
                start_new_session=True,
            )
        processes.append(process)
        return process

    def wait(url):
        for _ in range(150):
            try:
                with urllib.request.urlopen(url, timeout=1):
                    return
            except Exception:
                time.sleep(0.1)
        raise RuntimeError(
            "Local service did not become ready; inspect " + str(workspace)
        )

    def call(path, body=None, expected=200):
        data = None if body is None else json.dumps(body).encode()
        r = urllib.request.Request(
            base + path,
            data=data,
            headers={"Content-Type": "application/json"} if data else {},
        )
        try:
            with urllib.request.urlopen(r, timeout=12) as response:
                status = response.status
                raw = response.read()
        except urllib.error.HTTPError as error:
            status = error.code
            raw = error.read()
        assert status == expected, (path, status, raw[:300])
        assert token.encode() not in raw, (
            "A service credential reached a public response"
        )
        return json.loads(raw) if raw else None

    hub_command = [
        str(hub),
        "--data-dir",
        str(workspace / "hub"),
        "--grpc-addr",
        f"127.0.0.1:{grpc}",
        "--http-addr",
        f"127.0.0.1:{admin}",
    ]
    try:
        build = subprocess.run(
            [
                "cargo",
                "build",
                "--locked",
                "-p",
                "market-app",
                "--example",
                "smoke_fixture",
                "--message-format=json",
            ],
            cwd=root,
            env=base_env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        (workspace / "fixture-build.log").write_text(build.stdout)
        build.check_returncode()
        fixture_bin = fixture_executable(build.stdout)
        with (workspace / "frontend-build.log").open("w") as log:
            subprocess.run(
                ["npm", "--prefix", "web", "run", "build"],
                cwd=root,
                env=dict(base_env, MARKET_SCHEMA_BIN=str(app)),
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )

        def fixture(action):
            with (workspace / "fixture.log").open("a") as log:
                subprocess.run(
                    [str(fixture_bin)],
                    env=dict(
                        base_env,
                        MARKET_HUB_TOKEN=token,
                        MARKET_FIXTURE_HUB_URL=hub_url,
                        MARKET_FIXTURE_ACTION=action,
                    ),
                    stdout=log,
                    stderr=log,
                    check=True,
                )

        server = start("hub", hub_command, env, root)
        wait(f"http://127.0.0.1:{admin}/healthz")
        fixture("seed")
        app_command = [
            str(app),
            "--bind",
            f"127.0.0.1:{http}",
            "--hub-url",
            hub_url,
            "--web-dir",
            str(root / "web/dist"),
            "--data-dir",
            str(workspace / "replica"),
        ]
        app_process = start(
            "app", app_command, dict(base_env, MARKET_HUB_TOKEN=token), root
        )
        wait(base + "/readyz")
        query = {
            "filters": {"collection_id": "tg.gift.123", "currency_id": "GRAM"},
            "side": "ask",
            "limit": 1,
        }
        page = call("/orders/query", query)
        assert page["data"][0]["price"] == "1.25"
        assert page["data"][0]["book"]["venue_id"] == "partner"
        assert "T" in page["data"][0]["observed_at"]
        stale = page["next_page_token"]
        second = call("/orders/query", dict(query, page_token=page["next_page_token"]))
        assert second["data"][0]["price"] == "2.25"
        assert second["cursor"] == page["cursor"]
        q = dict(query, limit=200)
        result = call("/orders/query", q)
        prices = [Decimal(v["price"]) for v in result["data"]]
        assert len(prices) == 30 and prices == sorted(prices)
        descending = call("/orders/query", dict(q, sort="price_desc"))
        assert [Decimal(v["price"]) for v in descending["data"]] == sorted(
            prices, reverse=True
        )
        bounded = call(
            "/orders/query",
            dict(q, filters=dict(q["filters"], price={"min": "1.5", "max": "3"})),
        )
        assert [v["price"] for v in bounded["data"]] == ["2.25"]
        stars = call(
            "/orders/query", dict(q, filters=dict(q["filters"], currency_id="XTR"))
        )
        assert stars["data"][0]["price"] == "1250"
        executions = call("/history/query", {"filters": query["filters"]})
        assert (
            len(executions["data"]) == 1 and executions["data"][0]["kind"] == "executed"
        )
        for old in ["/items", "/trades", "/status", "/market.MarketWrite/Publish"]:
            call(old, expected=404)
        call("/orders/query", dict(query, limit=201), expected=400)
        control = workspace / "control"
        control.mkdir(exist_ok=True)
        shots = workspace / "screenshots"
        shots.mkdir(exist_ok=True)
        for f in control.glob("*"):
            f.unlink()
        browser = start(
            "browser",
            ["node", "scripts/smoke.mjs"],
            dict(
                base_env,
                MARKET_APP_TEST_URL=base,
                MARKET_APP_TEST_CONTROL=str(control),
                MARKET_APP_TEST_SCREENSHOTS=str(shots),
            ),
            root / "web",
        )
        deadline = time.monotonic() + 40
        while not (control / "update.ready").exists():
            if browser.poll() is not None:
                raise RuntimeError(
                    "Browser check stopped; inspect " + str(workspace / "browser.log")
                )
            assert time.monotonic() < deadline, "Browser fixture timed out"
            time.sleep(0.1)
        fixture("update")
        (control / "update.done").write_text("done")
        assert browser.wait(timeout=35) == 0, "Browser check failed; inspect " + str(
            workspace / "browser.log"
        )

        def eventually(check):
            deadline = time.monotonic() + 15
            while not check():
                assert time.monotonic() < deadline, "Replica convergence timed out"
                time.sleep(0.05)

        # All public queries remain available with the hub offline, including after an app restart.
        server.kill()
        server.wait()
        eventually(lambda: call("/sync")["state"] == "stale")
        assert call("/orders/query", query)["data"][0]["price"] == "0.500000001"
        assert len(call("/history/query", {"filters": query["filters"]})["data"]) == 1
        # A frozen local view survives a hub failure.
        assert (
            call("/orders/query", dict(query, page_token=stale))["data"][0]["price"]
            == "2.25"
        )
        app_process.kill()
        app_process.wait()
        app_process = start(
            "app", app_command, dict(base_env, MARKET_HUB_TOKEN=token), root
        )
        wait(base + "/readyz")
        assert call("/orders/query", query)["data"][0]["price"] == "0.500000001"
        assert len(call("/history/query", {"filters": query["filters"]})["data"]) == 1
        assert len(call("/catalog")["collections"]) == 2
        call("/orders/query", dict(query, page_token=stale), expected=409)
        # Hub restart invalidates the Watch cursor: rebuild and swap the replica atomically.
        server = start("hub", hub_command, env, root)
        wait(f"http://127.0.0.1:{admin}/healthz")
        fixture("seed-live")
        eventually(
            lambda: (
                call("/sync")["state"] == "live"
                and len(call("/orders/query", q)["data"]) == 30
                and call("/orders/query", query)["data"][0]["price"] == "1.25"
            )
        )
        assert len(call("/history/query", {"filters": query["filters"]})["data"]) == 1
        # REST cursors remain reusable as live boundaries after their snapshot root expires.
        live = call("/orders/query", dict(query, page_token=stale, consistency="live"))
        assert live["data"][0]["price"] == "2.25"
        assert call("/orders/query", dict(query, page_token=live["next_page_token"]))["data"][0]["price"] == "3.25"
        # WebSockets use portable positions, not REST continuation tokens.
        subprocess.run(
            ["node", str(root / "scripts/window-probe.mjs")],
            env=dict(base_env, PUBLIC_QUERY=json.dumps(query), PUBLIC_URL=base),
            check=True,
            timeout=30,
        )
        # Execution transitions and late backfills are indexed locally and remain durable offline.
        fixture("execute")
        eventually(
            lambda: (
                len(call("/orders/query", q)["data"]) == 29
                and len(call("/history/query", {"filters": query["filters"]})["data"])
                == 2
            )
        )
        fixture("backfill")
        eventually(
            lambda: (
                len(call("/history/query", {"filters": query["filters"]})["data"]) == 3
            )
        )
        history = call("/history/query", {"filters": query["filters"]})["data"]
        assert history[0]["id"] == "old-execution"
        server.kill()
        server.wait()
        eventually(lambda: call("/sync")["state"] == "stale")
        app_process.kill()
        app_process.wait()
        app_process = start(
            "app", app_command, dict(base_env, MARKET_HUB_TOKEN=token), root
        )
        wait(base + "/readyz")
        assert len(call("/history/query", {"filters": query["filters"]})["data"]) == 3
        assert len(call("/orders/query", q)["data"]) == 29
        # Leave the optional preview connected after testing persisted, disconnected reads.
        server = start("hub", hub_command, env, root)
        wait(f"http://127.0.0.1:{admin}/healthz")
        fixture("seed-recover")
        eventually(
            lambda: (
                call("/sync")["state"] == "live"
                and len(call("/orders/query", q)["data"]) == 29
            )
        )
        assert len(call("/history/query", {"filters": query["filters"]})["data"]) == 3
        (workspace / "preview.json").write_text(
            json.dumps(
                {
                    "url": base,
                    "hub_http": f"http://127.0.0.1:{admin}",
                    "grpc": hub_url,
                    "pids": [server.pid, app_process.pid],
                }
            )
        )
        print(
            "Verified local REST/WebSocket reads, snapshot catch-up, price ranking, filters, portable window anchors and pause/resume, persisted offline app restart, hub restart rebuild, terminal transitions and late execution backfills."
        )
        print("Preview: " + base if args.keep else "Disposable services stopped.")
    except BaseException:
        for process in processes:
            stop_group(process, signal.SIGKILL)
        raise
    finally:
        if not args.keep:
            for p in reversed(processes):
                stop_group(p, signal.SIGTERM)
            for p in processes:
                try:
                    p.wait(timeout=6)
                except subprocess.TimeoutExpired:
                    stop_group(p, signal.SIGKILL)
                    p.wait()


if __name__ == "__main__":
    main()
