# Native release manifest v1

This contract describes native executable artifacts. It is independent of user deployment manifests and does not contain commands or arbitrary download URLs.

Generate a manifest from a trusted **host-native** local build:

```sh
atelier-manager release-manifest --from target/release --version 0.1.0-alpha.1 --revision "$(git rev-parse HEAD)" shell settings > release-manifest.json
atelier-manager release-verify release-manifest.json --from target/release
```

Schema 1 includes a release version label, full lowercase Git revision, Linux OS and host architecture, concrete dependency-complete native product IDs, and each allowlisted executable's byte size and lowercase SHA-256. Aggregate products expand to their concrete dependencies. The producer supplies the version and revision; the generator does not inspect Git state, executable architecture, or semantic version ordering. Do not use the host-native generator for cross-compiled builds. OS/architecture metadata is not an ABI or hardware compatibility guarantee.

The verifier refuses unknown fields, oversized or symlinked manifests, unknown products, missing dependencies, duplicate or unexpected artifact names, invalid lengths/hashes, and host target mismatches. It hashes local regular executable files without running them. Extra files in the source directory are ignored and never copied by this verifier. It performs no downloads, staging, activation, or network access. A trusted manifest is required: an attacker controlling both manifest and artifact can produce matching hashes. Publisher authentication, signed manifests, anti-rollback policy and verified remote acquisition are separate future work. No releases are published by these commands.
