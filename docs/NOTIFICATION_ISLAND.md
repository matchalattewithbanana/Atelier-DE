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

## Experimental presentation

Run `atelier-shell --panel --notification-island` for the centred, three-card layer-shell stack, or `atelier-shell --panel --notification-popups` for a single floating card. Both remain opt-in and mutually exclusive; neither starts a notification daemon. Cards display plain text, expandable details, progress and bounded action buttons. The revision watermark prevents startup replay, and DND clears the visible stack. Incomplete progress and active actions can remain visible while eligible; other cards use a five-second local lifetime. The existing shell palette and button radius supply card styling. The slide-in transition respects reduced motion. The presentation can be customized live from Atelier Settings → Notifications (`atelier-settings --page notifications`); saving uses the existing appearance transaction and previous-version backup. Layouts are follow-launch, island, floating and compact. Placement is automatic or top left/center/right. Width (260–480 logical pixels), stacked cards (1–3), ordinary timeout (2–15 seconds), slide-in duration (off or 80–400 ms), corner radius (0–32) and background opacity (85–100%) are validated. The shell reuses its appearance watcher; settings do not enable an otherwise disabled presenter. Progress-only updates with stable card order and no actions patch the progress widget in place, while replacements requiring a rebuild preserve expansion state for the same daemon instance. The timeout applies to subsequent card updates, not the original daemon expiry. Geometry is a preferred size rather than a hard maximum on narrow displays. Graphical behavior still requires CI and desktop testing.

## Pause and built-in presets

The inbox offers timed pauses of 15 minutes, 30 minutes, 1 hour, 2 hours and 8 hours, plus an end-pause button. The CLI supports 1 second through 24 hours. A pause expires automatically against a monotonic clock. Manual DND is independent; ending a pause cannot disable manual DND. Existing popups disappear and do not replay. Notifications published during suppression remain in the active inbox or bounded history, marked missed; transient notices still do not enter history. The missed-only view filters both active and closed notices. State is in memory and resets on daemon restart.

```sh
instance=$(atelier-notifications list | jq -r .instance)
atelier-notifications pause "$instance" 30m
atelier-notifications pause "$instance" off
atelier-notifications send success "Download complete" "Ready to use"
atelier-notifications send error "Update failed" "Check the package manager"
```

Built-in presets: info, success, warning, error, download and update. They supply theme icon names, urgency and an expiry hint. Error cards use critical urgency and remain active until dismissed. Only simple theme icon names are rendered, not notification-supplied file paths. Custom presets and application-specific rules are supported through the local preferences file. Critical notifications obey DND and pause; neither mode is a lock-screen privacy boundary.

## Application preferences and custom presets

Edit `~/.config/atelier-de/notifications.toml` (or the equivalent under an absolute XDG_CONFIG_HOME):

```toml
[applications."Example App"]
popups = false
history = true

[presets.backup]
icon = "document-save-symbolic"
urgency = 1
timeout_ms = 8000
```

Publish with `atelier-notifications send backup "Backup complete" "Files saved"`. Custom names are lowercase ASCII letters, digits and hyphens, up to 48 bytes; built-in names cannot be overridden. Presets accept a simple theme icon name, urgency 0–2 and timeout 0 (persistent) or 1000–60000 milliseconds. No executable hooks or commands are accepted.

App rules match the exact case-sensitive reported application label, not a trusted identity. Disabling popups keeps notices in the active inbox and marks new suppressed deliveries as missed. Disabling history prevents active notices from entering closed history; it does not remove existing history or prevent active inbox visibility. Existing active notices are suppressed on a valid rule change, and reenabling a rule does not replay them. Replacements are new deliveries. Rules never override global DND or pause. The daemon checks for changes about once per second and retains the last valid settings on malformed live edits; deleting the file restores defaults. Invalid preferences at startup prevent the optional daemon from starting. Built-in CLI presets remain usable even when the custom preferences file is malformed. Custom presets are loaded on each CLI invocation.

The notification centre also offers optional application grouping for active and closed notices, combinable with the missed-only filter. Individual records remain independent. A graphical editor for custom policy and presets is not yet available.

## Next phases

- Refine transitions and responsive geometry; verify live placement changes, accessibility and visual behavior on actual desktops.
- Add lock-aware suppression and multi-output placement.
- Add a persistent script-friendly client for command tracking, safe action selection. AI-agent integrations are explicitly out of scope. Do not use notifications for privileged or security-sensitive approvals.
- Expose custom policy and preset editing in Atelier Studio. Keep the interface compositor-independent wherever layer-shell is supported.

No new daemon, AI integrations, default shell changes or compositor lock integration are introduced. Do not use the experimental popup surface for sensitive messages on a locked desktop; lock-aware suppression is still pending. CI must verify the changes before merge.
