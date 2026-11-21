import http from "k6/http";
import { check } from "k6";
import { Counter } from "k6/metrics";

import { authHeaders, baseUrl, batchSize, messages } from "./config.js";

export const decisions = new Counter("decisions");
export const accepted = new Counter("accepted");

function params(endpoint) {
  return {
    headers: authHeaders(),
    timeout: "10s",
    tags: { endpoint, batch: String(batchSize()) },
  };
}

// MockProvider scores "hello" as allow. A batch shares one 5 second budget:
// if the whole batch is not ready, /v1/score returns 504 and this adds nothing.
function isReady(body, expected) {
  if (!Array.isArray(body) || body.length !== expected) {
    return false;
  }
  for (let i = 0; i < body.length; i++) {
    const decision = body[i];
    if (
      decision.status !== "ready" ||
      decision.action !== "allow" ||
      decision.error !== null
    ) {
      return false;
    }
  }
  return true;
}

export function postScore() {
  const count = batchSize();
  const res = http.post(
    `${baseUrl()}/v1/score`,
    JSON.stringify(messages(count)),
    params("score"),
  );
  let ready = false;
  if (res.status === 200) {
    try {
      ready = isReady(res.json(), count);
    } catch (_) {
      ready = false;
    }
  }
  check(res, {
    "score status is 200": (r) => r.status === 200,
    "score decisions are ready": () => ready,
  });
  if (ready) {
    decisions.add(count);
  }
}

// 202 means the rows were saved. The decision finishes later, on the queue.
export function postMessages() {
  const count = batchSize();
  const res = http.post(
    `${baseUrl()}/v1/messages`,
    JSON.stringify(messages(count)),
    params("messages"),
  );
  const ok = check(res, {
    "messages status is 202": (r) => r.status === 202,
  });
  if (ok) {
    accepted.add(count);
  }
}
