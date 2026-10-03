import { createContext, useContext } from "react";
import { useExchange, type Exchange } from "./useExchange";

const ExchangeContext = createContext<Exchange | null>(null);

export function ExchangeProvider({ children }: { children: React.ReactNode }) {
  const exchange = useExchange();
  return <ExchangeContext.Provider value={exchange}>{children}</ExchangeContext.Provider>;
}

export function useExchangeContext(): Exchange {
  const ctx = useContext(ExchangeContext);
  if (!ctx) throw new Error("useExchangeContext outside ExchangeProvider");
  return ctx;
}
