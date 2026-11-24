# Atelier Manager

The independent manager provides local inventory, planning and explicit user-local deployment staging and selection. It has no Smithay, GTK or Mango Atelier dependency. Experimental Arch-family package installation is available through explicitly confirmed commands; login integration is not implemented.

```sh
cargo run -p atelier-manager -- doctor
cargo run -p atelier-manager -- doctor --json
```

The schema-1 JSON report includes distribution ID/family/version, CPU architecture, session type/desktop identity, detected package managers, Atelier products, common external compositor/shell tools and optional service providers. It reads os-release without interpreting shell syntax and checks executable-file presence in absolute PATH directories, plus the manager's sibling directory and user-local bin for desktop products. Missing paths are null. Presence is only an inventory signal: it is not a claim that the executable runs, that a distribution is supported, or that its package repository has the requested product.

The doctor command performs no subprocesses, installation, downloads, sudo, privileged hardware access or file mutations. Paths and session identity are local information; review before sharing a diagnostic report.

Future work: product catalog, dependency/compatibility resolution, interactive selection, explicit package/file change review, installation transactions, recovery, and a graphical frontend using the same engine. A bootable ISO remains outside the current development scope.

## Catalog and preview

```sh
atelier-manager catalog
atelier-manager plan shell settings
atelier-manager plan desktop --json
atelier-manager plan mango-atelier hyprland
```

The catalog separates native Atelier components, the Mango Atelier distribution, external compositors and the native desktop aggregate. Selection expands only explicit dependencies and deduplicates them. A standalone shell/settings plan never silently installs a compositor; a session requires the native compositor. The preview reports detected and missing executable names per product. Missing is not a package transaction or package availability claim. The JSON contract marks `apply_supported: false`; planning performs no installation, sudo, network or file modification. Installer backends and interactive setup will build on this catalog.

## Interactive ecosystem installer

```sh
atelier-manager setup --install
```

This interactive menu offers system dependency installation, native build staging/selection, and [Mango Atelier setup or preview](MANGO_INTEGRATION.md). Choose each operation independently; the menu returns after it completes or is declined, and exits on a backend error. No other backend is started automatically. Native builds and Mango checkouts must already exist locally and be trusted. Enter their actual directory paths, without shell expansion. Package and deployment approvals remain separate; the Mango wizard owns its own selections and confirmations. This is an orchestrating menu, not an all-or-nothing multi-backend transaction or a downloader. Plain `setup` and `setup --json` remain preview-only.

## Guided setup preview

Run `atelier-manager setup` (or `setup --json` to send prompts to stderr and obtain an accepted preview or null on stdout). Choose Express, Custom or Minimal, then select native products or external compositors, per-compositor rice choices where Mango Atelier is selected, a native shell preference, optional service providers, and first session preference. The final review is explicitly non-mutating, including no sudo, package changes, backups or recovery operations. A confirmation accepts only the preview; it does not persist preferences or enable installation. Type q or end input to cancel. Custom rice/profile IDs are planning preferences, not proof of installation or compatibility. With `atelier-manager setup --from BUILD_DIR`, accepting the preview offers a separate native staging review and confirmation, followed on success by an independent deployment-selection confirmation. Mixed or external product selections are rejected before writes. Rice, provider and first-session choices remain preview-only. Use `atelier-manager setup --packages` for an independently confirmed system package transaction including selected optional providers. The accepted preview is not a package transaction approval. Neither setup mode installs rices or changes login sessions.

## Arch-family package preview

```sh
atelier-manager packages desktop
atelier-manager packages shell settings --json
```

On Arch-family systems, missing source-build products expand into conservative package candidates, with deduplicated reasons. The wizard also includes selected optional provider packages. A bounded, read-only `pacman -T` query checks the local dependency database when pacman and coreutils timeout are detected; unexpected results are unknown, not marked installed. No package database refresh, repository query, sudo or installation occurs. Candidates do not guarantee runtime compatibility or repository availability; native hardware remains experimental, and Mango Atelier's own setup preview is authoritative for rice/app groups. Other distributions currently report that package mapping is unavailable. The initial doctor command remains subprocess-free.

