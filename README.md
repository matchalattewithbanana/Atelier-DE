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

### Smooth GPU ambient shadows

The shared scene now attempts a renderer-local GLES pixel shader on first use of nonzero shadows. Four clipped strips per ordinary window replace up to 32 band elements when compilation succeeds. The shader computes continuous quadratic opacity from Euclidean distance to the outline, including rounded shadow corners, using premultiplied output. It does not cover the window interior, sample the framebuffer or allocate an offscreen texture. Shader compilation failure logs once per renderer and retains the tested banded fallback. Existing radius/opacity, stacking, fullscreen/maximize exclusion and input geometry are unchanged. This is smooth procedural shadow rendering, not Gaussian convolution, rounded client clipping or backdrop blur. Runtime shader execution and performance need graphical acceptance.

The layer-panel CI smoke additionally enables a temporary shadow profile, opens an ordinary launcher window under nested software GLES, requires the shader-ready log, and checks that the launcher, panel and session stay alive after rendering. This verifies the software-driver path without asserting pixel-perfect visual output or physical GPU behavior.

### Rounded native client windows

Optional corner radius (0–24 logical pixels, default zero) clips the ordinary root surface tree to disjoint pixel-aligned rounded rectangles. The render wrapper restricts damage to that shape and conservatively advertises no opaque regions, preserving texture coordinates and client transforms. Popups are rendered separately and retain their normal hit regions. Pointer hit-testing traverses front-to-back windows using the same rounded geometry; clipped corners may target the window below. Border geometry follows the same shape, and fullscreen/maximized windows remain unrounded. Both native GLES backends share this implementation. This is pixel-aligned clipping, not shader-antialiased masking; client-drawn shadows outside xdg geometry are clipped when enabled, and ambient compositor shadows still use a rectangular outline. Portable themes default to zero radius when absent. Current output scale is 1; fractional-scale input/render agreement and graphical performance require further testing. No backdrop blur claim.

### Experimental focused-window backdrop blur

Appearance Studio exposes 0–16 logical-pixel blur, disabled by default. Only a translucent focused ordinary window is eligible. The compositor renders the lower scene to one reusable RGBA scratch texture, excluding the focused window and higher layers, samples it through a fixed 13-tap GPU texture shader and masks the replacement to the focused rounded geometry. No CPU readback or recursive blur. The first implementation is limited to scale 1 and at most 4,194,304 output pixels; larger outputs skip the effect. GPU setup or offscreen-render failures disable this path for the renderer lifetime and preserve transparency. The scratch texture is released when no window qualifies. Both native backends share the pipeline. Portable themes include an optional field (absent = off). Nested smoke requires the offscreen pass log and continued session operation. Orientation, visual accuracy, driver compatibility and frame cost still require real acceptance. Dock, panel, widget, inactive-window and high-resolution backdrop blur are not implemented by this milestone.

### Native panel and dock backdrop blur (experimental)

Appearance Studio adds independent 0–16 blur strengths for the panel and dock. The matching shell background must be translucent. The existing focused-window effect is unchanged, and Blue Glass loads modest panel/dock blur in its draft. The shared compositor scene recognizes the native layer namespaces and uses the arranged panel rectangle and rounded dock bounds; duplicate namespaces are limited to the frontmost mapped surface per role. All snapshots are prepared from the original unblurred lower scene, each in a distinct reusable texture, then inserted in reverse scene order. A single texture shader is shared. Maximum three passes and 48 MiB of RGBA scratch texture storage at the existing 4,194,304-pixel output cap, excluding driver overhead and other renderer allocations. Inactive slots are released. Failure retains ordinary transparency; effects default off. Shell namespaces are hints, not authentication. Widget-card blur needs per-card regions and is intentionally excluded; independent inactive-window blur, high-resolution/downsampled blur, exact dock child geometry and visual/performance acceptance remain pending. CI nested software rendering now requests and checks all three blur roles, not pixel-perfect output.

### Downsampled native backdrop snapshots

All three native backdrop roles now render their lower scene directly at half output resolution on each axis, with ceiling dimensions for odd-sized outputs. The shared shader samples the entire smaller texture onto the original full-resolution output; strength remains measured in output logical pixels. The same per-axis scale is applied to rounded surface masks, and the scaled scene conservatively avoids advertising rounded opaque regions. No additional full-resolution scratch texture or CPU readback is introduced. At 1920×1080 each scratch uses 960×540 RGBA pixels (~1.98 MiB), or ~5.93 MiB for three active slots, versus ~23.73 MiB previously. At 3840×2160 the three slots use ~23.73 MiB. Scale-1 outputs up to the existing 4,194,304 scratch-pixel cap (8192 maximum output edge) are eligible, including 4K and 5120×2880; 8K is not. This is a fourfold reduction in scratch pixels for even dimensions, not a measured fourfold GPU speedup. The output shader still samples visible regions at full resolution. Downsampling can soften fine details and the scene is still drawn once per active role; pixel accuracy, high-resolution driver behavior and actual performance need acceptance testing.

