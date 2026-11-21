# Load tests

k6 against Sheriff (`SHERIFF_PROVIDER=mock`, `SHERIFF_API_KEY=dev-key`).

```sh
docker run --rm --network sheriff_default \
  -v "$PWD/loadtest:/scripts" \
  -e BASE_URL=http://app:3000 \
  -e API_KEY=dev-key \
  grafana/k6:1.7.1 run /scripts/<script>
```

Pass each variable below with `-e`. `sheriff_default` is the compose network for a checkout in this directory.

## Parameters

Shared:

| Env | Default |
| --- | --- |
| `BASE_URL` | `http://localhost:3000` |
| `API_KEY` | `dev-key` |
| `BATCH` | `1` |
| `PRE_VUS` | from the rate |
| `MAX_VUS` | from the rate |

`loadtest/score.js`, `loadtest/messages.js`:

| Env | Default |
| --- | --- |
| `RATE` | `20` requests/s. Decisions/s = `RATE * BATCH` when every request is accepted |
| `DURATION` | `2m` |

`loadtest/ws.js`. One VU, one socket. Offered rate = `VUS * BATCH / (INTERVAL_MS / 1000)`. Sending stops 5s before close.

| Env | Default |
| --- | --- |
| `VUS` | `20` |
| `HOLD_MS` | `120000` |
| `INTERVAL_MS` | `1000` |

`loadtest/spike.js` posts to `/v1/messages`. `SPIKE_RATE` must be at least `BASE_RATE`. Stages: 30s at `BASE_RATE`, 15s up, 20s at `SPIKE_RATE`, 20s down, 90s at `BASE_RATE`.

| Env | Default |
| --- | --- |
| `BASE_RATE` | `10` |
| `SPIKE_RATE` | `100` |

`/v1/score` returns 504 after 5s. `/v1/messages` returns 202 before the decision. WebSocket latency is `decision_latency`.

## Runs

1. `score.js` — `RATE=5` `DURATION=30s` `BATCH=1`
2. `messages.js` — `RATE=5` `DURATION=30s` `BATCH=1`
3. `ws.js` — `VUS=5` `HOLD_MS=20000`
4. `score.js` — `RATE=20` `DURATION=5m` `BATCH=1`
5. `score.js` — `RATE=20` `DURATION=5m` `BATCH=10`
6. `messages.js` — `RATE=20` `DURATION=5m` `BATCH=1`
7. `messages.js` — `RATE=20` `DURATION=5m` `BATCH=10`
8. `ws.js` — `VUS=50` `HOLD_MS=120000` `BATCH=1`
9. `spike.js` — `BASE_RATE=10` `SPIKE_RATE=100`
10. Repeat 4–8 with `docker compose up -d --scale app=2`, then `--scale app=4`. Host port 3000 reaches one replica; the command above uses `http://app:3000` and reaches all of them.

Double `RATE` from step 4 until `/v1/score` returns 504 or `http_req_failed` crosses 1%. The step below that, held for 5m, is the sustained rate. Set `SPIKE_RATE` in step 9 to about 10× the sustained `/v1/messages` rate.
