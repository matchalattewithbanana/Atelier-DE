# Native access theme format (v1)

Atelier Lock and Atelier Greeter are intended to share a portable appearance contract while keeping their session and authentication responsibilities independent. This initial contract is implemented in atelier-config::access_theme and the Manager's offline Native access theme Studio. It does not activate a greeter or locker.

Built-in presets: Atelier Classic (centered), SekaiPop (split), Monogram (bento), Industrial (editorial). These are original data-only palette and layout presets; the external rd and Qylock collections retain their own QML formats and are still available in the Manager catalog. No upstream art, media or QML is copied into the native presets.

The JSON contract requires format=atelier-access-theme, version=1, name, layout, typography, palette, corner_radius, clock, date and motion_ms. Palette colors are strict #RRGGBB. Text, muted and accent must meet 4.5:1 contrast against both background and panel. Layout is a fixed enum, mapped to two nonoverlapping normalized rectangles; compact output uses a stacked fallback. The host always owns the authentication card and actual controls. Clock/date are optional. Font families are semantic (sans, monospace, display) rather than paths. Motion is capped at 400 ms and a reduced-motion preference resolves to zero. Unknown fields, invalid colors, unsupported versions and oversized files are rejected. No script, executable, dynamic URL, machine-local path, authentication policy, unlock operation or privilege configuration is part of the format.

Use Manager → Native access theme Studio to load a preset, edit the JSON draft, validate it, compare a compact and normal layout, switch between illustrative login and lock previews, and import/export JSON. Preview uses fixed sample time and text, no actual credentials or session state. Invalid drafts retain the last validated preview. Export is no-clobber; it never updates an active theme. The preview is static: motion is a validated preference, not animated in this first editor.

CLI:
```sh
atelier-manager access-theme list
atelier-manager access-theme show monogram
atelier-manager access-theme export industrial /absolute/industrial.json
atelier-manager access-theme check /absolute/industrial.json
```

Future work: graphical field controls, asset references with bounded local decoding and explicit licensing, safe per-display placement, real layout rendering and accessibility testing, and an independent trusted greeter/ordinary lock runtime. Do not feed these themes into the emergency recovery client. Do not claim authentication or unlock support from a visual preview. Native lock activation stays gated on real compositor protection, input isolation, crash recovery and trusted authentication validation.
