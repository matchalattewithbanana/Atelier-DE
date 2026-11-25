# Atelier DE — Native desktop

> **Official direction:** [Atelier Ecosystem Vision](VISION.md) — our shared product architecture, independence principles, installation strategy and development priorities.

Atelier DE is an independent Wayland desktop environment being developed in Rust. It is separate from the existing MangoWM-based distribution and uses Smithay directly. This repository is the native desktop, not an extension of a third-party compositor.

**Development status: 0.1.0-alpha.1.** The first implementation targets a nested window in an existing Wayland or X11 session. It is not yet suitable as your login session or as a replacement for your daily compositor.

## Components

| Component | Responsibility |
| --- | --- |
| `atelier-compositor` | Smithay 0.7 Wayland server, nested GLES rendering, output, toplevel placement/focus and seat input |
| `atelier-shell` | Native GTK4 launcher window with installed application discovery, search and settings/terminal shortcuts |
| `atelier-session` | Supervised, non-privileged startup, socket readiness checks, bounded shell crash recovery and child shutdown |
| `atelier-settings` | Native appearance, default terminal and shell-startup preferences |
| `atelier-config` | Versioned XDG settings, validation and atomic persistence; shared by components |

The shell and settings can be developed on any existing desktop. They depend on the shared config crate and standard desktop/Wayland interfaces, not on Smithay or another compositor's IPC.

## Build and try the nested session

Linux, a recent stable Rust toolchain, Cargo, EGL/OpenGL ES, Wayland/XKB libraries and GTK4 development headers are required. No privileged command is run by Atelier.

On Arch/CachyOS, install the development dependencies using your package manager if they are missing (one-time, explicit sudo):

```sh
sudo pacman -S --needed rust base-devel pkgconf wayland libxkbcommon libxkbcommon-x11 mesa gtk4
```

