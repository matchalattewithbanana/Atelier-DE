#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
binary=target/debug/atelier-notifications
test -x "$binary"
test -n "${DBUS_SESSION_BUS_ADDRESS:-}"
# Must be invoked under dbus-run-session (CI uses an isolated bus).
"$binary" daemon &
daemon_pid=$!
trap 'kill "$daemon_pid" 2>/dev/null || true; wait "$daemon_pid" 2>/dev/null || true' EXIT
for _ in {1..60}; do
    if "$binary" list >/dev/null 2>&1; then break; fi
    kill -0 "$daemon_pid"
    sleep 0.1
done
"$binary" list
status=0
timeout 5s "$binary" daemon || status=$?
[[ "$status" != 0 && "$status" != 124 ]] || { echo 'Second daemon did not refuse ownership' >&2; exit 1; }
call() {
    gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications --method "org.freedesktop.Notifications.$1" "${@:2}"
}
call Notify 'Atelier smoke' 0 '' 'Test title' 'Test body' '[]' '{}' 0
"$binary" list | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["active"][0]["summary"]=="Test title"; assert not s["history"]'
call Notify 'Atelier smoke' 1 '' 'Updated title' 'Updated body' '[]' '{}' 0
"$binary" list | python3 -c 'import json,sys; s=json.load(sys.stdin); assert len(s["active"])==1 and s["active"][0]["id"]==1 and s["active"][0]["summary"]=="Updated title"'
call CloseNotification 1
"$binary" list | python3 -c 'import json,sys; s=json.load(sys.stdin); assert not s["active"] and s["history"][0]["closed_reason"]==3'
"$binary" clear-history
"$binary" list | python3 -c 'import json,sys; assert not json.load(sys.stdin)["history"]'
call Notify 'Atelier smoke' 0 '' 'Dismiss me' 'Body' '[]' '{}' 0
instance="$("$binary" list | python3 -c 'import json,sys; print(json.load(sys.stdin)["instance"])')"
if "$binary" dismiss wrong-instance 2; then echo 'Stale instance accepted' >&2; exit 1; fi
"$binary" dismiss "$instance" 2
"$binary" list | python3 -c 'import json,sys; s=json.load(sys.stdin); assert not s["active"] and s["history"][0]["closed_reason"]==2'
"$binary" clear-history "$instance"
"$binary" list | python3 -c 'import json,sys; assert not json.load(sys.stdin)["history"]'
echo 'Notification D-Bus smoke passed'
