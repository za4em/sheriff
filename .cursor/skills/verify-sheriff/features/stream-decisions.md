# Stream a decision

Stream a decision opens a websocket, sends a batch as one text frame, and reads the decision as a later text frame.

## Sub-features

- `stream-allow` sends `hello` and reads a `ready` decision whose `action` is `allow`.

## How to get to it (user POV)

- `GET /v1/stream` with `Authorization: Bearer verify-key`, upgraded to a websocket.
- One client text frame containing a JSON array of messages.
- One server text frame containing the decision.

## Driving it with verify-sheriff

Preconditions:

- `verify-sheriff doctor` printed `doctor ok`.
- No earlier journey in this run used `message_id` `m-stream`.

- **Send and read.** Run `.cursor/skills/verify-sheriff/verify-sheriff stream "hello" m-stream`. The command prints one JSON object. `message_id` is `m-stream`, `status` is `ready`, `action` is `allow`, and `error` is null. The same text is in `evidence/stream-decisions/<run-id>/roundtrip.body`.

## Gotchas

- The decision frame arrives after the worker finishes. A closed socket before that frame is a failure, not an empty success.
- A problem before the job starts arrives immediately as `{"error":"..."}`. That object is not a decision. `status` will be absent.
- `POST /v1/score` is a different entry point. Do not count a score response as this websocket path.
- The helper sends a single text frame and reads the first server text frame. It does not keep the socket open for a second message.
