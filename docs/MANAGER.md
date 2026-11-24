# Atelier Manager

The independent manager provides local inventory, planning and explicit user-local deployment staging and selection. It has no Smithay, GTK or Mango Atelier dependency. System package installation and login integration are not implemented.

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

## Guided setup preview

Run `atelier-manager setup` (or `setup --json` to send prompts to stderr and obtain an accepted preview or null on stdout). Choose Express, Custom or Minimal, then select native products or external compositors, per-compositor rice choices where Mango Atelier is selected, a native shell preference, optional service providers, and first session preference. The final review is explicitly non-mutating, including no sudo, package changes, backups or recovery operations. A confirmation accepts only the preview; it does not persist preferences or enable installation. Type q or end input to cancel. Custom rice/profile IDs are planning preferences, not proof of installation or compatibility. With `atelier-manager setup --from BUILD_DIR`, accepting the preview offers a separate native staging review and confirmation, followed on success by an independent deployment-selection confirmation. Mixed or external product selections are rejected before writes. Rice, provider and first-session choices remain preview-only. System package transactions remain future work.

## Arch-family package preview

```sh
atelier-manager packages desktop
atelier-manager packages shell settings --json
```

On Arch-family systems, missing source-build products expand into conservative package candidates, with deduplicated reasons. The wizard also includes selected optional provider packages. A bounded, read-only `pacman -T` query checks the local dependency database when pacman and coreutils timeout are detected; unexpected results are unknown, not marked installed. No package database refresh, repository query, sudo or installation occurs. Candidates do not guarantee runtime compatibility or repository availability; native hardware remains experimental, and Mango Atelier's own setup preview is authoritative for rice/app groups. Other distributions currently report that package mapping is unavailable. The initial doctor command remains subprocess-free.

## Native build staging (experimental)

After building the selected native components from a trusted local checkout, run as your normal desktop user:

```sh
atelier-manager stage --from target/debug shell settings
```

`stage` resolves native product dependencies and verifies each selected artifact before showing an explicit confirmation. It creates a new private deployment under `${XDG_DATA_HOME:-$HOME/.local/share}/atelier-manager/deployments/`, copies only allowlisted native executable filenames, and records sizes and SHA-256 hashes in a completion manifest. The hash detects later corruption; it is not a signature or proof of provenance. Source builds must be trusted. No binaries are run, existing deployments overwritten, system packages installed, login sessions changed or PATH modified. A failed copy removes its own temporary staging directory; an interrupted directory without the completion manifest is not a valid deployment. A complete deployment can be launched directly from its `bin` folder for testing. Package transactions and Mango integration are follow-up work. Plain `setup` ends at preview; `setup --from BUILD_DIR` offers explicit native staging after preview acceptance. After successful staging, guided setup offers a separate activation confirmation using the newly created deployment ID. Declining activation leaves the deployment staged and the current selection unchanged. Standalone `stage` never activates.

## Select or roll back staged deployments

```sh
atelier-manager deployments
atelier-manager verify DEPLOYMENT_ID
atelier-manager activate DEPLOYMENT_ID
atelier-manager active-path
atelier-manager rollback
```

Activation and rollback confirm the exact target, verify the manifest and every artifact hash, and atomically update a single selection record with current and previous IDs under an exclusive lock. A stale review or corrupt deployment is rejected without switching. Previous deployment files remain intact. `active-path` prints the selected bin directory after structural and size checks; use `verify` for a full hash check. This is a manager-local selection, not a display-manager change or system-wide install. To test a selected bundle, explicitly launch the desired binary from that directory or add it to PATH for the current shell yourself. Running sessions and shell startup files are not modified. A rollback switches the record back to the previous complete deployment, not system packages or user settings. Hashes detect corruption, not authenticity.
