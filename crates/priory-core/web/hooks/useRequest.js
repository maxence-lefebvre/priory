import { useQuery } from "@tanstack/react-query";
import { request } from "../reads/request.js";
import { useReads } from "./useData.js";

export const useRequest = () => useQuery({ ...request(), enabled: useReads() });
