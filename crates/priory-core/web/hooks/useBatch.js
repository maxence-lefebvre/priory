import { useQuery } from "@tanstack/react-query";
import { batchKey } from "../reads/batch.js";
import { useReads } from "./useData.js";

/** A batch, by its id or its planning task's key. Another batch, or another go ask's revision, starts empty. */
export const useBatch = (which, version = null) => useQuery({ queryKey: batchKey(which, version), enabled: useReads() });
