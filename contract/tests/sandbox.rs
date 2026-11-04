//! Sandbox (near-workspaces) tests: real neard sandbox, real transfers.
use near_workspaces::types::NearToken;
use serde_json::json;

const WASM: &str = "target/near/aegis_vault.wasm";

fn y(n: u128) -> String { (n * 10u128.pow(24)).to_string() }

#[tokio::test]
async fn full_flow_on_sandbox() -> anyhow::Result<()> {
    let w = near_workspaces::sandbox().await?;
    let wasm = std::fs::read(WASM).expect("run `cargo near build non-reproducible-wasm` first");
    let owner = w.dev_create_account().await?;
    let agent = w.dev_create_account().await?;
    let vendor = w.dev_create_account().await?;
    let attacker = w.dev_create_account().await?;
    let vault = w.dev_deploy(&wasm).await?;

    vault.call("new").args_json(json!({
        "owner": owner.id(), "agent": agent.id(),
        "policy": {"per_tx_cap": y(3), "approval_threshold": y(1), "daily_cap": y(2),
                   "max_requests_per_window": 10, "velocity_window_sec": 60},
        "allowlist": [vendor.id()]
    })).transact().await?.into_result()?;
    owner.call(vault.id(), "deposit").deposit(NearToken::from_near(5)).transact().await?.into_result()?;

    let before = vendor.view_account().await?.balance;
    let spend = |to: String, amt: u128, inv: &str| {
        let a = agent.clone(); let v = vault.id().clone(); let inv = inv.to_string();
        async move {
            let r = a.call(&v, "request_spend")
                .args_json(json!({"to": to, "amount": (amt / 10).to_string(), "memo": "test", "invoice_hash": inv}))
                .transact().await?.into_result()?;
            anyhow::Ok(r.json::<serde_json::Value>()?)
        }
    };
    // 0.5 NEAR → paid
    let d = spend(vendor.id().to_string(), 5 * 10u128.pow(24), "INV-1").await?;
    assert_eq!(d["outcome"], "Paid", "{d}");
    // duplicate
    let d = spend(vendor.id().to_string(), 5 * 10u128.pow(24), "INV-1").await?;
    assert_eq!(d["rule"], "DUPLICATE_INVOICE");
    // attacker payee
    let d = spend(attacker.id().to_string(), 5 * 10u128.pow(24), "INV-X").await?;
    assert_eq!(d["rule"], "PAYEE_NOT_ALLOWLISTED");
    // 1.5 NEAR → held
    let d = spend(vendor.id().to_string(), 15 * 10u128.pow(24), "INV-2").await?;
    assert_eq!(d["outcome"], "Held");
    // agent can't approve
    let r = agent.call(vault.id(), "approve").args_json(json!({"id": d["id"]})).transact().await?;
    assert!(r.is_failure());
    // stranger can't spend
    let r = attacker.call(vault.id(), "request_spend")
        .args_json(json!({"to": attacker.id(), "amount": y(1), "memo": "", "invoice_hash": "Z"})).transact().await?;
    assert!(r.is_failure());
    // owner approves
    let a: serde_json::Value = owner.call(vault.id(), "approve").args_json(json!({"id": d["id"]})).transact().await?.into_result()?.json()?;
    assert_eq!(a["outcome"], "Approved");

    let after = vendor.view_account().await?.balance;
    assert_eq!(after.as_yoctonear() - before.as_yoctonear(), 2 * 10u128.pow(24));
    let decisions: Vec<serde_json::Value> = vault.view("get_decisions").args_json(json!({})).await?.json()?;
    assert_eq!(decisions.len(), 5);
    Ok(())
}
