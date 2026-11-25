// Where the app stands, as the shell tells its pages: the address, the trail, the layout and the
// listings the page shows.
import { createContext, useContext } from "react";

export const Place = createContext(null);
export const usePlace = () => useContext(Place);
