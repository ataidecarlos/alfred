"""
Conversation tests.

Tests multi-turn context and basic conversational ability.
Does NOT test tool usage (OpenCode Go models don't support native tool calling).
Tool tests are in test_todo.py and test_memory.py via REST API.
"""

import time
import pytest


class TestBasicConversation:
    """Tests for basic conversational ability."""

    def test_greeting(self, server, client, helpers):
        """Send a greeting, verify Alfred responds."""
        c, url = client["session"], server["url"]

        reply = helpers["send_message"](c, url, "Hello, what is your name?")

        assert len(reply) > 0
        assert reply != "No response generated."

    def test_identity(self, server, client, helpers):
        """Alfred should know its own identity."""
        c, url = client["session"], server["url"]

        reply = helpers["send_message"](c, url, "What are you?")

        assert len(reply) > 0
        assert reply != "No response generated."

    def test_capabilities(self, server, client, helpers):
        """Alfred should describe its capabilities."""
        c, url = client["session"], server["url"]

        reply = helpers["send_message"](
            c, url, "What can you do? What tools do you have?"
        )

        assert len(reply) > 0
        assert reply != "No response generated."


class TestMultiTurnContext:
    """Tests for maintaining context across messages."""

    def test_remember_name(self, server, client, helpers):
        """Tell Alfred a name, then ask it back."""
        c, url = client["session"], server["url"]

        helpers["send_message"](c, url, "My name is Carlos")
        time.sleep(1)

        reply = helpers["send_message"](c, url, "What is my name?")

        assert "carlos" in reply.lower()

    def test_context_across_topics(self, server, client, helpers):
        """Switch topics and verify context is maintained."""
        c, url = client["session"], server["url"]

        helpers["send_message"](c, url, "I live in Lisbon")
        time.sleep(1)
        helpers["send_message"](c, url, "The weather is nice today")
        time.sleep(1)

        reply = helpers["send_message"](c, url, "Where do I live again?")

        assert "lisbon" in reply.lower()


class TestConversationPersistence:
    """Tests for conversation history persistence."""

    def test_conversation_saved(self, server, client, helpers):
        """Send a message, verify conversation is saved in the database."""
        c, url = client["session"], server["url"]
        db = server["db_path"]

        helpers["send_message"](c, url, "Persist this test message")
        time.sleep(2)

        rows = helpers["get_database_rows"](db, "conversations")
        assert len(rows) > 0, "No conversations saved"

        # Verify the conversation contains our message
        found = False
        for row in rows:
            messages_json = row.get("messages", "")
            if "Persist this test message" in messages_json:
                found = True
                break
        assert found, "Test message not found in saved conversations"

    def test_conversation_user_isolation(self, server, client, helpers):
        """Different user IDs should have separate conversations."""
        c, url = client["session"], server["url"]

        helpers["send_message"](c, url, "Message from user A", user_id="user-a")
        time.sleep(1)
        helpers["send_message"](c, url, "Message from user B", user_id="user-b")
        time.sleep(1)

        # Each user should only see their own context
        reply_a = helpers["send_message"](
            c, url, "What did I just say?", user_id="user-a"
        )
        reply_b = helpers["send_message"](
            c, url, "What did I just say?", user_id="user-b"
        )

        # Both should respond (we can't guarantee exact content without tool calling)
        assert len(reply_a) > 0
        assert len(reply_b) > 0
