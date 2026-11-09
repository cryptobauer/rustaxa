use super::*;

fn trace_nonce(index: usize) -> String {
    let mut digits = b"13407807929942597099574024998205846127479365820592393377723561443721764030073546976801874298166903427690031858186486050853753882811946569946433649006084096".to_vec();
    for _ in 0..index {
        for digit in digits.iter_mut().rev() {
            if *digit < b'9' {
                *digit += 1;
                break;
            }
            *digit = b'0';
        }
    }
    String::from_utf8(digits).expect("decimal nonce")
}

fn expected_case_requests(fixture: &Value, case_name: &str, rules: &Value) -> Result<Vec<Value>> {
    const NONCE: &str = "13407807929942597099574024998205846127479365820592393377723561443721764030073546976801874298166903427690031858186486050853753882811946569946433649006084096";
    let periods = fixture["periods"].as_array().context("periods")?;
    let mut base = Vec::new();
    for (period, row) in periods.iter().enumerate() {
        for (occurrence, tx) in row["transactions"]
            .as_array()
            .context("transactions")?
            .iter()
            .enumerate()
        {
            let target = tx["to"].as_str().unwrap_or("");
            let input = tx["input"].as_str().unwrap_or("");
            if (target != "00000000000000000000000000000000000000fe"
                && target != "00000000000000000000000000000000000000ee")
                || input.len() < 8
            {
                continue;
            }
            let name = format!(
                "{case_name}/p{}/t{occurrence}/native/{}",
                period + 1,
                tx["name"].as_str().context("transaction name")?
            );
            base.push(serde_json::json!({"name":name,"sender":tx["sender"],"to":tx["to"],"value":tx["value"],"gas_price":"1","input":tx["input"],"supplied_nonce":NONCE}));
        }
    }
    let generic = rules["generic_requests"]
        .as_array()
        .context("reviewed generic rules")?;
    let boundary = rules["boundary_requests"]
        .as_array()
        .context("reviewed boundary rules")?;
    ensure!(
        generic.len() == 14 && boundary.len() == 4,
        "variant rule shape"
    );
    base.extend(generic.iter().cloned());
    let mut selectors = std::collections::BTreeSet::new();
    let mut representatives = Vec::new();
    for request in &base {
        let input = request["input"].as_str().context("input")?;
        let selector = input.get(..8).context("selector")?;
        if selectors.insert(selector.to_owned()) {
            representatives.push(request.clone());
        }
    }
    let mut expected = Vec::new();
    for head in 0..=12 {
        let mut selected = base.clone();
        if head == 3 {
            for request in &representatives {
                let input = request["input"].as_str().context("input")?;
                let selector = input.get(..8).context("selector")?;
                let mut paid = request.clone();
                paid["name"] = Value::String(format!("boundary/value/{selector}"));
                paid["value"] = Value::String("1".into());
                selected.push(paid);
                if input.len() > 8 {
                    let mut malformed = request.clone();
                    malformed["name"] = Value::String(format!("boundary/selector_only/{selector}"));
                    malformed["value"] = Value::String("0".into());
                    malformed["input"] = Value::String(selector.to_owned());
                    selected.push(malformed);
                }
            }
        }
        if head == 3 || head == 7 {
            selected.push(boundary[0].clone());
        }
        if head == 3 {
            selected.extend(boundary[1..].iter().cloned());
        }
        for request in selected {
            let name = format!(
                "{head}/{}",
                request["name"].as_str().context("request name")?
            );
            expected.push(serde_json::json!({"name":name,"head":head,"request":request}));
        }
    }
    Ok(expected)
}

