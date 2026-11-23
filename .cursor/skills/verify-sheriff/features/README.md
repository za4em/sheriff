# Sheriff verification map

This directory is the maintained source for verifying Sheriff's user-facing behavior. Read this index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Launch with `.cursor/skills/verify-sheriff/verify-sheriff launch` from the repo root.
- The server is `http://127.0.0.1:3000`. The API key is `verify-key`.
- Postgres, Redis, and RabbitMQ belong to this run (ports `55432`, `56379`, `55672`).
- Run `verify-sheriff doctor` and require `doctor ok` plus the recorded pid owning port `3000`.
- Never drive an instance that was not started by this verification run.
- Sheriff always binds port `3000`. Do not start a second process.

## Driving conventions

- Start every recipe from the baseline state unless its preconditions say otherwise.
- Use a fresh `room_id` and `message_id` per journey unless the journey needs prior room context.
- Treat every command as literal. Keep quoted JSON and flags unchanged.
- Run HTTP actions through `verify-sheriff request`.
- Run the websocket action through `verify-sheriff stream`.
- Leave files under `evidence/` in place. Cleanup removes only the process and containers.

## Proof and skip reporting

- Capture the request and the resulting state, not only the final body.
- HTTP proof includes the status file, the response body, and a second read that shows whether the message was stored.
- Websocket proof includes the text frame that was sent and the decision frame that came back.
- Compare category scores numerically.
- Record the feature id and the entry point with every artifact (`--feature` and `--name`).
- Report an unreachable path with the attempted command and the unmet precondition.
- Do not report a skipped entry point as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with verify-sheriff` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

## Features

- [Score a batch without saving](./score-batch.md) covers `POST /v1/score` for an allowed message, a hidden insult, a missing API key, and the proof that nothing was stored.
- [Save a message and read the decision](./save-decision.md) covers `POST /v1/messages` and `GET /v1/rooms/{room_id}/decisions`.
- [Register a webhook](./register-webhook.md) covers `POST /v1/webhooks`.
- [Stream a decision](./stream-decisions.md) covers `GET /v1/stream`.
