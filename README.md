# Sheriff

Sheriff scores chat messages. You send a batch, and each message comes back as a decision: an action (`allow`, `hide`, `mute`, or `escalate`) and a score per category.

Send `Authorization: Bearer $SHERIFF_API_KEY` on every call.

## Score without saving

`POST /v1/score` returns one result per message. It is a one-time call: those messages are not kept, they are not used as context for later messages, and the other endpoints do not return them. If scoring does not finish within 5 seconds, the call returns 504.

```json
[
  {
    "message_id": "m1",
    "room_id": "room-1",
    "user_id": "u1",
    "text": "hello",
    "timestamp": 1710000000
  }
]
```

```json
[
  {
    "message_id": "m1",
    "status": "ready",
    "action": "allow",
    "categories": { "insult": 0.01, "spam": 0.02 },
    "error": null
  }
]
```

`categories` has a score for each of these: `insult`, `bullying`, `grooming`, `hate`, `threat`, `scam`, `illegal_activity`, `doxxing`, `spam`.

A message that could not be scored uses the same fields. `status` is `failed`, `action` and `categories` are null, and `error` says why.

Sheriff asks one set of questions about each window of messages. It scores a message on its own only when a window that contains that message hits a category. `SHERIFF_MESSAGES_WINDOW` defaults to 30. `SHERIFF_BATCH_OVERLAP` defaults to half of that window.

## Save, then get the decision later

`POST /v1/messages` takes the same JSON and returns `202`. The decision is stored. Sending the same `message_id` again stores another message and another decision, so a retry after a timeout can duplicate work. Register a webhook and Sheriff posts each finished decision there, including a decision that failed.

`GET /v1/stream` is the same save, over a websocket. Send the JSON array as a text frame. Each decision arrives later as its own text frame. A problem before the job starts arrives immediately as `{"error":"..."}`.

## Cache

Redis is the cache, on port 6379. A finished decision is written to a stream every instance reads, so the instance still holding the HTTP call or websocket can deliver it. Redis also claims the webhook, so only one instance posts it.

## Queue

RabbitMQ is the queue, on port 5672. A score job goes there so any instance can take it, and a failed attempt goes back on the queue.

## Observability

Prometheus, on port 9090, scrapes what the process records: decisions, latency, queue depth, retries, webhook failures, and open websockets. Grafana, on port 3001, shows that scrape.

## Load testing

k6 can run three load tests against the mock provider. Measured numbers are not written here yet.

`loadtest/score.js` calls `POST /v1/score`. `loadtest/messages.js` calls `POST /v1/messages`. Both take:

- `RATE`, requests per second. Default `20`.
- `DURATION`, how long the run lasts. Default `2m`.
- `BATCH`, messages in one request. Default `1`.

`loadtest/ws.js` holds websockets open. It takes:

- `VUS`, how many sockets stay open. Default `20`.
- `HOLD_MS`, how many milliseconds each socket stays open. Default `120000`.
- `BATCH`, messages in one send. Default `1`.
- `INTERVAL_MS`, pause between sends on one socket. Default `1000`.

How to start a run is in `loadtest/README.md`.
