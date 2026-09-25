#!/bin/bash
# Setup autonomous development cron job for Alfred
# Runs hourly to pick and implement work items

set -e

LOG_FILE="${HOME}/.alfred/agent.log"
PROMPT_FILE="$(dirname "$0")/../prompts/autonomous_developer.md"
ALFRED_BIN="$(dirname "$0")/../target/release/alfred"

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

# Verify binary exists
if [ ! -f "$ALFRED_BIN" ]; then
    echo "Error: Alfred binary not found at $ALFRED_BIN"
    echo "Run: cargo build --release"
    exit 1
fi

# Verify prompt file exists
if [ ! -f "$PROMPT_FILE" ]; then
    echo "Error: Prompt file not found at $PROMPT_FILE"
    exit 1
fi

# Detect OS
OS="$(uname -s)"

case "$OS" in
    Linux)
        echo "Setting up Linux cron job..."
        CRON_CMD="0 * * * * $ALFRED_BIN --prompt \"$PROMPT_FILE\" >> $LOG_FILE 2>&1"
        
        # Check if cron job already exists
        if crontab -l 2>/dev/null | grep -q "alfred.*autonomous"; then
            echo "Cron job already exists. Updating..."
            crontab -l 2>/dev/null | grep -v "alfred.*autonomous" | { cat; echo "$CRON_CMD"; } | crontab -
        else
            echo "Adding new cron job..."
            (crontab -l 2>/dev/null; echo "$CRON_CMD") | crontab -
        fi
        echo "Cron job installed. Check with: crontab -l"
        ;;
        
    Darwin)
        echo "Setting up macOS launchd job..."
        PLIST_PATH="${HOME}/Library/LaunchAgents/com.alfred.autonomous.plist"
        
        cat > "$PLIST_PATH" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.alfred.autonomous</string>
    <key>ProgramArguments</key>
    <array>
        <string>$ALFRED_BIN</string>
        <string>--prompt</string>
        <string>$PROMPT_FILE</string>
    </array>
    <key>StartInterval</key>
    <integer>3600</integer>
    <key>StandardOutPath</key>
    <string>$LOG_FILE</string>
    <key>StandardErrorPath</key>
    <string>${LOG_FILE}.err</string>
    <key>WorkingDirectory</key>
    <string>$(dirname "$ALFRED_BIN")</string>
</dict>
</plist>
EOF
        
        # Unload existing job if present
        launchctl unload "$PLIST_PATH" 2>/dev/null || true
        
        # Load new job
        launchctl load "$PLIST_PATH"
        echo "Launchd job installed. Check with: launchctl list | grep alfred"
        ;;
        
    MINGW*|MSYS*|CYGWIN*)
        echo "Windows detected. Using Task Scheduler..."
        
        # Create task XML
        TASK_PATH="${HOME}/alfred_autonomous.xml"
        cat > "$TASK_PATH" << EOF
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2024-01-01T00:00:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByHour>
        <HoursInterval>1</HoursInterval>
      </ScheduleByHour>
    </CalendarTrigger>
  </Triggers>
  <Actions>
    <Exec>
      <Command>$ALFRED_BIN</Command>
      <Arguments>--prompt "$PROMPT_FILE"</Arguments>
    </Exec>
  </Actions>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
  </Settings>
</Task>
EOF
        
        echo "Task XML created at: $TASK_PATH"
        echo "Import with: schtasks /create /tn \"AlfredAutonomous\" /xml \"$TASK_PATH\""
        ;;
        
    *)
        echo "Unsupported OS: $OS"
        echo "Please set up hourly cron manually:"
        echo "  $ALFRED_BIN --prompt $PROMPT_FILE >> $LOG_FILE 2>&1"
        exit 1
        ;;
esac

echo ""
echo "Setup complete!"
echo "Logs: $LOG_FILE"
echo "Prompt: $PROMPT_FILE"
echo ""
echo "To test manually:"
echo "  $ALFRED_BIN --prompt $PROMPT_FILE"
