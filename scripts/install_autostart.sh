#!/bin/bash

# Configuration
AGENT_NAME="com.steer.local-os-agent"
PLIST_PATH="$HOME/Library/LaunchAgents/$AGENT_NAME.plist"
CORE_DIR="$(cd "$(dirname "$0")/../core" && pwd)"
BINARY_PATH="$CORE_DIR/target/release/local_os_agent"
LOG_DIR="$HOME/.local-os-agent"

# Colors
GREEN='\033[0;32m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Installing Local OS Agent as a Background Service...${NC}"

# 1. Build Release Binary
echo "📦 Building core binary..."
cd "$CORE_DIR" || exit
cargo build --release --bin local_os_agent

if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Build failed. local_os_agent binary not found."
    exit 1
fi

# 2. Create Log Directory
mkdir -p "$LOG_DIR"

# 3. Create LaunchAgent plist
echo "📝 Creating LaunchAgent plist..."
cat <<EOF > "$PLIST_PATH"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$AGENT_NAME</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BINARY_PATH</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>$CORE_DIR</string>
    <key>StandardOutPath</key>
    <string>$LOG_DIR/agent.log</string>
    <key>StandardErrorPath</key>
    <string>$LOG_DIR/agent.error.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>STEER_API_ALLOW_NO_KEY</key>
        <string>1</string>
        <key>STEER_DISABLE_EVENT_TAP</key>
        <string>1</string>
        <key>STEER_COLLECTOR_HANDOFF_AUTOCONSUME</key>
        <string>0</string>
        <key>PATH</key>
        <string>$PATH:/usr/local/bin:/opt/homebrew/bin</string>
    </dict>
</dict>
</plist>
EOF

# 4. Load Service
echo "🔄 Loading service..."
launchctl unload "$PLIST_PATH" 2>/dev/null
launchctl load "$PLIST_PATH"

echo -e "${GREEN}✅ Success! The Agent will now start automatically on login.${NC}"
echo "   - View Logs: tail -f $LOG_DIR/agent.log"
echo "   - API: http://localhost:5680"
