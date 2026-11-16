# Sheriff

Sheriff scores chat messages. You send a batch, and each message comes back as a decision: an action (`allow`, `hide`, `mute`, or `escalate`) and a score per category.

Send `Authorization: Bearer $SHERIFF_API_KEY` on every call.

## Score without saving

`POST /v1/score` returns one result per message. Nothing is stored.

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
    "categories": { "insult": 0.01, "spam": 0.02 }
  }
]
```

`categories` has a score for each of these: `insult`, `bullying`, `grooming`, `hate`, `threat`, `scam`, `illegal_activity`, `doxxing`, `spam`.

A message that could not be scored comes back in the same list as `{"message_id":"m1","error":"..."}`.

## Save, then get the decision later

`POST /v1/messages` takes the same JSON and returns `202`. The decision is stored. Register a webhook and Sheriff posts each ready decision there.

`GET /v1/stream` is the same save, over a websocket. Send the JSON array as a text frame. Each decision arrives later as its own text frame. A problem before the job starts arrives immediately as `{"error":"..."}`.
