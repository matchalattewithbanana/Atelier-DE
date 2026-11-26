# Atelier notification island

Design reference: [Drip](https://github.com/jrnxf/drip), a MIT-licensed macOS notification CLI. Atelier's implementation is native Rust and GTK/Wayland, not a port of the Swift application.

## Foundation

The existing opt-in notification daemon remains the single source of state. Existing schema-1 snapshots retain their fields; additive fields include progress (0–100, or null), icon name, bounded action pairs and revision. Popup snapshots additionally contain up to three eligible cards ordered by latest revision and an overflow count. DND suppresses the stack without replaying suppressed cards. The original single-notice field remains unchanged for old clients.

The notification centre shows progress percentages. For testing and scripting on an isolated Atelier notification bus:

```sh
id=$(atelier-notifications progress 0 0 "Build" "Starting")
atelier-notifications progress "$id" 72 "Build" "Testing"
atelier-notifications progress "$id" 100 "Build" "Done"
```

Use a replacement ID only within the same daemon lifetime. Legacy non-action notifications retain their existing replacement-by-ID behavior. Action-bearing replacements require the originating D-Bus unique name; action keys are opaque, consumed once per revision and sent to that origin, never executed as commands by the daemon. The custom Invoke method checks instance, ID, revision, expiry and action key. This is an ordinary same-session bus interface, not a privileged approval or authentication boundary. A disconnected origin cannot receive its action. The daemon does not yet advertise standard actions capability pending graphical controls and live-client delivery testing.

## Next phases

- Render compact, stacked and expanded cards in the layer-shell panel with smooth bounded transitions, progress bars and action controls. Preserve standard inbox fallback.
- Add explicit pause, missed-card handling, lock-aware suppression and output placement.
- Add a persistent script-friendly client for command tracking, safe action selection, presets and opt-in coding-agent hooks. Do not use notifications for privileged or security-sensitive approvals.
- Expose layout, position, animation and color options in Atelier Studio. Keep the interface compositor-independent wherever layer-shell is supported.

No new daemon, automatic hooks, default shell changes or compositor lock integration are introduced by this foundation. CI must verify the changes before merge.
