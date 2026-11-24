# Atelier DE — Native desktop

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

Choose Floating windows or Grid tiling in Settings (or set `layout = "grid"` in the preferences file). Grid placement is recalculated when windows map/unmap or the nested output resizes. Client size constraints remain future work.

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

### Experimental native input

On a spare local text login, use `cargo run -p atelier-compositor --features native-input -- --check-input` to enumerate accessible libinput devices through libseat. The diagnostic discards raw input contents; the common input router supports absolute/relative pointer motion, buttons, scrolling, keyboard focus and workspace shortcuts. Wiring the libinput event source into a continuous native compositor session remains pending.

## Verification

```sh
cargo check --workspace --all-targets
cargo test --workspace
```

For a protocol/socket smoke test in a disposable nested X11 session, install `xorg-server-xvfb`/`xvfb`, software Mesa and `wayland-utils`, then run `xvfb-run -a bash scripts/nested-smoke.sh` after building. This does not alter the host desktop or install a login session.

## Roadmap

Read [the native architecture and milestone gates](docs/architecture.md). Direct DRM/KMS, libseat, real panel surfaces, secure screen locking, workspaces/tiling, output management, native settings live updates and production session services are separate upcoming milestones. No support for these is implied by the current alpha.
