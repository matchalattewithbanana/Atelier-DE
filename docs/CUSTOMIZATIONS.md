# Customization lifecycle

Atelier should expose a single browse and review experience without a universal installer that treats every theme as inert data. Each component retains its own compatibility and activation contract.

| Category | Prepare | Select / activate | Recovery |
| --- | --- | --- | --- |
| Cursor | Pinned upstream user-local assets, or separately installed package | Native Settings; next session; external adapters later | Existing appearance previous-version backup; keep files |
| Login / SDDM | Review packages; inspect locally installed theme and trusted QML | Future explicit greeter test, root-owned config transaction; never switch display manager automatically | Preserve existing service/config; spare TTY recovery |
| greetd | Review daemon and separately selected greeter | Future reviewed greeter/session configuration, no implicit service change | Preserve previous display-manager configuration |
| Session locker | Install optional package; verify supported protocol and PAM | Future compositor-specific adapter; native blocked pending secure ext-session-lock implementation | Fail closed, never use a cosmetic fullscreen window as a lock |
| Icons and app style | Curated source and package review | Future GTK/Qt and external-desktop adapters | Back up only affected user settings |
| Wallpaper | Existing local preview and native wallpaper workflow | Existing appearance review | Existing appearance backup |
| Shell / rice | Existing deployment or Mango Atelier preparation and compatibility checks | Existing session profiles or Rice Manager | Existing deployment rollback / rice snapshots |

## Rules

- Browse, discover, preview, prepare, activate and recover are separate actions.
- A community listing is never authority to execute installation commands.
- No root operations on catalog refresh or preview. Package installation is explicit and does not enable a login service.
- Native and external compositor targets have distinct capabilities. Installed binaries and files are not proof of compatibility.
- Cursor assets and wallpapers are not equivalent to executable QML or authentication configuration.
- No automatic logout, service restart, authentication changes or screen lock while exploring the catalog.
- Future privileged transactions need an exact change plan, root-owned destinations, previous config backups, ownership checks, and tested restoration before activation.

Current implementation: shared typed category catalog, read-only local detection and source review, CLI JSON, GTK browse/filter and optional package choices. Cursor preparation and native preference selection use #115. Online community discovery for other categories, custom registry, trusted SDDM preview, per-target activation and recovery adapters remain separate future work. Do not represent catalog entries as installed or functional without validation.

## Custom component references

Import a local JSON manifest through Manager → Custom components or `atelier-manager customizations register --from /absolute/manifest.json`. Example:

```json
{
  "schema": 1,
  "id": "my-cursor",
  "name": "My cursor",
  "category": "cursor",
  "source": "https://example.org/cursors",
  "targets": [
    "atelier",
    "hyprland"
  ],
  "description": "My preferred custom cursor pack"
}
```

Allowed categories match the catalog; target IDs are atelier, mango, hyprland, niri and other. Targets are declarations, not proof of compatibility. A custom ID is namespaced as custom:my-cursor and cannot shadow curated entries. The source is only an HTTPS reference or absolute local path, not an executable command. Import, unregister and restore require explicit review; no download, installation or activation occurs. `atelier-manager customizations unregister custom:my-cursor` removes the reference only. `atelier-manager customizations restore-registry` restores the previous metadata registry. The registry lives under the user's config directory, with bounded reads, advisory locking, stale-review checks and a previous-version backup. A registered rice/shell still requires its dedicated trusted workflow.

## Online community discovery

Select one category in the Customization catalog and choose Discover online (GitHub), optionally entering keywords. CLI: `atelier-manager customizations discover cursor "minimal" --json`. Topic search is deliberately bounded to one page (20 repositories) and uses the existing trusted system gh/timeout transport. GitHub CLI authentication/network access may be required. Searches disclose the entered keywords to GitHub; do not enter secrets. Repository references, descriptions, stars, advertised SPDX licenses and update timestamps are metadata only. No image or archive downloads, automatic registry imports, compatibility claims or executable installation instructions are included. Topic coverage is incomplete; for example, the initial locker search uses the hyprlock topic and the application-theme search uses gtk-theme. Use the local registry for sources not found. Community sources must be separately inspected before preparation. The Rice Manager's existing community discovery remains available independently.
