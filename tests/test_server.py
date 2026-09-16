"""
Server lifecycle tests.

Priority: Highest - these must pass before any other tests can run.
"""

import pytest
import requests


class TestServerLifecycle:
    """Tests for server start, health, info, and stop."""

    def test_health_check(self, server):
        """GET /health returns 200 with status ok."""
        resp = requests.get(f"{server['url']}/health", timeout=5)
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "ok"

    def test_server_info(self, server):
        """GET /api/info returns server metadata."""
        resp = requests.get(f"{server['url']}/api/info", timeout=5)
        assert resp.status_code == 200
        data = resp.json()
        assert "pid" in data
        assert "port" in data
        assert "uptime_secs" in data
        assert "active_connections" in data
        assert data["port"] == server["port"]
        assert data["uptime_secs"] >= 0

    def test_server_responds_to_messages(self, server):
        """POST /api/messages accepts a message and returns a reply."""
        resp = requests.post(
            f"{server['url']}/api/messages",
            json={"user_id": "lifecycle-test", "text": "Say hello"},
            timeout=60,
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "reply" in data
        assert len(data["reply"]) > 0

    def test_server_handles_concurrent_requests(self, server):
        """Server handles multiple simultaneous requests without crashing."""
        import concurrent.futures

        def make_request(i):
            return requests.post(
                f"{server['url']}/api/messages",
                json={"user_id": f"concurrent-{i}", "text": f"Say {i}"},
                timeout=60,
            )

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(make_request, i) for i in range(3)]
            results = [f.result() for f in concurrent.futures.as_completed(futures)]

        for resp in results:
            assert resp.status_code == 200
            assert "reply" in resp.json()
