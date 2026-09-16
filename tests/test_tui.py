"""
Automated TUI test using the --dump flag.

Strategy:
1. Start Alfred server
2. Send messages via HTTP API
3. Use --dump to capture screen state
4. Verify the dump contains expected content
"""

import requests
import subprocess
import time
import glob
import os
import sys

DUMP_DIR = os.path.expanduser("~/.config/alfred/debug")
ALFRED = os.path.expanduser("~/.local/bin/alfred")
SERVER_URL = "http://localhost:18081"

def cleanup():
    os.makedirs(DUMP_DIR, exist_ok=True)
    for f in glob.glob(os.path.join(DUMP_DIR, "*.txt")):
        os.remove(f)

def start_server():
    subprocess.Popen([ALFRED], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(5)

def stop_server():
    subprocess.run(["pkill", "-f", "alfred"], capture_output=True)
    time.sleep(1)

def send_message(text):
    r = requests.post(f"{SERVER_URL}/api/messages",
                      json={"user_id": "test", "text": text},
                      timeout=30)
    return r.json().get("reply", "")

def trigger_dump():
    subprocess.run([ALFRED, "--dump"], capture_output=True, timeout=10)
    time.sleep(2)

def read_dump():
    dumps = sorted(glob.glob(os.path.join(DUMP_DIR, "*.txt")), reverse=True)
    if not dumps:
        return None
    with open(dumps[0]) as f:
        return f.read()

def test_tui():
    print("=== Alfred TUI Test ===\n")

    cleanup()
    print("1. Cleaned previous dumps")

    print("2. Starting Alfred server...")
    start_server()

    try:
        r = requests.get(f"{SERVER_URL}/health", timeout=5)
        if r.status_code != 200:
            print(f"   Server error: {r.status_code}")
            return False
        print("   Server running... OK")
    except Exception as e:
        print(f"   Server not running: {e}")
        return False

    print("3. Sending messages...")
    messages = [
        "Hello there! What is your name?",
        "What is 2+2?",
        "Tell me a short joke",
    ]

    for i, msg in enumerate(messages, 1):
        print(f"   [{i}/3] {msg}")
        reply = send_message(msg)
        print(f"   Reply: {reply[:80]}...")
        time.sleep(2)

    print("4. Triggering screen dump...")
    trigger_dump()

    print("5. Reading screen dump...")
    dump = read_dump()

    if not dump:
        print("   ERROR: No dump file found")
        return False

    print("\n" + "=" * 60)
    print("SCREEN DUMP:")
    print("=" * 60)
    print(dump)
    print("=" * 60)

    checks = [
        ("Hello there!" in dump, "User message found"),
        ("Alfred" in dump, "Agent name found"),
        ("▌" in dump, "Accent bar found"),
        ("You (" in dump, "User label found"),
        ("Type a message" in dump, "Input area found"),
    ]

    print("\n=== Checks ===")
    passed = sum(1 for ok, _ in checks if ok)
    total = len(checks)

    for ok, desc in checks:
        status = "✓" if ok else "✗"
        print(f"  {status} {desc}")

    print(f"\n{passed}/{total} checks passed")

    stop_server()
    return passed == total

if __name__ == "__main__":
    success = test_tui()
    sys.exit(0 if success else 1)
