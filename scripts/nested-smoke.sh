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
panel_pid=''
launcher_pid=''
cleanup() {
    if [[ -n "$launcher_pid" ]]; then
        kill -TERM "$launcher_pid" 2>/dev/null || true
        wait "$launcher_pid" || true
    fi
    if [[ -n "$panel_pid" ]]; then
        kill -TERM "$panel_pid" 2>/dev/null || true
        wait "$panel_pid" || true
    fi
    if [[ -n "$supervisor" ]]; then
        kill -TERM "$supervisor" 2>/dev/null || true
        wait "$supervisor" || true
    fi
    rm -f -- "$test_runtime/session.log" "$test_runtime/protocols.txt" "$test_runtime/panel.log" "$test_runtime/launcher.log" "$test_runtime/config/atelier-de/appearance.toml"
    rmdir -- "$test_runtime/config/atelier-de" "$test_runtime/config" 2>/dev/null || true
    rmdir -- "$test_runtime" 2>/dev/null || echo "Review remaining test runtime: $test_runtime" >&2
}
trap cleanup EXIT
if [[ "${ATELIER_TEST_PANEL:-0}" == 1 ]]; then
    mkdir -p -- "$test_runtime/config/atelier-de"
    printf 'version = 1\nbackdrop_blur = 8\npanel_backdrop_blur = 8\ndock_backdrop_blur = 8\nwidget_backdrop_blur = 8\ndock_enabled = true\nwindow_corner_radius = 12\n[window_border]\nwidth = 2\n[transparency]\nfocused = 85\npanel = 82\ndock = 82\nwidgets = 88\n[widgets]\nenabled = true\nclock = true\ncalendar = true\n[window_shadow]\nradius = 24\nopacity = 40\n' > "$test_runtime/config/atelier-de/appearance.toml"
fi
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
# Verify an actual workspace command and state transition.
python3 - "$test_runtime/$socket.workspace" "$test_runtime/$socket.workspace-state" <<'PY'
import pathlib, socket, sys, time
command, state = sys.argv[1:]
with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as client:
    for _ in range(100):
        if pathlib.Path(command).exists() and pathlib.Path(state).exists():
            break
        time.sleep(0.05)
    client.sendto(bytes([3]), command)
    for _ in range(100):
        if pathlib.Path(state).read_bytes() == bytes([3]):
            break
        time.sleep(0.05)
    else:
        raise SystemExit("workspace command did not change active workspace")
print("Workspace IPC smoke passed")
PY
if [[ "${ATELIER_TEST_PANEL:-0}" == 1 ]]; then
    env -u WAYLAND_SOCKET -u DISPLAY XDG_RUNTIME_DIR="$test_runtime" XDG_CONFIG_HOME="$test_runtime/config" WAYLAND_DISPLAY="$socket" GDK_BACKEND=wayland GSK_RENDERER=cairo target/debug/atelier-shell --panel >"$test_runtime/panel.log" 2>&1 &
    panel_pid=$!
    for _ in {1..100}; do
        if ! kill -0 "$panel_pid" 2>/dev/null; then
            cat "$test_runtime/panel.log" >&2
            exit 1
        fi
        if grep -q 'Atelier layer panel mapped' "$test_runtime/panel.log"; then break; fi
        sleep 0.1
    done
    grep -q 'Atelier layer panel mapped' "$test_runtime/panel.log" || { cat "$test_runtime/panel.log" >&2; exit 1; }
    # Open a real xdg-toplevel while software GLES is active. The shadow
    # program is compiled only when drawing an eligible mapped window.
    env -u WAYLAND_SOCKET -u DISPLAY XDG_RUNTIME_DIR="$test_runtime" XDG_CONFIG_HOME="$test_runtime/config" WAYLAND_DISPLAY="$socket" GDK_BACKEND=wayland GSK_RENDERER=cairo target/debug/atelier-shell --launcher >"$test_runtime/launcher.log" 2>&1 &
    launcher_pid=$!
    for _ in {1..100}; do
        if ! kill -0 "$launcher_pid" 2>/dev/null || ! kill -0 "$supervisor" 2>/dev/null; then
            cat "$test_runtime/launcher.log" "$test_runtime/session.log" >&2
            exit 1
        fi
        if grep -q 'Atelier rounded border shader ready' "$test_runtime/session.log" && grep -q 'Atelier smooth shadow shader ready' "$test_runtime/session.log" && grep -q 'Atelier focused backdrop rendered' "$test_runtime/session.log" && grep -q 'Atelier panel backdrop rendered' "$test_runtime/session.log" && grep -q 'Atelier dock backdrop rendered' "$test_runtime/session.log" && grep -q 'Atelier widgets backdrop rendered' "$test_runtime/session.log"; then break; fi
        sleep 0.1
    done
    grep -q 'Atelier smooth shadow shader ready' "$test_runtime/session.log" || { cat "$test_runtime/launcher.log" "$test_runtime/session.log" >&2; exit 1; }
    grep -q 'Atelier focused backdrop rendered' "$test_runtime/session.log" || { cat "$test_runtime/launcher.log" "$test_runtime/session.log" >&2; exit 1; }
    grep -q 'Atelier rounded border shader ready' "$test_runtime/session.log" || { cat "$test_runtime/session.log" >&2; exit 1; }
    grep -q "Atelier widget card input region published" "$test_runtime/panel.log" || { cat "$test_runtime/panel.log" "$test_runtime/session.log" >&2; exit 1; }
    for role in panel dock widgets; do
        grep -q "Atelier $role backdrop rendered" "$test_runtime/session.log" || { cat "$test_runtime/panel.log" "$test_runtime/session.log" >&2; exit 1; }
    done
    sleep 1
    kill -0 "$launcher_pid"
    kill -0 "$supervisor"
    kill -0 "$panel_pid"
    kill -TERM "$launcher_pid"
    wait "$launcher_pid" || true
    launcher_pid=''
    echo 'Atelier nested smooth shadow smoke passed'
    kill -TERM "$panel_pid"
    wait "$panel_pid" || true
    panel_pid=''
fi
kill -TERM "$supervisor"
wait "$supervisor"
supervisor=''
if [[ -S "$test_runtime/$socket" ]]; then
    echo 'Supervisor exited but compositor socket remains' >&2
    exit 1
fi
[[ ! -e "$test_runtime/$socket.workspace" && ! -e "$test_runtime/$socket.workspace-state" ]]
echo 'Atelier nested protocol and workspace smoke passed'
