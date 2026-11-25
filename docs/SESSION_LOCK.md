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
