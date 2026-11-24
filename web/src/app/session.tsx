import { createContext, useContext } from "react";
import type { ReactNode } from "react";
import type { TelegramContext } from "../telegram";
import { useTelegram } from "../telegram-ui";
const Context = createContext<ReturnType<typeof useTelegram> | null>(null);
export function SessionProvider({
  context,
  children,
}: {
  context: TelegramContext;
  children: ReactNode;
}) {
  const session = useTelegram(context);
  return <Context.Provider value={session}>{children}</Context.Provider>;
}
/** Standalone previews/tests use the same boundary without duplicating authentication in the app. */
export function SessionBoundary({
  context,
  children,
}: {
  context: TelegramContext;
  children: ReactNode;
}) {
  const shared = useContext(Context);
  return shared ? (
    children
  ) : (
    <SessionProvider context={context}>{children}</SessionProvider>
  );
}
export function useSession() {
  const value = useContext(Context);
  if (!value) throw new Error("Missing application session");
  return value;
}
