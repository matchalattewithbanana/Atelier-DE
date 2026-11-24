#!/usr/bin/env bash
set -euo pipefail

# Run under a dedicated Xvfb display. Never unset the caller's host environment.
cd "$(dirname "$0")/.."
test -n "${DISPLAY:-}" || { echo 'Run this with xvfb-run -a'; exit 1; }
command -v wayland-info >/dev/null
test -x target/debug/atelier-session
test -x target/debug/atelier-compositor

test_runtime="$(mktemp -d)"
supervisor=''
cleanup() {
    if [[ -n "$supervisor" ]]; then
        kill -TERM "$supervisor" 2>/dev/null || true
        wait "$supervisor" || true
    fi
    rm -f -- "$test_runtime/session.log" "$test_runtime/protocols.txt"
    rmdir -- "$test_runtime" 2>/dev/null || echo "Review remaining test runtime: $test_runtime" >&2
}
trap cleanup EXIT
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$test_runtime" XDG_CONFIG_HOME="$test_runtime/config" LIBGL_ALWAYS_SOFTWARE=1 target/debug/atelier-session --nested --no-shell >"$test_runtime/session.log" 2>&1 &
supervisor=$!
socket="wayland-atelier-$supervisor"

for _ in {1..100}; do
    [[ -S "$test_runtime/$socket" ]] && break
    if ! kill -0 "$supervisor" 2>/dev/null; then
        cat "$test_runtime/session.log" >&2
        exit 1
    fi
    sleep 0.1
done
if ! [[ -S "$test_runtime/$socket" ]]; then
    cat "$test_runtime/session.log" >&2
    exit 1
fi
env -u WAYLAND_SOCKET XDG_RUNTIME_DIR="$test_runtime" WAYLAND_DISPLAY="$socket" timeout 10s wayland-info >"$test_runtime/protocols.txt"
for global in wl_compositor wl_shm xdg_wm_base wl_seat wl_output zwlr_layer_shell_v1; do
    if ! grep -q "interface: '$global'" "$test_runtime/protocols.txt"; then
        echo "Missing protocol: $global" >&2
        exit 1
    fi
done
kill -TERM "$supervisor"
wait "$supervisor"
supervisor=''
if [[ -S "$test_runtime/$socket" ]]; then
    echo 'Supervisor exited but compositor socket remains' >&2
    exit 1
fi
echo 'Atelier nested protocol smoke passed'
