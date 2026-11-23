---
name: verify-sheriff
description: Drive Sheriff, the chat-scoring HTTP API, against a disposable Postgres, Redis, and RabbitMQ, and prove /v1/score, /v1/messages, /v1/rooms/{room_id}/decisions, /v1/webhooks, and /v1/stream. Use when a change to Sheriff needs proof from the real process, not only unit tests.
---

# Verify Sheriff

Sheriff is an HTTP API. A user sends chat messages and reads a decision: an action (`allow`, `hide`, `mute`, or `escalate`) and a score per category. The binary always binds `0.0.0.0:3000`. Grafana on port 3001 and the k6 scripts in `loadtest/` are not this harness.

The helper is `.cursor/skills/verify-sheriff/verify-sheriff`. Run it from the repo root. It records the pid and container names it started and never kills by process name.

Two Sheriff processes cannot share port 3000. If that port is taken, launch stops. Postgres, Redis, and RabbitMQ for a run use host ports `55432`, `56379`, and `55672` and containers named `sheriff-verify-{pg,redis,rabbit}-<run-id>`. Those containers have no named volume from `docker-compose.yml`. Do not point the run at a database, cache, or queue you did not start.

## Launch

From the repo root:

```bash
.cursor/skills/verify-sheriff/verify-sheriff launch
```

Launch refuses when `3000`, `55432`, `56379`, or `55672` is already listening, or when a recorded pid is still alive.

It starts three containers. Postgres and Redis use the image defaults. RabbitMQ is started as root with `RABBITMQ_ERLANG_COOKIE=sheriff-verify` and the user `sheriff`. On this Docker Desktop the `rabbitmq` user cannot read the mode-0400 Erlang cookie it creates (`eacces`), so the container never becomes ready as that user. Root can read the cookie. The broker still listens on container port `5672` and the app still connects as `sheriff`.

Then:

```bash
DATABASE_URL=postgres://sheriff:sheriff@127.0.0.1:55432/sheriff \
QUEUE_URL=amqp://sheriff:sheriff@127.0.0.1:55672/%2f \
CACHE_URL=redis://127.0.0.1:56379 \
SHERIFF_API_KEY=verify-key \
SHERIFF_PROVIDER=mock \
SHERIFF_PROVIDER_LATENCY_MS=0 \
SHERIFF_WORKERS=4 \
cargo run --quiet
```

Launch starts that command in its own session and ignores SIGHUP, so the process stays up after `launch` returns. `App::connect` runs `migrations/` before the process logs `listening on`. Ready means that log line is present, the recorded pid is alive, and `doctor` passes. A container health check alone is not ready.

The API key is `verify-key`. Send `Authorization: Bearer verify-key` on every call except the unauthorized check. The tenant id inside the process is `local`.

Tear down with `verify-sheriff cleanup`. Cleanup stops the recorded pid and removes the three recorded containers. It leaves `.cursor/skills/verify-sheriff/evidence/`.

## Doctor

```bash
.cursor/skills/verify-sheriff/verify-sheriff doctor
```

Doctor answers whether this instance is worth driving. It requires all of the following:

- The pid in `.cursor/skills/verify-sheriff/run/current.env` is alive.
- That same pid is the listener on TCP `3000`. `cargo run` is the parent; the recorded pid is the sheriff binary that bound the port.
- The three recorded containers are running.
- `POST /v1/score` without a bearer token returns `401`.
- `POST /v1/score` with `verify-key` and the text `hello` returns `200`, `status` `ready`, and `action` `allow`.

Doctor prints `doctor ok`, the run id, the pid, and the git SHA captured at launch. Anything else means stop. Do not drive a server you did not launch.

## Drive

Read `.cursor/skills/verify-sheriff/features/README.md`, then the feature file. Run one feature's commands. Do not report a skipped entry point as covered by another path.

`verify-sheriff request` sends one call and writes the evidence files named in **Evidence**. Flags before the method:

- `--feature <id>` selects the feature directory. Default `score-batch`.
- `--name <step>` selects the file prefix.
- `--expect <status>` exits non-zero when the HTTP status differs. The body is still kept.
- `--no-auth` omits the bearer token.

```bash
.cursor/skills/verify-sheriff/verify-sheriff request \
  --feature score-batch --name allow --expect 200 \
  POST /v1/score \
  '[{"message_id":"m-hello","room_id":"room-score","user_id":"u1","text":"hello","timestamp":1710000000}]'
```

`verify-sheriff stream` opens `GET /v1/stream`, sends one JSON text frame, and prints the first text frame the server sends:

```bash
.cursor/skills/verify-sheriff/verify-sheriff stream "hello" m-stream
```

The mock provider is the scorer for this harness. `SHERIFF_PROVIDER=mock` does not call a network model. A message whose text contains the word `idiot` scores `insult` at `0.95`. At the default threshold `0.8` that action is `hide`. The word `idiotic` does not. A second `idiot` from the same `user_id` in retained room context scores `bullying` at `0.95` and the action becomes `mute`. Use a fresh `room_id` per journey unless the journey is that repeat.

`POST /v1/score` does not keep the message. Prove that with a following `GET /v1/rooms/<room_id>/decisions?id[]=<message_id>` whose body is `[]`. A query written as `?id=<message_id>` returns `400` because one value does not deserialize into the id list. `POST /v1/messages` returns `202` before the worker finishes. Poll `GET /v1/rooms/<room_id>/decisions?id[]=<message_id>` until `status` is `ready`. Sending the same `message_id` again stores another row.

## Evidence

Each `request` or `stream` writes under:

`.cursor/skills/verify-sheriff/evidence/<feature>/<run-id>/`

For a request named `allow` the files are `allow.request.json`, `allow.headers.txt`, `allow.body`, and `allow.status`. Stream writes `roundtrip.body`.

A proof shows the call and the resulting state. For a score, keep the response body and the empty room listing that shows the message was not stored. For a saved message, keep the `202` and a later listing whose `action` and category scores match. Compare scores numerically. `0.95` may not be a unique JSON spelling. Do not treat `GET /metrics` as a decision proof. That route has no auth.

## Cleanup

```bash
.cursor/skills/verify-sheriff/verify-sheriff cleanup
```

Cleanup removes the recorded sheriff pid and the three recorded containers, then deletes `run/current.env`. It does not delete `evidence/`. After cleanup, the evidence files from the drive must still be on disk. A cleanup that removes them is a failed run.

## Helpers

`.cursor/skills/verify-sheriff/verify-sheriff` is the only helper. Subcommands: `launch`, `doctor`, `request`, `stream`, `cleanup`.
