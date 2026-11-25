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

## Local cursor visual previews

Manager → Cursor previews displays six installed XCursor shapes (default, link, text, wait, horizontal resize, move), nearest requested nominal size, first frame and hotspot. Custom installed names are accepted. This reads direct local theme assets using the same bounded image decoder as the native compositor. Missing or invalid shapes are identified, not substituted; runtime theme inheritance may supply additional shapes. No download, execution, GTK cursor override or appearance write occurs. SDDM and other visual previews remain separate future work.

## Save a community reference

Online result cards offer Review and save reference. Review the exact metadata and type the requested confirmation; the existing registry transaction retains its previous-version backup and stale-review protection. CLI: `atelier-manager customizations register-source cursor owner/repository`. The stable custom ID is derived from category and case-insensitive GitHub repository name, and duplicate references in the same category are refused. Targets stay unspecified. This saves a reference, not a clone, installation or trust grant. The CLI may register a repository name without fetching or checking its existence.

## SDDM local inspection and preparation

Manager → Login themes accepts an installed theme directory or an absolute local directory. Inspect reads a bounded tree (512 entries, eight directory levels, 64 MiB aggregate, 32 MiB per file), rejects symlinks/special files, checks the declared QML entry point, and shows a static screenshot from `metadata.desktop` or a conventional preview filename using the bounded raster decoder. Missing/invalid screenshots do not imply invalid QML. Inspection lists source files and a SHA-256 fingerprint of paths and content; it is not a security or compatibility audit. Prepare requires typed confirmation, rereads the tree and rejects a changed fingerprint. It copies only reviewed regular files to a new private directory under the user's XDG data directory, `atelier-manager/sddm-staged/theme-*/theme`, with `review.json` beside it. Failed copies are cleaned up by the temporary-directory guard. Each successful preparation is separate; no existing copy is overwritten. CLI: `atelier-manager login-theme inspect --from /absolute/theme` or `atelier-manager login-theme prepare --from /absolute/theme`. No root, remote download, QML execution, actual greeter preview, /etc write, login service activation or system theme installation. Staged files remain untrusted; symlink-dependent themes are intentionally unsupported in this initial adapter. The next step is a separate verified system installation/selection and recovery mechanism, not running arbitrary themes as root.

## Reviewed selection of an already installed SDDM theme

`atelier-manager login-theme selection-plan NAME` is read-only. The administrator-only CLI `login-theme select NAME` requires a separately installed root-owned Manager executable in a protected system path, a terminal, and exact typed confirmation. Do not run a user-writable checkout or deployment with sudo. Atelier does not auto-elevate. Only installed regular-file themes under /usr/share/sddm/themes with root-owned, protected paths are supported; symlinks and non-world-readable files are refused. Ownership is not a security audit: trust the QML and test greeter compatibility independently before selection. The dedicated /etc/sddm.conf.d/99-atelier-theme.conf sets only ThemeDir and Current. Non-comment legacy /etc/sddm.conf or later .conf drop-ins are refused because they can override this selection. The drop-in directory must already exist. Existing foreign content at the managed path is not overwritten. A root-owned 0600 checkpoint precedes the atomic drop-in write. `login-theme rollback-selection` restores the previous managed override or removes it, refuses unexpected current content, and does not modify other files. A previous managed theme must still be installed and pass ownership checks; if not, manually recover the managed override from a spare TTY. No service restart, logout, PAM changes or display-manager switching. The CLI is deliberately separate from user-local staging; installing a staged community theme into system directories is not yet supported. Actual system and greeter acceptance tests remain required. The GUI links the read-only plan workflow rather than spawning privileged binaries.

## Locker readiness diagnostics

Manager → Screen lockers and `atelier-manager locker check [--json]` inspect the current explicit WAYLAND_DISPLAY, using a bounded five-second wayland-info query with system-owned tools. The probe checks ext_session_lock_manager_v1, layer-shell and advertised output count; it also checks known locker executables and the first matching PAM service file in conventional system locations. Selecting swaylock, hyprlock or gtklock in the setup package review includes wayland-utils and coreutils. Missing/unsafe files and failed registry queries are distinguished from successful advertisements. This is not an authentication or lock test. It does not establish compositor correctness, output isolation, PAM validity, actual locker-version compatibility or host protection from a nested session. No locker is launched, no authentication is attempted, no config changes are made. Native secure locking remains blocked on a compositor implementation and physical acceptance. GTKLock also requires layer-shell in this initial conservative check.

Native lock groundwork is tracked separately in [SESSION_LOCK.md](SESSION_LOCK.md). There is no advertised native session-lock protocol yet; locker diagnostics must not imply native lock support.
