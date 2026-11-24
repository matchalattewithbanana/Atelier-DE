# Atelier Manager

The first independent manager component provides read-only local inventory. It is included in the native workspace while the installer contract matures; it has no Smithay, GTK or Mango Atelier dependency. It does not install or remove anything today.

```sh
cargo run -p atelier-manager -- doctor
cargo run -p atelier-manager -- doctor --json
```

The schema-1 JSON report includes distribution ID/family/version, CPU architecture, session type/desktop identity, detected package managers, Atelier products, common external compositor/shell tools and optional service providers. It reads os-release without interpreting shell syntax and checks executable-file presence in absolute PATH directories, plus the manager's sibling directory and user-local bin for desktop products. Missing paths are null. Presence is only an inventory signal: it is not a claim that the executable runs, that a distribution is supported, or that its package repository has the requested product.

No subprocesses, installation, downloads, sudo, privileged hardware access or file mutations occur. Paths and session identity are local information; review before sharing a diagnostic report.

Future work: product catalog, dependency/compatibility resolution, interactive selection, explicit package/file change review, installation transactions, recovery, and a graphical frontend using the same engine. A bootable ISO remains outside the current development scope.

## Catalog and preview

```sh
atelier-manager catalog
atelier-manager plan shell settings
atelier-manager plan desktop --json
atelier-manager plan mango-atelier hyprland
```

The catalog separates native Atelier components, the Mango Atelier distribution, external compositors and the native desktop aggregate. Selection expands only explicit dependencies and deduplicates them. A standalone shell/settings plan never silently installs a compositor; a session requires the native compositor. The preview reports detected and missing executable names per product. Missing is not a package transaction or package availability claim. The JSON contract marks `apply_supported: false`; there is no apply operation, sudo, network or file modification. Installer backends and interactive setup will build on this catalog.
