//! Lossless decoding of source-authenticated account code observations.
use super::*;
use std::collections::BTreeMap;

fn accounts(
    value: Option<&mut Value>,
    visit: &mut impl FnMut(&mut Value) -> Result<()>,
) -> Result<()> {
    if let Some(value) = value
        && !value.is_null()
    {
        for row in value.as_array_mut().context("code observation accounts")? {
            visit(row)?;
        }
    }
    Ok(())
}

fn transactions(
    value: Option<&mut Value>,
    visit: &mut impl FnMut(&mut Value) -> Result<()>,
) -> Result<()> {
    if let Some(value) = value
        && !value.is_null()
    {
        for tx in value
            .as_array_mut()
            .context("code observation transactions")?
        {
            if let Some(observer) = tx.get_mut("observer")
                && let Some(catalog) = observer.get_mut("catalog")
            {
                accounts(catalog.get_mut("accounts"), visit)?;
            }
        }
    }
    Ok(())
}

fn walk(document: &mut Value, visit: &mut impl FnMut(&mut Value) -> Result<()>) -> Result<()> {
    for field in ["genesis", "seed", "pre_reward"] {
        if let Some(state) = document.get_mut(field) {
            accounts(state.get_mut("accounts"), visit)?;
            for field in [
                "native_catalog",
                "declared_projection_catalog",
                "declared_donor_catalog",
            ] {
                if let Some(catalog) = state.get_mut(field) {
                    accounts(catalog.get_mut("accounts"), visit)?;
                }
            }
            if let Some(donor) = state.get_mut("donor_document") {
                walk(donor, visit)?;
            }
        }
    }
    if let Some(rows) = document
        .get_mut("pending_reward_facts")
        .and_then(Value::as_array_mut)
    {
        for row in rows {
            transactions(row.get_mut("transactions"), visit)?;
        }
    }
    if let Some(periods) = document.get_mut("periods").and_then(Value::as_array_mut) {
        for period in periods {
            if let Some(state) = period.get_mut("final") {
                accounts(state.get_mut("accounts"), visit)?;
                if let Some(catalog) = state.get_mut("native_catalog") {
                    accounts(catalog.get_mut("accounts"), visit)?;
                }
            }
            transactions(period.get_mut("transactions"), visit)?;
            if let Some(input) = period.get_mut("reward_input") {
                transactions(input.get_mut("transactions"), visit)?;
            }
            if let Some(parts) = period
                .get_mut("distributions")
                .and_then(Value::as_array_mut)
            {
                for part in parts {
                    if let Some(input) = part.get_mut("input") {
                        transactions(input.get_mut("transactions"), visit)?;
                    }
                }
            }
        }
    }
    if let Some(cases) = document.get_mut("api_cases").and_then(Value::as_array_mut) {
        for case in cases {
            for name in ["trace", "trace_sequence"] {
                if let Some(calls) = case
                    .get_mut(name)
                    .and_then(|trace| trace.get_mut("calls"))
                    .and_then(Value::as_array_mut)
                {
                    for call in calls {
                        for phase in ["before", "after"] {
                            if let Some(state) = call.get_mut(phase) {
                                accounts(state.get_mut("accounts"), visit)?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn decode(document: &mut Value) -> Result<()> {
    const FIELDS: [&str; 4] = [
        "code_objects_schema",
        "code_objects",
        "code_objects_count",
        "code_objects_sha256",
    ];
    let mut donor = document
        .get("seed")
        .and_then(|seed| seed.get("donor_document"));
    let mut depth = 0;
    while let Some(value) = donor {
        depth += 1;
        ensure!(
            depth <= 3 && FIELDS.iter().all(|field| value.get(*field).is_none()),
            "nested code metadata or donor depth"
        );
        donor = value
            .get("seed")
            .and_then(|seed| seed.get("donor_document"));
    }
    let encoded = document.get("code_objects_schema").is_some();
    if !encoded {
        ensure!(
            FIELDS.iter().all(|field| document.get(*field).is_none()),
            "partial code object metadata"
        );
        return walk(document, &mut |row| {
            ensure!(
                row.get("code_object").is_none(),
                "code reference without codec"
            );
            Ok(())
        });
    }
    ensure!(document["code_objects_schema"] == 1, "code object schema");
    let mut pool = BTreeMap::new();
    for (hash, code) in document["code_objects"]
        .as_object()
        .context("complete code object pool")?
    {
        let code = code.as_str().context("code object bytes")?;
        let raw = hex::decode(code)?;
        ensure!(
            hex::encode(&raw) == code
                && !raw.is_empty()
                && hex::encode(revm::primitives::keccak256(&raw)) == *hash,
            "code object hash/encoding"
        );
        pool.insert(hash.clone(), code.to_owned());
    }
    ensure!(
        document["code_objects_count"]
            .as_u64()
            .context("code object count type")?
            == pool.len() as u64,
        "code object count"
    );
    let canonical = serde_json::to_vec(&pool)?;
    let hash = revm::precompile::hash::sha256_run(&canonical, u64::MAX)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ensure!(
        Some(hex::encode(hash.bytes).as_str()) == document["code_objects_sha256"].as_str(),
        "code object pool digest"
    );
    let mut used = BTreeSet::new();
    walk(document, &mut |row| {
        if let Some(reference) = row.get("code_object") {
            ensure!(
                row.get("code").is_none(),
                "conflicting code and code object"
            );
            let reference = reference.as_str().context("code object reference")?;
            ensure!(
                row["code_hash"] == reference,
                "account code hash/reference identity"
            );
            let code = pool.get(reference).context("missing code object")?;
            ensure!(
                row["code_size"]
                    .as_u64()
                    .context("account code size type")?
                    == (code.len() / 2) as u64,
                "account code size/reference identity"
            );
            used.insert(reference.to_owned());
            let code = code.clone();
            row.as_object_mut()
                .context("code observation row")?
                .remove("code_object");
            row["code"] = Value::String(code);
        } else if let Some(code) = row.get("code") {
            ensure!(
                code.as_str() == Some(""),
                "nonempty code outside encoded pool"
            );
        }
        Ok(())
    })?;
    ensure!(
        used == pool.keys().cloned().collect(),
        "unused or omitted code object"
    );
    let object = document.as_object_mut().context("coded document")?;
    for field in [
        "code_objects_schema",
        "code_objects",
        "code_objects_count",
        "code_objects_sha256",
    ] {
        object.remove(field);
    }
    Ok(())
}
