# Atelier Services

An independent Rust component for desktop-independent system information. It does not require GTK, Smithay, a desktop session or sudo.

`atelier-services power` prints one JSON snapshot, for example:

```json
{"schema":1,"batteries":[{"name":"BAT0","percent":73,"status":"Charging"}],"external_power":true}
```

A desktop without a battery returns an empty array. Unknown fields are null; an unreadable inventory returns an error rather than falsely reporting no battery. Multiple system batteries are reported separately rather than averaged. Known peripheral batteries are excluded. The implementation uses Linux power-supply sysfs as a read-only baseline and does not claim UPower support or power-control authority.

Consumers can link the Rust library or use the versioned CLI from another language. No autostart or persistent daemon is required for this initial provider. Future providers should remain independent, use standard desktop interfaces where appropriate, and distinguish unavailable data from negative values. Power actions, notifications and network providers are future work.

## Media

The optional MPRIS bridge uses the installed `playerctl` command on the user's D-Bus session; it does not install it or require a specific compositor.

```sh
atelier-services media
atelier-services media-action vlc play-pause
```

Snapshots contain schema 1 and a list of discovered player IDs, playback status, title and artist. Missing properties are null, and an unavailable provider is an error. Queries are best-effort snapshots: a player may change tracks or exit between calls. Actions accept only a discovered, unambiguous player identifier and one of play, pause, play-pause, next or previous. Player availability can change after discovery; playerctl's instance matching still applies. A GUI should run these calls outside its event loop. No metadata is fetched over the network by Atelier.


## Audio

The optional provider uses a recent `pactl` with JSON output and coreutils `timeout`. It works with the user's PulseAudio server or PipeWire's PulseAudio-compatible server; no sudo or compositor integration is needed.

```sh
atelier-services audio
atelier-services audio-action output DEVICE volume 60
atelier-services audio-action input DEVICE mute on
atelier-services audio-action output DEVICE default
```

The snapshot includes named inputs and outputs, the current default IDs when available, channel volume percentages and mute state. Conventional monitor sources are marked separately. Unknown values are null; invalid inventory is an error. Existing amplified volumes can be displayed, but Atelier's absolute volume commands are limited to 0–100% and apply to all channels. Controls validate a fresh device inventory and accept only named devices and explicit actions. Default changes are delegated to the audio server; existing streams are not explicitly moved. A UI must run these bounded subprocess calls outside its event thread.
