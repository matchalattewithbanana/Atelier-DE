# Atelier Services

An independent Rust component for desktop-independent system information. It does not require GTK, Smithay, a desktop session or sudo.

`atelier-services power` prints one JSON snapshot, for example:

```json
{"schema":1,"batteries":[{"name":"BAT0","percent":73,"status":"Charging"}],"external_power":true}
```

A desktop without a battery returns an empty array. Unknown fields are null; an unreadable inventory returns an error rather than falsely reporting no battery. Multiple system batteries are reported separately rather than averaged. Known peripheral batteries are excluded. The implementation uses Linux power-supply sysfs as a read-only baseline and does not claim UPower support or power-control authority.

Consumers can link the Rust library or use the versioned CLI from another language. No autostart or persistent daemon is required for this initial provider. Future providers should remain independent, use standard desktop interfaces where appropriate, and distinguish unavailable data from negative values. Power actions, notifications, media, audio and network providers are future work.