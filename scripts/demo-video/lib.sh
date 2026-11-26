#!/usr/bin/env bash
# scripts/demo-video/lib.sh — shared setup for the CyberAPI demo recording.

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
WORK="${WORK:-/tmp/cyberapi-demo-run}"
DATA_DIR="$WORK/data"
LEGACY_DIR="$WORK/no-legacy"
MOCK_PORT="${MOCK_PORT:-17391}"
MOCK="http://127.0.0.1:$MOCK_PORT"
WIN_W="${WIN_W:-1280}"
WIN_H="${WIN_H:-720}"
WIN_X="${WIN_X:-116}"
WIN_Y="${WIN_Y:-120}"
DRV="$WORK/driver"

mkdir -p "$WORK"

demo_build_tools() {
    if [ ! -x "$DRV" ] || [ "$DEMO_DIR/driver.swift" -nt "$DRV" ]; then
        echo "==> Compiling input driver..."
        swiftc -O "$DEMO_DIR/driver.swift" -o "$DRV"
    fi
    if [ ! -x "$WORK/assets" ] || [ "$DEMO_DIR/assets.swift" -nt "$WORK/assets" ]; then
        echo "==> Compiling studio assets renderer..."
        swiftc -O "$DEMO_DIR/assets.swift" -o "$WORK/assets"
    fi
    if [ ! -x "$WORK/recorder" ] || [ "$DEMO_DIR/recorder.swift" -nt "$WORK/recorder" ]; then
        echo "==> Compiling window recorder..."
        swiftc -O "$DEMO_DIR/recorder.swift" -o "$WORK/recorder"
    fi
}

demo_find_binary() {
    if [ -n "${CYBERAPI_BIN:-}" ] && [ -x "$CYBERAPI_BIN" ]; then
        return
    fi
    local target
    target="$(cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" --format-version 1 --no-deps 2>/dev/null |
        python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"
    if [ "${DEMO_SKIP_BUILD:-0}" != "1" ]; then
        echo "==> Building cyberapi..."
        cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --bin cyberapi >&2
    fi
    CYBERAPI_BIN="$target/debug/cyberapi"
    if [ ! -x "$CYBERAPI_BIN" ]; then
        echo "error: binary not found at $CYBERAPI_BIN" >&2
        exit 1
    fi
}

demo_start_mock() {
    python3 "$DEMO_DIR/mock_server.py" "$MOCK_PORT" >"$WORK/mock.log" 2>&1 &
    MOCK_PID=$!
    for _ in {1..40}; do
        curl -s -o /dev/null "$MOCK/v1/products" && return
        sleep 0.1
    done
    echo "error: mock server failed to start on $MOCK_PORT" >&2
    exit 1
}

demo_seed() {
    rm -rf "$DATA_DIR" "$LEGACY_DIR"
    mkdir -p "$DATA_DIR" "$LEGACY_DIR"
    python3 "$DEMO_DIR/seed.py" \
        --data-dir "$DATA_DIR" \
        --base-url "$MOCK" \
        --width "$WIN_W" \
        --height "$WIN_H"
}

demo_launch_app() {
    CYBERAPI_DATA_DIR="$DATA_DIR" \
    CYBERAPI_LEGACY_DIR="$LEGACY_DIR" \
    RUST_LOG=info \
    "$CYBERAPI_BIN" >"$WORK/stdout.log" 2>"$WORK/stderr.log" &
    APP_PID=$!
    for _ in {1..60}; do
        "$DRV" bounds "$APP_PID" >/dev/null 2>&1 && break
        sleep 0.25
    done
    osascript <<OSA >/dev/null 2>&1 || true
tell application "System Events" to tell (first process whose unix id is $APP_PID)
    set frontmost to true
    set size of window 1 to {$WIN_W, $WIN_H}
    set position of window 1 to {$WIN_X, $WIN_Y}
end tell
OSA
    sleep 1.0
    WID="$("$DRV" wid "$APP_PID")"
    local actual_bounds
    actual_bounds="$("$DRV" bounds "$APP_PID")"
    IFS=',' read -r WIN_X WIN_Y WIN_W WIN_H <<< "$actual_bounds"
    export WIN_X WIN_Y WIN_W WIN_H
    echo "==> CyberAPI window $WID at ${WIN_X},${WIN_Y} (${WIN_W}x${WIN_H})"
}

demo_front() {
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $APP_PID) to true" >/dev/null 2>&1 || true
}

demo_cleanup() {
    for pid in ${APP_PID:-} ${MOCK_PID:-} ${CAFF_PID:-}; do
        kill "$pid" 2>/dev/null || true
    done
    if [ -n "${REC_PID:-}" ] && kill -0 "$REC_PID" 2>/dev/null; then
        kill -INT "$REC_PID" 2>/dev/null || true
        wait "$REC_PID" 2>/dev/null || true
    fi
    for pid in ${APP_PID:-} ${MOCK_PID:-}; do
        wait "$pid" 2>/dev/null || true
    done
    if [ "${DEMO_KEEP:-0}" != "1" ] && [[ "$WORK" == /tmp/* ]]; then
        rm -rf "$WORK"
    fi
}
