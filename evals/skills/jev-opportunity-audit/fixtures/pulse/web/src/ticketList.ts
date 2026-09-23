import { escapeHtml, formatRelativeTime } from "./format";

export type QueueName = "billing" | "shipping" | "account" | "other";

export interface Ticket {
  id: string;
  subject: string;
  customerName: string;
  queue: QueueName;
  urgency: number;
  unread: boolean;
  updatedAt: string; // ISO 8601, always UTC
}

export type SortKey = "urgency" | "updated" | "customer";

export interface ListOptions {
  sortBy: SortKey;
  queue?: QueueName;
  unreadOnly?: boolean;
}

const QUEUE_LABELS: Record<QueueName, string> = {
  billing: "Billing",
  shipping: "Shipping",
  account: "Account",
  other: "Other",
};

const collator = new Intl.Collator(undefined, { sensitivity: "base" });

function updatedAtMillis(ticket: Ticket): number {
  const parsed = Date.parse(ticket.updatedAt);
  // A ticket with an unparsable timestamp sorts to the bottom rather than
  // throwing; the list is the only way an agent can reach it to fix it.
  return Number.isNaN(parsed) ? 0 : parsed;
}

export function sortTickets(tickets: readonly Ticket[], sortBy: SortKey): Ticket[] {
  const sorted = [...tickets];

  switch (sortBy) {
    case "urgency":
      // Ties broken by recency so the order is stable between polls.
      sorted.sort(
        (a, b) => b.urgency - a.urgency || updatedAtMillis(b) - updatedAtMillis(a),
      );
      break;
    case "updated":
      sorted.sort((a, b) => updatedAtMillis(b) - updatedAtMillis(a));
      break;
    case "customer":
      sorted.sort(
        (a, b) =>
          collator.compare(a.customerName, b.customerName) ||
          updatedAtMillis(b) - updatedAtMillis(a),
      );
      break;
  }

  return sorted;
}

export function filterTickets(
  tickets: readonly Ticket[],
  options: ListOptions,
): Ticket[] {
  return tickets.filter((ticket) => {
    if (options.queue && ticket.queue !== options.queue) return false;
    if (options.unreadOnly && !ticket.unread) return false;
    return true;
  });
}

function renderRow(ticket: Ticket, now: Date): string {
  const classes = ["ticket-row"];
  if (ticket.unread) classes.push("ticket-row--unread");

  return [
    `<li class="${classes.join(" ")}" data-ticket-id="${escapeHtml(ticket.id)}">`,
    `<span class="ticket-row__queue">${QUEUE_LABELS[ticket.queue]}</span>`,
    `<span class="ticket-row__subject">${escapeHtml(ticket.subject)}</span>`,
    `<span class="ticket-row__customer">${escapeHtml(ticket.customerName)}</span>`,
    `<time class="ticket-row__time" datetime="${escapeHtml(ticket.updatedAt)}">`,
    formatRelativeTime(new Date(ticket.updatedAt), now),
    `</time>`,
    `</li>`,
  ].join("");
}

export function renderTicketList(
  tickets: readonly Ticket[],
  options: ListOptions,
  now: Date = new Date(),
): string {
  const visible = sortTickets(filterTickets(tickets, options), options.sortBy);

  if (visible.length === 0) {
    return `<p class="ticket-list__empty">Nothing here. Enjoy it while it lasts.</p>`;
  }

  const rows = visible.map((ticket) => renderRow(ticket, now)).join("");
  return `<ul class="ticket-list" data-sort="${options.sortBy}">${rows}</ul>`;
}
