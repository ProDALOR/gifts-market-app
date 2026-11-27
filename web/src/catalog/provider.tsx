import { createContext, useContext, useEffect, useMemo } from "react";
import type { ReactNode } from "react";
import { CatalogController } from "./controller";

const Context = createContext<CatalogController | null>(null);
/** Keep bounded catalog data across tabs; the visible page owns its live work. */
export function CatalogProvider({ children }: { children: ReactNode }) {
  const controller = useMemo(() => new CatalogController(), []);
  useEffect(() => () => controller.close(), [controller]);
  return <Context.Provider value={controller}>{children}</Context.Provider>;
}
export const useSharedCatalog = () => useContext(Context);
