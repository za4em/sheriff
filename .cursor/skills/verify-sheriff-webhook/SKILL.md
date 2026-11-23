---
name: verify-sheriff-webhook
description: Model-check Sheriff's webhook claim so one retained decision is posted by at most one instance. Use when a change touches score jobs, the ready stream, claim_webhook, or webhook delivery.
---

# Verify the webhook claim

A retained decision is posted to the tenant's webhook by at most one Sheriff instance. Two posts make the tenant act twice on one message.

The check is a TLA+ model of that handoff, run with TLC from `tla2tools` 1.7.4. TLA+ is the tool because the question is whether two instances can post the same decision, which is an ordering question. The Rust has no supported proof tool that runs on the source. The model is the check. A separate anchor refuses to pass when the Rust no longer matches the steps below.

## When to run

Run it when a change edits any of these:

- `src/core/engine.rs` `observe`, `claim_webhook`, `webhook`
- `src/core/engine/worker.rs` `score_group`
- `src/core/decisions/repository.rs` `mark_ready`
- `src/core/cache.rs` `ready_decision`, `claim_webhook`
- `src/core/cache/reader.rs` `follow_decisions`

The model was last confirmed against commit `31c1dddb1f2ca23df1208e986beec81736e93a12`. If one of those functions changed after that commit, the model may be stale. Do not move the commit forward until you have read the new code, updated the model if a step changed, and this check passes. A passing run on a stale model is not a pass for the new code.

## Run

From the repo root:

```bash
.cursor/skills/verify-sheriff-webhook/check.sh
```

The command needs `java` and `python3`. It uses `.cursor/skills/verify-sheriff-webhook/vendor/tla2tools-1.7.4.jar` (sha256 `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`). It does not download anything.

A passing run prints two lines: the claim holds for the faithful model, and the broken model (no `SET NX`) reports the violation. Any other ending is a failure. Read every TLC result with `/statechart`. On a violation, `/statechart` shows the shortest path. On a pass, `/statechart` draws how the instances move. Do not treat a green script as the explanation.

## What the model counts as one step

Each action is one await in the Rust, not a whole request.

- `ReadPending` is the group load in `worker.rs` `score_group` while this decision's `status` is `Pending`.
- `ReadDone` is that load after this decision has left `Pending`. The worker publishes `completed_now` false.
- `MarkReady` is `mark_ready`: `UPDATE decisions SET status = ready WHERE id = $1 AND status = pending`. At most one worker gets `true`.
- `Publish` is `cache.ready_decision`, an `XADD` on `sheriff.ready`.
- `Observe` is one instance reading the next stream entry. Every instance has its own cursor. This is not a Redis consumer group.
- `Claim` is `claim_webhook`: `SET sheriff.hook.<id> NX EX`. Only the instance that creates the key may post.
- `Post` is `webhook` calling `deliver_decision` after that claim returned true.

`UseClaim = TRUE` is this code. `WebhookClaimBroken.cfg` sets `UseClaim = FALSE`, which is `claim_webhook` returning true for every instance. That variant must violate `AtMostOneWebhook`. If it passes, the invariant is no longer doing its job.

## How to read a failure

The anchor failure is a modeling gap, not a TLC counterexample. The script names the function that drifted. Update the model to match the new step, or put the step back, then rerun.

A TLC failure of `AtMostOneWebhook` on the faithful config is a finding. Do not weaken the invariant. The counterexample is a sequence of the actions above. In the code's words it means more than one instance reaches `deliver_decision` for the same decision. `/statechart` is how you read that sequence.

`OneCompletion` failing means more than one worker's `mark_ready` returned true. That is the SQL predicate, not the webhook.

`TypeOK` failing is a broken model. Fix the model, not the Rust, unless the Rust change is what you are describing.

## Scope

The check covers one retained decision, two workers, and two instances that are already listening when the ready entry is written. It covers the `SET NX` claim while the key exists.

It does not cover:

- The claim key expiring (`CLAIM_TTL_SECS`, 86400). A later replay can claim again.
- The ready stream dropping old entries (`SHERIFF_CACHE_LIMIT`).
- A second `deliver_decision` from one instance to several webhook URLs. One instance may post to every registered URL.
- HTTP failure after the claim. The key is still held, so a retry from another instance does not post.
- `POST /v1/score`, which does not retain the row. `webhook` claims and then skips `deliver_decision`.
- The sweeper republishing a decision that is still `pending`. A second worker that arrives after `mark_ready` takes `ReadDone` and publishes false.

## Coverage

| Action | Code |
|---|---|
| `ReadPending`, `ReadDone`, `Publish` | `src/core/engine/worker.rs` `score_group` |
| `MarkReady` | `src/core/decisions/repository.rs` `mark_ready` |
| `Observe` | `src/core/cache/reader.rs` `follow_decisions` |
| `Claim` | `src/core/cache.rs` `claim_webhook` |
| `Post` | `src/core/engine.rs` `observe`, `webhook` |

Sync commit: `fb68f7da1baeab44e7ec4072fd71f08b8fb7a996`.
