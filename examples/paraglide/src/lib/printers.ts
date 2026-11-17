import { printers_status_offline, relay_discovered } from "./paraglide/messages.js";

export function statusText(offlineSince: string) {
  return printers_status_offline({ time: offlineSince });
}

export function discoveredToast(n: number) {
  return relay_discovered({ n });
}
