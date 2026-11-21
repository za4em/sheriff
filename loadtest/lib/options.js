import { envInt } from "./config.js";

// Spread connections across app replicas. Docker DNS returns every replica
// of `app`; localhost has one address, so this is a no-op there.
const dns = {
  ttl: "5s",
  select: "roundRobin",
};

// RATE is requests per second. Decisions per second are RATE * BATCH when
// every request is accepted. PRE_VUS and MAX_VUS scale with RATE because
// /v1/score holds the caller for up to 5 seconds.
export function arrivalOptions() {
  const rate = envInt("RATE", 20);
  const preAllocatedVUs = envInt("PRE_VUS", Math.max(rate, 10));
  const maxVUs = envInt("MAX_VUS", Math.max(rate * 6, preAllocatedVUs));
  return {
    scenarios: {
      load: {
        executor: "constant-arrival-rate",
        rate,
        timeUnit: "1s",
        duration: __ENV.DURATION || "2m",
        preAllocatedVUs,
        maxVUs,
      },
    },
    dns,
    thresholds: {
      checks: ["rate>0.99"],
      http_req_failed: ["rate<0.01"],
    },
  };
}

export function spikeOptions() {
  const base = envInt("BASE_RATE", 10);
  const spike = envInt("SPIKE_RATE", 100);
  if (spike < base) {
    throw new Error("SPIKE_RATE must be at least BASE_RATE");
  }
  const preAllocatedVUs = envInt("PRE_VUS", spike);
  const maxVUs = envInt("MAX_VUS", Math.max(spike * 4, preAllocatedVUs));
  return {
    scenarios: {
      spike: {
        executor: "ramping-arrival-rate",
        startRate: base,
        timeUnit: "1s",
        preAllocatedVUs,
        maxVUs,
        stages: [
          { target: base, duration: "30s" },
          { target: spike, duration: "15s" },
          { target: spike, duration: "20s" },
          { target: base, duration: "20s" },
          { target: base, duration: "90s" },
        ],
      },
    },
    dns,
    thresholds: {
      checks: ["rate>0.99"],
      http_req_failed: ["rate<0.01"],
    },
  };
}

export { dns };
