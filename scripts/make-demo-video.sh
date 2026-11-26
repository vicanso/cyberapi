#!/usr/bin/env bash
# scripts/make-demo-video.sh
# Records a Light-mode studio product demo of CyberAPI on macOS and renders
# ~/Movies/cyberapi-demo.mp4 (1920x1080, 60fps).
#
# Needs: macOS 14+, ffmpeg/ffprobe, swiftc, Screen Recording + Accessibility
# permissions for the terminal. Launches an isolated /tmp workspace and never
# modifies user data, OS keychain, or system Dark/Light mode.
#
# Env: CYBERAPI_BIN=<path> to skip building, DEMO_SKIP_BUILD=1, DEMO_OUT=<file.mp4>,
#      DEMO_KEEP=1 to keep /tmp/cyberapi-demo-run artifacts.

set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/demo-video/lib.sh"

OUT="${DEMO_OUT:-$HOME/Movies/cyberapi-demo.mp4}"
RAW="$WORK/raw.mov"
export DRV_LOG="$WORK/driver.log"
rm -f "$RAW" "$RAW.start" "$DRV_LOG"

trap demo_cleanup EXIT INT TERM

demo_build_tools
demo_find_binary
demo_start_mock
demo_seed
demo_launch_app
caffeinate -d -t 300 &
CAFF_PID=$!

d() { "$DRV" "$@"; }
mark() { d mark "$@"; }
cam() { d cam "$@"; }
hold() { sleep "$1"; }

d move 640 360 0.2
demo_front
echo "==> Recording window $WID..."
"$WORK/recorder" "$WID" "$RAW" &
REC_PID=$!
sleep 1.4
for _ in {1..20}; do
    demo_front
    sleep 0.25
    [ "$(osascript -e 'tell application "System Events" to get unix id of first process whose frontmost is true' 2>/dev/null)" = "$APP_PID" ] && break
done
sleep 0.6

# ----------------------------------------------------------------- storyboard
# Note: keep zoom in [1.08..1.14] with cx in [605..670] so all four window
# edges and the soft drop shadow always stay unclipped inside the 16:9 frame.
mark begin
hold 0.5

# Chapter 01: REST requests & live assertions
mark rest "Send REST requests and verify live assertions" "发送 REST 请求，实时校验断言"
cam 1.11 660 335 0.9
d click 686 91 0.55         # Send List products -> 200 OK, Assert 2/2
hold 2.2
d click 410 52 0.50         # Create order tab
hold 0.6
d click 686 91 0.45         # Send Create order -> 201 Created, Assert 2/2
hold 2.3

# Chapter 02: JSON tree & request timings
mark inspect "Inspect as a JSON tree or full request timings" "JSON 树视图与完整耗时追踪"
cam 1.13 670 335 0.9
d click 280 52 0.50         # Back to List products tab
hold 0.5
d click 1228 130 0.45       # Tree view toggle
hold 0.5
d click 805 173 0.40        # Expand data [3 items]
hold 0.4
d click 815 193 0.35        # Expand [0] item fields
hold 2.0
d click 960 130 0.50        # Details tab (timings + masked Authorization header)
hold 2.5

# Chapter 03: Live Server-Sent Events
mark sse "Stream Server-Sent Events live and stop anytime" "实时接收 SSE 事件流，随时停止并保留数据"
cam 1.12 665 335 0.9
d click 540 52 0.50         # Order events tab
hold 0.5
d click 686 91 0.45         # Send -> live SSE stream starts
hold 2.8
d click 686 91 0.45         # Stop -> stops stream and keeps received events
hold 2.0

# Chapter 04: GraphQL
mark graphql "Native GraphQL editor with variables" "原生 GraphQL 查询与变量编辑"
cam 1.11 650 335 0.9
d click 655 52 0.50         # Shop GraphQL tab
hold 0.5
d click 686 91 0.45         # Send GraphQL query
hold 2.3

# Chapter 05: Sync from OpenAPI source
mark sync "Sync from OpenAPI while keeping your parameter values" "OpenAPI 文档同步：新增接口并保留已填参数"
cam 1.12 620 330 0.9
d key 40 cmd                # Cmd+K Command Palette
hold 0.8
d type "Sync" 11
hold 0.9
d key 36                    # Return opens Sync from source dialog
hold 0.9
d click 898 199 0.50        # Fetch OpenAPI preview
hold 2.0
d click 783 444 0.55        # Sync -> adds Get a product to Catalog
hold 1.1
cam 1.10 615 335 0.9
d click 90 113 0.55         # Select List products in tree -> verify inStock=true stayed
hold 1.8
d click 90 169 0.50         # Select newly synced Get a product
hold 0.6
d click 686 91 0.45         # Send Get a product -> 200 OK
hold 2.5

cam 1.0 640 360 0.8
hold 0.9
mark end
# ---------------------------------------------------------------------------

sleep 0.4
kill -INT "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true
unset REC_PID
kill "$CAFF_PID" 2>/dev/null || true
unset CAFF_PID

echo "==> Composing video..."
mkdir -p "$(dirname "$OUT")"
python3 "$DEMO_DIR/compose.py" "$RAW" "$DRV_LOG" "$WORK" "$WORK/assets" "$REPO_ROOT/icons/cyberapi.png" "$OUT"
echo "==> Done: $OUT ($(du -h "$OUT" | cut -f1))"
