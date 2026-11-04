/** An outside attacker with its own funded account tries to drain the vault directly. All must fail on-chain. */
import { writeFileSync, mkdirSync } from "node:fs";
import { ATTACKER, call, explorerTx, yocto } from "./near.js";
const tries = [
  ["request_spend", { to: ATTACKER, amount: yocto("0.5"), memo: "drain", invoice_hash: "X-" + Date.now() }],
  ["approve", { id: "0" }],
  ["add_vendor", { account: ATTACKER }],
  ["set_agent", { agent: ATTACKER }],
  ["withdraw", { amount: yocto("1") }],
] as const;
const out: any[] = [];
for (const [m, a] of tries) {
  const r = await call(ATTACKER, m, a);
  const msg = (r.error ?? "").match(/(only the registered agent[^"\\]*|owner only)/)?.[1] ?? r.error;
  console.log(`${m.padEnd(14)} ${r.ok ? "!! SUCCEEDED" : "refused"}: ${msg}\n               ${explorerTx(r.hash)}`);
  out.push({ method: m, ok: r.ok, error: msg, tx: r.hash, explorer: explorerTx(r.hash) });
}
mkdirSync("out", { recursive: true });
writeFileSync("out/attacker-run.json", JSON.stringify(out, null, 2));

// Stolen agent key: even with the agent's real key, the protocol only allows request_spend on the vault.
import { AGENT, account, VAULT } from "./near.js";
import { actions } from "near-api-js";
const stolen = account(AGENT);
for (const [label, receiverId, act] of [
  ["agent key -> vault.approve", VAULT, actions.functionCall("approve", { id: "5" }, 30_000_000_000_000n, 0n)],
  ["agent key -> transfer to attacker", ATTACKER, actions.transfer(10n ** 23n)],
] as const) {
  try {
    await stolen.signAndSendTransaction({ receiverId, actions: [act as any] });
    console.log(`${label}: !! SUCCEEDED`);
  } catch (e: any) {
    console.log(`${label}: refused by protocol (function-call key): ${String(e?.message ?? e).slice(0, 140)}`);
    out.push({ method: label, ok: false, error: String(e?.message ?? e).slice(0, 200) });
  }
}
writeFileSync("out/attacker-run.json", JSON.stringify(out, null, 2));
