import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { TelegramContext } from "./telegram";
import { useSession } from "./app/session";
import { SharingController } from "./sharing";
export function useSharing(context: TelegramContext, scope: string) {
  const session = useSession();
  const publicUrl = session.config?.public_url || undefined;
  const controller = useMemo(
    () => new SharingController(context.app, publicUrl),
    [context.app, publicUrl],
  );
  const state = useSyncExternalStore(controller.subscribe, controller.current);
  useEffect(() => () => controller.cancel(), [controller, scope]);
  const available = !!(
    context.app?.switchInlineQuery || context.app?.openTelegramLink
  );
  return { ...state, available, share: controller.share };
}
export function ShareButton({
  sharing,
  click,
  label = "Share",
}: {
  sharing: ReturnType<typeof useSharing>;
  click: () => void;
  label?: string;
}) {
  return sharing.available ? (
    <button
      className="share-button"
      title={label}
      aria-label={label}
      disabled={sharing.busy}
      aria-busy={sharing.busy}
      onClick={click}
    >
      <svg
        aria-hidden="true"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
      >
        <path d="M13 5V2l8 7-8 7v-4c-6 0-9 3-10 7 0-8 4-13 10-14Z" />
      </svg>
    </button>
  ) : null;
}
export function ShareStatus({
  sharing,
}: {
  sharing: ReturnType<typeof useSharing>;
}) {
  return sharing.error ? (
    <p role="alert" className="share-error">
      {sharing.error}
    </p>
  ) : null;
}