fn expected_interactions(fixture: &Value, rules: &Value) -> Result<Vec<Value>> {
    let mut specs = Vec::new();
    for period in fixture["periods"].as_array().context("periods")? {
        for tx in period["transactions"].as_array().context("transactions")? {
            let to = tx["to"].as_str().unwrap_or("");
            if (to == "00000000000000000000000000000000000000fe"
                || to == "00000000000000000000000000000000000000ee")
                && tx["input"].as_str().unwrap_or("").len() >= 8
            {
                specs.push((number(&period["number"]), tx));
            }
        }
    }
    let proofs = specs
        .iter()
        .filter(|(_, tx)| matches!(tx["name"].as_str(), Some("slash_a" | "slash_duplicate")))
        .map(|(_, tx)| *tx)
        .collect::<Vec<_>>();
    ensure!(
        proofs.len() == 2
            && proofs[0]["name"] == "slash_a"
            && proofs[1]["name"] == "slash_duplicate",
        "proof order"
    );
    let request = |tx: &Value, nonce: usize| serde_json::json!({"name":tx["name"],"sender":tx["sender"],"to":tx["to"],"value":tx["value"],"gas_price":"1","input":tx["input"],"supplied_nonce":trace_nonce(nonce)});
    let mut expected = Vec::new();
    for (index, tx) in proofs.iter().enumerate() {
        expected.push(serde_json::json!({"name":format!("case_proof/{index}"),"head":3,"block":4,"prefix":[],"request":request(tx, 0)}));
    }
    expected.push(serde_json::json!({"name":"case_proof/first_then_second","head":3,"block":4,"prefix":[request(proofs[0], 0)],"request":request(proofs[1], 1)}));
    let shared = rules["shared_trace_inputs"]
        .as_array()
        .context("shared trace rules")?;
    ensure!(shared.len() == 9, "shared trace rule count");
    let add_shared = |rows: &[Value], result: &mut Vec<Value>| -> Result<()> {
        for row in rows {
            let head = number(&row["head"]);
            result.push(serde_json::json!({"name":row["name"],"head":head,"block":head+1,"prefix":row["prefix"],"request":row["request"]}));
        }
        Ok(())
    };
    add_shared(&shared[..7], &mut expected)?;
    let mut selectors = std::collections::BTreeSet::new();
    for (period, tx) in specs {
        let input = tx["input"].as_str().context("nested input")?;
        let selector = input.get(..8).context("nested selector")?;
        let name = tx["name"].as_str().context("nested name")?;
        if !selectors.insert(selector.to_owned()) && name != "slash_a" && name != "slash_duplicate"
        {
            continue;
        }
        let sender: [u8; 20] = fixed(&tx["sender"]);
        let mut encoded = vec![0xf8, 88, 0x94];
        encoded.extend_from_slice(&sender);
        encoded.extend_from_slice(&[0xb8, 65, 1]);
        encoded.extend_from_slice(&[0; 64]);
        let hash = keccak256(&encoded);
        let wrapper = hex::encode(&hash.0[12..]);
        let mut runtime = hex::decode("366000600037600060003660003473")?;
        runtime.extend(hex::decode(string(&tx["to"]))?);
        runtime.extend(hex::decode("62061a80f15060006000fd")?);
        let len = u16::try_from(runtime.len())?.to_be_bytes();
        let mut init = vec![
            0x61, len[0], len[1], 0x61, 0, 15, 0x60, 0, 0x39, 0x61, len[0], len[1], 0x60, 0, 0xf3,
        ];
        init.extend(runtime);
        let deploy = serde_json::json!({"name":"deploy_parent","sender":tx["sender"],"to":null,"value":"0","gas_price":"1","input":hex::encode(init),"supplied_nonce":trace_nonce(0)});
        let nested_name = format!("{period}/{name}/{selector}");
        let target = serde_json::json!({"name":format!("nested_{nested_name}"),"sender":tx["sender"],"to":wrapper,"value":tx["value"],"gas_price":"1","input":tx["input"],"supplied_nonce":trace_nonce(1)});
        expected.push(serde_json::json!({"name":format!("nested_parent_revert/{nested_name}"),"head":3,"block":4,"prefix":[deploy],"request":target}));
    }
    add_shared(&shared[7..], &mut expected)?;
    Ok(expected)
}

