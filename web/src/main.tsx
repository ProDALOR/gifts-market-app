import { createRoot } from "react-dom/client";
import { initializeTelegram } from "./telegram";
import { initialLocation } from "./app/routes";
import { selectionFromParams, selectionAt } from "./market-selection";
import { AppShell } from "./app/shell";
import { MarketPage } from "./market-page";
import { LoadingScreen } from "./app/loading-screen";
import "./style.css";
import { applyAppearance } from "./app/appearance";
applyAppearance();
const root = createRoot(document.getElementById("root")!);
let controller: AbortController | undefined;
async function start() {
  controller?.abort();
  const current = (controller = new AbortController());
  root.render(<LoadingScreen />);
  try {
    const context = await initializeTelegram();
    const selected = initialLocation(
      context,
      new URL(location.href),
      history.state,
    );
    if (selected.route.page === "market")
      selectionFromParams(selectionAt(new URL(selected.href, location.origin)));
    if (current.signal.aborted) return;
    root.render(
      <AppShell context={context} market={<MarketPage context={context} />} />,
    );
  } catch (error) {
    if (!current.signal.aborted)
      root.render(
        <main className="workspace">
          <div role="alert">
            <p>
              {error instanceof Error ? error.message : "Could not open market"}
            </p>
            <button onClick={() => void start()}>Retry</button>
          </div>
        </main>,
      );
  }
}
void start();
