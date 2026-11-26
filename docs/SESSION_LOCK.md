# Native session lock security contract

Status: internal foundation only. No advertised ext-session-lock global, no lock command, and no runtime activation flag. Do not treat Atelier as a secure locker.

## Current implementation

- Exact lock-object identity and monotonic presentation generations; only the current confirmed owner can unlock. A duplicate request or stale frame completion cannot alter the current lock. Losing the owner leaves an orphaned protected state.
- A native protected-frame token is captured from the rendered state, recorded only after successful atomic submission and consumed at page-flip completion. A previous desktop flip cannot acknowledge a lock. Full opaque-black repaint; no ordinary scene, cursor surface, frame callback, ordinary input or workspace activation. Nested never claims host protection.
- These internal transitions are intentionally unreachable until a complete protocol adapter exists.

## Required before exposing the protocol

Smithay v0.7.0's session-lock `lock.rs` dispatch posts InvalidUnlock for a nonlocked request but still calls `state.unlock()` and clears its output bookkeeping. A bare implementation of its handler is insufficient. Inspect the pinned upstream source and enforce exact resource authorization before forwarding GetLockSurface and UnlockAndDestroy. A same-client second object must not unlock or poison the current owner's output registry. Reject invalid requests without changing the accepted owner's state. Bind lifetime to the actual resource; never acknowledge until a protected frame has completed for all active outputs.

Implement and test the following before enabling any global:
1. Dispatch isolation for rejected/duplicate/foreign lock objects; configure only accepted owner surfaces for actual outputs. Handle surface destruction and disconnect without falling back to the desktop.
2. Lock-only scene over opaque black, output-local clipping, no ordinary cursor or decorations. All pending scanout slots are invalidated; no old desktop data can reappear. Input is routed only to accepted lock surfaces with correct focus, keyboard state, pointer coordinates and protocol roles. Cancel or safely replace ordinary grabs and deny ordinary selection/clipboard and drag access at lock entry; restore only after authorized unlock. No ordinary input delivered while protected.
3. Handle compositor/seat/route loss, suspend and output changes without revealing previously protected desktop content. The current experimental KMS backend exits on route or seat loss and restores scanout; this is a release blocker for secure locking, not a supported recovery path.
4. Real malicious-client tests: rejected second lock, same-client second resource, invalid unlock, disconnected owner before and after acknowledgment, destroy/recreate surface, stale flips, input focus theft, alpha holes, output loss, suspend, VT switches, compositor crash. Verify actual screen and input on spare TTY hardware. Provide controlled recovery without weakening authentication.
5. Only then advertise the protocol on the verified native backend. Keep nested separate and do not advertise host security. Configure trusted lockers independently and test authentication with the installed PAM configuration.

No production security claim follows from unit tests or protocol availability.

## Guarded protocol adapter (still disabled)

The native build now implements Smithay's lock-manager and resource dispatch traits without creating the global. A custom lock-object dispatcher checks exact resource ownership and confirmed presentation before forwarding unlock, rejects foreign/duplicate-output surface construction before upstream bookkeeping, and handles owner destruction by retaining the protected state. The lock callback stores Smithay's confirmation until the native protected flip completes. The protected scene can draw only the accepted mapped lock-surface tree over opaque black; no normal surface or ordinary cursor is drawn. Only lock-tree frame callbacks are eligible in protected states. No input is routed to the lock surface yet.

The pinned Smithay lock-surface resource data is private and does not expose exact resource-to-root lifetime tracking; this adapter conservatively clears the accepted tree on same-client lock-role destruction and refuses a replacement for the same lock. That can blank the lock UI, but cannot reveal the desktop. Precise role lifetime/replacement, root commit/size validation, real malicious-client dispatch tests, focus and grab isolation, selection isolation, and seat/route-loss recovery are required before registration. The original KMS backend still restores the original scanout on exit. Do not advertise or enable the protocol yet.

## Lock input routing groundwork

Native input is now separated by focus ownership. On admission, active keyboard/pointer grabs cause refusal rather than inheriting an ordinary drag. Focus and data-device focus are cleared at the protection boundary; the previous clipboard selection is cleared (deliberate loss, not preserved across lock). A physical key/button release gate tracks inputs independently of focus and delays new focus until all held controls are released. The event which clears the gate is processed with no focus. The accepted mapped root must match the actual logical buffer size as well as configure state; keyboard goes only to that root and pointer hit-testing is restricted to its tree. No desktop shortcuts, task/workspace commands, client cursor images or new data-device drag/selection requests are admitted while protected. A compositor-owned cursor glyph may be drawn when the lock root is mapped. Unlock clears locker focus and restores a surviving desktop window only after the release gate. Surface loss clears focus and does not unlock. No protocol global is created.

This is not a complete clipboard security boundary: outstanding previously issued data offers, multi-device pressed-state accounting, stale same-client role lifetime, lock-client DnD/resource requests, recovery/seat loss and adversarial integration tests still need review. Do not advertise secure locking until these are resolved and actual hardware acceptance passes.

## Exact lock resource lifetime

The native adapter now owns ext-session-lock manager, lock and surface dispatch using Smithay's public compositor primitives, instead of forwarding to the pinned private session-lock bookkeeping. Each accepted surface stores the actual role resource and root, configure serial, expected logical dimensions and committed state. Only exact role or root destruction clears it; the same accepted owner can create a fresh role on a new root after destruction. Stale role destruction and acknowledgments cannot affect a replacement. Rejected lock resources cannot create surfaces or unlock. Root commits are checked after buffer processing and before use, and invalid serials, missing buffers and wrong dimensions are rejected and blank the protected scene. A new root must acknowledge its configure and commit the correct buffer before receiving input or rendering. The compositor explicitly refuses restoring the original framebuffer on protected-session exit, but descriptor release, VT, GPU, suspend and seat-manager behavior remain unverified; this does NOT provide a secure recovery guarantee. Prior descriptions of conservative same-client role destruction and Smithay delegation are superseded by this section. No global is registered. Real protocol-client integration tests and hardware recovery are still required. The adapter currently supports a single fixed unscaled output, no output reconfiguration, and no orphaned-owner re-lock recovery.

## Protocol wire regression tests

The native-session unit-test build registers the lock global only inside an in-process private Wayland display connected by Unix socket pairs. Raw version-1 protocol requests exercise the production dispatch implementation, not a simulated lock handler. Tests cover the locked event after a synthetic protected-frame completion, valid unlock, rejected same-client unlock, foreign rejected surface construction, exact role replacement and stale root destruction, invalid configure serial, commit before acknowledgment, premature unlock and owner disconnect before/after confirmation. The tests do not use DRM, actual presentation, PAM, live input devices or a desktop socket. The ordinary native and nested backends still do not advertise the global. Real buffer attachment, input events, data offers and actual recovery remain to be tested.

## Actual shared-memory buffer tests

Private wire tests now create wl_shm pools using real file descriptors passed through SCM_RIGHTS, attach ARGB8888 buffers, and verify that a configured output-sized lock root maps and receives only its own keyboard/pointer focus. Invalid dimensions, null-buffer commits, preexisting attached buffers and role replacement are checked against actual Smithay buffer processing. The tests still simulate protected-frame completion rather than drive a GPU or physical seat. No production global is exposed.
