# Native release manifest v1

This contract describes native executable artifacts. It is independent of user deployment manifests and does not contain commands or arbitrary download URLs.

Generate a manifest from a trusted **host-native** local build:

```sh
atelier-manager release-manifest --from target/release --version 0.1.0-alpha.1 --revision "$(git rev-parse HEAD)" shell settings > release-manifest.json
atelier-manager release-verify release-manifest.json --from target/release
```

Schema 1 includes a release version label, full lowercase Git revision, Linux OS and host architecture, concrete dependency-complete native product IDs, and each allowlisted executable's byte size and lowercase SHA-256. Aggregate products expand to their concrete dependencies. The producer supplies the version and revision; the generator does not inspect Git state, executable architecture, or semantic version ordering. Do not use the host-native generator for cross-compiled builds. OS/architecture metadata is not an ABI or hardware compatibility guarantee.

The verifier refuses unknown fields, oversized or symlinked manifests, unknown products, missing dependencies, duplicate or unexpected artifact names, invalid lengths/hashes, and host target mismatches. It hashes local regular executable files without running them. Extra files in the source directory are ignored and never copied by this verifier. It performs no downloads, staging, activation, or network access. A trusted manifest is required: an attacker controlling both manifest and artifact can produce matching hashes. Use the signed verification workflow below to authenticate the manifest. Persistent trust and remote acquisition are described in the later sections. No releases are published by these commands.

## Publisher signatures (OpenSSH Ed25519)

Use an independently authenticated, dedicated release-signing public key. The publisher signs the exact manifest bytes with OpenSSH:

```sh
ssh-keygen -Y sign -f /secure/path/release-signing-key -n atelier-release-v1 release-manifest.json
atelier-manager release-verify-signed release-manifest.json --signature release-manifest.json.sig --key /trusted/path/release-signing-key.pub --from target/release
```

The manager uses the fixed system `/usr/bin/ssh-keygen` executable (OpenSSH must be installed). It accepts only one plain Ed25519 public key, uses a fixed principal and namespace, snapshots bounded input bytes into a private temporary directory and authenticates the exact bytes before parsing the manifest or hashing artifacts. Unsigned `release-verify` remains a local integrity-only check. No private keys are requested, generated for the publisher, uploaded or stored by the manager. Tests generate disposable keys solely for the test fixture. Never obtain the trusted public key from an unverified release alongside the signature; that would allow an attacker to replace both. This stateless command does not use persistent trust or sequence protection. There is no bundled official trust anchor, automatic trust-on-first-use, or freshness guarantee. See the local trust workflow below. A valid signature is not proof of source-code correspondence or hardware compatibility.

## Persistent local trust (schema 2)

The optional `--sequence N` argument after `--revision SHA` generates schema 2, with a positive signed publisher sequence. Assign an increasing sequence for every new release, across keys and channels. Do not reuse a sequence for a different artifact identity. Schema 1 remains supported for stateless verification only.

```sh
atelier-manager release-trust pin /trusted/publisher.pub
atelier-manager release-trust
atelier-manager release-accept release-manifest.json --signature release-manifest.json.sig --key-id KEY_ID --from target/release
atelier-manager release-trust revoke KEY_ID
```

Key IDs are lowercase SHA-256 of the normalized public-key text (not OpenSSH fingerprints). Pin only after independently authenticating the full key. Pin and revoke require typed confirmation; rotation means pinning the independently authenticated replacement and revoking the old key. Revoked keys remain tombstoned. Acceptance verifies the signature and local artifact hashes, then atomically records per-architecture, per-executable high-water marks shared across all pinned keys. Lower sequences and conflicting identities at equal sequence are rejected; identical repeats are accepted. No install or deployment selection occurs. State is stored in `${XDG_STATE_HOME:-$HOME/.local/state}/atelier-manager/release-trust/`. A first acceptance cannot detect older releases, and deleting or restoring old local state defeats replay protection. No expiry, remote revocation, channel separation, official root key, or protection against a compromised currently trusted signer is provided. Stateless verification and trusted local staging remain separate workflows and do not enforce these marks. Never treat this experimental local policy as a complete secure remote updater.

## Remote release discovery (experimental)

```sh
atelier-manager release-discover native
atelier-manager release-discover native --prerelease
```

