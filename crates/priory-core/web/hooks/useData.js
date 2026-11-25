// What a page reads: one hook per listing and per detail. A query reads while a component that
// observes it is mounted, the person is signed in and the tab is shown.
import { useSyncExternalStore } from "react";
import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { keyOf } from "../queries.js";
import { session } from "../session.js";
import { shown } from "../shown.js";

export const useStore = (store) => useSyncExternalStore(store.subscribe, store.get, store.get);

export function useReads() {
  const { token } = useStore(session);
  return useStore(shown) && Boolean(token);
}

function useListing(name, wanted) {
  return useQuery({ queryKey: keyOf(name, wanted), enabled: useReads(), placeholderData: keepPreviousData });
}

export const useViews = () => useListing("views");
export const useLinks = () => useListing("links").data ?? NO_LINKS;
export const useDesk = () => useListing("desk");
export const useAsks = (wanted) => useListing("asks", wanted);
export const useQueue = () => useListing("queue");
export const useTasks = (wanted) => useListing("tasks", wanted);
export const useRuns = (wanted) => useListing("runs", wanted);
export const useReport = (wanted) => useListing("report", wanted);
export const useReports = () => useListing("reports");

/** A task as `/api/tasks/{id}` serves it. Another task starts empty. */
export const useTask = (id) => useQuery({ queryKey: ["task", id], enabled: useReads() });

/** What the cache holds of a task and of the tasks list, read by no one for this. */
export const useKeptTask = (id) => useQuery({ queryKey: ["task", id], enabled: false }).data;
export const useKeptTasks = () => useQuery({ queryKey: keyOf("tasks"), enabled: false }).data?.tasks;

/** Reads several listings for a legacy page, which draws once none of them is being read. */
export function useListings(names, wanted) {
  const enabled = useReads();
  const results = useQueries({ queries: names.map((name) => ({ queryKey: keyOf(name, wanted), enabled })) });
  return {
    reading: results.some((result) => result.fetchStatus === "fetching"),
    read: results.map((result) => `${result.status} ${result.dataUpdatedAt}`).join(),
  };
}

const NO_LINKS = [];
