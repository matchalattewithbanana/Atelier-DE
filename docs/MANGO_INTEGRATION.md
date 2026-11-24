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
