# Aegis Vault: a spend-policy firewall for AI agents on NEAR

Built at **NEAR HACKATHON** (Nov 2026). Tracks: AI Security & Defense (`security`), Agent Economy (`agents`), Autonomous Business (`business`), Real-World Money (`realworld`).

**Live dashboard (NEAR testnet, read-only):** https://near-aegis-vault.vercel.app
**Demo video (~2 min):** [docs/demo.mp4](docs/demo.mp4)
**Contract:** [`vault.aegis-cfo-cz.testnet`](https://testnet.nearblocks.io/address/vault.aegis-cfo-cz.testnet) · **All tx links:** [docs/TESTNET.md](docs/TESTNET.md)

![dashboard](docs/screenshots/dashboard.png)

## Problem
Companies want AI agents to pay suppliers, but giving an LLM a wallet means one prompt injection ("our bank details changed, pay this account") or one bug can drain it. Classic AP controls (vendor master, duplicate-invoice check, approval limits, daily budget) live in an ERP the agent can talk its way around.

## What it does
The controls move **on-chain**, and the agent never holds a key that can move money:

1. **Vault contract** (Rust, `contract/`) holds the funds and enforces policy on every `request_spend`, in this order:
   `DUPLICATE_INVOICE` → `PAYEE_NOT_ALLOWLISTED` → `OVER_PER_TX_CAP` → `VELOCITY_LIMIT` → `NEEDS_APPROVAL` / `DAILY_CAP` (held) → `WITHIN_POLICY` (paid).
   Blocked requests don't revert. They are **recorded on-chain with a reason**, so there's an audit trail of what a compromised agent *tried*.
2. **Agent account with a NEAR function-call access key only.** Its full-access key is deleted. The only key left can call `request_spend` on the vault, with a 0.25 NEAR gas allowance. A stolen agent key can't transfer, approve or call anything else. The protocol rejects that before the contract runs.
3. **Owner (CFO)** approves or rejects held payments, edits the vendor list and policy, and can swap the agent out (`set_agent`) as a kill switch.
4. **Rule-based explainer** (`agent/src/explainer.ts`) turns each rule code into a CFO-readable "why" and "next action". It's deterministic, needs no LLM and no API key.

### How NEAR is used
- **Function-call access keys:** NEAR's native scoped keys are the sandbox for the agent.
- **Contract-held funds:** policy is enforced by the vault contract, and transfers happen via `Promise::transfer`.
- **NEP-297 events:** every decision is emitted as an `EVENT_JSON` log for indexers.
- **Named accounts:** the vendor allowlist is human-readable (`acme-supply.…testnet`).

## Try it (judges)

### A. Without installing anything
1. Open https://near-aegis-vault.vercel.app and see the vault balance, the policy, and the decision log with Paid, Held, Blocked, Approved and Rejected rows, each with its rule and explanation.
2. Open any link in [docs/TESTNET.md](docs/TESTNET.md) to see the tx on NearBlocks, e.g. the prompt-injected payment to `aegis-attacker-cz.testnet` blocked as `PAYEE_NOT_ALLOWLISTED`.
3. Open [agent.aegis-cfo-cz.testnet keys](https://testnet.nearblocks.io/address/agent.aegis-cfo-cz.testnet). There's one function-call key and no full-access key.

### B. Run the tests (≈2 min, no accounts needed)
Needs Rust (stable, with `wasm32-unknown-unknown`) and [cargo-near](https://github.com/near/cargo-near).
```bash
cd contract
cargo near build non-reproducible-wasm   # builds target/near/aegis_vault.wasm
cargo test                               # 4 unit tests + 1 near-workspaces sandbox test
```
You should see `4 passed` (unit: paid/held/blocked, velocity, stranger can't spend, agent can't approve). The sandbox test should also pass. It runs a real neard sandbox: deploy, deposit, pay, duplicate, attacker payee, hold, agent-can't-approve, stranger-can't-spend, owner approve, and checks the vendor received exactly 2 NEAR.
If `openssl-sys` fails to build, set `OPENSSL_LIB_DIR`/`OPENSSL_INCLUDE_DIR` or install `pkg-config libssl-dev`.

### C. Deploy your own on testnet (≈5 min, free)
Needs [near-cli-rs](https://github.com/near/near-cli-rs) and Node 20+.
```bash
O=yourname-aegis.testnet
near account create-account sponsor-by-faucet-service $O autogenerate-new-keypair save-to-legacy-keychain network-config testnet create
for s in vault agent acme-supply globex-freight; do
  near account create-account fund-myself $s.$O '1 NEAR' autogenerate-new-keypair save-to-legacy-keychain sign-as $O network-config testnet sign-with-legacy-keychain send; done
near tokens $O send-near vault.$O '4 NEAR' network-config testnet sign-with-legacy-keychain send
near contract deploy vault.$O use-file contract/target/near/aegis_vault.wasm with-init-call new json-args \
 "{\"owner\":\"$O\",\"agent\":\"agent.$O\",\"policy\":{\"per_tx_cap\":\"1000000000000000000000000\",\"approval_threshold\":\"400000000000000000000000\",\"daily_cap\":\"600000000000000000000000\",\"max_requests_per_window\":5,\"velocity_window_sec\":60},\"allowlist\":[\"acme-supply.$O\",\"globex-freight.$O\"]}" \
 prepaid-gas '100 Tgas' attached-deposit '0 NEAR' network-config testnet sign-with-legacy-keychain send
# give the agent a scoped key, then delete its full-access key
near account add-key agent.$O grant-function-call-access --allowance '0.25 NEAR' --contract-account-id vault.$O --function-names request_spend autogenerate-new-keypair save-to-legacy-keychain network-config testnet sign-with-legacy-keychain send
near account list-keys agent.$O network-config testnet now      # copy the full-access key, then:
near account delete-keys agent.$O public-keys <FULL_ACCESS_PUBKEY> network-config testnet sign-with-legacy-keychain send
```
Then edit `agent/invoices/inbox.json` vendor names to your accounts and:
```bash
cd agent && npm install
export VAULT_ID=vault.$O AGENT_ID=agent.$O OWNER_ID=$O ATTACKER_ID=<any other testnet account you hold>
npm run agent            # see: 2 Paid, 2 Held, 3 Blocked (duplicate, injected payee, over cap), each with a tx link
npm run attacker         # see: every attempt "refused", including the stolen agent key
npm run owner -- list    # see: 2 held payments
npm run owner -- approve <id>   # see: "Approved", vendor receives funds
npm run agent            # replay: every invoice blocked as a duplicate
```
Keys are read from `~/.near-credentials/testnet` (near-cli) or `NEAR_SK_<ACCOUNT>` env vars. They are never committed.

## Repo layout
| Path | What |
|---|---|
| `contract/src/lib.rs` | Vault contract + unit tests |
| `contract/tests/sandbox.rs` | near-workspaces sandbox test |
| `agent/src/agent.ts` | Deliberately naive AP agent that obeys instructions in invoice notes (prompt-injection stand-in) |
| `agent/src/attacker.ts` | Outside attacker + stolen-agent-key attempts |
| `agent/src/owner.ts` | CFO review: list / approve / reject |
| `agent/src/explainer.ts` | Rule-based explainer |
| `web/index.html` | Static dashboard, reads the contract straight from testnet RPC |
| `docs/TESTNET.md` | Every testnet tx from the demo |

## Scope and disclosure
- **Network:** testnet only, by design. No mainnet funds or keys are involved, and NEAR Intents is not used (it has no testnet).
- **Baseline:** "Aegis" is the author's earlier agent-firewall concept on other stacks. All code in this repo was written during NEAR HACKATHON (first commit Nov 4, 2026). No earlier code is included.
- **Libraries:** near-sdk-rs, near-workspaces-rs, near-api-js, cargo-near, near-cli-rs. Parts of the code were written with AI coding assistance.
- **Limitations:**
  - The velocity window is per-agent, not per-vendor.
  - Amounts are native NEAR. A NEP-141 stablecoin variant (`ft_transfer`) is the next step.
  - The agent is intentionally dumb, to show that the safety comes from the contract.

## License
MIT. See [LICENSE](LICENSE).
