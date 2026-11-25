// Where the app stands, as the shell tells its pages: the address, the trail, the screen and the view
// file's layout.
import { createContext, useContext } from "react";

export const Place = createContext(null);
export const usePlace = () => useContext(Place);