pub(super) fn validate_case_packet(
    fixture: &Value,
    case_name: &str,
    api: &Value,
    cpp: &Value,
    roster: &Value,
    rules: &Value,
) -> Result<()> {
    ensure!(
        api["proof_case"] == case_name && roster["case"] == case_name,
        "case identity"
    );
    ensure!(
        api["configuration"] == fixture["configuration"] && api["registry"] == fixture["registry"],
        "configuration and registry"
    );
    let periods = fixture["periods"].as_array().context("fixture periods")?;
    ensure!(periods.len() == 12, "fixture period count");
    let roots = api["seed_roots"].as_object().context("seed roots")?;
    ensure!(
        roots.len() == 13 && roots.get("0") == Some(&fixture["genesis"]["root"]),
        "genesis root"
    );
    for (index, period) in periods.iter().enumerate() {
        let number = index + 1;
        ensure!(
            period["number"] == number as u64
                && roots.get(&number.to_string()) == Some(&period["final"]["root"]),
            "period root {number}"
        );
    }
    let cases = api["cases"].as_array().context("API cases")?;
    let independently_expected = expected_case_requests(fixture, case_name, rules)?;
    ensure!(
        cases.len() == independently_expected.len(),
        "fixture and variant request count"
    );
    for (row, want) in cases.iter().zip(&independently_expected) {
        ensure!(
            row["name"] == want["name"]
                && row["head"] == want["head"]
                && row["request"] == want["request"],
            "fixture and variant request order"
        );
    }
    let expected = roster["cases"]
        .as_array()
        .context("reviewed request roster")?;
    ensure!(cases.len() == expected.len(), "exact request count");
    let cpp_rows = cpp["cases"].as_array().context("C++ cases")?;
    let mut cpp_index = 0;
    let mut heads = [false; 13];
    let mut identities = std::collections::BTreeSet::new();
    for (row, want) in cases.iter().zip(expected) {
        ensure!(
            row["name"] == want["name"]
                && row["head"] == want["head"]
                && row["request"] == want["request"],
            "ordered request identity"
        );
        let name = row["name"].as_str().context("request name")?;
        ensure!(identities.insert(name.to_owned()), "duplicate request");
        let head = row["head"].as_u64().context("request head")? as usize;
        ensure!(head <= 12, "request head range");
        heads[head] = true;
        ensure!(row["call"]["gas"] == want["call_gas"], "call gas");
        let estimates = row["estimates"].as_array().context("estimates")?;
        let wanted_estimates = want["estimates"].as_array().context("estimate roster")?;
        ensure!(
            estimates.len() == 3 && estimates.len() == wanted_estimates.len(),
            "estimate count"
        );
        for (estimate, wanted) in estimates.iter().zip(wanted_estimates) {
            ensure!(
                estimate["name"] == wanted["name"] && estimate["cap"] == wanted["cap"],
                "estimate identity"
            );
            let probes = estimate["probes"].as_array().context("probes")?;
            let gases = wanted["probe_gases"]
                .as_array()
                .context("probe gas roster")?;
            ensure!(
                probes.len() == gases.len()
                    && probes.iter().zip(gases).all(|(p, g)| p["gas"] == *g),
                "ordered probe gases"
            );
            let cpp_row = cpp_rows.get(cpp_index).context("missing C++ row")?;
            ensure!(
                cpp_row["name"] == estimate["name"]
                    && cpp_row["occurrence"] == cpp_index as u64
                    && cpp_row["consumed"] == probes.len() as u64,
                "C++ row identity"
            );
            cpp_index += 1;
        }
        for field in ["trace", "trace_sequence"] {
            ensure!(
                row[field]["prefix"] == want[field]["prefix"]
                    && row[field]["request"] == want[field]["request"]
                    && want[field]["block"] == (head + 1) as u64,
                "trace input and block"
            );
            ensure!(
                row[field]["calls"].as_array().context("trace calls")?.len()
                    == row[field]["prefix"]
                        .as_array()
                        .context("trace prefix")?
                        .len()
                        + 1,
                "complete trace calls"
            );
            let prefix = row[field]["prefix"].as_array().context("trace prefix")?;
            let expected_length = if field == "trace" { 0 } else { 1 };
            ensure!(prefix.len() == expected_length, "trace prefix length");
            let mut request = row["request"].clone();
            if !prefix.is_empty() {
                ensure!(prefix[0] == request, "trace prefix request");
            }
            request["supplied_nonce"] = Value::String(trace_nonce(prefix.len()));
            ensure!(row[field]["request"] == request, "trace target request");
        }
    }
    ensure!(
        heads.iter().all(|present| *present) && cpp_index == cpp_rows.len(),
        "all heads and C++ rows"
    );
    let mut native_names = Vec::new();
    for (period_index, period) in periods.iter().enumerate() {
        for (occurrence, tx) in period["transactions"]
            .as_array()
            .context("transactions")?
            .iter()
            .enumerate()
        {
            let target = tx["to"].as_str().unwrap_or("");
            let input = tx["input"].as_str().unwrap_or("");
            if (target != "00000000000000000000000000000000000000fe"
                && target != "00000000000000000000000000000000000000ee")
                || input.len() < 8
            {
                continue;
            }
            let suffix = format!(
                "{case_name}/p{}/t{occurrence}/native/{}",
                period_index + 1,
                tx["name"].as_str().context("transaction name")?
            );
            for head in 0..=12 {
                let name = format!("{head}/{suffix}");
                native_names.push(name.clone());
                let row = cases
                    .iter()
                    .find(|row| row["name"] == name)
                    .context("missing native transaction occurrence")?;
                for key in ["sender", "to", "value", "input", "gas_price"] {
                    ensure!(
                        row["request"][key] == tx[key],
                        "native request {name} field {key}"
                    );
                }
            }
        }
    }
    native_names.sort();
    let mut actual_native = cases
        .iter()
        .filter_map(|row| row["name"].as_str())
        .filter(|name| name.contains("/native/"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    actual_native.sort();
    ensure!(
        actual_native == native_names,
        "exact native occurrence roster"
    );
    let extras = api["interacting_traces"]
        .as_array()
        .context("interacting traces")?;
    let wanted_extras = roster["interacting_traces"]
        .as_array()
        .context("interacting trace roster")?;
    let independently_expected = expected_interactions(fixture, rules)?;
    ensure!(
        wanted_extras == &independently_expected,
        "fixture and rules interacting roster"
    );
    ensure!(
        extras.len() == wanted_extras.len(),
        "interacting trace count"
    );
    for (row, want) in extras.iter().zip(wanted_extras) {
        ensure!(
            row["name"] == want["name"]
                && row["head"] == want["head"]
                && row["trace"]["prefix"] == want["prefix"]
                && row["trace"]["request"] == want["request"],
            "interacting trace identity"
        );
        ensure!(
            row["trace"].get("panic").is_none(),
            "interacting trace panic"
        );
        ensure!(
            row["trace"]["calls"]
                .as_array()
                .context("interacting calls")?
                .len()
                == row["trace"]["prefix"]
                    .as_array()
                    .context("interacting prefix")?
                    .len()
                    + 1,
            "complete interacting calls"
        );
        ensure!(
            want["block"] == row["head"].as_u64().context("interacting head")? + 1,
            "interacting block"
        );
        let prefix = row["trace"]["prefix"]
            .as_array()
            .context("interacting prefix")?;
        for (index, request) in prefix.iter().enumerate() {
            ensure!(
                request["supplied_nonce"] == trace_nonce(index),
                "interacting prefix nonce"
            );
        }
        ensure!(
            row["trace"]["request"]["supplied_nonce"] == trace_nonce(prefix.len()),
            "interacting target nonce"
        );
    }
    let proofs = periods[3]["transactions"]
        .as_array()
        .context("proof transactions")?
        .iter()
        .filter(|tx| matches!(tx["name"].as_str(), Some("slash_a" | "slash_duplicate")))
        .collect::<Vec<_>>();
    ensure!(
        proofs.len() == 2
            && proofs[0]["name"] == "slash_a"
            && proofs[1]["name"] == "slash_duplicate",
        "proof occurrences"
    );
    for (index, proof) in proofs.iter().enumerate() {
        let name = format!("case_proof/{index}");
        let row = extras
            .iter()
            .find(|row| row["name"] == name)
            .context("case proof trace")?;
        ensure!(
            row["trace"]["request"]["input"] == proof["input"]
                && row["trace"]["request"]["sender"] == proof["sender"],
            "case proof trace input"
        );
        let marker = format!("/{}/", proof["name"].as_str().context("proof name")?);
        let nested = extras
            .iter()
            .filter(|row| {
                row["name"].as_str().is_some_and(|name| {
                    name.starts_with("nested_parent_revert/") && name.contains(&marker)
                })
            })
            .collect::<Vec<_>>();
        ensure!(
            nested.len() == 1 && nested[0]["trace"]["request"]["input"] == proof["input"],
            "nested proof trace input"
        );
    }
    let pair = extras
        .iter()
        .find(|row| row["name"] == "case_proof/first_then_second")
        .context("ordered proof trace")?;
    ensure!(
        pair["trace"]["prefix"]
            .as_array()
            .context("proof prefix")?
            .len()
            == 1
            && pair["trace"]["prefix"][0]["input"] == proofs[0]["input"]
            && pair["trace"]["request"]["input"] == proofs[1]["input"],
        "ordered proof trace inputs"
    );
    for row in cases {
        for field in ["trace", "trace_sequence"] {
            ensure!(row[field].get("panic").is_none(), "trace panic");
        }
    }
    Ok(())
}

pub(super) fn mutation_controls(
    fixture: &Value,
    case_name: &str,
    api: &Value,
    cpp: &Value,
    roster: &Value,
    rules: &Value,
) -> Result<()> {
    for kind in 0..17 {
        let mut bad_api = api.clone();
        let mut bad_cpp = cpp.clone();
        let mut bad_roster = roster.clone();
        match kind {
            0 => bad_api["proof_case"] = Value::String("wrong_case".into()),
            1 => bad_api["seed_roots"]["0"] = Value::String("wrong_root".into()),
            2 => bad_api["cases"][0]["request"]["input"] = Value::String("wrong_input".into()),
            3 => {
                let rows = bad_api["cases"].as_array_mut().context("cases")?;
                let index = rows
                    .iter()
                    .position(|row| {
                        row["name"]
                            .as_str()
                            .is_some_and(|name| name.contains("/native/slash_duplicate"))
                    })
                    .context("second proof")?;
                rows.remove(index);
            }
            4 => {
                let rows = bad_api["cases"].as_array_mut().context("cases")?;
                let index = rows
                    .iter()
                    .position(|row| row["head"] == 12)
                    .context("head 12")?;
                rows.remove(index);
            }
            5 => {
                let rows = bad_api["cases"].as_array_mut().context("cases")?;
                rows[1] = rows[0].clone();
            }
            6 => {
                let rows = bad_api["cases"].as_array_mut().context("cases")?;
                rows.push(rows[0].clone());
            }
            7 => bad_api["cases"].as_array_mut().context("cases")?.swap(0, 1),
            8 => bad_cpp["cases"]
                .as_array_mut()
                .context("C++ cases")?
                .swap(0, 1),
            9 => bad_api["cases"][0]["trace"]["prefix"] = serde_json::json!([{}]),
            10 => {
                let probes = bad_api["cases"][0]["estimates"][0]["probes"]
                    .as_array_mut()
                    .context("probes")?;
                ensure!(!probes.is_empty(), "mutation control needs a probe");
                probes.pop();
            }
            11 => {
                let rows = bad_cpp["cases"].as_array_mut().context("C++ cases")?;
                rows[1] = rows[0].clone();
            }
            12 => {
                let rows = bad_cpp["cases"].as_array_mut().context("C++ cases")?;
                rows.push(rows[0].clone());
            }
            13 => {
                bad_cpp["cases"].as_array_mut().context("C++ cases")?.pop();
            }
            14 => {
                let api_rows = bad_api["interacting_traces"]
                    .as_array_mut()
                    .context("trace rows")?;
                let index = api_rows
                    .iter()
                    .position(|row| row["name"] == "rewarded_current_source_full")
                    .context("shared trace")?;
                api_rows.remove(index);
                bad_roster["interacting_traces"]
                    .as_array_mut()
                    .context("roster traces")?
                    .remove(index);
            }
            15 => {
                bad_api["interacting_traces"]
                    .as_array_mut()
                    .context("trace rows")?
                    .swap(0, 1);
                bad_roster["interacting_traces"]
                    .as_array_mut()
                    .context("roster traces")?
                    .swap(0, 1);
            }
            _ => {
                bad_api["interacting_traces"][3]["trace"]["request"]["input"] =
                    Value::String("00".into());
                bad_roster["interacting_traces"][3]["request"]["input"] =
                    Value::String("00".into());
            }
        }
        ensure!(
            validate_case_packet(fixture, case_name, &bad_api, &bad_cpp, &bad_roster, rules)
                .is_err(),
            "packet mutation {kind} must fail"
        );
    }
    Ok(())
}
