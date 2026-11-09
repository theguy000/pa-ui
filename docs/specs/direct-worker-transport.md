# Direct worker transport

## Purpose
Use the daemon's direct session transport when offered, while keeping the supervisor connection for control and fallback.

## Behavior
- After a successful attach, request a direct worker link for the session.
- Session commands and events then use that link; the supervisor connection remains open.
- If the capability, ticket, or handshake fails, keep supervisor routing.
- On `direct_link_lost` for the selected session, drop the dead link and force the normal attach path; the next attach retries the upgrade. It does not clear the status strip or an in-flight prompt.
- A prompt that needs a different session reattaches and retries the upgrade.
- No polling.

## Acceptance
- Direct upgrade is attempted on attach and prompt rebind.
- A loss only triggers reattach for the matching session.
- Failure leaves supervisor routing.
- `cargo test -p pa-client -p pa-ui` passes.
