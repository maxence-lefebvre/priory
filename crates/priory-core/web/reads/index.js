// The reads beside the listings, by the first part of their key.
import { batch } from "./batch.js";
import { attention, costs } from "./costs.js";
import { flows } from "./flows.js";
import { inbox } from "./inbox.js";
import { request } from "./request.js";

export const READS = Object.fromEntries([batch, costs, attention, flows, inbox, request].map((each) => [each.name, each]));
