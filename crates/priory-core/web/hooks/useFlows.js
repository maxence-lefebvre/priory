import { useQuery } from "@tanstack/react-query";
import { flows } from "../reads/flows.js";
import { usePageReads } from "./useData.js";

export const useFlows = () => useQuery({ ...flows(), enabled: usePageReads() });
