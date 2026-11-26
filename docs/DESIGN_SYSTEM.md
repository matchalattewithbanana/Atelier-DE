# Atelier Classic design foundation

Material 3 Expressive is the primary visual direction for Classic, informed by Caelestia's desktop adaptations. It is not a requirement for every rice or a new toolkit dependency.

## Shared design module

`atelier_config::design` is toolkit-independent. GTK, native lock/greeter presentation, and later components may consume the same vocabulary without depending on one another.

From Material 3: the corner scale (0, 4, 8, 12, 16, 20, 28, 32, 48 and full), optical nested corner subtraction, all 15 baseline and 15 emphasized typography styles, and the expressive/standard fast/default/slow spatial and effects spring parameters from the Compose token references. See [Material 3 shape](https://m3.material.io/styles/shape/overview), [typography](https://m3.material.io/styles/typography/overview), [motion](https://m3.material.io/styles/motion/overview) and [color](https://m3.material.io/styles/color/overview).

Atelier desktop adaptations: logical GTK units; bounding radii to half the shorter edge; the normalized analytic spring response from rest, reduced motion snapping immediately to the end, and a four-second numerical completion cap. These are local implementation decisions, not official desktop Material specifications. The sampler does not retain velocity across interruptions. Effects are critically damped and bounded; spatial motion may overshoot. Security transitions must never be controlled by visual animation completion.

The palette adapter preserves the user's existing surface, text and accent input. It produces explicit readable fill/foreground pairs, including primary, secondary, tertiary, error and surface roles. Its secondary/tertiary RGB blends and two tinted container strengths are Atelier-specific, not official HCT tonal palette generation or a complete M3 color scheme. Every pair targets at least 4.5:1 contrast; the resource renderer additionally checks the shared foreground against both fill levels. Error is a fixed semantic color selected for light/dark surfaces, not derived from the wallpaper. The existing shell accent foreground calculation now uses the same color utility; no custom rice settings are overwritten.

## Integration and roadmap

The resource meters use the shared role pairs rather than an independent color algorithm and use the headline typography token as a relative design reference. The existing renderer is Cairo with static sample data; it is not a full Material component implementation. No live widgets or spring animations are activated by this change.

Next: official HCT-based scheme generation with contrast options; reusable vector shape paths and morphing; frame-clock driven interruptible animations; gradual migration of quick settings, launcher, dock, notifications and settings to semantic tokens with visual acceptance testing on light/dark and compact/wide layouts. Preserve existing portable theme compatibility and explicit user geometry. Recovery authentication remains independent of customizable appearance. No Android, Qt or Quickshell dependency is introduced.
