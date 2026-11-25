import { useQuery } from "@tanstack/react-query";
import { inboxKey } from "../reads/inbox.js";
import { useReads } from "./useData.js";

export const useInbox = () => useQuery({ queryKey: inboxKey(), enabled: useReads() });
