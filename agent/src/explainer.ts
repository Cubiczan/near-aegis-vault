/** Deterministic, rule-based explainer (no LLM). Maps each on-chain rule code to a CFO-readable note + next action. */
export const EXPLAIN: Record<string, { label: string; why: string; action: string }> = {
  WITHIN_POLICY: { label: "Paid", why: "Approved vendor, under the approval threshold and today's cap.", action: "None. Funds sent on-chain." },
  NEEDS_APPROVAL: { label: "Held", why: "Large payment. Policy requires a human sign-off at or above the threshold.", action: "Owner reviews the invoice and calls approve or reject." },
  DAILY_CAP: { label: "Held", why: "Auto-pay budget for today is used up.", action: "Approve now, or let it wait until the cap resets." },
  LOW_BALANCE: { label: "Held", why: "Vault would drop below its reserve.", action: "Top up the vault, then approve." },
  DUPLICATE_INVOICE: { label: "Blocked", why: "This invoice number was already paid or queued. Classic duplicate-payment leak.", action: "None. Check vendor statement if it recurs." },
  PAYEE_NOT_ALLOWLISTED: { label: "Blocked", why: "Payee is not in the vendor master. Typical of fraud or prompt injection redirecting funds.", action: "Investigate. Add the vendor only after normal onboarding (add_vendor)." },
  OVER_PER_TX_CAP: { label: "Blocked", why: "Above the hard per-payment cap. The agent can never send this much, even with approval.", action: "Pay outside the agent, or raise the cap via set_policy." },
  VELOCITY_LIMIT: { label: "Blocked", why: "Too many payments in a short window. Looks like a loop or a hijacked agent.", action: "Pause the agent (set_agent) and review logs." },
  OWNER_APPROVED: { label: "Approved", why: "Owner signed off on a held payment.", action: "None." },
  OWNER_REJECTED: { label: "Rejected", why: "Owner declined a held payment.", action: "Notify vendor if needed." },
};
export const explain = (rule: string) => EXPLAIN[rule] ?? { label: rule, why: "Unknown rule.", action: "Review." };
