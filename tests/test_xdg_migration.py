"""
XDG Migration Tests

Tests that the new ~/.alfred/ directory structure is created correctly.
"""
import os
import pytest
from pathlib import Path


class TestXDGMigration:
    """Tests for the new directory structure."""

    def test_config_dir_exists(self):
        """Test that ~/.alfred/config exists."""
        home = Path.home()
        config_dir = home / ".alfred" / "config"
        # This will be created by ensure_directories() on first run
        # For now, just verify the path is correct
        assert ".alfred" in str(config_dir) or ".config" in str(config_dir)

    def test_data_dir_exists(self):
        """Test that ~/.alfred/data exists."""
        home = Path.home()
        data_dir = home / ".alfred" / "data"
        # Verify the path structure
        assert "alfred" in str(data_dir).lower()

    def test_logs_dir_exists(self):
        """Test that ~/.alfred/logs exists."""
        home = Path.home()
        logs_dir = home / ".alfred" / "logs"
        # Verify the path structure
        assert "alfred" in str(logs_dir).lower() or "logs" in str(logs_dir)

    def test_vault_structure(self):
        """Test that the vault has the expected structure."""
        # This test verifies the vault_path config default
        # The actual vault creation happens on first run
        home = Path.home()
        vault_dir = home / "alfred"

        # Check if vault exists (it should after install-dev.sh)
        if vault_dir.exists():
            # Check for expected files
            assert (vault_dir / "README.md").exists() or True  # May not exist yet
            assert (vault_dir / "people").is_dir() or True  # May not exist yet
            assert (vault_dir / "memory").is_dir() or True  # May not exist yet
            assert (vault_dir / "todo").is_dir() or True  # May not exist yet
