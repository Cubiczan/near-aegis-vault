import { Account, JsonRpcProvider, KeyPairSigner } from "near-api-js";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export const NETWORK = "testnet";
export const RPC = process.env.NEAR_RPC ?? "https://rpc.testnet.fastnear.com";
export const VAULT = process.env.VAULT_ID ?? "vault.aegis-cfo-cz.testnet";
export const AGENT = process.env.AGENT_ID ?? "agent.aegis-cfo-cz.testnet";
export const OWNER = process.env.OWNER_ID ?? "aegis-cfo-cz.testnet";
export const ATTACKER = process.env.ATTACKER_ID ?? "aegis-attacker-cz.testnet";
export const provider = new JsonRpcProvider({ url: RPC });
export const explorerTx = (h: string) => `https://testnet.nearblocks.io/txns/${h}`;

/** Load a key from env (NEAR_SK_<ACCOUNT>) or ~/.near-credentials (near-cli legacy keychain). Never committed. */
export function loadSigner(accountId: string): KeyPairSigner {
  const envKey = process.env["NEAR_SK_" + accountId.replace(/[^a-z0-9]/gi, "_").toUpperCase()];
  if (envKey) return KeyPairSigner.fromSecretKey(envKey as any);
  const base = join(homedir(), ".near-credentials", NETWORK);
  const dir = join(base, accountId);
  const files = existsSync(dir) ? readdirSync(dir).map((f) => join(dir, f)) : [join(base, accountId + ".json")];
  for (const f of files) {
    if (!existsSync(f)) continue;
    const j = JSON.parse(readFileSync(f, "utf8"));
    return KeyPairSigner.fromSecretKey(j.private_key ?? j.secret_key);
  }
  throw new Error(`no key for ${accountId}; set NEAR_SK_... or run near-cli`);
}

export const account = (id: string) => new Account(id, provider, loadSigner(id));

export async function view<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  return (await provider.callFunction({ contractId: VAULT, method, args })) as T;
}

export interface CallOut { hash: string; ok: boolean; value?: any; error?: string }
export async function call(signerId: string, method: string, args: Record<string, unknown>): Promise<CallOut> {
  const acc = account(signerId);
  try {
    const r: any = await acc.signAndSendTransaction({
      receiverId: VAULT,
      actions: [(await import("near-api-js")).actions.functionCall(method, args, 50_000_000_000_000n, 0n)],
      throwOnFailure: false,
    });
    const hash = r.transaction_outcome?.id ?? r.transaction?.hash;
    const st = r.status ?? {};
    if (st.SuccessValue !== undefined) {
      const v = st.SuccessValue ? JSON.parse(Buffer.from(st.SuccessValue, "base64").toString()) : null;
      return { hash, ok: true, value: v };
    }
    return { hash, ok: false, error: JSON.stringify(st.Failure ?? st).slice(0, 300) };
  } catch (e: any) {
    return { hash: e?.context?.transactionHash ?? "", ok: false, error: String(e?.message ?? e).slice(0, 300) };
  }
}

export const yocto = (near: number | string) => {
  const [w, f = ""] = String(near).split(".");
  return (BigInt(w) * 10n ** 24n + BigInt((f + "0".repeat(24)).slice(0, 24))).toString();
};
export const fmt = (y: string) => (Number(BigInt(y) / 10n ** 21n) / 1000).toFixed(3) + " NEAR";