### Widget-card backdrop blur (experimental)

The native widget group uses a versioned layer namespace and publishes its visible rounded card input shape, clipped to the scrolling viewport. The standard Wayland input region doubles as a native compositor rendering hint. Transparent spaces between cards are click-through; the non-overlay scrollbar remains interactive and may receive blur. The shell recomputes the shape on scrolling and at 100 ms allocation checks; it does not send unchanged regions. The compositor requires an explicit, bounded (256 rectangles), additive, non-overlapping input region; invalid or absent hints produce no widget blur. The root-relative rectangles are translated and clipped to the mapped output. One scratch texture serves the entire widget group, not one per card. The fourth role adds an independent 0–16 live blur setting, included in portable themes and Blue Glass, off by default and conditional on translucent widget backgrounds. At most four half-resolution passes and 64 MiB scratch storage (about 7.91 MiB at 1080p for all four); other allocations excluded. Tests cover clipped rounded cards, gaps, invalid regions, and four-role nested smoke. Input-region updates may lag layout changes by up to the poll interval; the GTK backend's region persistence and actual visual/input behavior need graphical acceptance. No blur for older widget namespaces or external compositors is claimed.

### Shape-aware GPU window decorations

Rounded ordinary windows now use signed-distance GPU outlines with a softly antialiased outer border edge and shadows that follow the rounded outer border instead of a rectangular box. The shaders shade at most eight disjoint clipped tiles per decoration (four surrounding strips and four corner tiles), excluding the rounded client interior. Square windows retain their previous border path. A failed border/shadow shader compile uses the existing pixel-aligned/banded renderer. No new scratch textures or appearance settings are needed. The root client-content mask and border inner edge are still pixel-aligned, not fully antialiased; graphical seam, shadow and performance acceptance remains outstanding. The tests cover disjoint tiles, clipping and small window radii.

### Per-application native appearance rules

Settings → Application effects (`atelier-settings --page window-rules`) edits a local draft of up to 64 exact, case-sensitive xdg-shell app IDs. A rule can independently override focused/inactive opacity (70–100%), corner radius (0–24), focused backdrop blur (0–16) or disable the global compositor shadow. Unset values inherit global settings; 0 disables radius/blur, 100 makes the window opaque. Fullscreen/maximize exclusions take precedence, blur still requires focused translucency, and existing borders/layout are unchanged. Rule edits and app ID changes redraw without restarting. The ID is a client-supplied appearance hint, not security identity. No regex, path or executable matching. Rules are stored with reviewed appearance preferences, recoverable through the previous profile and deliberately preserved rather than imported/exported by portable themes. Tests cover exact matching, bounds, inheritance and profile/theme persistence. Older builds may reject the new nonempty field. Physical acceptance remains pending.

### Native opening motion and reduced-motion preferences

Settings → Motion & accessibility (`atelier-settings --page motion`) controls an optional 80–400 ms opening fade for ordinary newly mapped windows. The default is off, and reduced motion overrides it without discarding the chosen duration. A monotonic cubic ease-out blends client content and decorations from 55% of their settled opacity to their configured opacity; geometry, focus, input and protocol lifetimes are unchanged. Native frame invalidation continues while opacity changes, including the final settled frame, and stops afterwards; nested rendering advances the same clock. At most 64 opening fades are tracked and unmapping, fullscreen/maximize, workspace changes or motion/opacity/blur edits cancel relevant fades. Focused windows with active backdrop blur do not animate in this milestone, avoiding inconsistent replacement opacity. Motion preferences are local, saved with reviewed appearance, recoverable from the previous profile, and not overwritten by portable themes. Tests cover time progression, final-frame invalidation, caps, cancellation, configuration and reduced motion. Closing/resizing/workspace transitions and physical visual validation remain outstanding. Existing builds may reject a nondefault motion field.

### Antialiased textured client corners

Rounded normal client surface trees now attempt a renderer-local GLES texture shader that evaluates a rounded-rectangle signed distance in output-local coordinates. It applies a roughly one-physical-pixel coverage transition to premultiplied RGBA, preserving buffer transforms and texture sampling. Normal/native and Flipped180/nested framebuffer orientations and downsampled scratch factors are explicitly handled. Shader compilation failure, unsupported transforms and single-pixel solid-color buffers retain the existing pixel-aligned mask. Input hit-testing and backdrop masks remain pixel-aligned; GPU decorations use their own signed-distance shaders; popups and maximized/fullscreen windows are unaffected. No new offscreen allocation or configuration field. The nested smoke checks shader initialization with rounded client windows and continued operation; actual edge appearance, orientation, driver variants and alpha with overlapping subsurfaces require graphical verification.

