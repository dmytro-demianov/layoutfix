#!/bin/bash
# raw-mode line editor (readline, like an interactive shell): each entered line → $1
while IFS= read -r -e l; do printf '%s\n' "$l" >> "$1"; done
