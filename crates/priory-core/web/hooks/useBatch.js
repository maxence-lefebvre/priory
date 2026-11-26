import { useQuery } from "@tanstack/react-query";
import { batch } from "../reads/batch.js";
import { usePageReads } from "./useData.js";

/** A batch, by its id or its planning task's key, or none for null. Another batch, or another go ask's revision, starts empty. */
export const useBatch = (which, version = null) => useQuery({ ...batch(which, version), enabled: usePageReads() && which !== null });
