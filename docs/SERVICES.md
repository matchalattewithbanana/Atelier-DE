# Atelier Services

An independent Rust component for desktop-independent system information. It does not require GTK, Smithay, a desktop session or sudo.

`atelier-services power` prints one JSON snapshot, for example:

```json
{"schema":1,"batteries":[{"name":"BAT0","percent":73,"status":"Charging"}],"external_power":true}
```

A desktop without a battery returns an empty array. Unknown fields are null; an unreadable inventory returns an error rather than falsely reporting no battery. Multiple system batteries are reported separately rather than averaged. Known peripheral batteries are excluded. The implementation uses Linux power-supply sysfs as a read-only baseline and does not claim UPower support or power-control authority.

Consumers can link the Rust library or use the versioned CLI from another language. No autostart or persistent daemon is required for this initial provider. Future providers should remain independent, use standard desktop interfaces where appropriate, and distinguish unavailable data from negative values. Power actions, notifications, audio and network providers are future work.

## Media

The optional MPRIS bridge uses the installed `playerctl` command on the user's D-Bus session; it does not install it or require a specific compositor.

```sh
atelier-services media
atelier-services media-action vlc play-pause
```

Snapshots contain schema 1 and a list of discovered player IDs, playback status, title and artist. Missing properties are null, and an unavailable provider is an error. Queries are best-effort snapshots: a player may change tracks or exit between calls. Actions accept only a discovered, unambiguous player identifier and one of play, pause, play-pause, next or previous. Player availability can change after discovery; playerctl's instance matching still applies. A GUI should run these calls outside its event loop. No metadata is fetched over the network by Atelier.
