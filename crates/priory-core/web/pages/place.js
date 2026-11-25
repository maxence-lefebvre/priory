// Where the app stands, as the shell tells its pages: the address, the trail, the layout and the
// listings the page shows.
import { createContext, useContext } from "react";

export const Place = createContext(null);
export const usePlace = () => useContext(Place);

/** The class of the element a page's list draws in. A phone gives it no box: one tree fits every size, so a rotation keeps its elements. */
export const listClass = (size, name) => (size === "compact" ? "flat" : name);
