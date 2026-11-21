# Load test plan

k6 against Sheriff with `SHERIFF_PROVIDER=mock`. One script is one scenario. Run them separately so a number belongs to one endpoint and one batch size.

The scripts check correctness and record latency. They do not encode a latency budget. The result of a sustained run is the highest rate that stays healthy for the whole window. `/v1/score` already supplies the hard bound: a call that is not finished in 5 seconds returns 504.

## What a healthy run is

- `/v1/score`: status 200, and every decision `ready` / `allow` / `error: null`. The body is the text `hello`, which MockProvider scores as allow.
- `/v1/messages`: status 202. That is accept latency, not decision latency. Completed decisions are `sum(rate(sheriff_decisions_total[1m]))` in Prometheus. In steady state that rate matches accepted messages per second, and `sheriff_score_queue_depth` is flat.
- WebSocket: the upgrade is 101, each sent message comes back `ready` before the socket closes, and `decision_latency` is the time from send to that frame.
- Error rate under 1 percent. k6 exits non-zero when `checks` or `http_req_failed` miss that.

`decisions` (score and websocket) and `accepted` (messages and spike) count messages, not HTTP calls. With `BATCH=10`, one request is ten of those.

## Setup

Compose already sets `SHERIFF_PROVIDER=mock` and `SHERIFF_API_KEY=dev-key`. Mock latency defaults to 0, so the first number is Sheriff's own pipeline: Postgres, the queue, the cache, and the workers. `SHERIFF_WORKERS` defaults to 100. The database pool is 20, so a run can stall on the pool while CPU still looks idle. Record both.

```sh
docker compose up -d --build
```

Grafana is on port 3001 (`admin` / `admin`), dashboard Sheriff. Prometheus is on port 9090. Panels used below: Request latency, Decisions, Provider latency, Queue depth, Websocket connections.

Two passes, same scripts:

1. `SHERIFF_PROVIDER_LATENCY_MS` unset (0). This is the ceiling.
2. Set `SHERIFF_PROVIDER_LATENCY_MS` to the latency the real model usually takes, and repeat the rate you kept. This shows queueing in front of a slow provider.

`/v1/messages` and the spike keep rows. The room window then sits at 50 messages per VU. Between recorded messages runs, reset the database volume so the table size is not part of the comparison. `/v1/score` deletes its rows when the call finishes.

Install k6 1.7 locally, or use `grafana/k6:1.7.1` as in the scaling command. The scripts were inspected with that version. A k6 warning about dropped iterations means `PRE_VUS` / `MAX_VUS` is too small for the rate.

## Order

1. Smoke, 30 seconds, small rate. Confirms the stack answers.
2. Find the sustained rate for `/v1/score` at batch 1, then batch 10.
3. The same for `/v1/messages`.
4. WebSocket, at a connection count you care about.
5. Repeat 2–4 with provider latency set.
6. Scaling: the kept rate at 1, 2, and 4 replicas.
7. Spike, then the resilience steps.
8. Write the numbers and the machine spec into `docs/benchmarks/`.

Smoke:

```sh
k6 run -e RATE=5 -e DURATION=30s -e BATCH=1 loadtest/score.js
k6 run -e RATE=5 -e DURATION=30s -e BATCH=1 loadtest/messages.js
k6 run -e VUS=5 -e HOLD_MS=20000 loadtest/ws.js
```

## HTTP, sustained

`loadtest/score.js` and `loadtest/messages.js`. Constant arrival rate.

| Env | Default | Meaning |
| --- | --- | --- |
| `BASE_URL` | `http://localhost:3000` | Sheriff |
| `API_KEY` | `dev-key` | Bearer token |
| `RATE` | `20` | Requests per second |
| `BATCH` | `1` | Messages per request. Run 1 and 10. |
| `DURATION` | `2m` | Use `5m` for the run you record |
| `PRE_VUS`, `MAX_VUS` | derived from `RATE` | Raise them if k6 cannot start iterations |

Start at 20 requests per second and double until the run goes unhealthy: score returns 504, messages fall behind (`sheriff_score_queue_depth` climbs and does not level off), or the error rate crosses 1 percent. The step below that, held for 5 minutes, is the sustained rate. Record p50, p95, and p99 from the k6 summary.

For `/v1/score`, p50/p95/p99 are the k6 `http_req_duration` values. A batch shares one 5 second budget, so batch 10 is a different number from batch 1.

For `/v1/messages`, k6 latency is time-to-202. Decision latency for this path is the websocket scenario. On the dashboard, Decisions should match `accepted` per second, and Queue depth should be flat by the end of the 5 minutes.

```sh
k6 run -e RATE=20 -e DURATION=5m -e BATCH=1 loadtest/score.js
k6 run -e RATE=20 -e DURATION=5m -e BATCH=10 loadtest/score.js
k6 run -e RATE=20 -e DURATION=5m -e BATCH=1 loadtest/messages.js
k6 run -e RATE=20 -e DURATION=5m -e BATCH=10 loadtest/messages.js
```

## WebSocket

`loadtest/ws.js`. One VU keeps one connection for `HOLD_MS`, sends `BATCH` messages every `INTERVAL_MS`, and stops sending 5 seconds before close so a healthy decision can arrive.

