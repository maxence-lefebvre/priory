// Every page of the app. A port changes its own entry, never this list.
import { register } from "../registry.js";
import { asks } from "./asks.js";
import { batches } from "./batches.js";
import { costs } from "./costs.js";
import { desk } from "./desk.js";
import { flows } from "./flows.js";
import { inbox } from "./inbox.js";
import { memories } from "./memories.js";
import { more } from "./more.js";
import { queue } from "./queue.js";
import { reports } from "./reports.js";
import { runs } from "./runs.js";
import { settings } from "./settings.js";
import { tasks } from "./tasks.js";
import { terminal } from "./terminal.js";

register([terminal, costs, flows, settings, memories, tasks, runs, batches, desk, queue, inbox, reports, more, asks]);
