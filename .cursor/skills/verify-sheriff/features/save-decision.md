# Save a message and read the decision

Save a message accepts a batch, stores it, and lets the caller read the decision later from the room.

## Sub-features

- `save-accept` accepts the batch with `202` before the decision is ready.
- `save-read` lists the stored decision as `ready` with `action` `allow`.
- `save-hide` lists a stored `idiot` message as `hide` with `insult` at `0.95`.

## How to get to it (user POV)

- `POST /v1/messages` with `Authorization: Bearer verify-key` and a JSON array of messages.
- `GET /v1/rooms/room-save/decisions?id[]=m-saved` after the accept.
- `GET /v1/rooms/room-save/decisions?id[]=m-saved-idiot` after the insult accept.

## Driving it with verify-sheriff

Preconditions:

- `verify-sheriff doctor` printed `doctor ok`.
- No earlier journey in this run used `room_id` `room-save`.

- **Accept.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature save-decision --name accept --expect 202 POST /v1/messages '[{"message_id":"m-saved","room_id":"room-save","user_id":"u1","text":"hello","timestamp":1710000000}]'`. HTTP `202`. The body is empty.
- **Read the decision.** Poll until `status` is `ready`. Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature save-decision --name read --expect 200 GET '/v1/rooms/room-save/decisions?id[]=m-saved'`. HTTP `200`. The array has one object: `message_id` `m-saved`, `status` `ready`, `action` `allow`, `error` null, every category score `0`. If `status` is still `pending`, repeat this command. A still-pending body is not a pass.
- **Store an insult.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature save-decision --name accept-idiot --expect 202 POST /v1/messages '[{"message_id":"m-saved-idiot","room_id":"room-save","user_id":"u2","text":"you idiot","timestamp":1710000001}]'`. HTTP `202`.
- **Read the insult.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature save-decision --name read-idiot --expect 200 GET '/v1/rooms/room-save/decisions?id[]=m-saved-idiot'`. HTTP `200`. `action` is `hide` and `insult` is `0.95`. Repeat while `status` is `pending`.

## Gotchas

- `202` means the job was queued. It is not the decision.
- The same `message_id` sent again stores a second message. A timeout retry is not idempotent. This journey sends each id once.
- `u2` sends the insult so it is not a second `idiot` from `u1`. A second `idiot` from the same user in this retained room scores bullying and the action becomes `mute`.
- The list can return `pending`. Waiting once and declaring success from `202` fails the journey.
- `POST /v1/score` is a different entry point. A score response does not prove this feature.
- `?id=m-saved` returns `400`. Filter one id with `?id[]=m-saved`.
