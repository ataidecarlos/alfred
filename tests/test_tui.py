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

def run_dump(*args):
    """Run alfred with dump flags, return the latest dump content."""
    if not args:
        args = ("--dump",)
    subprocess.run([ALFRED] + list(args), capture_output=True, timeout=180)
    time.sleep(1)
    return latest_dump("screen_dump")

def read_dump():
    dumps = sorted(glob.glob(os.path.join(DUMP_DIR, "*.txt")), reverse=True)
    if not dumps:
        return None
    with open(dumps[0]) as f:
        return f.read()

def latest_dump(prefix):
    dumps = sorted(glob.glob(os.path.join(DUMP_DIR, f"{prefix}_*.txt")), reverse=True)
    if not dumps:
        return None
    with open(dumps[0]) as f:
        return f.read()

def check(name, results, condition, desc):
    results.append((condition, desc))
    print(f"  {'✓' if condition else '✗'} [{name}] {desc}")
    return condition

def test_tui():
    print("=== Alfred TUI Test ===\n")
    results = []

    cleanup()
    print("1. Cleaned previous dumps")

    # --- Chat dump (live LLM conversation through real render path) ---
    print("2. Chat dump (--dump)...")
    dump = run_dump()
    check("chat", results, dump is not None, "dump file created")
    if dump:
        print(dump)
        check("chat", results, "Hello there!" in dump, "user message found")
        check("chat", results, "Alfred" in dump, "agent name found")
        check("chat", results, "▌" in dump, "accent bar found")
        check("chat", results, "You (" in dump, "user label found")
        check("chat", results, "Type a message" in dump, "input area found")

    # --- Palette dump, scrolled to last item ---
    print("3. Palette dump (--dump-palette --palette-select 8)...")
    cleanup()
    palette = run_dump("--dump-palette", "--palette-select", "8")
    check("palette", results, palette is not None, "palette dump created")
    if palette:
        print(palette)
        check("palette", results, "Command Palette" in palette, "panel title found")
        # select=8 scrolls the list: /clear drops out, marker lands on /quit.
        for cmd in ["/help", "/todos", "/memories", "/dump",
                    "/theme-dark", "/theme-light", "/config", "/quit"]:
            check("palette", results, cmd in palette, f"{cmd} listed")
        check("palette", results, "/clear" not in palette, "/clear scrolled out (scroll works)")
        check("palette", results, ">/quit" in palette, "selection marker on /quit")
        check("palette", results, "Enter Select" in palette, "footer found")

    # --- Filtered palette dump ---
    print("4. Filtered palette (--palette-filter /theme)...")
    cleanup()
    filtered = run_dump("--dump-palette", "--palette-filter", "/theme")
    check("filter", results, filtered is not None, "filtered dump created")
    if filtered:
        print(filtered)
        check("filter", results, "/theme-dark" in filtered, "/theme-dark shown")
        check("filter", results, "/theme-light" in filtered, "/theme-light shown")
        check("filter", results, "/quit" not in filtered, "/quit filtered out")

    # --- Light theme dump ---
    print("5. Light theme dump (--dump --dump-theme light)...")
    cleanup()
    light = run_dump("--dump-theme", "light")
    check("light", results, light is not None, "light dump created")
    if light:
        check("light", results, "Alfred" in light, "content rendered")

    passed = sum(1 for ok, _ in results if ok)
    total = len(results)
    print(f"\n{passed}/{total} checks passed")
    return passed == total

if __name__ == "__main__":
    success = test_tui()
    sys.exit(0 if success else 1)
