# Atelier Notifications

The separate `atelier-notifications` executable implements an experimental, headless freedesktop notification inbox. It is intentionally not autostarted: a desktop without a graphical notification centre should continue using its existing daemon.

Test it on an isolated D-Bus session after building the workspace:

```sh
cargo build -p atelier-notifications
dbus-run-session -- bash scripts/notification-smoke.sh
```

`atelier-notifications daemon` explicitly requests the standard notification bus name without replacing an existing owner or queuing for ownership. `atelier-notifications list` reads the Atelier schema-1 JSON inbox; `clear-history` clears closed notices. Both operate on the current user session bus, not a system service. Notification content remains in process memory only; active messages are capped at 128, closed history at 100, and transient messages are never retained after closing. The history is accessible to other processes on the same user bus; do not assume it is private from those processes. Stopping the daemon clears all data.

The server handles plain text summaries and bodies, active replacement, expiration, explicit close and close signals. Zero timeout leaves a notice active until closed; negative timeout uses five seconds. Missing replacement IDs allocate a new ID. Images, actions, markup, urgency-specific policy, persistent storage, graphical popups, Do Not Disturb are not implemented yet. This is not advertised as full freedesktop specification compliance or a replacement for a production desktop daemon. No sudo, MangoWM, shell or compositor dependency is required.

## Graphical inbox

Open Notifications from the Atelier panel or standalone launcher. It reads the explicitly started Atelier daemon only and never starts or replaces an existing service. It presents plain text active notifications and closed history, refreshes while the window is open, and offers explicit dismiss and clear-history controls. The inbox uses a daemon-instance identifier to reject stale actions after a daemon restart. GUI helper calls require coreutils `timeout`. Popups and Do Not Disturb remain future work.

## Experimental popups and Do Not Disturb

On a dedicated test session with the Atelier daemon already running, build the shell with `layer-panel` and run `atelier-shell --panel --notification-popups` instead of another panel. Popups are opt-in; the shell does not claim the notification service. The presenter uses a non-interactive layer-shell overlay, showing only the newest newly observed eligible active notification for up to five seconds. It polls a compact snapshot once per second through the bounded helper. Very short-lived notices, bursts and replacements can be missed; no replay of older notices occurs. It is not a lock-screen-safe or production notification renderer. Do not run the experimental presenter in a session where private notification content must be hidden on lock.

The centre's Do Not Disturb button changes the daemon's in-memory session policy. It suppresses this presenter without deleting notifications, and suppressed notices are not replayed when disabled. It does not control other notification daemons. Persistent preferences, urgency handling, actions, icons, multi-output and lock integration remain future work.
