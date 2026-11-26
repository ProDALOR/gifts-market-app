import type { TelegramContext } from "../telegram";
import { TelegramAccount } from "../telegram-ui";
import { BrandMark } from "../brand/brand-mark";
import { CurrencyPicker } from "../ui/pickers";
import { selectionAt } from "../market-selection";
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
          selection={selectionAt(location)}
        />
      </div>
    </div>
  );
}