## Arch-family package installation (experimental)

Run as your regular desktop user in an interactive terminal:

```sh
atelier-manager install-packages shell settings
```

This separate command recomputes catalog targets, checks missing dependencies with fixed system executables, and requires typing `upgrade and install` before invoking sudo. It uses `pacman -Syu --needed` rather than a standalone database refresh or partial upgrade. **This may upgrade the whole system.** Pacman's own transaction confirmation remains enabled. Repository availability and additional dependency changes are determined by pacman, not by the initial candidate list. No AUR helper is invoked. For guided selection, use `atelier-manager setup --packages`; the separate package review includes explicitly selected optional providers. Rice-specific groups are not applied by either command.

Before invoking sudo, the manager stores the installed-package inventory and requested targets in a new private transaction directory under `${XDG_STATE_HOME:-$HOME/.local/state}/atelier-manager/package-transactions/`. It records the exit outcome and attempts a post-transaction inventory. These files are diagnostic checkpoints, not backups of packages, databases or configuration. Create a separate system snapshot before upgrading if recovery is needed. On failure, inspect pacman's output and use the distribution's recovery process; Atelier does not automatically downgrade or remove system packages. Native binary staging and login changes remain separate.

## Native build staging (experimental)

After building the selected native components from a trusted local checkout, run as your normal desktop user:

```sh
atelier-manager stage --from target/debug shell settings
```

`stage` resolves native product dependencies and verifies each selected artifact before showing an explicit confirmation. It creates a new private deployment under `${XDG_DATA_HOME:-$HOME/.local/share}/atelier-manager/deployments/`, copies only allowlisted native executable filenames, and records sizes and SHA-256 hashes in a completion manifest. The hash detects later corruption; it is not a signature or proof of provenance. Source builds must be trusted. No binaries are run, existing deployments overwritten, system packages installed, login sessions changed or PATH modified. A failed copy removes its own temporary staging directory; an interrupted directory without the completion manifest is not a valid deployment. A complete deployment can be launched directly from its `bin` folder for testing. Package transactions are separate; Mango Atelier uses its own delegated installer. Plain `setup` ends at preview; `setup --from BUILD_DIR` offers explicit native staging after preview acceptance. After successful staging, guided setup offers a separate activation confirmation using the newly created deployment ID. Declining activation leaves the deployment staged and the current selection unchanged. Standalone `stage` never activates.

## Select or roll back staged deployments

```sh
atelier-manager deployments
atelier-manager verify DEPLOYMENT_ID
atelier-manager activate DEPLOYMENT_ID
atelier-manager active-path
atelier-manager rollback
```

Activation and rollback confirm the exact target, verify the manifest and every artifact hash, and atomically update a single selection record with current and previous IDs under an exclusive lock. A stale review or corrupt deployment is rejected without switching. Ordinary activation also rejects a selective update whose recorded base is no longer selected. Restage against the current deployment or use `activate ID --historical` with explicit confirmation. `rollback` intentionally selects the previous complete deployment regardless of ancestry. Historical selection never bypasses hash checks. Previous deployment files remain intact. `active-path` prints the selected bin directory after structural and size checks; use `verify` for a full hash check. This is a manager-local selection, not a display-manager change or system-wide install. To test a selected bundle, explicitly launch the desired binary from that directory or add it to PATH for the current shell yourself. Running sessions and shell startup files are not modified. A rollback switches the record back to the previous complete deployment, not system packages or user settings. Hashes detect corruption, not authenticity.

## Verified nested session launch

```sh
atelier-manager session-check
atelier-manager session-launch --nested
atelier-manager session-launch --nested --no-shell
```

The manager resolves the selected deployment, verifies all recorded artifacts, and requires the supervisor and compositor (plus the shell unless explicitly disabled) inside that bundle. It launches the exact selected supervisor, not a PATH lookup, and hands process ownership to the supervisor. The readiness check only checks the bundle, host display environment and runtime-directory presence; it does not prove runtime compatibility. The existing desktop remains the graphical host. Rollback selects a previous deployment for the next launch, not the currently running session. Native TTY mode and display-manager registration remain gated on physical hardware and recovery acceptance; this command cannot start native DRM mode or create login entries.

