import { useQuery } from "@tanstack/react-query";
import { flows } from "../reads/flows.js";
import { useReads } from "./useData.js";

export const useFlows = () => useQuery({ ...flows(), enabled: useReads() });
