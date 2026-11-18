# Defensive analysis: overlays backed by privileged access

## Scope

FL84Ext is a Rust and egui prototype that reads game-related process data through a local driver interface and renders an overlay. This write-up considers the trust boundaries involved; it does not claim to test the target game's protections or the driver's security.

## Threat model

An untrusted companion application may obtain information from a game process, including through a privileged component. The overlay can then present that information outside the game's normal interface.

## Defensive considerations

- Keep shared gameplay rules and outcomes authoritative on the server. Data available only in a player's client should not be treated as proof of a valid action.
- Apply least privilege to any driver interface. Restrict access, validate request types and sizes, and avoid exposing general-purpose memory operations when a narrower interface is possible.
- Keep the user-mode overlay separate from privileged operations. A graphics or display feature should not require broad kernel access by default.
- Treat local integrity signals as supplementary and evaluate false positives before using them for enforcement.

## Limits

The code is unfinished and depends on external driver and game versions. It is not a security review of the driver, a tested defensive implementation, or evidence about the target game's anti-cheat.