### Nested idle presentation

The nested backend now shares the native scene's invalidation and animation clock, additionally treating host events, accepted connections and dispatched Wayland requests as conservative redraw triggers. With no such work it skips framebuffer binding, scene construction, backdrop passes and submission until a 250 ms fallback refresh; protocol dispatch and flush continue at the existing polling cadence. A zero-sized host window forces repaint on restoration. Active clients are not subject to the idle refresh interval. Full-frame rendering remains in use and host redraw events can still cause additional submissions; no measured CPU/GPU improvement is claimed until runtime acceptance. The native KMS scheduler is unchanged.

### Optional frame diagnostics

Set `ATELIER_FRAME_METRICS=1` in the compositor process environment to print one aggregate metrics line per five seconds, including idle intervals. Both nested and native sessions report successful submissions per second and mean/max elapsed wall time for preparation (framebuffer binding, scene construction and backdrop snapshots), final rendering, submission and total work. Native rendering includes the existing fence wait and additionally reports observed page-flip completion latency from submission; nested submission is not proof of physical presentation and its render phase may finish asynchronously. These are CPU-observed durations, not GPU timestamps, display FPS guarantees or per-effect timings. No added synchronization, dependencies, per-frame logging or growing sample buffers. Default off. Compare identical idle/active workloads with effects off/on; record GPU/CPU/RSS separately for issue #97.

### Scene allocation reduction

Rounded pointer hit testing evaluates only the relevant scanline, using the same inset formula as pixel-aligned rendering, without allocating a region vector. Root client surfaces share one immutable clipping region per window rather than copying all scanlines per subsurface. The direct corner shader uses four stack-allocated uniforms. Backdrop requests with no visible destination are rejected before allocating or rendering a scratch texture; all visible requests retain the existing full-output half-resolution sampling and shader behavior. Exhaustive small-geometry regression comparisons protect input semantics at fractional and boundary points. No measured speedup is claimed; compare frame diagnostics and resource use under issue #97.

### Region-bounded backdrop snapshots

Each active backdrop still owns its existing full-output half-resolution scratch texture and uses the same projection and 13-tap shader. Instead of clearing and drawing the entire scratch on every pass, compute a single clipped destination bounding box, expand by the maximum blur tap radius and a conservative 16-scratch-pixel guard for filtering and coordinate precision, then clear and draw only that area. Skip snapshot elements whose scaled geometry cannot intersect it. Every required area is freshly painted, including after resize or effect changes; no scene caching, stale content reuse, extra texture, or new dependency. For a 1920×1080 output with a 40-pixel panel at maximum blur strength, the planned damage is 960×44 rather than 960×540 scratch pixels; this is a geometric reduction in repaint area, not measured GPU savings. Texture memory allocation remains unchanged. Odd dimensions, multiple card regions, output edges and empty requests are covered by tests. Existing nested smoke continues to require all four roles. Pixel-level visual equivalence and driver behavior need acceptance testing in #97.

### Compact backdrop scratch textures

Even-sized scale-1 outputs now allocate each backdrop scratch only for the padded damage region, rounded outward to 64-pixel scratch tiles (clamped at output edges). This retains the original half-resolution pixel grid, translates scene geometry and rounded masks to local framebuffer coordinates, and maps the resulting texture back to its original output destination. The blur tap distance remains in output pixels. Odd-sized outputs keep the previous full-size texture and regional damage path. Texture slots remain independent and reused at identical allocation sizes; moving a crop can still change its tile span and cause reallocation. At 1920×1080, a 40-pixel full-width panel with maximum blur uses a 960×64 RGBA allocation (240 KiB) instead of 960×540 (2025 KiB), excluding driver overhead. No new sampling pass or configuration knob. Actual allocation behavior, pixel equivalence and frame cost remain subject to #97 acceptance.

### Guarded native scanout recovery

