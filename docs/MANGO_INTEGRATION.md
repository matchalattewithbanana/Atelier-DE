# Mango Atelier integration

The native manager can hand off to a trusted local Mango Atelier checkout with the version-1 setup contract (release-candidate branch, once the matching handoff is merged).

```sh
atelier-manager mango-setup --from ~/Projects/mango-atelier --plan
atelier-manager mango-setup --from ~/Projects/mango-atelier
atelier-manager mango-setup --from ~/Projects/mango-atelier --rollback
```

The command explicitly confirms execution of the checkout, checks the exact setup API marker, and delegates to Mango Atelier's interactive wizard. The original wizard remains the authority for compositor selection, separate rice choices, package groups, sudo approval and configuration recovery. The manager does not pass unattended approval flags, fetch code, infer a trusted source from a URL, or modify native deployment selection. The marker proves compatibility only; running the checkout requires trust in its code, including preview mode. Keep the checkout in its original location because configuration links depend on it.

This is a dedicated manager subcommand rather than an automatic application of the native wizard's preview. Native preview choices are not transferred; users choose them in the delegated wizard. Mango Atelier recovery restores its managed user files, not system packages or upgrades. Actual CachyOS/Wayland testing remains necessary.
