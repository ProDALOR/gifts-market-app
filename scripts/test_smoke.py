"""Regression tests for the disposable harness; no processes or services are started."""

import json
from pathlib import Path
import unittest

from smoke import clean_environment, fixture_executable


class SmokeIsolationTests(unittest.TestCase):
    def test_service_settings_and_credentials_never_reach_disposable_processes(self):
        original = {
            "PATH": "/bin",
            "HOME": "/home/test",
            "CARGO_TARGET_DIR": "/build-cache",
            "TELEGRAM_BOT_TOKEN": "fake-production-token",
            "TELEGRAM_API_ID": "123",
            "TELEGRAM_API_HASH": "fake-hash",
            "MARKET_BOT_DATA_DIR": "/persistent-bot",
            "MARKET_HUB_URL": "http://production",
            "MARKET_HUB_TOKEN": "fake-hub-token",
            "MARKET_SESSION_KEY": "fake-session",
            "MARKET_MINIAPP_BOT_ID": "42",
            "MARKET_NO_WEB": "true",
            "VITE_MARKET_MINIAPP": "true",
            "HTTP_PROXY": "http://proxy",
        }
        cleaned = clean_environment(original)
        self.assertFalse(
            any(
                key.startswith(("MARKET_", "TELEGRAM_", "VITE_MARKET_"))
                for key in cleaned
            )
        )
        self.assertEqual(cleaned["CARGO_TARGET_DIR"], "/build-cache")
        self.assertEqual(cleaned["PATH"], "/bin")
        self.assertEqual(cleaned["NO_PROXY"], "127.0.0.1,localhost")
        self.assertEqual(original["MARKET_NO_WEB"], "true")

    def test_fixture_path_comes_from_cargo_including_custom_target_directories(self):
        output = "\n".join(
            [
                "Building fixture",
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"name": "unrelated"},
                        "executable": "/wrong",
                    }
                ),
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {"name": "smoke_fixture"},
                        "executable": "/custom-target/debug/examples/smoke_fixture",
                    }
                ),
            ]
        )
        self.assertEqual(
            fixture_executable(output),
            Path("/custom-target/debug/examples/smoke_fixture"),
        )
        with self.assertRaises(RuntimeError):
            fixture_executable('{"reason":"build-finished","success":true}')


if __name__ == "__main__":
    unittest.main()
