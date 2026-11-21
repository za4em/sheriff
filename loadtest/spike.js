import { postMessages } from "./lib/http.js";
import { spikeOptions } from "./lib/options.js";

// Burst of POST /v1/messages, then back to the base rate. The last 90 seconds
// are the recovery window: jobs accepted during the burst should finish then.
export const options = spikeOptions();

export default function () {
  postMessages();
}