| Env | Default | Meaning |
| --- | --- | --- |
| `VUS` | `20` | Open connections. Offered rate is `VUS * BATCH / (INTERVAL_MS/1000)` |
| `HOLD_MS` | `120000` | How long each connection stays open |
| `INTERVAL_MS` | `1000` | Time between sends |
| `BATCH` | `1` | Messages per send |

Record connection count, `decision_latency` p50/p95/p99, and `sheriff_websocket_connections` (it should sit on `VUS` during the hold). `decisions_missing` must stay 0.

```sh
k6 run -e VUS=50 -e BATCH=1 -e HOLD_MS=120000 loadtest/ws.js
```

## Scaling

Same HTTP and websocket commands at 1, 2, and 4 replicas. Record decisions per second at each count. The interesting result is where the line flattens: the pool, the broker, or the cache, rather than a straight line through 4.

`docker compose up --scale app=N` publishes host port 3000 once, so `localhost:3000` reaches a single replica. For 2 and 4, run k6 on the compose network. Docker DNS for `app` returns every replica. The scripts resolve it with a 5 second TTL and round-robin.

```sh
docker compose up -d --scale app=2
docker run --rm --network sheriff_default \
  -v "$PWD/loadtest:/scripts" \
  -e BASE_URL=http://app:3000 \
  -e API_KEY=dev-key \
  -e RATE=20 -e DURATION=5m -e BATCH=1 \
  grafana/k6:1.7.1 run /scripts/score.js
```

The network name is `<project>_default`. `sheriff_default` matches a checkout in this directory.

Prometheus currently scrapes the single target `app:3000`, which is one address. For the 2- and 4-replica runs, point it at every replica or the Decisions panel shows one process. k6's `decisions` and `accepted` counters stay the client-side total either way. Queue depth is the broker's ready count, so one scrape is enough for that panel.

```yaml
scrape_configs:
  - job_name: sheriff
    metrics_path: /metrics
    dns_sd_configs:
      - names: ["app"]
        type: A
        port: 3000
        refresh_interval: 5s
```

## Spike

`loadtest/spike.js` posts to `/v1/messages`. It holds `BASE_RATE` for 30 seconds, ramps to `SPIKE_RATE` over 15 seconds, holds that for 20 seconds, ramps back over 20 seconds, then holds `BASE_RATE` for 90 seconds. That last hold is the recovery window.

Set `SPIKE_RATE` to about ten times the sustained messages rate from the HTTP run. Defaults are 10 and 100, for a first look.

During recovery, Queue depth returns to the level it had before the burst, and `sum(increase(sheriff_decisions_total[5m]))` catches up with the k6 `accepted` count. New calls still return 202 while the backlog drains. `/v1/score` is the wrong call for this: anything unfinished at 5 seconds becomes a 504, so the queue never holds the burst.

```sh
k6 run -e BASE_RATE=10 -e SPIKE_RATE=100 loadtest/spike.js
```

The 1 percent error threshold will fail a spike that is large enough to shed load. For a shedding run, look at the summary anyway: the question is whether the recovery window finishes the jobs that were accepted, which Prometheus answers.

## Resilience

Do this with 2 replicas and `loadtest/messages.js` running at a modest rate (about a quarter of the sustained rate). Watch Decisions, Queue depth, and the k6 error rate.

1. `docker kill` one app container. Jobs that replica had not acknowledged go back to the queue. A job already marked ready is not scored again: `sheriff_decisions_total` stays one per message, it does not jump by the number of redeliveries. The other replica keeps accepting.
2. Restart the cache container. Calls to `/v1/score` that were waiting on the ready stream can return 504. Messages already on the queue still reach a decision after the cache is back.
3. Restart the queue container. The queue is durable. Publishes fail while it is down; rows already saved stay pending, and the sweeper republishes them after 30 seconds. Those decisions finish late. `/v1/score` calls in that window return 504, because 5 seconds is shorter than the sweep.

Bring the killed app container back before the next scenario.

## Resources

During each recorded sustained run:

```sh
docker stats --no-stream --format '{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}'
```

Sample a few times, not once at startup. Write CPU and RAM per container: app, postgres, cache, queue. With 2 and 4 replicas, sample every app container.

## Not in this pass

Provider faults, and the check that a dead-lettered job stays failed. MockProvider does not fail, and a failed attempt is put back on the queue with no dead letter. Both wait on the retries work in the handoff. Webhook retries are the same gap: do not register a webhook for these runs, or a dead receiver will add failures that are not part of the scenario.

## What to write down

`docs/` is gitignored, so the notes stay on the machine that ran the bench. In `docs/benchmarks/`, one note per machine:

- CPU, RAM, disk, and OS of the host
- replica count, `SHERIFF_WORKERS`, database pool (20), `SHERIFF_PROVIDER_LATENCY_MS`
- for each script: rate or VU count, batch, duration, p50/p95/p99, decisions or accepted per second, error rate
- spike: depth before, peak, and at the end of the 90 second hold
- resilience: what failed open, and that redelivered jobs did not add a second decision
- CPU and RAM per container at the sustained rate
