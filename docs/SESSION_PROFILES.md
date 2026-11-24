# Atelier session profiles

The native compositor can be launched without a bundled shell. `atelier-session` supervises only its own compositor and the explicitly chosen shell. No third-party compositor or Mango Atelier installation is required.

```sh
atelier-session --nested
atelier-session --nested --no-shell
atelier-session --nested --shell-command quickshell --shell-arg -c --shell-arg my-profile
```

The default respects `start_shell` in Atelier Settings. Explicit `--shell-command` overrides that preference and launches the selected executable directly, passing each `--shell-arg` as one literal argument. This does not install or configure that shell. Compatibility depends on the shell's Wayland protocol requirements; unsupported compositor-specific features are not guaranteed.

The selected shell receives the new Wayland socket and Atelier desktop identity, not the host display or known host compositor IPC markers. The supervisor waits for socket readiness, restarts a crashing shell at most three times within a minute, and stops its own children on shutdown. A normally exiting shell is not restarted.

## Experimental direct hardware session

After building the compositor with `--features native-session`, on a spare local text login only:

```sh
atelier-session --native --no-shell
```

The compositor retains its existing seat checks. This is not a production login session, and physical GPU, input and VT recovery validation remain outstanding. Do not run with sudo or replace your working desktop session.

## Saved shell profiles

Create `${XDG_CONFIG_HOME:-$HOME/.config}/atelier-de/session-profiles.toml` yourself:

```toml
version = 1

[profiles.classic]
program = "quickshell"
args = ["-c", "my-profile"]

[profiles.native-panel]
program = "atelier-shell"
args = ["--panel"]
```

Then use `atelier-session --list-profiles` to inspect available profiles or `atelier-session --nested --profile classic` to explicitly select one. Named profiles override the default shell-start preference and cannot be combined with `--no-shell` or `--shell-command`. They are trusted user-authored configuration: a selected profile launches its executable with literal argument strings, without shell expansion. Listing does not execute anything; malformed files are rejected without modification. Do not import or run untrusted profile commands. The default session remains unchanged if a profile is not selected.
