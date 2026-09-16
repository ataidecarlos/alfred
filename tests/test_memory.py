"""
Memory vault operation tests.

Tests store and recall via the agent (basic conversation).
Tests archive, update, and file verification via REST API.
"""

import time
import pytest


class TestMemoryRecall:
    """Tests for recalling memories via the agent."""

    def test_recall_basic(self, server, client, helpers):
        """Ask Alfred to recall a known fact."""
        c, url = client["session"], server["url"]

        reply = helpers["send_message"](
            c, url, "What is the capital of France?"
        )

        assert len(reply) > 0
        assert reply != "No response generated."
        assert "paris" in reply.lower()


class TestMemoryViaApi:
    """Tests for memory operations via the REST API."""

    def test_create_memory_via_api(self, server):
        """Create a memory directly via the REST API."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/memories",
            json={"content": "User prefers dark mode for all applications"},
            timeout=5,
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "id" in data

    def test_list_memories_via_api(self, server):
        """List memories via the REST API."""
        import requests
        # Create a memory first
        requests.post(
            f"{server['url']}/api/memories",
            json={"content": "Test memory for listing"},
            timeout=5,
        )

        resp = requests.get(f"{server['url']}/api/memories", timeout=5)
        assert resp.status_code == 200
        memories = resp.json()
        assert isinstance(memories, list)

    def test_delete_memory_via_api(self, server, helpers):
        """Delete a memory via the REST API."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/memories",
            json={"content": "Memory to delete"},
            timeout=5,
        )
        memory_id = resp.json()["id"]

        resp = requests.delete(
            f"{server['url']}/api/memories/{memory_id}",
            timeout=5,
        )
        assert resp.status_code == 204

        # Verify it's gone
        rows = helpers["get_database_rows"](server["db_path"], "memories")
        assert all(r["id"] != memory_id for r in rows)


class TestVaultScaffolding:
    """Tests for vault directory structure."""

    def test_vault_structure_exists(self, server):
        """Verify vault directories were created."""
        vault = server["vault_path"]

        expected_dirs = [
            "_templates", "_attachments", "_config",
            "inbox", "preferences", "facts", "decisions",
            "lessons", "action-items", "daily",
        ]
        for d in expected_dirs:
            assert (vault / d).exists(), f"Directory {d} not found in vault"

    def test_vault_templates_exist(self, server):
        """Verify vault templates were created."""
        vault = server["vault_path"]

        expected_files = [
            "_templates/memory.md",
            "_templates/decision.md",
            "_templates/moc.md",
            "_config/topics.md",
            "index.md",
            "alfred-index.base",
        ]
        for f in expected_files:
            assert (vault / f).exists(), f"File {f} not found in vault"

    def test_vault_index_content(self, server):
        """Verify vault index.md has correct content."""
        vault = server["vault_path"]
        index_content = (vault / "index.md").read_text()

        assert "Alfred Memory Vault" in index_content
        assert "preferences" in index_content
        assert "facts" in index_content
