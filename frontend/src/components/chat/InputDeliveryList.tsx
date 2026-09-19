import type { InputDelivery } from "../../lib/chatInput";
import "./chatActivity.css";

const labels: Record<InputDelivery["delivery"], string> = {
  saving: "Saving", waiting: "Waiting for delivery", sent: "Accepted by current run",
  queued: "Queued", uncertain: "Delivery uncertain", failed: "Not delivered",
};

export function InputDeliveryList({ receipts, deliveredRequestIds = [] }: { receipts: InputDelivery[]; deliveredRequestIds?: string[] }) {
  // BlockList renders every accepted receipt in the transcript (including
  // restored receipts). Keep that data; only remove its duplicate delivery UI.
  const delivered = new Set(deliveredRequestIds);
  const pending = receipts.filter(receipt => receipt.delivery !== "sent" &&
    !(receipt.delivery === "queued" && (receipt.runStartedAt || delivered.has(receipt.requestId))));
  if (!pending.length) return null;
  return (
    <div className="chat-input-deliveries" aria-label="Message delivery">
      <ol className="chat-input-deliveries__list">
        {pending.map(receipt => (
          <li key={receipt.requestId} className="chat-input-delivery">
            <span className="chat-input-delivery__status" data-delivery={receipt.delivery}>{labels[receipt.delivery]}</span>
            <div>
              <p>{receipt.message}</p>
              {receipt.note && receipt.delivery !== "queued" && <p className="chat-input-delivery__note">{receipt.note}</p>}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
