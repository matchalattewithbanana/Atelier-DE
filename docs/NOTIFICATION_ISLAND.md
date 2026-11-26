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

The notification centre now presents graphical notification cards, with text search, application filtering, missed-only filtering, expandable application groups, progress indicators, per-card dismissal and active application actions. Individual records remain independent. Group expansion is preserved on updates within the same daemon instance; pause countdown changes do not rebuild card content.

Open `atelier-settings --page notification-rules` to add, update or remove application rules and custom presets. Changes remain in a draft until explicitly saved. The editor and daemon share one validated configuration implementation. Saves reject stale drafts and use atomic file replacement with `notifications.previous.toml` as a previous-version backup. Load previous preferences into a draft and save to restore them. The editor does not start the daemon.

## Direct shell transport

Both the notification centre and the layer-shell presenter use the same GIO session-bus transport rather than spawning the notification CLI and timeout processes for each refresh. D-Bus calls use a three-second method timeout, do not auto-start a daemon and are serialized on a worker outside the GTK thread. The daemon coalesces state changes into payload-free `org.atelier.Notifications1.Changed` signals on its existing 200 ms maintenance loop; notification revisions remain separate from this invalidation counter. The shell subscribes to change and bus-name ownership signals, bounds its pending requests and re-reads the authoritative snapshot. A three-second fallback refresh covers older services, initial subscription races and missed signals. No claim of lossless delivery of every intermediate notification state is made. GTK still checks its local reply queue on a short timer to handle animation and UI state. The CLI remains unchanged for scripts. Actual latency and idle resource consumption require measurement on a running desktop.

## Command execution tracking

Use an explicit task label and separate the executable from notification options with `--`:

```sh
atelier-notifications run "Build Atelier" -- cargo build --release
atelier-notifications run "Create backup" -- tar -cf backup.tar documents/
```

The wrapper runs the executable directly with original argument bytes, working directory, environment and inherited standard streams. It does not insert a shell, capture output, escalate privileges or publish arguments. Notifications contain the supplied label, elapsed whole seconds and outcome/exit code. Avoid secrets in labels. The child runs even if notification delivery fails. Normal exit codes are preserved; a child terminated by a signal maps to 128 plus signal, missing executables to 127 and other execution errors to 126. Both notification calls target the same unique bus owner. After completion the wrapper waits at most two seconds for best-effort notification delivery. The running card expires after 60 seconds to avoid persistent orphans; long jobs can still receive a completion card. No percentage is invented.

The wrapper is not a process supervisor and does not install custom signal handlers. If the wrapper itself is killed, a completion notice is not guaranteed. Existing suppression and history rules apply. No automatic command hooks or AI integrations.

## Monitor placement and responsive geometry

Atelier Settings → Notifications now exposes a display selector. Choose Follow panel (the default) or an exact GDK output connector, such as DP-2. The choice is saved with the existing appearance transaction and reloads live. A disconnected preference remains saved and falls back to the panel output, then the first available display. The shell checks for output changes approximately once per second and restores the selected connector when it returns. The settings display list can be refreshed by reloading saved appearance.

Only one popup presenter is routed to the selected output: notifications and actions are not duplicated across monitors. Width and available stack height are calculated using logical monitor geometry, and tall stacks use vertical scrolling. Text and action labels wrap at word or character boundaries. These are preferred geometry bounds, not a hard guarantee against toolkit minimum sizes or compositor-specific constraints. Live output remapping, scaling, panel placement and hotplug behavior still need acceptance tests on real multi-monitor Wayland sessions.

## Optional session privacy guard

The Notifications appearance page offers **Hide popups unless this logind session reports active and unlocked**. It is off by default to preserve compatibility with environments that do not report lock hints. When enabled, the shell probes the system bus for its own process's session using GetSessionByPID and reads Active and LockedHint. It never guesses another session or modifies global DND. Unknown, inactive, locked, malformed or stale (two-second) observations suppress this presenter's popups. Calls run outside GTK; the probe waits about 500 ms between samples. On suppression the presenter hides immediately, discards visible content and rejects snapshot requests started before resume. The first fresh snapshot after resume establishes a new no-replay watermark.

This is an advisory, popup-only measure. Lockers must reliably update logind LockedHint; a missing integration can report false while actually locked. Polling has a detection delay, and a shell launched outside a logind session may suppress all popups. It does not protect inbox content, session-bus access, notification history or compositor rendering. A real security boundary requires the compositor to exclude ordinary shell surfaces while locked. Native lock activation remains separate. Do not rely on this setting for secrets.

## Next phases

- Refine transitions and responsive geometry; verify live placement changes, accessibility and visual behavior on actual desktops.
- Validate the advisory guard and output placement; integrate compositor-enforced lock isolation separately; measure the direct transport's real-world latency and idle resource use.
- Consider a persistent script-friendly client for safe action selection only if needed; validate command tracking in actual use. AI-agent integrations are explicitly out of scope. Do not use notifications for privileged or security-sensitive approvals.
- Improve graphical editor ergonomics and preview; validate the notification centre on real displays. Keep the interface compositor-independent wherever layer-shell is supported.

No new daemon, AI integrations, default shell changes or compositor lock integration are introduced. Do not use the experimental popup surface for sensitive messages on a locked desktop; the advisory popup guard is not a security boundary. CI must verify the changes before merge.
