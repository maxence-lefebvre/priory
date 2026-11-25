import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { attentionKey, costsKey } from "../reads/costs.js";
import { useReads } from "./useData.js";

/** The spend split by `by`: the last split stays while another is read. */
export const useCosts = (by) => useQuery({ queryKey: costsKey(by), enabled: useReads(), placeholderData: keepPreviousData });
export const useAttention = () => useQuery({ queryKey: attentionKey(), enabled: useReads() });
