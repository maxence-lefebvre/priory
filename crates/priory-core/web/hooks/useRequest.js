import { useQuery } from "@tanstack/react-query";
import { requestKey } from "../reads/request.js";
import { useReads } from "./useData.js";

export const useRequest = () => useQuery({ queryKey: requestKey(), enabled: useReads() });
