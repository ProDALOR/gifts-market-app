import type { ComponentProps } from "react";
import type { Order } from "../market";
import { GiftCard } from "./gift-card";
import { orderKey } from "./order-activity";
import { VirtualGiftGrid } from "./virtual-gift-grid";
export function GiftGrid({
  orders,
  card,
  busy,
  visible,
}: {
  orders: Order[];
  card: (order: Order) => Omit<ComponentProps<typeof GiftCard>, "order">;
  busy: boolean;
  visible: (start: number, end: number, anchor?: number) => void;
}) {
  return (
    <VirtualGiftGrid
      orders={orders}
      keyOf={orderKey}
      busy={busy}
      visible={visible}
      className={orders[0]?.kind === "bid" ? "gift-grid-bids" : ""}
      label="Gift listings"
      render={(order, index) => (
        <GiftCard
          order={order}
          {...card(order)}
          position={index + 1}
          total={orders.length}
        />
      )}
    />
  );
}