This explicit read-only network operation queries the first 20 GitHub release records of `matchalattewithbanana/Atelier-DE` through system `gh` and `timeout`. For private repositories authenticate GitHub CLI separately. No token is requested or printed by Atelier. Drafts are excluded; prereleases are opt-in. The v1 discovery naming convention is `atelier-release-linux-ARCH.json` and `atelier-release-linux-ARCH.json.sig`, where ARCH matches Rust's host architecture. Both metadata files must appear exactly once as uploaded assets within their size limits. The JSON output reports candidates in API order, not semantic version or authenticated sequence order. All candidates remain unverified; no assets are downloaded and no release is selected or installed. An empty page or no matching assets is not proof that no release exists. Mango Atelier's distribution has a different installer contract and is intentionally not included yet. Future acquisition must authenticate manifest bytes and enforce pinned-key policy before trusting any release artifact metadata.

## Authenticated acquisition (experimental)

```sh
atelier-manager release-fetch native RELEASE_ID --key-id KEY_ID
```

Use an already independently pinned key and numeric release ID from discovery. Add `--prerelease` for prereleases. The manager fetches bounded manifest/signature assets, authenticates exact manifest bytes and checks schema-2 sequence policy before acquiring binary assets. Binary assets are named `EXECUTABLE-linux-ARCH`, with sizes matching the signed manifest. Each download is streamed into a private temporary directory, checked against signed SHA-256 and only then made executable. All artifacts and current trust policy are rechecked before writing the completion manifest. Failed attempts are cleaned up on normal error paths; interrupted directories without completion markers are invalid. Completed bundles are under `${XDG_CACHE_HOME:-$HOME/.cache}/atelier-manager/releases/`. This is acquisition only; no code runs, no deployment is staged, no high-water mark is advanced. Cache consumers must reauthenticate against current pinned trust, verify bytes and enforce sequence policy at time of acceptance. No reuse, automatic cleanup, freshness guarantee or real remote acceptance test is provided. GitHub CLI must already be authenticated for private repos.

## Stage a verified cached release

```sh
atelier-manager release-stage CACHE_ID --key-id KEY_ID
atelier-manager deployments
atelier-manager activate DEPLOYMENT_ID
```

Use the cache directory basename (release-...) printed by release-fetch. Staging requires a terminal and typed approval. It authenticates the saved exact manifest with current pinned trust, checks all local artifact hashes, then under the trust lock rechecks the signature and sequence policy and copies only the signed artifact identities into a new deployment. The exact signed manifest, signature and release provenance are retained in the deployment. High-water marks are persisted before publishing the completion marker; a later storage failure may leave the release accepted but not staged. Retrying the same accepted identity is allowed. Existing deployment selection is unchanged. This stages the entire signed bundle, not a selective merge with an existing deployment. Activation and local rollback remain separate, and are not remote acceptance operations; they do not enforce current release revocations or sequence floors. No automatic activation or package installation occurs.

## Selective authenticated component updates

```sh
atelier-manager release-update CACHE_ID --key-id KEY_ID
```

The signed cached bundle determines the update scope (for example, a release containing only shell). All declared artifacts must already belong to the current deployment. The manager fully verifies the base deployment and cached release, previews replacement/unchanged/retained artifacts, and requires typed approval. It rechecks trust and release sequence under the trust lock, refuses a changed selected base at staging start and copies only reviewed hashes into a new complete deployment. Unchanged/retained components are copied from the verified base, not fetched again. No changed bytes means no new deployment or sequence acceptance. Release provenance explicitly lists covered artifacts; retained artifacts are not claimed to be signed by this release. The base deployment ID preserves lineage, so cleanup protects referenced ancestors if provenance history is to remain available. This is not a cryptographic attestation of the entire mixed bundle; activation and rollback remain separate local operations. Late storage failure can advance acceptance without completing staging. No mixed-build compatibility guarantee or live remote acceptance testing yet.

## Release cache maintenance

```sh
atelier-manager release-cache
atelier-manager release-cache remove CACHE_ID
```

Fetch reuses an existing cache only when exact signed manifest and signature bytes match and current trust and all artifact hashes verify. The cache lock serializes cooperating fetch, staging, update and removal operations (including approval prompts). Inventory is read-only and structural, not an authenticity check. Removal requires a valid ID and typed confirmation, renames the cache out of its namespace and deletes it without following symlinks. Deployed copies, selection and trust records are untouched. A crash can leave a removed-* quarantine directory, not a usable cache; automated orphan cleanup is not yet implemented. Older manager binaries and manual modifications do not participate in the lock.
