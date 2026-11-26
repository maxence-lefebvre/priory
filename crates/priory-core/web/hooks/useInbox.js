import { useQuery } from "@tanstack/react-query";
import { inbox } from "../reads/inbox.js";
import { useReads } from "./useData.js";

export const useInbox = () => useQuery({ ...inbox(), enabled: useReads() });
