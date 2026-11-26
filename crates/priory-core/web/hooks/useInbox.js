import { useQuery } from "@tanstack/react-query";
import { inbox } from "../reads/inbox.js";
import { usePageReads } from "./useData.js";

export const useInbox = () => useQuery({ ...inbox(), enabled: usePageReads() });
