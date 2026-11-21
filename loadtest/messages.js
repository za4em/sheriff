import { postMessages } from "./lib/http.js";
import { arrivalOptions } from "./lib/options.js";

export const options = arrivalOptions();

export default function () {
  postMessages();
}
