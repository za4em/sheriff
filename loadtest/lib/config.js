// Environment and request bodies shared by the k6 scripts.

export function baseUrl() {
  const raw = __ENV.BASE_URL || "http://localhost:3000";
  return raw.replace(/\/$/, "");
}

export function streamUrl() {
  return `${baseUrl().replace(/^http/, "ws")}/v1/stream`;
}

export function apiKey() {
  return __ENV.API_KEY || "dev-key";
}

export function envInt(name, fallback) {
  const raw = __ENV[name];
  if (raw === undefined || raw === "") {
    return fallback;
  }
  const value = Number(raw);
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${name} must be a positive number, got ${raw}`);
  }
  return value;
}

export function batchSize() {
  return envInt("BATCH", 1);
}

// One room per VU. /v1/score does not keep the rows, so that room stays empty.
// /v1/messages keeps them, so the room window fills to 50 and stays there.
// `seq` distinguishes sends that share one VU iteration. The websocket script
// holds a single iteration open and sends many times; HTTP gets a new __ITER.
export function messages(count, seq) {
  const now = Math.floor(Date.now() / 1000);
  const room = `room-${__VU}`;
  const user = `user-${__VU}`;
  const stamp = seq === undefined ? "" : `-${seq}`;
  const batch = [];
  for (let i = 0; i < count; i++) {
    batch.push({
      message_id: `${__VU}-${__ITER}-${i}${stamp}`,
      room_id: room,
      user_id: user,
      text: "hello",
      timestamp: now,
    });
  }
  return batch;
}

export function authHeaders() {
  return {
    Authorization: `Bearer ${apiKey()}`,
    "Content-Type": "application/json",
  };
}
