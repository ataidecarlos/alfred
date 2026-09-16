"""
Todo database operation tests.

Tests create, list, complete, delete via the REST API.
Agent-based tests are removed since OpenCode Go models don't support native tool calling.
"""

import pytest


class TestTodoCreate:
    """Tests for creating todos."""

    def test_create_todo_via_api(self, server):
        """Create a todo directly via the REST API."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Buy groceries", "description": "Milk, eggs, bread", "priority": "medium"},
            timeout=5,
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "id" in data

    def test_create_todo_with_priority(self, server):
        """Create a high-priority todo."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Call dentist", "priority": "high"},
            timeout=5,
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "id" in data


class TestTodoList:
    """Tests for listing todos."""

    def test_list_todos(self, server, helpers):
        """List todos via the REST API."""
        import requests
        # Create a todo first
        requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Listable todo"},
            timeout=5,
        )

        resp = requests.get(f"{server['url']}/api/todos", timeout=5)
        assert resp.status_code == 200
        todos = resp.json()
        assert isinstance(todos, list)
        assert len(todos) > 0

    def test_list_todos_content(self, server, helpers):
        """Verify todo content is correct."""
        import requests
        requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Test todo alpha", "priority": "high"},
            timeout=5,
        )

        resp = requests.get(f"{server['url']}/api/todos", timeout=5)
        todos = resp.json()
        titles = [t["title"] for t in todos]
        assert "Test todo alpha" in titles


class TestTodoDelete:
    """Tests for deleting todos."""

    def test_delete_todo_via_api(self, server, helpers):
        """Delete a todo via the REST API."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Deletable todo"},
            timeout=5,
        )
        todo_id = resp.json()["id"]

        resp = requests.delete(
            f"{server['url']}/api/todos/{todo_id}",
            timeout=5,
        )
        assert resp.status_code == 204

        # Verify it's gone
        rows = helpers["get_database_rows"](server["db_path"], "todos")
        assert all(r["id"] != todo_id for r in rows)


class TestTodoViaAgent:
    """Tests for todo operations via the agent conversation."""

    def test_agent_knows_about_todos(self, server, client, helpers):
        """Ask Alfred about its capabilities."""
        c, url = client["session"], server["url"]

        reply = helpers["send_message"](
            c, url, "What tools do you have available?"
        )

        assert len(reply) > 0
        assert reply != "No response generated."
