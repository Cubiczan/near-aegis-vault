/** Owner (CFO) reviews held payments: `npm run owner -- list | approve <id> | reject <id>` */
import { OWNER, call, explorerTx, fmt, view } from "./near.js";
const [cmd = "list", id] = process.argv.slice(2);
if (cmd === "list") {
  const p: any[] = await view("get_pending");
  for (const x of p) console.log(`#${x.id} ${fmt(x.amount)} -> ${x.to}  ${x.memo}\n    ${x.reason}`);
  if (!p.length) console.log("No held payments.");
} else {
  const r = await call(OWNER, cmd, { id });
  console.log(r.ok ? `${r.value.outcome}: ${r.value.reason}` : `failed: ${r.error}`, "\n", explorerTx(r.hash));
}