On Debian/Ubuntu, use a current stable Rust toolchain and these development packages:

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 libegl1-mesa-dev libgl1-mesa-dev libgtk-4-dev
```

```sh
cargo build --workspace
cargo test --workspace
./target/debug/atelier-session --nested
```

The supervisor prints the actual Wayland socket name. From a second terminal in the **same user session**, connect an installed Wayland-native application with that name:

```sh
WAYLAND_DISPLAY=wayland-atelier-12345 foot # substitute the printed socket name
```

Alternatively, start only the compositor with an explicit socket name:

```sh
./target/debug/atelier-compositor --nested --socket wayland-atelier-dev
# In another terminal:
WAYLAND_DISPLAY=wayland-atelier-dev ./target/debug/atelier-shell
```

Close the nested host window or interrupt the supervisor to end the test. Use `--no-shell` to test the compositor without GTK clients. Change the default terminal in Settings if `foot` is not installed. The launcher uses installed desktop entries; applications that activate an already running instance over the host D-Bus may open in that host session during development.

## Settings

Preferences are stored under `${XDG_CONFIG_HOME:-$HOME/.config}/atelier-de/settings.toml`. A missing file uses defaults; unreadable or malformed settings are reported and are not silently overwritten. Settings applies changes after the relevant component is restarted.

Choose Floating windows or Grid tiling in Settings (or set `layout = "grid"` in the preferences file). Grid placement is recalculated when windows map/unmap or the nested output resizes. Client size constraints and workspace switching remain future work.

### Nested workspace controls

Four virtual workspaces are available. With the nested window focused:

- `Alt+1` through `Alt+4`: switch workspace.
- `Alt+Shift+1` through `Alt+Shift+4`: move the focused window to another workspace without switching.

Hidden windows are removed from the rendered scene and restored when their workspace is selected. The host desktop may intercept these shortcuts; change its conflicting bindings for nested testing. Workspace persistence, per-output workspace sets and a shell workspace indicator are not implemented yet.


`atelier-shell` and `atelier-settings` also work as standalone GTK windows, so work on them does not have to wait for the compositor's hardware backend.

## Hardware inventory

Before native TTY bring-up, run `cargo run -p atelier-compositor -- --probe-hardware` as your normal user. This lists Linux DRM primary/render nodes, connector status and advertised modes, and input event nodes visible in sysfs. Missing directories or devices are reported without failing the diagnostic. It never opens device nodes or acquires a seat, and does not verify access, KMS or rendering compatibility.

### Experimental native-seat bring-up

On a spare local text login (not from a graphical terminal), the optional `native-seat` feature can check seat ownership and whether libseat can open and release a seat-assigned DRM card. Install the libseat, libudev and libinput development libraries first. On Arch/CachyOS these are supplied by `seatd`, `systemd` and `libinput`; on Debian/Ubuntu use `libseat-dev libudev-dev libinput-dev`.

```sh
cargo run -p atelier-compositor --features native-seat -- --check-seat
```

This explicitly acquires the current seat and briefly opens a DRM card through the seat provider. Do not run it with sudo or inside your active graphical session. It does not set a mode, present a frame or read input events. The default nested build remains unchanged.

### Experimental GPU allocation

After the seat check succeeds on a spare text login, install GBM/DRM development libraries (`libgbm-dev libdrm-dev` on Debian/Ubuntu; `mesa libdrm` on Arch/CachyOS), then run:

```sh
cargo run -p atelier-compositor --features native-gbm -- --check-gbm
```

The check acquires the current seat, opens a seat-assigned DRM card, allocates a small XRGB8888 rendering buffer, exports it as a DMA-BUF and releases every resource. It does not verify EGL import, scanout compatibility, rendering or KMS presentation. Do not run from a graphical session or with sudo.

### Experimental off-screen rendering

On the same spare local text login, the next diagnostic initializes EGL/GLES on a seat-owned DRM device, negotiates renderable XRGB8888 modifiers, allocates a GBM buffer and exports it as a DMA-BUF. It clears the framebuffer, synchronizes and reads all pixels back to verify the expected color.

```sh
cargo run -p atelier-compositor --features native-gbm -- --check-render
```

This is an off-screen diagnostic, not a monitor presentation test. No modesetting, client composition or input dispatch is performed. CI compiles the feature and tests the pixel verifier; hardware execution requires a real local seat.

### KMS output topology

On a spare local TTY, run:

```sh
cargo run -p atelier-compositor --features native-kms -- --check-kms
```

The seat-owned DRM device is queried for cached connected connectors, advertised display modes, encoder-compatible CRTCs and candidate routes. Mode ranking prefers a display's advertised preferred mode, then resolution and refresh rate. No modesetting or atomic commit is performed; a candidate route does not establish scanout compatibility or exclusive ownership. Continue with real hardware validation before enabling monitor presentation.

### Atomic scanout validation

From a spare local TTY, run:

```sh
cargo run -p atelier-compositor --features native-kms -- --check-atomic
```

This selects one connector, compatible CRTC and XRGB8888 primary plane, allocates a bounded linear GBM scanout buffer, registers a temporary framebuffer and mode blob, then submits an atomic **TEST_ONLY** request. The framebuffer and blob are released on all normal result paths. No physical monitor mode is changed. Driver rejection is an explicit diagnostic; retries, modifier negotiation, multiple outputs, display restoration and real presentation are later stages.

### Experimental physical presentation (not production-ready)

Only after running the earlier hardware diagnostics successfully on a spare local text login, an explicit preview can attempt one synchronous atomic framebuffer swap:

```sh
cargo run -p atelier-compositor --features native-kms -- --present-test
```

This refuses inactive, scaled or mirrored routes, validates the swap and original framebuffer with TEST_ONLY, renders a static frame using EGL/GLES and attempts to restore the original framebuffer after one second or on Ctrl-C/SIGTERM. It never allows a modeset. A seat loss, driver failure or forced process termination can prevent restoration; use a spare TTY and be prepared to switch VTs if necessary. Do not use on a daily desktop or with sudo. Continuous page flips, inactive output activation and production display restoration remain pending.

### Experimental native input

On a spare local text login, use `cargo run -p atelier-compositor --features native-input -- --check-input` to enumerate accessible libinput devices through libseat. The diagnostic discards raw input contents; the common input router supports absolute/relative pointer motion, buttons, scrolling, keyboard focus and workspace shortcuts. Wiring the libinput event source into a continuous native compositor session remains pending.

### Experimental continuous native session

On a spare local TTY only, after all diagnostics have passed, build with `native-session` and use `--native`. The implementation accepts Wayland clients, composites them with GLES, alternates two GBM scanout framebuffers using nonblocking atomic swaps with page-flip completion, and routes libinput events through the common input router. It requires one already-active unscaled non-mirrored output and exits on seat loss. On ordinary shutdown it attempts original framebuffer restoration. Driver failures, forced exits, and seat loss can prevent recovery. Idle sessions skip frame submission until client/input invalidation; a simple software cursor is drawn. No inactive-output activation, multi-monitor, hotplug, client cursor surfaces, region-level damage tracking or production login integration is provided. This is untested on physical hardware.

```sh
cargo run -p atelier-compositor --features native-session -- --native --socket wayland-atelier-native
```

Launch a native Wayland client with that socket from another terminal in the same user's session. Do not run as root or in your daily graphical session.

### Experimental layer-shell

Both compositors expose `zwlr_layer_shell_v1`, arrange layer surfaces with anchor/margin/exclusive-zone information, render background/bottom/top/overlay layers around ordinary windows, and deliver frame callbacks. Grid tiling uses the non-exclusive output region. Layer surfaces may receive keyboard focus on click when requested; automatic exclusive keyboard grabs and secure session locking are not implemented. The native GTK shell remains an ordinary application until its layer-shell client integration lands.

### Optional Atelier desktop panel

Build `atelier-shell` with `--features layer-panel` (install GTK4 layer-shell library, `gtk4-layer-shell` on Arch/CachyOS). In a supported Wayland session the shell opens an anchored top panel with exclusive space, searchable launcher access, workspace selection, clock, battery and terminal/settings shortcuts. Without the feature or layer-shell protocol it opens the normal launcher. Workspace control is local to Atelier's own display via a user-only command socket; no cross-compositor automation is used.

### Compatibility with other Atelier products

The optional panel can run on Atelier DE, MangoWM, Hyprland or Niri. Native sessions use their display-specific workspace socket. For other Atelier products install the existing Mango Atelier `atelier-workspace` helper; the panel reuses its `status` and `focus` commands, recognizes up to nine external workspace numbers, and highlights active workspaces reported across monitors. Conflicting compositor signals or missing helper commands disable external controls instead of guessing. The Rices window calls the existing `atelier rice list` command without changing rice selections. Classic, SekaiPop, Serpantinum and registered custom products retain their existing compatibility limits; the panel does not install or change their configuration. External desktop launches default to the ordinary launcher. Use `atelier-shell --panel` to explicitly opt in to a panel, or `atelier-shell --launcher` for the application launcher on any product. Disable an overlapping bar yourself if desired. Native settings remain separate from rice preferences.

## Verification

```sh
cargo check --workspace --all-targets
cargo test --workspace
```

For a protocol/socket smoke test in a disposable nested X11 session, install `xorg-server-xvfb`/`xvfb`, software Mesa and `wayland-utils`, then run `xvfb-run -a bash scripts/nested-smoke.sh` after building. This does not alter the host desktop or install a login session.

## Roadmap

Read [the native architecture and milestone gates](docs/architecture.md). Direct DRM/KMS, libseat, real panel surfaces, secure screen locking, workspaces/tiling, output management, native settings live updates and production session services are separate upcoming milestones. No support for these is implied by the current alpha.

### Native shell appearance

Open `atelier-settings --page shell-appearance` for a separate native appearance profile at `${XDG_CONFIG_HOME:-$HOME/.config}/atelier-de/appearance.toml`. Customize the shell highlight accent, panel minimum height/spacing, button radius, launcher icon size and date/battery/media/notifications/audio/terminal panel visibility. Save explicitly; a previous profile is retained and can be loaded into a draft and saved to restore. The shared configuration crate validates values and rechecks exact saved bytes under an advisory lock. Existing `settings.toml` and Classic rice files remain independent. Restart the native shell to apply; actual panel height may exceed the minimum to fit widgets. Native appearance does not change compositor wallpaper or window borders. Uses a recent stable Rust toolchain (standard-library file locking requires Rust 1.89+).

### Optional native application dock

Enable the bottom application dock in `atelier-settings --page shell-appearance`, choose installed desktop entries or edit the ordered case-sensitive `.desktop` IDs, and save. Up to 32 pins and 16–64 px icons are supported. The dock appears in layer-panel mode after saving the preference; it is disabled by default and never auto-starts on an external desktop without explicit panel mode. It launches through GIO, preserves unavailable pins as disabled placeholders, and shares the native appearance review/recovery file. With a compatible native compositor it also displays running windows and supports focus/cycling; drag-and-drop remains unimplemented. The persistent dock reserves space; auto-hide mode does not. physical output sizing and layer-shell behavior require real testing. Older builds of the new native appearance editor may reject profiles with the added dock fields; rebuild shell and Settings together.

### Native task inventory (experimental local interface)

The compositor publishes an atomic bounded JSON task inventory at `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY.tasks` alongside the existing workspace datagram socket. It includes mapped xdg-toplevel windows across four workspaces, compositor-assigned session-local IDs, client-supplied (untrusted) title/app ID, workspace and focus state. Metadata scans are limited to five per second and unchanged snapshots are not rewritten. A fixed 25-byte activation packet contains a versioned opcode, random session nonce and window ID; stale IDs and other-session requests are ignored. Activation switches to the target workspace and focuses the window only if no input grab or interactive layer focus blocks it. No shell command execution or process-name guessing. This user-local interface shares the existing workspace socket permissions; it is not a security boundary between same-user applications or a standardized Wayland protocol. Inventory is capped at 128 windows with a truncation marker. External compositors require independent protocol adapters. Actual graphical acceptance remains pending.

With a compatible native compositor task inventory, the dock shows individual windows across workspaces, highlights the focused window and marks running pins. Exact case-sensitive app IDs (with optional `.desktop` suffix) associate windows with pins. Clicking a running pin focuses or cycles matching windows; a non-running pin launches through GIO. Individual window buttons use compositor IDs and never infer a process match. Missing inventory leaves pinned launching available, and external desktops remain launch-only. Window metadata is client-supplied; the native channel is not a security boundary between same-user apps. Focus requests are not reported as success until authoritative inventory changes. Physical nested and hardware acceptance are pending.

### Fullscreen and dock visibility

Native xdg-toplevel fullscreen requests now configure the single output size, preserve floating coordinates and exclude fullscreen windows from normal grid tiling. Unfullscreen restores the saved location and ordinary layout. Task inventory v2 exposes whether the focused window is fullscreen; new clients also accept v1 without fullscreen information. Older strict v1 clients fall back to launch-only on v2; update compositor and shell together. Native shell appearance offers optional auto-hide (bottom-center reveal strip, no exclusive reservation) and hiding the dock on focused native fullscreen, enabled by default. Missing or v1 task data does not guess fullscreen; external desktops support auto-hide but not native fullscreen detection. The top panel and other overlays are not suppressed by this milestone, and multi-output fullscreen, client acknowledgement timing and physical layer remapping need graphical acceptance tests.

### Native wallpaper

Open `atelier-settings --page wallpaper` to select a local PNG, JPEG or WebP, preview it, choose fill/crop, fit or stretch, and explicitly save. Clearing the path disables the wallpaper. The native shell uses a noninteractive all-edge Background layer with no exclusive reservation, independent of Mango Atelier, and retains the previous successful wallpaper on decode failure, or the compositor solid color if none was loaded. Decoding runs in a worker, with a 32 MiB file limit, 8192-pixel dimension limit, 24 megapixel cap and at most 4096 pixels per decoded texture edge. Images are referenced rather than copied; original file changes take effect on shell restart. Wallpaper uses the existing native appearance review and previous-profile recovery; reload the shell appearance editor after a wallpaper save to avoid stale edits. Requires GTK 4.8+ for image fitting. Native single-output panel mode only; no per-monitor selection, EXIF orientation correction, or external compositor wallpaper takeover yet. Physical rendering/input acceptance remains pending.

### Native desktop widgets

Open `atelier-settings --page widgets` to enable a single desktop widget group, choose top-left or top-right placement, set its logical width (220–360 px), and select clock/date, calendar, media and system cards. The group uses a bottom layer, does not reserve workspace area and stays below ordinary windows. Clock/calendar are local; media and system summaries refresh on bounded-lifecycle background workers at 15 and 5 second intervals respectively (service latency can lengthen intervals). Media opens the existing multi-player control window. System shows memory based on MemAvailable, one-minute load average (not CPU percent), and existing power information. Widgets default disabled and start only with native panel mode. Preferences save through the same reviewed appearance transaction, preserve unrelated fields and support restoring the previous widget draft. Reload other appearance pages after saving; the updated native shell applies changes live. Fixed card order and group-level placement only; no drag-and-drop, independent positioning, external plugin execution or external desktop takeover. Graphical acceptance remains pending.

### Native window layout

`atelier-settings --page windows` edits inner (0–48) and outer (0–64) logical-pixel gaps in the native grid layout. Defaults are zero for compatibility; tiny outputs reduce gaps to retain positive tile dimensions. Grid uses the layer-shell non-exclusive zone. Maximize fills that zone and excludes the window from grid tiling; fullscreen takes precedence and uses the entire output. Leaving fullscreen restores maximized state if requested, otherwise normal layout. Normal floating position is retained across workspace changes. Save through the reviewed appearance profile; running updated compositors apply gaps live, after active input grabs finish. Window border rendering, rounded client clipping, shadows, size constraints and multi-output placement remain separate work. Graphical maximize/fullscreen lifecycle acceptance is pending.

### Live native appearance (first milestone)

The shared appearance observer checks the bounded local profile at most twice per second, compares exact bytes, validates changes and reports persistent errors once. Both nested and native compositor backends apply changed grid gaps through their existing common control loop, deferring during pointer/keyboard grabs and refreshing pointer focus after reflow. The GTK shell updates its existing stylesheet provider for accent and radius, panel spacing/minimum height, clock date and panel button/battery visibility without rebuilding windows. Newly opened launchers read saved icon sizes. Invalid or removed profiles retain the last valid running state; restoring valid settings resumes updates. Dock, wallpaper and widget changes also apply live as described below. General settings.toml theme and floating/grid mode still require their existing restart paths. Image and profile reads use no-follow regular-file validation; same-user concurrent manual writes are not an authorization boundary. Visual and input acceptance remains pending.

### Native window outlines

The Window appearance page supports 0–8 logical-pixel solid outlines with separate focused/unfocused hex colors. Defaults are disabled. A common GLES scene builder renders upper layers, front-to-back windows with their own outline strips, then lower layers. Borders do not receive input, are clipped to the usable output region, and are suppressed for maximized/fullscreen windows. Grid tiles reserve outline space where possible; small tiles reduce the inset, while client size constraints are not yet enforced. Live appearance updates redraw colors and reflow changed thickness after input grabs finish. No rounded client clipping, shadows or server titlebars. The new scene composition order, transparency, overlapping clients, and layer positioning require graphical acceptance on nested and real outputs.

### Live auxiliary desktop layers

The native panel owns a single auxiliary layer controller. Saved dock structural changes replace only the dock and its reveal strip, explicitly remove the old polling sources and exclusive reservation, and leave applications, panel and compositor running. Native widget changes replace only the widget group; old refresh workers stop after their in-progress service call returns. Color-only changes do not recreate either surface. Wallpaper changes use one bounded decoder at a time, ignore superseded paths, and update the existing surface in place after successful decoding; a rejected replacement retains the previous image, and clearing the preference removes the background surface. Fit changes apply directly. Failed paths are not repeatedly decoded; change the path or clear and reselect to retry. Same-path edits to the original image are not watched. Native detection is fixed at panel startup; external panel mode supports live dock settings but does not claim external wallpapers or widgets. Existing launcher windows still need reopening for changed icon sizes, general theme/layout remain restart-based. Real graphical lifecycle acceptance is pending.

### Native portable themes

`atelier-settings --page themes` captures the native visual preferences in a versioned `atelier-native-theme` JSON draft. Generate dark/light schemes from bounded local wallpaper samples, validate and preview swatches, import existing JSON or export to a new absolute `.json` path, and explicitly apply through the reviewed appearance transaction. Files contain accent, optional shell surface/text palette, panel/dock/launcher geometry, window gaps and outline preferences; never wallpaper paths, application pins, widget selection or executable commands. Palette text and muted colors require 4.5:1 contrast against both surfaces. Generated themes preserve geometry and outline width. Themes affect native shell surfaces and compositor outlines, not arbitrary third-party GTK applications or global light/dark preference. Missing palette restores GTK default shell surfaces. No overwrite on export; invalid/stale drafts cannot apply. Previous visual preferences can be loaded from the shared previous-profile backup. Reload other Settings pages after applying. Existing launcher icon sizes require reopening. Runtime graphical contrast and CSS acceptance remain pending.

### Graphical native theme studio

The native Themes page now edits the portable format through named color pickers, optional custom accent and shell palette, and bounded geometry spin controls. A local schematic reflects valid draft colors, panel height/spacing, dock icon size, window gaps and borders; default GTK colors are illustrative and the preview is not a live compositor capture. Invalid drafts show their validation error and cannot be applied or exported. Each edit invalidates explicit review; applying compares the exact reviewed typed theme and the existing appearance transaction rejects stale disk preferences. Wallpaper generation keeps unsaved geometry and does not change the live wallpaper. File choosers select local image and JSON paths; imports populate a draft and export never overwrites a destination. A read-only validated JSON inspector remains available. Reload, defaults and previous visual preferences do not mutate the current desktop. The portable theme schema and its exclusion of machine-local preferences are unchanged. Existing launchers require reopening for icon-size changes. Graphical usability and rendering acceptance remain pending.

### Native wallpaper library and slideshow

The native Wallpaper page browses one local folder at a time (non-recursive, up to 4096 entries and 1000 supported image candidates), selects images by filename, previews through the bounded decoder, and edits an ordered playlist of 2–32 unique absolute paths. A folder scan never follows final symlinks; candidates are not presumed decodable. The playlist interval is 1–1440 minutes. Save validates each file's regular-file/size boundary and decodes the first slide; remaining slides are decoded only when selected by the runtime. Static mode uses the separate image path, and solid-background mode disables the playlist. Previous-profile recovery restores wallpaper and slideshow together, without changing other appearance settings. Native panel rotation starts with the first slide, advances in order, wraps and defers while a decode is pending; malformed later slides retain the last successful image until the next interval. Rotation restarts on playlist edits or shell restart and never writes configuration per transition. No image copying, online discovery, shuffle, independent monitor playlists, thumbnail grid or persisted playback position. Portable themes still exclude all local wallpaper preferences. Graphical acceptance remains pending.

### Native ambient window shadows

Optional shadow radius (0–32 logical pixels) and opacity (0–60%) are available in Window appearance and the graphical native theme editor. The compositor draws a bounded set of up to eight disjoint translucent rectangular rings with a quadratic fade, outside each ordinary window's outline, clipped to the usable output. Both GLES backends use the same scene, and fullscreen/maximized windows receive no shadows. Changes redraw live without altering window geometry or reserving space. Shadows default disabled; old portable themes lacking the optional shadow field use that default. No shader blur, rounded client clipping, animation or backdrop blur is claimed. Graphical acceptance and GPU performance measurement remain outstanding.

### Native transparency and wallpaper softening

Appearance Studio offers focused/inactive window opacity (70–100%) and panel/dock/widget background opacity (65–100%). All default to 100%. Window surfaces use Smithay render-element alpha, preserving input and geometry; maximized/fullscreen windows remain opaque. Subsurfaces and popups rendered with their parent inherit the alpha individually, not as an offscreen composited group. Shell background opacity is scoped to the Atelier layer windows, leaving foreground widgets opaque; actual contrast depends on the content behind translucent surfaces. Portable themes accept an optional transparency field; old themes default to opaque. The Blue glass preset is draft-only and uses a blue generated palette and modest transparency. Wallpaper Settings adds a separate 0–24 CPU image blur strength for static images and slideshow frames, with a bounded 1536-pixel working edge when enabled. The single decode worker keys results by image and blur strength and discards obsolete results. This is real image softening, not live backdrop blur or rounded window clipping. Preview and runtime graphics require acceptance testing; no GPU performance claim.
