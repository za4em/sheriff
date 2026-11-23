#!/usr/bin/env bash
# Confirm the webhook model still describes the Rust, then model-check it.
set -euo pipefail

SKILL_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$SKILL_DIR/../../.." && pwd)
JAR="$SKILL_DIR/vendor/tla2tools-1.7.4.jar"
JAR_SHA=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88

die() {
  printf 'verify-sheriff-webhook: %s\n' "$*" >&2
  exit 1
}

[[ -f "$JAR" ]] || die "missing $JAR"
actual=$(shasum -a 256 "$JAR" | awk '{print $1}')
[[ "$actual" == "$JAR_SHA" ]] || die "tla2tools jar hash is $actual, want $JAR_SHA"

python3 - "$REPO_ROOT" <<'PY' || die "the model no longer describes the code, so this check does not pass"
import pathlib, sys
root = pathlib.Path(sys.argv[1])

def text(path):
    return (root / path).read_text()

cache = text("src/core/cache.rs")
claim = cache.split("fn claim_webhook", 1)
if len(claim) != 2 or '.arg("NX")' not in claim[1].split("fn ", 1)[0]:
    sys.exit("claim_webhook no longer uses SET NX. Two instances can both post the webhook.")

engine = text("src/core/engine.rs")
observe = engine.split("async fn observe", 1)[1].split("async fn claim_webhook", 1)[0]
if "finished.completed_now" not in observe or "claim_webhook" not in observe or "self.webhook(" not in observe:
    sys.exit("observe no longer posts only after completed_now and claim_webhook.")
if observe.find("claim_webhook") > observe.find("self.webhook("):
    sys.exit("observe posts the webhook before claim_webhook.")

ready = text("src/core/decisions/repository.rs").split("async fn mark_ready", 1)[1].split("async fn mark_failed", 1)[0]
if "AND status = $4" not in ready or "Status::Pending" not in ready:
    sys.exit("mark_ready no longer updates only a pending row.")

worker = text("src/core/engine/worker.rs").split("async fn score_group", 1)[1].split("async fn classify_windows", 1)[0]
if "Status::Pending" not in worker or "ready_decision" not in worker:
    sys.exit("worker.rs score_group no longer publishes completed_now from the pending check and mark_ready.")

reader = text("src/core/cache/reader.rs")
if "xread_options" not in reader:
    sys.exit("the ready reader no longer follows the stream on each instance.")
PY

run_tlc() {
  local config=$1
  local out=$2
  java -cp "$JAR" tlc2.TLC -workers 1 -config "$config" WebhookClaim.tla >"$out" 2>&1
}

cd "$SKILL_DIR"
faithful=$(mktemp)
broken=$(mktemp)
trap 'rm -f "$faithful" "$broken"' EXIT

if ! run_tlc WebhookClaim.cfg "$faithful"; then
  cat "$faithful" >&2
  die "the faithful model failed. Read the TLC result with /statechart before changing the model."
fi
if ! grep -q "No error has been found" "$faithful"; then
  cat "$faithful" >&2
  die "the faithful model did not report a clean pass"
fi

set +e
run_tlc WebhookClaimBroken.cfg "$broken"
broken_status=$?
set -e
if [[ "$broken_status" -eq 0 ]]; then
  cat "$broken" >&2
  die "the broken model passed. AtMostOneWebhook no longer catches a missing SET NX."
fi
if ! grep -q "AtMostOneWebhook" "$broken"; then
  cat "$broken" >&2
  die "the broken model failed without violating AtMostOneWebhook"
fi

printf 'webhook claim holds for one retained decision, two workers, and two instances\n'
printf 'a missing SET NX lets both instances post; the broken model reports that violation\n'
