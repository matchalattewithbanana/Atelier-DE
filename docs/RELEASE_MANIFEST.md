# Native release manifest v1

This contract describes native executable artifacts. It is independent of user deployment manifests and does not contain commands or arbitrary download URLs.

Generate a manifest from a trusted **host-native** local build:

```sh
atelier-manager release-manifest --from target/release --version 0.1.0-alpha.1 --revision "$(git rev-parse HEAD)" shell settings > release-manifest.json
atelier-manager release-verify release-manifest.json --from target/release
```

Schema 1 includes a release version label, full lowercase Git revision, Linux OS and host architecture, concrete dependency-complete native product IDs, and each allowlisted executable's byte size and lowercase SHA-256. Aggregate products expand to their concrete dependencies. The producer supplies the version and revision; the generator does not inspect Git state, executable architecture, or semantic version ordering. Do not use the host-native generator for cross-compiled builds. OS/architecture metadata is not an ABI or hardware compatibility guarantee.

The verifier refuses unknown fields, oversized or symlinked manifests, unknown products, missing dependencies, duplicate or unexpected artifact names, invalid lengths/hashes, and host target mismatches. It hashes local regular executable files without running them. Extra files in the source directory are ignored and never copied by this verifier. It performs no downloads, staging, activation, or network access. A trusted manifest is required: an attacker controlling both manifest and artifact can produce matching hashes. Use the signed verification workflow below to authenticate the manifest. Anti-rollback policy and verified remote acquisition are separate future work. No releases are published by these commands.

## Publisher signatures (OpenSSH Ed25519)

Use an independently authenticated, dedicated release-signing public key. The publisher signs the exact manifest bytes with OpenSSH:

```sh
ssh-keygen -Y sign -f /secure/path/release-signing-key -n atelier-release-v1 release-manifest.json
atelier-manager release-verify-signed release-manifest.json --signature release-manifest.json.sig --key /trusted/path/release-signing-key.pub --from target/release
```

The manager uses the fixed system `/usr/bin/ssh-keygen` executable (OpenSSH must be installed). It accepts only one plain Ed25519 public key, uses a fixed principal and namespace, snapshots bounded input bytes into a private temporary directory and authenticates the exact bytes before parsing the manifest or hashing artifacts. Unsigned `release-verify` remains a local integrity-only check. No private keys are requested, generated for the publisher, uploaded or stored by the manager. Tests generate disposable keys solely for the test fixture. Never obtain the trusted public key from an unverified release alongside the signature; that would allow an attacker to replace both. There is no bundled official trust anchor, automatic trust-on-first-use, key rotation/revocation mechanism, freshness or anti-rollback policy yet. A valid signature is not proof of source-code correspondence or hardware compatibility.

## Remote release discovery (experimental)

```sh
atelier-manager release-discover native
atelier-manager release-discover native --prerelease
```

This explicit read-only network operation queries the first 20 GitHub release records of `matchalattewithbanana/Atelier-DE` through system `gh` and `timeout`. For private repositories authenticate GitHub CLI separately. No token is requested or printed by Atelier. Drafts are excluded; prereleases are opt-in. The v1 discovery naming convention is `atelier-release-linux-ARCH.json` and `atelier-release-linux-ARCH.json.sig`, where ARCH matches Rust's host architecture. Both metadata files must appear exactly once as uploaded assets within their size limits. The JSON output reports candidates in API order, not semantic version or authenticated sequence order. All candidates remain unverified; no assets are downloaded and no release is selected or installed. An empty page or no matching assets is not proof that no release exists. Mango Atelier's distribution has a different installer contract and is intentionally not included yet. Future acquisition must authenticate manifest bytes and enforce pinned-key policy before trusting any release artifact metadata.
