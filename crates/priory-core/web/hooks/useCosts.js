import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { attention } from "../reads/attention.js";
import { costs } from "../reads/costs.js";
import { useReads } from "./useData.js";

/** The spend split by `by`: the last split stays while another is read. */
export const useCosts = (by) => useQuery({ ...costs(by), enabled: useReads(), placeholderData: keepPreviousData });
export const useAttention = () => useQuery({ ...attention(), enabled: useReads() });
