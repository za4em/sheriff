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

## Save, then get the decision later

`POST /v1/messages` takes the same JSON and returns `202`. The decision is stored. Sending the same `message_id` again stores another message and another decision, so a retry after a timeout can duplicate work. Register a webhook and Sheriff posts each finished decision there, including a decision that failed.

`GET /v1/stream` is the same save, over a websocket. Send the JSON array as a text frame. Each decision arrives later as its own text frame. A problem before the job starts arrives immediately as `{"error":"..."}`.

## RabbitMQ and Redis

RabbitMQ lets several instances share the scoring work. Redis wakes the instance still holding the HTTP call or websocket when another instance finishes the job.
