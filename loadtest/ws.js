import ws from "k6/ws";
import { check } from "k6";
import { Counter, Rate, Trend } from "k6/metrics";

import { apiKey, batchSize, envInt, messages, streamUrl } from "./lib/config.js";
import { dns } from "./lib/options.js";

const decisions = new Counter("decisions");
const decisionsMissing = new Counter("decisions_missing");
const decisionBad = new Rate("decision_bad");
const decisionLatency = new Trend("decision_latency", true);

const vus = envInt("VUS", 20);
const holdMs = envInt("HOLD_MS", 120000);
const intervalMs = envInt("INTERVAL_MS", 1000);
// Stop sending one score-timeout before close, so a healthy decision can land.
const drainMs = 5000;

export const options = {
  scenarios: {
    sockets: {
      executor: "per-vu-iterations",
      vus,
      iterations: 1,
      maxDuration: `${Math.ceil(holdMs / 1000) + 30}s`,
    },
  },
  dns,
  thresholds: {
    checks: ["rate>0.99"],
    decision_bad: ["rate<0.01"],
    decisions_missing: ["count==0"],
  },
};

export default function () {
  const sent = new Map();
  let sending = true;
  let seq = 0;
  const count = batchSize();

  const res = ws.connect(
    streamUrl(),
    { headers: { Authorization: `Bearer ${apiKey()}` } },
    (socket) => {
      socket.on("open", () => {
        send();
        socket.setInterval(send, intervalMs);
        socket.setTimeout(() => {
          sending = false;
        }, Math.max(holdMs - drainMs, 0));
        socket.setTimeout(() => socket.close(), holdMs);
      });

      socket.on("message", (data) => {
        let decision;
        try {
          decision = JSON.parse(data);
        } catch (_) {
          decisionBad.add(true);
          return;
        }
        if (!decision.message_id) {
          decisionBad.add(true);
          return;
        }
        const started = sent.get(decision.message_id);
        if (started === undefined) {
          decisionBad.add(true);
          return;
        }
        sent.delete(decision.message_id);
        const ok =
          decision.status === "ready" &&
          decision.action === "allow" &&
          decision.error === null;
        decisionBad.add(!ok);
        if (ok) {
          decisionLatency.add(Date.now() - started);
          decisions.add(1);
        }
      });

      socket.on("close", () => {
        decisionsMissing.add(sent.size);
      });

      function send() {
        if (!sending) {
          return;
        }
        const batch = messages(count, seq);
        seq += 1;
        const started = Date.now();
        for (let i = 0; i < batch.length; i++) {
          sent.set(batch[i].message_id, started);
        }
        socket.send(JSON.stringify(batch));
      }
    },
  );

  check(res, {
    "websocket upgrade": (r) => r && r.status === 101,
  });
}
