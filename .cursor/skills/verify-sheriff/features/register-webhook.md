# Register a webhook

Register a webhook stores a URL for the tenant and returns the id, the URL, and a secret the caller will use to check later deliveries.

## Sub-features

- `webhook-create` registers an `http://` URL and returns `201` with `id`, `url`, and `secret`.
- `webhook-reject` rejects an empty URL.

## How to get to it (user POV)

- `POST /v1/webhooks` with `Authorization: Bearer verify-key` and a JSON object `{"url":"http://127.0.0.1:9/sheriff-verify"}`.
- `POST /v1/webhooks` with `{"url":""}`.

## Driving it with verify-sheriff

Preconditions:

- `verify-sheriff doctor` printed `doctor ok`.
- Nothing is listening on `127.0.0.1:9`. A delivery attempt to that port fails, which is what this registration journey wants. Do not point the URL at a live service.

- **Register.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature register-webhook --name create --expect 201 POST /v1/webhooks '{"url":"http://127.0.0.1:9/sheriff-verify"}'`. HTTP `201`. The body has a numeric `id`, `url` equal to `http://127.0.0.1:9/sheriff-verify`, and a non-empty `secret`.
- **Reject an empty URL.** Run `.cursor/skills/verify-sheriff/verify-sheriff request --feature register-webhook --name empty --expect 400 POST /v1/webhooks '{"url":""}'`. HTTP `400`.

## Gotchas

- Registration does not by itself prove a later decision was posted. This journey stops at the `201` and the stored fields in that response. A delivery proof needs a saved message and a receiver, which this map does not seed. Report delivery as unproven rather than treating `201` as a delivery.
- The URL must start with `http://` or `https://`. A bare host is `400`.
- The secret is new on every register. Do not assert a fixed secret string.
- Port `9` is discarded traffic. If a process is already bound there, the precondition fails. Do not switch the URL to a real endpoint to make the call succeed.
