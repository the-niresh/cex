import { Route, Routes } from "react-router-dom";
import { ExchangeProvider } from "./ExchangeContext";
import { PortfolioPage } from "./components/PortfolioPage";
import { TradeScreen } from "./TradeScreen";

export default function App() {
  return (
    <ExchangeProvider>
      <Routes>
        <Route path="/" element={<TradeScreen />} />
        <Route path="/portfolio" element={<PortfolioPage />} />
      </Routes>
    </ExchangeProvider>
  );
}