On native session shutdown or a recoverable error, explicit framebuffer restoration/removal requires an active seat, no outstanding page flip and a revalidated matching active route (connector, CRTC, primary plane, mode and unscaled non-mirrored geometry). Restore the original framebuffer only when the current primary framebuffer belongs to Atelier, including either allocated slot; never overwrite an external framebuffer. Check all planes for references to Atelier buffers before explicitly removing them. Uncertain ownership, route changes, seat loss and failed restoration defer cleanup to descriptor/seat release and report the recovery error, retaining the original session error in diagnostics. This is deliberately fail-closed, not guaranteed recovery or VT resume. Actual DRM recovery and driver behavior still require spare-TTY hardware acceptance (#97).

### Native cursor-only frame damage

Native scanout tracks scene validity and the previous software cursor rectangle independently for each retained GBM buffer. For ungrabbed pointer motion with no other scene invalidation, the next reusable buffer repaints only the bounding union of its own previous cursor and the current cursor, restoring the background before drawing the pointer. Client requests, non-motion input, grabs, appearance/animation changes and other existing scene invalidations mark both slots stale; each is fully repainted on next use. Existing periodic route checks also discard damage baselines without scheduling an idle frame. Scene construction and backdrop snapshots still run; only final clear and drawing damage is reduced. No DRM damage-clip property, region tracking for client commits, hardware cursor or measured speedup is claimed. Tests cover alternating buffers and invalidation during pending flips. Native buffer preservation, visual trails and performance need physical acceptance (#97).

### Persistent native scanout exports

Each of the two retained native GBM allocations now exports its DMA-BUF once during startup, validates renderer binding before framebuffer registration, and retains that same export object throughout presentation. Frames bind the stored export rather than requesting another export on every submission. Allocation ownership, page-flip sequencing, the existing render fence and guarded framebuffer cleanup remain unchanged. This removes repeated export calls and permits stable renderer import-cache identity; actual cache hits and descriptor or frame-time savings depend on the driver and Smithay implementation. No added buffer or scanout capability is claimed. Native binding and cleanup require physical acceptance (#97).

### Experimental native client damage

Set `ATELIER_CLIENT_DAMAGE=1` in the native compositor environment to enable conservative whole-surface damage comparison. Each retained scanout slot stores at most 256 ordinary Wayland surface signatures and its own cursor rectangle after successful submission. With matching surface identities/order, geometry, source, transform and alpha, changed Smithay commit counters repaint their entire visible surface bounds plus old/new cursor bounds. Shader effects, rounded masks, duplicate IDs, changed placement/order, missing baselines and periodic route checks force full repaint; the default path remains unchanged. Scene construction and submission scheduling still run and damage is a bounding rectangle, not exact client buffer subregions. No claimed measured improvement; enable only for spare-TTY acceptance under #97 until driver/pixel behavior is verified.

### Frame repaint coverage diagnostics

With `ATELIER_FRAME_METRICS=1`, five-second reports also include `full_repaints`, `partial_repaints`, `repaint_pixels`, `output_pixels` and `repaint_percent`. Counters include only successful submissions and clip repaint geometry to output bounds. Percentage is the sum of submitted damage rectangle areas divided by the sum of corresponding output areas, not an average of per-frame percentages or actual GPU fragments. Nested rendering currently reports full coverage; native reports the selected cursor/client/full-frame damage. Compare native runs with and without `ATELIER_CLIENT_DAMAGE=1` on identical workloads. No extra GPU queries or changes to damage selection; hardware performance and correctness remain #97 acceptance.

### Experimental client buffer-region damage

The opt-in native client tracker now queries Smithay's element-local physical damage history against the commit counter stored for the reused scanout slot. For normal-transform, integral-source, 1:1 textured surfaces, bounded valid damage rectangles are padded by one pixel, translated and clipped to the output. Their bounding union is combined with the slot's previous and new cursor rectangles. Expired histories use Smithay's whole-buffer fallback; empty, invalid, or more than 64 rectangles and unsupported transforms/scaling/solid-color buffers fall back to whole-surface repaint. Existing full-frame fallback for effects, order or placement changes and missing baselines is retained. No change to default rendering; opt-in still requires #97 visual acceptance and metrics comparisons. This is a bounding rectangle, not disjoint region rendering or measured GPU savings.

### Hardware cursor feasibility probe

For spare-TTY native acceptance, `ATELIER_CURSOR_PROBE=1 atelier-compositor --native` optionally queries unused cursor planes compatible with the active CRTC and ARGB8888. A temporary 64×64 linear GBM allocation is registered, then tested with atomic TEST_ONLY at the origin and a clipped bottom-right position. No cursor plane is activated, no modeset is permitted, and the temporary framebuffer is removed after the tests. Occupied planes are excluded. A rejected probe logs diagnostics and retains the software cursor. This is a deliberately restrictive feasibility check, not hardware cursor support or a guarantee that a driver will accept real presentation; some valid drivers may reject the clipped-size case. Physical testing and plane ownership/restoration for future live cursor commits remain #97 acceptance.
