import { useQuery } from "@tanstack/react-query";
import { flowsKey } from "../reads/flows.js";
import { useReads } from "./useData.js";

export const useFlows = () => useQuery({ queryKey: flowsKey(), enabled: useReads() });
