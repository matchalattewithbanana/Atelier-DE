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
"$binary" dnd "$instance" on
call Notify 'Atelier smoke' 0 '' 'Silent title' 'Body' '[]' '{}' 0
"$binary" popup | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["dnd"] and s["notice"] is None'
"$binary" dnd "$instance" off
"$binary" popup | python3 -c 'import json,sys; s=json.load(sys.stdin); assert not s["dnd"] and s["notice"] is None'
call Notify 'Atelier smoke' 0 '' 'Visible title' 'Body' '[]' '{}' 0
"$binary" popup | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["notice"]["summary"]=="Visible title"'
progress_id="$("$binary" progress 0 25 'Building Atelier' 'Compiling')"
"$binary" progress "$progress_id" 72 'Building Atelier' 'Testing' | grep -Fx "$progress_id"
"$binary" popup | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["cards"][0]["progress"]==72 and s["cards"][0]["summary"]=="Building Atelier"; assert len(s["cards"])<=3'
if "$binary" progress 0 101 'Invalid' ''; then echo 'Invalid progress accepted' >&2; exit 1; fi
call Notify 'Atelier smoke' 0 '' 'Action title' 'Body' "['view', 'View']" '{}' 0
read -r action_id revision < <("$binary" list | python3 -c 'import json,sys; n=json.load(sys.stdin)["active"][-1]; assert n["actions"]==[{"key":"view","label":"View"}]; print(n["id"],n["revision"])')
if "$binary" invoke "$instance" "$action_id" "$((revision+1))" view; then echo 'Stale revision accepted' >&2; exit 1; fi
if "$binary" invoke "$instance" "$action_id" "$revision" wrong; then echo 'Unknown action accepted' >&2; exit 1; fi
# gdbus Notify is a short-lived sender, so this tests rejection and
# metadata only; delivery to a live application requires a separate test.
echo 'Notification D-Bus smoke passed'
