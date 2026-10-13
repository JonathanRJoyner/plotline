import type { BankPen } from "./events";

/** The bank's pens of `kind` ("" for all) whose kind and name hold every word of `query`, in
 * any case, grouped by model in the bank's order. */
export function filterBank(bank: BankPen[], kind: string, query: string): [string, BankPen[]][] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const groups = new Map<string, BankPen[]>();
  for (const p of bank) {
    const text = `${p.kind} ${p.name}`.toLowerCase();
    if ((kind && p.kind !== kind) || !words.every((w) => text.includes(w))) continue;
    groups.set(p.model, [...(groups.get(p.model) ?? []), p]);
  }
  return [...groups];
}
