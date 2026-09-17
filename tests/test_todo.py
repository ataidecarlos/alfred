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


class TestTodoUpdate:
    """Tests for updating todos via PUT /api/todos/{id}."""

    def test_update_todo_title_and_priority(self, server):
        """Update title and priority, verify via list."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Old title", "priority": "low"},
            timeout=5,
        )
        todo_id = resp.json()["id"]

        resp = requests.put(
            f"{server['url']}/api/todos/{todo_id}",
            json={"title": "New title", "priority": "high"},
            timeout=5,
        )
        assert resp.status_code == 204

        resp = requests.get(f"{server['url']}/api/todos", timeout=5)
        by_id = {t["id"]: t for t in resp.json()}
        assert by_id[todo_id]["title"] == "New title"
        assert by_id[todo_id]["priority"] == "high"

    def test_update_unknown_todo_returns_404(self, server):
        """PUT on a nonexistent id returns 404."""
        import requests
        resp = requests.put(
            f"{server['url']}/api/todos/does-not-exist",
            json={"title": "Nope"},
            timeout=5,
        )
        assert resp.status_code == 404


class TestTodoComplete:
    """Tests for POST /api/todos/{id}/complete."""

    def test_complete_todo_via_api(self, server, helpers):
        """Complete a todo; it leaves the active list and is flagged in DB."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos",
            json={"title": "Completable todo"},
            timeout=5,
        )
        todo_id = resp.json()["id"]

        resp = requests.post(
            f"{server['url']}/api/todos/{todo_id}/complete",
            timeout=5,
        )
        assert resp.status_code == 204

        # Completed todos are filtered from the active list
        resp = requests.get(f"{server['url']}/api/todos", timeout=5)
        assert all(t["id"] != todo_id for t in resp.json())

        # ...but still present in the DB flagged completed
        rows = helpers["get_database_rows"](server["db_path"], "todos")
        matches = [r for r in rows if r["id"] == todo_id]
        assert len(matches) == 1
        assert matches[0]["completed"] == 1

    def test_complete_unknown_todo_returns_404(self, server):
        """POST complete on a nonexistent id returns 404."""
        import requests
        resp = requests.post(
            f"{server['url']}/api/todos/does-not-exist/complete",
            timeout=5,
        )
        assert resp.status_code == 404


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
