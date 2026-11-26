# Native access theme format (v1)

Atelier Lock and Atelier Greeter are intended to share a portable appearance contract while keeping their session and authentication responsibilities independent. This initial contract is implemented in atelier-config::access_theme and the Manager's offline Native access theme Studio. It does not activate a greeter or locker.

Built-in presets: Atelier Classic (centered), SekaiPop (split), Monogram (bento), Industrial (editorial). These are original data-only palette and layout presets; the external rd and Qylock collections retain their own QML formats and are still available in the Manager catalog. No upstream art, media or QML is copied into the native presets.

The JSON contract requires format=atelier-access-theme, version=1, name, layout, typography, palette, corner_radius, clock, date and motion_ms. Palette colors are strict #RRGGBB. Text, muted and accent must meet 4.5:1 contrast against both background and panel. Layout is a fixed enum, mapped to two nonoverlapping normalized rectangles; compact output uses a stacked fallback. The host always owns the authentication card and actual controls. Clock/date are optional. Font families are semantic (sans, monospace, display) rather than paths. Motion is capped at 400 ms and a reduced-motion preference resolves to zero. Unknown fields, invalid colors, unsupported versions and oversized files are rejected. No script, executable, dynamic URL, machine-local path, authentication policy, unlock operation or privilege configuration is part of the format.

Use Manager → Native access theme Studio to load a preset, edit the draft through graphical layout and typography selectors, five color pickers, radius/motion controls and clock/date toggles, compare a compact and normal layout, switch between illustrative login and lock previews, and import/export JSON. Preview uses fixed sample time and text, no actual credentials or session state. Changes are validated live; invalid drafts retain the last validated preview and disable export. Loading a preset resets the draft; importing JSON populates the same controls. Export revalidates the current form. Export is no-clobber; it never updates an active theme. The preview is static: motion is a validated preference, not animated in this first editor.

CLI:
```sh
atelier-manager access-theme list
atelier-manager access-theme show monogram
atelier-manager access-theme export industrial /absolute/industrial.json
atelier-manager access-theme check /absolute/industrial.json
```

Future work: animated media with explicit licensing, safe per-display placement, real layout rendering and accessibility testing, and an independent trusted greeter/ordinary lock runtime. Do not feed these themes into the emergency recovery client. Do not claim authentication or unlock support from a visual preview. Native lock activation stays gated on real compositor protection, input isolation, crash recovery and trusted authentication validation.

## Backgrounds and portable wallpaper bundles

The optional v1 backdrop field defaults to solid for old JSON themes. Gradient uses the palette background and a second validated #RRGGBB color with horizontal, vertical or diagonal direction. Wallpaper uses cover, contain or stretch and 0–80% dimming. Authentication and clock cards remain opaque; no wallpaper-derived text or arbitrary layout is accepted. Reduced-motion preferences remain independent. New backdrop fields require an updated reader; earlier v1 readers may reject them.

Studio can load an explicitly selected local PNG, JPEG or WebP through the existing bounded decoder, downsampled to at most 2048 pixels per edge. The Cairo surface is constructed once per successful load and reused during edits. Previewing an image neither captures the desktop nor changes session settings. Missing/invalid wallpaper drafts retain the last valid preview and disable export. Preset reset and JSON imports clear the loaded asset. File reads, decoding and bundle encoding run off the GTK thread. The preview is static, including for any supported animated image input; no video or animation playback is implemented.

Solid/gradient drafts export as ordinary .json. Wallpaper drafts export to a new absolute .atelier-access directory with exactly theme.json and wallpaper.png. No original path, image metadata, scripts, additional files or external references are included. Pixels are normalized to PNG, potentially at reduced resolution; original licenses and redistribution rights remain the user's responsibility. Import checks the directory entries, manifest, bounded raster and wallpaper backdrop. The directory is atomically published without replacing an existing path. No system installation or runtime selection occurs.

```sh
atelier-manager access-theme wallpaper-bundle classic /absolute/wallpaper.jpg /absolute/classic.atelier-access
atelier-manager access-theme check /absolute/classic.atelier-access
```

The CLI's wallpaper-bundle command uses cover fit and 25% dimming; the Studio exposes the editable settings. A standalone wallpaper manifest is not a complete theme package. The recovery locker never loads these themes or assets.
