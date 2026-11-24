import { createRoot } from "react-dom/client";
import { initializeTelegram } from "./telegram";
import { AppShell } from "./app/shell";
import { MarketPage } from "./market-page";
import "./style.css";
void initializeTelegram().then((context) =>
  createRoot(document.getElementById("root")!).render(
    <AppShell context={context} market={<MarketPage context={context} />} />,
  ),
);