## Deployment inventory and build identity

```sh
atelier-manager deployments --json
atelier-manager deployment-info DEPLOYMENT_ID
```

Inventory schema 1 is derived directly from existing completion manifests and the current/previous selection record; it does not maintain a second registry. It includes staged bundles, requested and resolved product IDs, the staging manager version, source directory and artifact sizes/hashes. Missing selected directories are reported as invalid, not silently omitted. Inventory performs structural and size checks only. The individual deployment-info command verifies all hashes before emitting details. The recorded manager version is **not** the version of each copied binary; artifact hashes identify the actual copied bytes. No version probing, remote update check, download, package change or binary execution occurs. Source paths are local information; review JSON before sharing it. Independent product updates will build on these manifests.

## Selective native updates (experimental)

```sh
atelier-manager update-plan --from target/release shell
atelier-manager update --from target/release shell
atelier-manager deployments
atelier-manager activate DEPLOYMENT_ID
```

The update plan verifies the currently selected bundle and hashes candidate binaries without running them. It reports replacement, unchanged and retained artifacts. Selected products expand through their catalog dependencies and must already exist in the selected bundle. The confirmed update copies all artifacts into a new private deployment, rechecks each copied hash against the review and records the base deployment ID. A changed candidate or retained file aborts staging. An identical candidate creates no new bundle. The original remains selected; activation is a separate operation. The selection is rechecked under the lifecycle lock during staging, which does not change the current selection. Mixed build runtime compatibility is not established by hashes; test nested before relying on the update. No network, packages, settings or live session changes occur. This is local build comparison, not semantic-version ordering or remote release discovery.

## Transaction history and recovery diagnostics

```sh
atelier-manager package-history
atelier-manager package-history --json
atelier-manager recovery-status
```

Package history reads existing checkpoints without creating directories or running commands. New transactions persist atomic outcome and final dependency-verification markers separately; a missing outcome can indicate interruption, while older records without a verification marker remain unknown rather than assumed successful. A missing after-inventory is reported independently. History is a historical record, not a current package-health check; entries are listed by ID, not chronology. Recovery-status emits schema-1 JSON with full hash checks of current and previous native deployments independently, structural failures of other bundles, and package transaction history. Backend errors are reported separately. No automatic repair, package rollback, native activation, deletion or Mango configuration restoration is performed. Mango recovery remains delegated to its installer. Review local paths and diagnostic data before sharing.

## Native release metadata

See [Native release manifest v1](RELEASE_MANIFEST.md) for local manifest generation, detached publisher signature authentication and artifact verification. Local manifest commands do not download or activate releases; the separate discovery, fetch and stage workflows are documented there.

## Deployment storage planning

`atelier-manager deployment-cleanup-plan` reports logical artifact sizes and protection reasons for selected/rollback bundles, referenced bases, and incomplete inventories. It never deletes deployments: running processes may still rely on an older bundle even if it is no longer selected. See the offline removal procedure below.

## Managed runtime leases

```sh
atelier-manager session-launch --nested
atelier-manager run-product settings
atelier-manager run-product shell
atelier-manager deployment-cleanup-plan
```

Manager-mediated nested sessions and independent shell/settings/services/notifications launches verify the selected deployment and acquire a shared per-deployment runtime lock under the selection lock. The descriptor survives exec and may be inherited by children, conservatively protecting the bundle until the last holder closes it. No PID files or stale-PID heuristics are used. The report distinguishes managed-in-use, no-managed-lease, untracked and unknown. Do not unlink runtime.lock files. Older binaries, direct executable launches and applications that close inherited descriptors are not covered. No managed lease is not proof that a bundle is unused; deployment removal requires explicit offline confirmation. Arguments to run-product are passed literally to the catalog-selected native executable. No session registration, package change or activation occurs.

