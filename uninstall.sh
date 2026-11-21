#!/bin/bash
launchctl bootout "gui/$(id -u)/local.layoutfix" 2>/dev/null || true
rm -rf /Applications/LayoutFix.app ~/Library/LaunchAgents/local.layoutfix.plist
tccutil reset All local.layoutfix 2>/dev/null || true
echo "Removed."
