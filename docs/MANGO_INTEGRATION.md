# Mango Atelier integration

The native manager can hand off to a trusted local Mango Atelier checkout with the version-1 setup contract (available in the `atelier/mango-release-candidate` branch).

```sh
atelier-manager mango-setup --from ~/Projects/mango-atelier --plan
atelier-manager mango-setup --from ~/Projects/mango-atelier
atelier-manager mango-setup --from ~/Projects/mango-atelier --rollback
```

The command explicitly confirms execution of the checkout, checks the exact setup API marker, and delegates to Mango Atelier's interactive wizard. The original wizard remains the authority for compositor selection, separate rice choices, package groups, sudo approval and configuration recovery. The manager does not pass unattended approval flags, fetch code, infer a trusted source from a URL, or modify native deployment selection. The marker proves compatibility only; running the checkout requires trust in its code, including preview mode. Keep the checkout in its original location because configuration links depend on it.

The same handoff is available from `atelier-manager setup --install`. The menu delegates to this backend rather than automatically applying the native wizard's preview. Native preview choices are not transferred; users choose them in the delegated wizard. Mango Atelier recovery restores its managed user files, not system packages or upgrades. Actual CachyOS/Wayland testing remains necessary.

## Graphical Rice Manager integration

The native Manager’s Rices page takes an explicit trusted local checkout path and compositor target. It delegates to the existing `scripts/rice_manager.py` interface after the version-1 setup handshake. The handshake is a compatibility signal, not a signature or proof of code safety. No code is executed until the user checks trust and requests an action. The local catalog, online Rice Cooker discovery and snapshot listing are shown through bounded responses in background workers. Community entries are informational only; no community metadata becomes executable arguments. Discovery may refresh the existing community cache. Only the curated SekaiPop profile can be prepared via this UI; its existing pinned-source installer owns validation. Ready local profiles can be selected for next login with an exact typed confirmation and an explicit compositor. Live compositor detection markers are cleared for these offline-targeted operations; no live preview, switch or snapshot mutation is exposed yet. No native deployment or package transaction is performed. The existing Mango CLI and Rice Picker remain authoritative for advanced appearance, recovery and live shell operations. Real compositor acceptance is outstanding.

## Graphical rice backups

Rice backups adds listing, manifest inspection, named creation, import, export, rename, offline restore and deletion. The existing Mango snapshot engine owns archive format, scope, checksum validation, private ZIP export, recovery snapshots and restoration rollback. Import stores a validated archive without applying settings. Export refuses an existing destination. Mutating operations require typed confirmation, and editing any input clears it. Restore is deliberately offline-only: the native manager refuses recognized live MangoWM, Hyprland or Niri environment markers/session names and never passes `--allow-active`. Close every Mango Atelier session and preview (including other logins) before confirming from a separate desktop. Environment checks cannot prove all other processes are stopped. Snapshot IDs are validated, not pinned to an immutable archive identity between separate inspection and execution; avoid concurrent edits of the same snapshot. The native Manager does not copy or interpret snapshot file contents itself, change native deployments or run sudo. Live backup acceptance remains outstanding.

## Live rice previews

The Live rice preview page connects directly to the user-only running Mango Atelier supervisor (requires [Mango Atelier PR #17](https://github.com/matchalattewithbanana/mango-atelier/pull/17) and a new login after updating). It does not execute a checkout, use an offline target override, or silently select a saved profile. A single compositor instance marker, matching Wayland display, private runtime directory/socket and same-UID peer are required. The versioned supervisor contract verifies display/instance again. Start a prepared rice with typed approval; the supervisor owns the existing 30-second preview, panel, fallback and expiry. Status shows time remaining at the last refresh, not a continuous countdown. Apply/revert use the exact preview token from that response, so a stale action cannot affect a replacement preview. Errors require refreshing; no automatic retry. Older supervisors or missing live markers fail closed. No sudo or native deployment changes. Real compositor acceptance remains outstanding.