## Offline deployment removal

```sh
atelier-manager deployment-cleanup-plan
atelier-manager deployment-remove DEPLOYMENT_ID --offline
```

Close all applications and sessions using the target bundle, including direct/legacy launches. Use a manager from another deployment or an independent build. Do not run older managers concurrently. Offline means the user confirms no target runtime use, not a network disconnection or a verified system-wide logout. The command requires a terminal and an exact ID-specific confirmation. It rechecks selection, all inventory records, base references, artifact hashes, and an exclusive runtime lease while holding the lifecycle lock shared with staging and activation. It refuses the current or previous deployment, referenced bases, incomplete inventory, the running manager's own bundle, managed runtime use, and unknown files. It renames the bundle into a private quarantine and removes only known flat files and empty directories; interrupted removal can leave a quarantine for manual inspection. No recursive deployment deletion, automatic cleanup, package changes, trust changes, or settings deletion. Runtime locks cannot detect unmanaged processes or older software, and users must keep the target offline throughout removal.

## Graphical manager (preview)

```sh
cargo build -p atelier-manager --features gui
./target/debug/atelier-manager --gui
```

The optional GTK4 frontend runs on an existing graphical desktop without Atelier compositor protocols. The default CLI build remains GTK-free. Overview, products, deployments, recovery, storage and publisher trust are backed directly by the existing manager's read-only operations; inspection runs off the GTK thread, and refresh errors are shown without silently declaring success. Initial views are loaded on selection and can be refreshed explicitly. Product detection is executable presence, deployment and cache inventories are structural, and recovery hashes current and previous deployments. No setup, network download, trust mutation, removal, rollback or activation is triggered by this preview. Continue to use the CLI for confirmed operations. Graphical installation and update controls, appearance integration, packaging and actual desktop acceptance are future work. GTK4 development libraries are required to build the GUI.

## Graphical installation review and local staging

The Installation page supports independent native and external product selections, presets, optional providers, Mango rice IDs and first-session preferences. The CLI and GTK frontend share backend validation and the dependency review. The review is read-only. For a native-only selection, optionally enter an absolute trusted local build directory; the manager hashes its selected artifacts during review and enables staging only after a separate approval checkbox. Staging copies exactly the reviewed identities into a new private deployment and does not activate it. Refresh Deployments to inspect the result. No package transaction, sudo, remote download, rice application or shell/session preference change is initiated from this page. Those transactions remain separate CLI workflows until their graphical confirmation and authentication boundaries are implemented. Package dependency inspection uses the fixed trusted system pacman/timeout paths rather than arbitrary PATH executables.

## Graphical selection and recovery controls

Deployments supports full hash verification, reviewed activation and explicitly marked historical selection. Recovery provides reviewed rollback to the exact previous deployment. The shared CLI/GTK activation review records the current/previous selection and serialized verified target manifest; on approval the backend rechecks both under the lifecycle lock and hashes every artifact again. A changed selection or changed manifest fails closed. No session restart, PATH change, system package rollback, trust change or remote acceptance occurs. GUI verification and selection run off the GTK thread. Other pages are independent snapshots and may require refresh. No unattended activation or automatic recovery is provided.

## Graphical update center

The Updates page explicitly browses up to 20 native repository release records, with opt-in prereleases. Candidate tags and metadata are untrusted. Refresh pinned keys and select one independently verified active key before authenticated acquisition. Downloads use the same bounded cache backend as CLI and preserve release identity without accepting or activating it. Review a cache ID for full staging or a signed component update against the selected deployment. The shared opaque review retains exact manifest/signature bytes and reviewed artifact identities; after explicit approval it reacquires the cache lock, rejects changed metadata, revalidates current publisher/sequence policy and artifacts, then stages under the existing trust and lifecycle locks. Cancel does not accept or stage. No automatic refresh, automatic publisher selection, unattended update, live restart, or system package change. UI operations run on background workers. Publisher pinning remains a CLI operation; remote live acceptance and graphical interaction tests remain outstanding.
