# Atelier Classic design foundation

Material 3 Expressive is the primary visual direction for Classic, informed by Caelestia's desktop adaptations. It is not a requirement for every rice or a new toolkit dependency.

## Shared design module

`atelier_config::design` is toolkit-independent. GTK, native lock/greeter presentation, and later components may consume the same vocabulary without depending on one another.

From Material 3: the corner scale (0, 4, 8, 12, 16, 20, 28, 32, 48 and full), optical nested corner subtraction, all 15 baseline and 15 emphasized typography styles, and the expressive/standard fast/default/slow spatial and effects spring parameters from the Compose token references. See [Material 3 shape](https://m3.material.io/styles/shape/overview), [typography](https://m3.material.io/styles/typography/overview), [motion](https://m3.material.io/styles/motion/overview) and [color](https://m3.material.io/styles/color/overview).

Atelier desktop adaptations: logical GTK units; bounding radii to half the shorter edge; the normalized analytic spring response from rest, reduced motion snapping immediately to the end, and a four-second numerical completion cap. These are local implementation decisions, not official desktop Material specifications. The original stateless sampler does not retain velocity; the retargetable SpringValue controller described below does. Effects are critically damped and bounded; spatial motion may overshoot. Security transitions must never be controlled by visual animation completion.

The palette adapter preserves the user's existing surface, text and accent input. It produces explicit readable fill/foreground pairs, including primary, secondary, tertiary, error and surface roles. Its secondary/tertiary RGB blends and two tinted container strengths are Atelier-specific, not official HCT tonal palette generation or a complete M3 color scheme. Every pair targets at least 4.5:1 contrast; the resource renderer additionally checks the shared foreground against both fill levels. Error is a fixed semantic color selected for light/dark surfaces, not derived from the wallpaper. The existing shell accent foreground calculation now uses the same color utility; no custom rice settings are overwritten.

## Integration and roadmap

The resource meters use the shared role pairs rather than an independent color algorithm and use the headline typography token as a relative design reference. The existing renderer is Cairo with static sample data; it is not a full Material component implementation. No live widgets are activated; the offline resource preview now exercises spring animations.

Next: extend generated HCT semantic roles to all renderers; broader vector shape coverage and frame-clock animation integration; gradual migration of quick settings, launcher, dock, notifications and settings to semantic tokens with visual acceptance testing on light/dark and compact/wide layouts. Preserve existing portable theme compatibility and explicit user geometry. Recovery authentication remains independent of customizable appearance. No Android, Qt or Quickshell dependency is introduced.

## Native HCT palette generation

Atelier now uses the pinned Apache-2.0 [material-color-utilities-rust 0.1.0](https://github.com/SaKongA/material-color-utilities-rust) port for native HCT/CAM16 scheme generation. This is a third-party implementation of Material Color Utilities, not an official Google Rust SDK. The generator supports tonal spot, expressive, vibrant and monochrome variants, light/dark surfaces, normal (4.5:1) and high (7:1) contrast targets for the exported shared foregrounds. It uses the scheme's surface, surface container, primary, on-surface, on-surface-variant and outline roles; when necessary, the legacy two-surface projection chooses a readable black or white foreground. Generation fails rather than exporting an unreadable projection.

The existing desktop wallpaper theme generator now uses tonal spot at normal contrast. Access Studio offers a manual source-color picker or a bounded sample from the loaded static wallpaper, scheme variant, light/dark and contrast selectors, then an explicit Generate button. Only the five editable palette colors change. No wallpaper change, live setting write or authentication action occurs. Export stores resolved colors in the existing theme format, not the seed or variant. Secondary/tertiary scheme colors are available to code, but legacy theme files and the resource meter's existing custom RGB role adapter do not yet persist/use the full generated scheme. No automatic regeneration on wallpaper changes. Source-color selection uses Atelier's existing bounded RGB histogram heuristic, not official Material image quantization/scoring. High contrast applies to generated palette text/accent on both solid surfaces, not every live UI element or later manual edits. Full semantic role persistence and dynamic theme switching remain future work.

```sh
atelier-manager access-theme generate classic '#6750a4' expressive dark normal /absolute/classic.json
atelier-manager access-theme generate classic '#4285f4' tonal-spot light high /absolute/high-contrast.json
```

New tests exercise deterministic generation, four variants, extreme seeds, light/dark and normal/high contrast, compatibility with both existing theme schemas and different output for distinct seeds/variants. The tests require fresh CI. No change to the emergency recovery locker.

## Native shape morphs and frame-clock motion

The toolkit-independent Shape/Outline API supplies five original silhouettes (circle, pentagon, slanted, gem and soft burst) in a common 120-ray representation. Rounded polygon radii and bounded equal-angle interpolation avoid topology changes. These are Atelier paths inspired by the Material shape library, not exact Google paths or all 35 Material shapes. The renderer bounds interpolation even when spatial springs overshoot. Resource meters consume the shared paths rather than retaining their own polygons.

SpringValue advances the analytic damped oscillator with the actual elapsed time, preserves position and velocity when spatial targets change, and resets effects velocity on retarget to keep non-geometric transitions monotonic. It snaps when reduced motion is enabled or after a long frame gap and stops at a small settling tolerance. These are Atelier implementation decisions; the underlying spring parameters come from Material's published Compose tokens. The controller accepts normalized targets and is not a general-purpose arbitrary-coordinate animation engine.

Native access Studio has an explicit offline high-load simulation. CPU, RAM and disk fill levels move between sample values, while the CPU temperature badge morphs from a circle toward a soft burst and switches to the semantic error pair at 90°C. The threshold is a sample visualization decision, not a hardware safety limit. GTK frame-clock callbacks redraw only during an active transition and rapid toggles retarget the same controller. GTK's animation setting and a zero theme motion duration snap to the endpoint; nonzero legacy duration values do not tune the spring. No actual thermal measurements, login events or session commands are used. Tests cover all shape pairs, bounds and angular ordering, spring interruption, monotonic effects and reduced-motion sample behavior. Actual desktop frame timing and visual acceptance still need validation. No authentication decision depends on animation.
