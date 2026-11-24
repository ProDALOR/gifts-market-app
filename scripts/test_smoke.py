"""Regression tests for the disposable harness; no processes or services are started."""

import json
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

from smoke import clean_environment, fixture_executable, stop_group


class SmokeIsolationTests(unittest.TestCase):
    def test_cleanup_never_signals_a_completed_childs_reused_process_group(self):
        process = Mock()
        process.poll.return_value = 0
        with patch("smoke.os.killpg") as kill:
            stop_group(process, 15)
            kill.assert_not_called()

    def test_cleanup_falls_back_to_the_child_handle_when_group_signal_is_denied(self):
        process = Mock()
        process.poll.return_value = None
        with patch("smoke.os.killpg", side_effect=PermissionError):
            stop_group(process, 15)
        process.send_signal.assert_called_once_with(15)
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
            "RUST_LOG": "warn",
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
        self.assertEqual(cleaned["RUST_LOG"], "info")
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
