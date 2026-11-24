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

Choose Floating windows or Grid tiling in Settings (or set `layout = "grid"` in the preferences file). Grid placement is recalculated when windows map/unmap or the nested output resizes. Client size constraints and workspace switching remain future work.

`atelier-shell` and `atelier-settings` also work as standalone GTK windows, so work on them does not have to wait for the compositor's hardware backend.

## Hardware inventory

Before native TTY bring-up, run `cargo run -p atelier-compositor -- --probe-hardware` as your normal user. This lists Linux DRM primary/render nodes, connector status and advertised modes, and input event nodes visible in sysfs. Missing directories or devices are reported without failing the diagnostic. It never opens device nodes or acquires a seat, and does not verify access, KMS or rendering compatibility.

## Verification

```sh
cargo check --workspace --all-targets
cargo test --workspace
```

For a protocol/socket smoke test in a disposable nested X11 session, install `xorg-server-xvfb`/`xvfb`, software Mesa and `wayland-utils`, then run `xvfb-run -a bash scripts/nested-smoke.sh` after building. This does not alter the host desktop or install a login session.

## Roadmap

Read [the native architecture and milestone gates](docs/architecture.md). Direct DRM/KMS, libseat, real panel surfaces, secure screen locking, workspaces/tiling, output management, native settings live updates and production session services are separate upcoming milestones. No support for these is implied by the current alpha.
