import type { TelegramContext } from "../telegram";
import { TelegramAccount } from "../telegram-ui";
import { BrandMark } from "../brand/brand-mark";
import { CurrencyPicker } from "../ui/pickers";
import { selectionAt } from "../market-selection";
import { pathRoute, portfolioUser } from "./routes";
import { useSession } from "./session";

/** Market and Portfolio share the same compact currency/profile row. */
export function AppHeader({
  context,
  currencies,
  currency,
  changeCurrency,
  accountOpen,
  setAccountOpen,
}: {
  context: TelegramContext;
  currencies: readonly string[];
  currency: string;
  changeCurrency: (currency: string) => void;
  accountOpen: boolean;
  setAccountOpen: (open: boolean) => void;
}) {
  const telegram = useSession();
  const selection = selectionAt(location);
  const route = pathRoute(location.pathname);
  selection.set("currency", currency);
  if (route.page === "portfolio") {
    const user = route.user ?? portfolioUser(telegram.identity?.user.id);
    selection.set("page", "portfolio");
    selection.set("user", user ? String(user) : "");
  }
  return (
    <div className="sidebar-heading">
      <BrandMark animated />
      <div className="header-actions">
        <CurrencyPicker
          currencies={currencies}
          value={currency}
          change={changeCurrency}
        />
        <TelegramAccount
          telegram={telegram}
          context={context}
          open={accountOpen}
          setOpen={setAccountOpen}
          selection={selection}
        />
      </div>
    </div>
  );
}
