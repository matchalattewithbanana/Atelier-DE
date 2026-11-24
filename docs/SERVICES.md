# Atelier Services

An independent Rust component for desktop-independent system information. It does not require GTK, Smithay, a desktop session or sudo.

`atelier-services power` prints one JSON snapshot, for example:

```json
{"schema":1,"batteries":[{"name":"BAT0","percent":73,"status":"Charging"}],"external_power":true}
```

A desktop without a battery returns an empty array. Unknown fields are null; an unreadable inventory returns an error rather than falsely reporting no battery. Multiple system batteries are reported separately rather than averaged. Known peripheral batteries are excluded. The implementation uses Linux power-supply sysfs as a read-only baseline and does not claim UPower support or power-control authority.

Consumers can link the Rust library or use the versioned CLI from another language. No autostart or persistent daemon is required for this initial provider. Future providers should remain independent, use standard desktop interfaces where appropriate, and distinguish unavailable data from negative values. Power actions and notifications are future work.

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


## Networking

The optional NetworkManager provider uses `nmcli` and coreutils `timeout`. Devices, saved connections (UUID, name, type), Wi-Fi radio status and access points are represented as schema-1 JSON. Access point inventory reads the existing cache unless an explicit rescan is requested. Terse output escapes are decoded rather than splitting raw colons in SSIDs or MAC addresses.

```sh
atelier-services network
atelier-services wifi
atelier-services wifi --rescan
atelier-services network-action wifi on
atelier-services network-action activate CONNECTION_UUID
atelier-services network-action deactivate CONNECTION_UUID
```

Actions are explicit and affect the current system network; NetworkManager and its authorization policy remain in control. Atelier requests no sudo, never reads or stores network passwords and never modifies or deletes connection profiles. Activation is limited to discovered saved UUIDs and can fail if authentication or authorization is required. New protected network setup and secret-agent integration remain future work. No connectivity probes or external requests are made by the inventory provider.


## Bluetooth

The optional BlueZ provider uses `bluetoothctl` (with the paired-device filter) and coreutils `timeout`. It reports adapters, power state, paired device names and connection status. Unknown state is null. With zero or multiple adapters, power and paired-device state are intentionally not inferred; control commands refuse to run. Adapter-scoped D-Bus support remains future work.

```sh
atelier-services bluetooth
atelier-services bluetooth-action power on
atelier-services bluetooth-action connect AA:BB:CC:DD:EE:FF
atelier-services bluetooth-action disconnect AA:BB:CC:DD:EE:FF
```

Actions require an explicit request and a single adapter, validate device addresses against the paired inventory, and verify the requested state afterwards. No discovery scans, pairing, trust changes, removal or credential handling are performed. BlueZ and system authorization remain in charge. Commands are bounded, but GUI integrations must execute them in a worker. The provider's text parser is intentionally strict and may report unrecognized output as unavailable; real BlueZ acceptance testing is required.
