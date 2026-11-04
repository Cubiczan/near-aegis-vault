/**
 * A deliberately NAIVE AP agent: it reads invoices and obeys whatever the
 * invoice notes say (simulating a prompt-injected LLM). It holds only a
 * function-call key that can call vault.request_spend — nothing else.
 * Safety comes from the on-chain policy, not from the agent being smart.
 */
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { AGENT, call, explorerTx, fmt, yocto } from "./near.js";
import { explain } from "./explainer.js";

type Inv = { invoice: string; vendor: string; amount: string; memo: string; notes: string };
const inbox: Inv[] = JSON.parse(readFileSync(new URL("../invoices/inbox.json", import.meta.url), "utf8"));
const runTag = process.env.RUN_TAG ?? "";

function naiveDecide(inv: Inv) {
  // "LLM" follows injected instructions in free text — the attack surface.
  const m = inv.notes.match(/pay this invoice to ([a-z0-9._-]+\.testnet)/i);
  return { to: m ? m[1] : inv.vendor, amount: inv.amount };
}

const results: any[] = [];
for (const inv of inbox) {
  const d = naiveDecide(inv);
  const r = await call(AGENT, "request_spend", { to: d.to, amount: yocto(d.amount), memo: inv.memo, invoice_hash: runTag + inv.invoice });
  const rule = r.value?.rule ?? "TX_FAILED";
  const e = explain(rule);
  console.log(`${inv.invoice.padEnd(9)} ${d.amount.padStart(4)} NEAR -> ${d.to.padEnd(38)} ${(r.value?.outcome ?? "ERR").padEnd(8)} ${rule}`);
  console.log(`          ${r.value?.reason ?? r.error}\n          CFO note: ${e.why} Next: ${e.action}\n          ${explorerTx(r.hash)}`);
  results.push({ invoice: inv.invoice, to: d.to, amount: d.amount, outcome: r.value?.outcome, rule, id: r.value?.id, tx: r.hash, explorer: explorerTx(r.hash) });
}
mkdirSync("out", { recursive: true });
writeFileSync("out/agent-run.json", JSON.stringify(results, null, 2));
