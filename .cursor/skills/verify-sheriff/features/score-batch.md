# Score a batch without saving

Score a batch lets a caller submit messages and read each decision in the same response. Those messages are not kept and do not show up in a later room listing.

## Sub-features

- `score-allow` scores the text `hello` as `allow` with every category at `0`.
- `score-hide` scores the word `idiot` as `hide` with `insult` at `0.95`.
- `score-unauthorized` rejects a call that has no bearer token.
- `score-not-stored` shows the room listing does not contain the scored message.

## How to get to it (user POV)

- `POST /v1/score` with `Authorization: Bearer verify-key` and a JSON array of messages.
- `POST /v1/score` with no `Authorization` header.
- `GET /v1/rooms/room-score/decisions?id[]=m-hello` after the allow score, to see that it was not saved.

## Driving it with verify-sheriff

Preconditions:

- `verify-sheriff doctor` printed `doctor ok`.
- No earlier journey in this run used `room_id` `room-score`.

- **Allow.** Send one message whose text is `hello`. Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature score-batch --name allow --expect 200 POST /v1/score '[{"message_id":"m-hello","room_id":"room-score","user_id":"u1","text":"hello","timestamp":1710000000}]'`. HTTP `200`. The single object has `message_id` `m-hello`, `status` `ready`, `action` `allow`, `error` null, and every category score equal to `0`.
- **Hide.** Send one message whose text is `you idiot`. Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature score-batch --name hide --expect 200 POST /v1/score '[{"message_id":"m-idiot","room_id":"room-score","user_id":"u1","text":"you idiot","timestamp":1710000001}]'`. HTTP `200`. `action` is `hide`, `insult` is `0.95`, and `bullying` is `0`.
- **Reject a missing key.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature score-batch --name missing-key --expect 401 --no-auth POST /v1/score '[{"message_id":"m-noauth","room_id":"room-score","user_id":"u1","text":"hello","timestamp":1710000002}]'`. HTTP `401`. The body says the API key is missing or wrong.
- **Confirm nothing was stored.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature score-batch --name not-stored --expect 200 GET '/v1/rooms/room-score/decisions?id[]=m-hello'`. HTTP `200`. The body is an empty array.

## Gotchas

- `POST /v1/score` waits for the worker and then deletes the rows. An empty room listing is the proof those rows are gone. A `200` from score alone does not show that.
- `GET /metrics` is unauthenticated and is not a decision.
- `idiotic` is not the word `idiot`. It stays `allow`.
- Scoring `idiot` twice from the same user inside one retained room is the bullying journey, not this one. This feature uses `POST /v1/score`, which does not retain context.
- If `room-score` was already used by a saved-message journey, the empty-list check can fail for a reason that is not this feature. Report that precondition instead of picking another room and calling this feature passed.
- `?id=m-hello` returns `400`: one query value does not deserialize into the id list. The one-id form that returns `200` is `?id[]=m-hello`.
