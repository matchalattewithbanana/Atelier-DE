# Atelier — Ecosystem vision

**Adopted 24 November 2026.** This is the shared architectural direction for Atelier-DE and Mango Atelier. It describes the intended ecosystem, not a claim that every product or compatibility target is already implemented.

Atelier is a modular Linux desktop ecosystem, not merely a compositor, shell, or collection of dotfiles. Install everything together for a cohesive experience, or use any Atelier product independently without ecosystem lock-in.

## Product architecture

| Product | Responsibility |
| --- | --- |
| Atelier Compositor | Native Rust/Smithay Wayland compositor |
| Atelier Shell | Independent, compositor-compatible desktop shell |
| Atelier Session | Desktop lifecycle and session management |
| Atelier Services | Shared background services |
| Atelier Settings | Unified system settings |
| Atelier Studio | Visual customization and appearance editing |
| Atelier Rice Manager | Discover, install, customize and manage desktop configurations |
| Atelier Manager | Ecosystem installation, updates, diagnostics and recovery |
| Atelier Desktop | Complete, integrated desktop experience |
| Atelier OS | Potential future standalone Linux distribution; deferred |

Product boundaries do not require a separate repository for every component today. Atelier-DE develops the native desktop independently; Mango Atelier continues supporting existing compositor and rice workflows. Share stable interfaces and tooling instead of requiring one desktop implementation to run the other.

## Non-negotiable principles

- No mandatory dependency on MangoWM or any third-party compositor for the native desktop.
- Compositor and shell independence in both directions, including shell-free sessions and third-party shells.
- Individually installable and replaceable products.
- One-command, interactive ecosystem installation.
- Modular updates, backups and recovery.
- Extensive personalization without sacrificing performance.
- Standard Wayland protocols wherever practical; capability detection and optional adapters for compositor-specific features.
- A polished experience accessible to ordinary Linux users.

Compatibility is a tested contract, not a universal guarantee: unsupported capabilities must degrade gracefully, and compatibility claims require validation.

## Accessible installation

The intended entry point is `atelier setup`, with a graphical installer sharing the same backend. Express, Custom and Minimal setup modes should let users:

1. Detect distribution, hardware, dependencies and existing desktop components.
2. Choose one or multiple supported compositors (Atelier, MangoWM, Hyprland, Niri) or retain their current compositor.
3. Choose a shell or rice, including Atelier Native, Classic, SekaiPop, Serpantinum, compatible external shells, custom configurations or no shell.
4. Select optional applications, services and utilities.
5. Review packages, file changes, backups and explicit privilege requirements before applying anything.
6. Install with progress, failure recovery and actionable diagnostics.
7. Choose their first session without removing their existing recovery desktop.

The planned Atelier Manager provides product management, desktop and rice discovery, appearance, sessions, updates, diagnostics and recovery. Prefer independent packages and an optional desktop meta-package, with signed distribution, release channels and component-level updates. Prioritize CachyOS and Arch-based systems, then expand to other distributions after validation.

## Development priorities

1. Stabilize the native compositor.
2. Complete the independent shell, settings and session services.
3. Establish cross-product compatibility.
4. Build the unified installer and ecosystem manager.
5. Expand distribution support and conduct real-hardware testing.
6. Mature Atelier into a complete desktop environment.

## Explicitly deferred: Atelier OS

A standalone Linux distribution and bootable ISO are distant, long-term ambitions, not active deliverables or release blockers. Do not divert current development into distribution building until the desktop ecosystem is mature and independently deployable.

Repository-specific implementation roadmaps and release gates remain authoritative for current status. This document governs the long-term direction of both repositories.
