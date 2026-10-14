//! Sealed diagnostic VRF windows. Authenticate fixed raw authority and all71 prior
//! proofs before constructing47 requests. Each profile has fixed vote-row indices;
//! no caller can supply a voter, root, key or ceiling. Failures qualify no prefix.
//! Embedded22 source pins require the separately reviewed complete74-source freeze.
use crate::{
    artifact_admission as admission, interval_history, native_witness as native,
    parent_input_handoff, parent_reward_fields as fields, paths, vrf_witness as first,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteDiagnosticPlan, ConcreteDiagnosticTuple, ConcreteVrfPlan, ConcreteVrfResidual49f8Plan,
    ConcreteVrfResidualPlan, ConcreteVrfVoterPlan, ConcreteWitnessLimits, ConcreteWitnessLogical,
    ConcreteWitnessPhysical, capture_vrf_residual_49f8_run, capture_vrf_residual_run,
    capture_vrf_voter_run, replay_vrf_bundle, replay_vrf_residual_49f8_bundle,
    replay_vrf_residual_bundle, replay_vrf_voter_bundle,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
pub const REPORT_CAP: usize = 131072;
const FIRST: u64 = 25706894;
const PRIOR_PATH: &str =
    "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d.rcv1";
const PRIOR_BYTES: usize = 417594;
const PRIOR_SHA: &str = "6382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61";
/// Only crate-owned fixed entrypoints can select a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Profile {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    Sixth,
    Seventh,
    Eighth,
    Ninth,
    Tenth,
    ResidualFirst,
    ResidualB92a,
    ResidualE50b,
    Residual49f8,
}
const RESIDUAL_VOTER: &str = "03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b";
const RESIDUAL_PROFILE: &str = "n4_residual_vrf_03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b_v1";
const RESIDUAL_KEY: &str = "89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b";
const RESIDUAL_INDICES: [usize; 46] = [
    62, 119, 179, 236, 344, 401, 461, 521, 578, 638, 695, 749, 806, 866, 920, 977, 1034, 1091,
    1151, 1211, 1271, 1328, 1382, 1430, 1487, 1547, 1604, 1661, 1715, 1772, 1832, 1889, 1943, 1997,
    2057, 2114, 2171, 2231, 2285, 2345, 2399, 2459, 2516, 2576, 2624, 2684,
];
const B92A_VOTER: &str = "b92a29912b08b4fa7404b02dfa2b8a971abdeda1";
const B92A_PROFILE: &str = "n4_residual_vrf_b92a29912b08b4fa7404b02dfa2b8a971abdeda1_v1";
const B92A_KEY: &str = "92f670429dc4076aa7771488f1512225a3bbf93eb17aa2e37a8cedcd463fff65";
const B92A_INDICES: [usize; 46] = [
    95, 155, 215, 269, 320, 377, 437, 497, 557, 614, 671, 728, 782, 842, 899, 953, 1010, 1067,
    1127, 1187, 1247, 1307, 1358, 1409, 1463, 1523, 1580, 1637, 1694, 1751, 1808, 1865, 1919, 1973,
    2033, 2147, 2207, 2261, 2321, 2378, 2435, 2492, 2552, 2603, 2660, 2720,
];
// Exact ordered roots/references from the accepted b92a46 candidate, SHA256
// 1bd797ee59d36855cd52fdf80109c984d90f3a67059af2dbfe63b80952f0483e.
const B92A_BINDINGS: &str = r#"[{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706902,"vote_hash_hex":"45fc6437af94116f3e383d5751289f8fdf2bd8939e5197288f5ececdc0ebef23"}]},{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706903,"vote_hash_hex":"b20267ef04b7ab7a4e3d22fd929c63d304bab66140939f873ed938818b3afa13"}]},{"root":"b07e806de23f0a4b16b6cef287118ab09699f32a0a4d8ffea3d436c9d53782ce","refs":[{"period":25706904,"vote_hash_hex":"ce80468963b29a10d6f7e4dc5131a5ee012ae6e288acbc08a252cc621dba572f"}]},{"root":"8c7fdbb2b39c01255ead8379e312bc79508558e1d1bd0a4dbe2a570a48fbf08b","refs":[{"period":25706905,"vote_hash_hex":"94c95f96f3969948d5f584d3901ce03761f6c0570ab915488b75f8b86feab201"}]},{"root":"a5fe6be45511742ec6a5b7c508a447507cb34dcfa321beedc81d5ad5e4aa0f8b","refs":[{"period":25706906,"vote_hash_hex":"7d2a7ad0b5fb7639529f39a9f3981a8c0a9088e4d89998f5f008ed76a5d94fdc"}]},{"root":"1b8815f01250eeeec908fd7d9093322f21909ae6ee234c9151109f063de9fc38","refs":[{"period":25706907,"vote_hash_hex":"981ee0e73b10368a7bc0ba5bae5de9ffcd0184f6516714c146eccd865b44ea5b"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706908,"vote_hash_hex":"a36c879ec8206be4c6b6e9b48ec553934992e1e4eee253c779ff4533b443d1a1"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706909,"vote_hash_hex":"c1524b050103c76e146cafb29dc9457489fb9cc7bc04ef73d71540cd49253b07"}]},{"root":"5315842400d2c61336a33774e4ddd92eb90cfa19da449dab6c2e7202adbd2821","refs":[{"period":25706910,"vote_hash_hex":"3da96df308f97fc8092c7a27361f7759566bda4abaa7c5a30feedd1fc56c421a"}]},{"root":"48240ffcddb5897ff34093c83b3ec9a9460d47290ba211fefae036fd83a107c2","refs":[{"period":25706911,"vote_hash_hex":"21de1cc8e9af0dae8e7a5f5d8931dae5c1cd33e8e1785a1d71a91350f925157d"}]},{"root":"e70dd5ea2193dc4f38ab3d1ae8e8faad7988a0579716d15f79058649973d7964","refs":[{"period":25706912,"vote_hash_hex":"b5b5fefe6a51c81d41b9fa6496659c5fdc8455d4744464798ee5f766b543987d"}]},{"root":"e8f84241116c5f485fa97046d7ed79f8776f3705bf586b3bb17b81b008a1e9a4","refs":[{"period":25706913,"vote_hash_hex":"665ad3935e24a9b9f51f9fa770baf88c6a76316d936fccd10db9ac6362b44b65"}]},{"root":"8f0c15bca3c202dad12c44f060e074748572ade6a1dba6dfea967b4d47b2a808","refs":[{"period":25706914,"vote_hash_hex":"60bc32f41159c15641bd083c9a68b5a236acd86d0c399305766878962ba77d7f"}]},{"root":"5986c863eaab74a42da8642f740f7296d6f326443f4bb883a59c65675c31c56a","refs":[{"period":25706915,"vote_hash_hex":"58ec1150705de0153dd1ea9d3ea508dbc4ce2721d1464caa6df08cec5b299c33"}]},{"root":"750e6dfcd464b29c1430473ea8338056169734f66c5e213da1cd9eba7566961e","refs":[{"period":25706916,"vote_hash_hex":"8ce2b9f4dc6db4c277050c8c151db0f756bea39faf3246de0f9d04ad1fbb913e"}]},{"root":"6389f33e77403f0f92fd654c208b18355f2da19895e08498f9f0aebc61da2075","refs":[{"period":25706917,"vote_hash_hex":"398b5d25c8c1a6e8561ae5b5a0869374f052a95d586df348fc1df0afcbd9ec94"}]},{"root":"3ce52115126f3ff130560bbf477f389159ad89eb7cc543abe1eda1ba38e74366","refs":[{"period":25706918,"vote_hash_hex":"f93fc28637beb185ea0d67adaa1910160c2ac406d8e317f167d8059c46db2d0b"}]},{"root":"97f12663ea38b8ab1e88572ec16cb992a969529c83b0c5d7e7f19dbec317844c","refs":[{"period":25706919,"vote_hash_hex":"d8924c476fa17fb4c84e5cd838d1422aa99ba356bb5afd757b9110b1ab9e9821"}]},{"root":"76f9d6edc45e54c2f705c93b57a9ddc8025240c2678258767a4618477446c59d","refs":[{"period":25706920,"vote_hash_hex":"a5c573ba710ecaeb0100c55c79374aa81688b4ad6b5b11c7218c2110d077ab4b"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706921,"vote_hash_hex":"1e2a8465b2729bf67864c801218e17e1965caf40195d42cc607fc69de810bac4"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706922,"vote_hash_hex":"86e4861d82da9055c725404c00942b697b70f10afb2e8d137cc57a1515cf2158"}]},{"root":"8fc69b3a7343817f4c55e8ab7e90680896c45c5bf03d4fd0aa0075c749a6a223","refs":[{"period":25706923,"vote_hash_hex":"e6d779bec73bd83e897036ef3c248b5a9fabd188d06352122641fef1dae6e7cf"}]},{"root":"131bcb5151b5e52bbace2653d3a44edf700db6b6337e82a04d70c6a1695ce803","refs":[{"period":25706924,"vote_hash_hex":"63a7eea394adc7c8e6c6a25a56e2ebf565fb005cb93cb5200b802ff72961ef11"}]},{"root":"4a54404b443f071ae60a07b74dcc601ab375da61f81531e1f7b143fa3c8df222","refs":[{"period":25706925,"vote_hash_hex":"0d76bd0d5c72365843a31577ed8bcd79d6632942abfb37e6d145bb6e7317184d"}]},{"root":"715127dfc215df7d43d63b58cd58a4446edb425bf2e23502932e293ef021d4fa","refs":[{"period":25706926,"vote_hash_hex":"1a1b358854ce38e6c542895af42bb5a1f01967e5354b877491ac57ed484971ab"}]},{"root":"04c3266846de8285c05f021bc6fa8730c4ddecd4db10b82c4f9007c8e821d282","refs":[{"period":25706927,"vote_hash_hex":"e59addfd35287f90a38bff16a89cf1d65125b53db5db5454352818c56e159799"}]},{"root":"827e4d2929e8871824a36220b4a59bd75f33f0d2da55294a9ff128c4697671d2","refs":[{"period":25706928,"vote_hash_hex":"532344048a6b7a5e930be1855d6a6a81b85f323314d2ab132f85707f400d999c"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706929,"vote_hash_hex":"676438daceaa6262aee46519b4e47cb46b95fd603165f25a7b7a8423a6212fc7"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706930,"vote_hash_hex":"65f83d680e79fc1fa1a7fefddf197e17750c03e6d6623a44dbb6c3271558405e"}]},{"root":"60b89dcf9e9ecbb1c038b381d144fabda53064e6b46a002d9deeece69dad3697","refs":[{"period":25706931,"vote_hash_hex":"3609ebff3bb626375969c71e6b1914b740535627268c188720cf84b26af96faf"}]},{"root":"f4c00f1fe89e446c63aa25d8fd80d138fb0eedfba6bb1ce51b63cd2ef2d0dd56","refs":[{"period":25706932,"vote_hash_hex":"c5ffc8f4b8b41f13fcbefb14709729ae2a9c92f8a62d363cc4fc9291bc35eb91"}]},{"root":"9109390dc21e981e3ef34a44fb06359ccff8d9f5da9785ad769d4c451c629853","refs":[{"period":25706933,"vote_hash_hex":"e94e5bf0fc36f220de800140eb6c495eb7d714fd3298910b2d16fe1b04270e9d"}]},{"root":"1d9242c2aa421de9d4e5ef8126ab5ecf1ba49f53a72ef19f3243463bb21bc486","refs":[{"period":25706934,"vote_hash_hex":"b7dafef2a117bd29c9895d7fdf826d4ec54432cc8e8dba84402437e9e2b31b73"}]},{"root":"ece3968a30c0b8dedd5792e99d88c426bcff348385f281035eafb80b5f000315","refs":[{"period":25706935,"vote_hash_hex":"8c737db634c1e8fbf07c468c2abb6e375af2733db6d0a4ec030dff2e0bc62f18"}]},{"root":"64a4a559c1911b6d98e270d7c17805db3deaf3f4961f9773c3f98ccd136dd139","refs":[{"period":25706936,"vote_hash_hex":"5a2cd44ec134e7f650437ffbd13a74d8f9737163698a993fdf08d0ddceab3f54"}]},{"root":"b918017816a1b0ebc4dd3e43bbbb6e0490b9e5c94d33aed4d0cd84359b840145","refs":[{"period":25706938,"vote_hash_hex":"89f719d0a22af5b4ec656f7dab1b43caf580e906bbd25809e62d0e020deaf769"}]},{"root":"54a7068396b9d331e9bf9d3815ae26a96517e2af3596f7d4cf6a004ed5652252","refs":[{"period":25706939,"vote_hash_hex":"4dfb3f9bc2e33344b96bce92935e53d2898512784970cccd0be6f05f630a5ea4"}]},{"root":"3fedb74070694d46637ea9fb65e70060f9ce8ae264a033dc1404872e0ab339ac","refs":[{"period":25706940,"vote_hash_hex":"1a297bd1c8b51fd3b62e2ef9f2cd21a5d32bf4d6495d459d39c72f0d2f120198"}]},{"root":"51fdb8fa9573a32977862bddf804d4d211ef4a46c242a284d697033c85e1d0c7","refs":[{"period":25706941,"vote_hash_hex":"9e6c7628f77016ef4d6bd87f0efc1014c33ed8e0314886375f7841ef6490fcfb"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706942,"vote_hash_hex":"c3e8446c8c07b16b4598aee81b02d4f4d031dbaf20855de0ea1d12a51b0399f4"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706943,"vote_hash_hex":"a797ed1ac1ff92b3272a732b47dadf3a3c3fff7f6e18099a73bbbb66c3ba01a8"}]},{"root":"b5cee51b7dc57bc68bd2cd5144daefcfca83686170bf226cf2297795371a2f83","refs":[{"period":25706944,"vote_hash_hex":"ec40bc65c27ffccaa6199686fe44a1dea3ab6a297f62cd671b19d928658fb283"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706945,"vote_hash_hex":"2e004a6684a310411abbb28025918e199c5325f6c6ea02360ef59392acb363de"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706946,"vote_hash_hex":"70611a9dcd7ae66dd4ec8575a978506d2fcc720f08870ede0220c5c80a966f08"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706947,"vote_hash_hex":"068df2cf1da56de33ef63ca4e369d4a2b0ccb8f7b9fea64f360651c8601ee0f0"}]},{"root":"4a31ec5fbb426cd88c29ed42b7428aae41951bb9ca4a617073df5a9e3312a624","refs":[{"period":25706948,"vote_hash_hex":"8a21dc7087cf407353d9e8114a8060e6a692402dadfef0fc97c46964344b4426"}]}]"#;
const E50B_VOTER: &str = "e50b5452b2e8435404dbe06e6a05410c47b7583d";
const E50B_PROFILE: &str = "n4_residual_vrf_e50b5452b2e8435404dbe06e6a05410c47b7583d_v1";
const E50B_KEY: &str = "97c2d7a6a0dc3ae78ab1fe9147bd5db10088db1c01850d9b8bce75ae287361d2";
const E50B_INDICES: [usize; 46] = [
    107, 167, 227, 281, 332, 389, 449, 509, 569, 626, 683, 794, 854, 911, 965, 1022, 1079, 1139,
    1199, 1259, 1319, 1370, 1421, 1475, 1535, 1592, 1649, 1706, 1763, 1820, 1877, 1931, 1985, 2045,
    2102, 2159, 2219, 2273, 2333, 2390, 2447, 2504, 2564, 2615, 2672, 2732,
];
// Fixed candidate41913/SHA9f3fa64dda8af3daf4a11671b685c953daf294ed62f81b24183776f08f8a0dee.
const E50B_BINDINGS: &str = r#"[{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706902,"vote_hash_hex":"0f26edc30522bf84dabbb78041e2093007aa86f50311c27b94f50c9b091749e1"}]},{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706903,"vote_hash_hex":"ae803b1baf9e10887d6c709c62be7c58ba8dfbf3a3ab715279c5396c34228ec6"}]},{"root":"b07e806de23f0a4b16b6cef287118ab09699f32a0a4d8ffea3d436c9d53782ce","refs":[{"period":25706904,"vote_hash_hex":"4cf553536022cf87b617c2b38442bd03adfb369d3ba148cea08cd79d73319722"}]},{"root":"8c7fdbb2b39c01255ead8379e312bc79508558e1d1bd0a4dbe2a570a48fbf08b","refs":[{"period":25706905,"vote_hash_hex":"62d9ac8b9a91198dbc6b2a865710facd320b04bb727237d3c09a95decac3ed3f"}]},{"root":"a5fe6be45511742ec6a5b7c508a447507cb34dcfa321beedc81d5ad5e4aa0f8b","refs":[{"period":25706906,"vote_hash_hex":"8f13c6b9ed55f442b167e0c7040891949a762819054d014a6c5a932a819aae00"}]},{"root":"1b8815f01250eeeec908fd7d9093322f21909ae6ee234c9151109f063de9fc38","refs":[{"period":25706907,"vote_hash_hex":"c6bdff2a26596fcd118a73fe7087e6a9b40ff0b93b0b5fd87eb8ce940581e0e1"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706908,"vote_hash_hex":"cd518cb646001e15f24e527cfc63e27455f30ca455bc6e4512a409ea30ee78bf"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706909,"vote_hash_hex":"6359cbeaff8189f8f6e5fde72a45b5918afe8920add599e1154d11ff9f78478f"}]},{"root":"5315842400d2c61336a33774e4ddd92eb90cfa19da449dab6c2e7202adbd2821","refs":[{"period":25706910,"vote_hash_hex":"642fce221893daf55ef85d208d5168503dadfaf251f479157ab1250c25e37075"}]},{"root":"48240ffcddb5897ff34093c83b3ec9a9460d47290ba211fefae036fd83a107c2","refs":[{"period":25706911,"vote_hash_hex":"981a1923e9c4bc56faea69f5773a01e10056e04ca088d2078608093aaa56b061"}]},{"root":"e70dd5ea2193dc4f38ab3d1ae8e8faad7988a0579716d15f79058649973d7964","refs":[{"period":25706912,"vote_hash_hex":"e49d5fd59145813bd219590ebe388e83b67daba8dddee20be842eff9d043feb1"}]},{"root":"8f0c15bca3c202dad12c44f060e074748572ade6a1dba6dfea967b4d47b2a808","refs":[{"period":25706914,"vote_hash_hex":"6017616d1643c33731440dcff2b59717123dcc9dcbcfb8bebb1a6a21c09e1566"}]},{"root":"5986c863eaab74a42da8642f740f7296d6f326443f4bb883a59c65675c31c56a","refs":[{"period":25706915,"vote_hash_hex":"3c25ba761749af9dd627c289808aa6e36198bf969cb3ff3d46f20e64766950dc"}]},{"root":"750e6dfcd464b29c1430473ea8338056169734f66c5e213da1cd9eba7566961e","refs":[{"period":25706916,"vote_hash_hex":"f2a98dd9eb64038205a5b7e39a655e72bb134d8739df7b47548930786a484833"}]},{"root":"6389f33e77403f0f92fd654c208b18355f2da19895e08498f9f0aebc61da2075","refs":[{"period":25706917,"vote_hash_hex":"9163d3672acf8d4cb9a42a7eef05101251ed03e0f9653983a998799c0d64337e"}]},{"root":"3ce52115126f3ff130560bbf477f389159ad89eb7cc543abe1eda1ba38e74366","refs":[{"period":25706918,"vote_hash_hex":"8f46be036d34ddc9a22d6788924a721a257f7e1c12c77d4c82373ea84b5fb708"}]},{"root":"97f12663ea38b8ab1e88572ec16cb992a969529c83b0c5d7e7f19dbec317844c","refs":[{"period":25706919,"vote_hash_hex":"4d90afcea05b93765b5d99bc7f393cdf067b326affb06e12191eb5f42effb8f0"}]},{"root":"76f9d6edc45e54c2f705c93b57a9ddc8025240c2678258767a4618477446c59d","refs":[{"period":25706920,"vote_hash_hex":"23bef124d08790d808f3c7e4cdedbdc886543ea4874b36af1fea4a0b1b422225"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706921,"vote_hash_hex":"1318cbd0918850ad947b0129d718bd449f27a8055a02b16eaeb2184e0d0bcbc1"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706922,"vote_hash_hex":"21d3229b91909e67f1524ace36964a9cffceb3d9992c689da026c31d20bd8d7f"}]},{"root":"8fc69b3a7343817f4c55e8ab7e90680896c45c5bf03d4fd0aa0075c749a6a223","refs":[{"period":25706923,"vote_hash_hex":"2c60de71bce34102968507146cc0c3c87dbdfd214f7e6443eb761e08afa678d6"}]},{"root":"131bcb5151b5e52bbace2653d3a44edf700db6b6337e82a04d70c6a1695ce803","refs":[{"period":25706924,"vote_hash_hex":"64dc5ebb1bc74302f790f4a2daa4b66763fab3720dea58b6840415a0310a3cc9"}]},{"root":"4a54404b443f071ae60a07b74dcc601ab375da61f81531e1f7b143fa3c8df222","refs":[{"period":25706925,"vote_hash_hex":"051b2f3adf684e36e53a05a61678e086ee1d39d6a3435a870b56ea0e8db3789a"}]},{"root":"715127dfc215df7d43d63b58cd58a4446edb425bf2e23502932e293ef021d4fa","refs":[{"period":25706926,"vote_hash_hex":"60032d055373a8ba7dbc5d01b3e4ace85aa1a84a6c5d129d7fb363ac97c6d54f"}]},{"root":"04c3266846de8285c05f021bc6fa8730c4ddecd4db10b82c4f9007c8e821d282","refs":[{"period":25706927,"vote_hash_hex":"54ffee7126d85559dcb3e62873596cac57998bb9c1f8499ad5ccb676fdca2fb4"}]},{"root":"827e4d2929e8871824a36220b4a59bd75f33f0d2da55294a9ff128c4697671d2","refs":[{"period":25706928,"vote_hash_hex":"cff966fc7002e829bc78a7b8bb4d5a7201bf6074f6a2c6893f9954f350c21a26"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706929,"vote_hash_hex":"a61970a271f2779d505a4bbf2aed64905f19829c42711c3afa9d772931b0cfd7"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706930,"vote_hash_hex":"d622681c1380f3639bbfe2f43b02c9154039ea1b12d4eccbe09ea8fdf43b372c"}]},{"root":"60b89dcf9e9ecbb1c038b381d144fabda53064e6b46a002d9deeece69dad3697","refs":[{"period":25706931,"vote_hash_hex":"8765f13517ba522d9bccf85eb38f9a6af53d21cdb36b13dc0509ba45350138c2"}]},{"root":"f4c00f1fe89e446c63aa25d8fd80d138fb0eedfba6bb1ce51b63cd2ef2d0dd56","refs":[{"period":25706932,"vote_hash_hex":"e21babfe4032a1af2e8dd968febc53d3e207aed5ad11a171c8c68ba032e2c38f"}]},{"root":"9109390dc21e981e3ef34a44fb06359ccff8d9f5da9785ad769d4c451c629853","refs":[{"period":25706933,"vote_hash_hex":"36ddd276e4287c6dedafdcc73e0d3acd5d395557c6579fa7b8fe73beb0536672"}]},{"root":"1d9242c2aa421de9d4e5ef8126ab5ecf1ba49f53a72ef19f3243463bb21bc486","refs":[{"period":25706934,"vote_hash_hex":"e50b960f34e47e6e6cb71972f1c82a168bc3b2cc2119c75fcdca05ec179345e2"}]},{"root":"ece3968a30c0b8dedd5792e99d88c426bcff348385f281035eafb80b5f000315","refs":[{"period":25706935,"vote_hash_hex":"b650954d80b45d694ed079c9c3e29c052b6d39054ea431fc46fc89eefbea8992"}]},{"root":"64a4a559c1911b6d98e270d7c17805db3deaf3f4961f9773c3f98ccd136dd139","refs":[{"period":25706936,"vote_hash_hex":"ae4fa1e49a119821e7548b0e8bc331be2e3ec739d2701bf0eb33f6432c27da2d"}]},{"root":"b918017816a1b0ebc4dd3e43bbbb6e0490b9e5c94d33aed4d0cd84359b840145","refs":[{"period":25706937,"vote_hash_hex":"030f139a4e917b9a10287d2bc7eda1bc7040ea30f2fe4ddd5ea12b29c687c9a3"}]},{"root":"b918017816a1b0ebc4dd3e43bbbb6e0490b9e5c94d33aed4d0cd84359b840145","refs":[{"period":25706938,"vote_hash_hex":"2ca368e9b3eba509d2ba8d0c774d8b238c5a13ddb909e029f02ad27ddfb5d4c3"}]},{"root":"54a7068396b9d331e9bf9d3815ae26a96517e2af3596f7d4cf6a004ed5652252","refs":[{"period":25706939,"vote_hash_hex":"edcd6737c1e3091506fab12d97ba42ea5a1589d7fb75215f97f46efa2c868187"}]},{"root":"3fedb74070694d46637ea9fb65e70060f9ce8ae264a033dc1404872e0ab339ac","refs":[{"period":25706940,"vote_hash_hex":"bcc71e5176abe241995b16710b2588e2358bd6d72d1d38ae1d607f2881bdd8b2"}]},{"root":"51fdb8fa9573a32977862bddf804d4d211ef4a46c242a284d697033c85e1d0c7","refs":[{"period":25706941,"vote_hash_hex":"c0854452faca250226dfccf7bd9cc64566952c733900cba761df6996c7f35637"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706942,"vote_hash_hex":"468e6616363be182004b86d3d479cc5f570a7e79b0e88c491978d720eb3881b6"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706943,"vote_hash_hex":"4f972d3fdb422809d0bff41bdf4fe8a8382d825b2e6532febdffe253e7191b20"}]},{"root":"b5cee51b7dc57bc68bd2cd5144daefcfca83686170bf226cf2297795371a2f83","refs":[{"period":25706944,"vote_hash_hex":"c155bccb1a835dad32a8628d9c20453c6d5c280e85c55fd35d0592bde16ccd19"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706945,"vote_hash_hex":"4782a909880b7aff0cfb270fc7a2f3ecfcc61143a19fe81d2f72360ca6155ae2"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706946,"vote_hash_hex":"a1a795c6c5fb872dffe8e8b951f3d58d80e521937e96ab6de56af36949b5d495"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706947,"vote_hash_hex":"9b0512fbb8b80020e24bad9c8a75b555f3eab4958fe22b73fc328d179b966c14"}]},{"root":"4a31ec5fbb426cd88c29ed42b7428aae41951bb9ca4a617073df5a9e3312a624","refs":[{"period":25706948,"vote_hash_hex":"5f3d5acee666407dc0d8dcccee25b5cf832901f9fbe4fa194c5d04b9899d83b4"}]}]"#;
const F49_VOTER: &str = "49f85534bbcf461d44fd3fc96feecf9abb6d38ad";
const F49_PROFILE: &str = "n4_residual_vrf_49f85534bbcf461d44fd3fc96feecf9abb6d38ad_v1";
const F49_KEY: &str = "446a40c15b86d1effc8e92cf6bb7db72c56131bd9491c6a6bcb70cd0c0bac9f2";
const F49_INDICES: [usize; 45] = [
    77, 134, 194, 248, 305, 356, 416, 476, 536, 593, 653, 761, 821, 878, 935, 989, 1046, 1106,
    1166, 1226, 1286, 1343, 1394, 1442, 1502, 1562, 1616, 1676, 1730, 1787, 1844, 1901, 1955, 2012,
    2072, 2129, 2186, 2243, 2300, 2357, 2414, 2471, 2531, 2639, 2699,
];
// Exact41550-byte candidate SHA367e9d547ee98f2ef6cda7869d6119661b5623efc2b3ac191cd265b448bdac21.
const F49_BINDINGS: &str = r#"[{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706902,"vote_hash_hex":"2747a698f0c6db05aac90e93e4f343d6da3b1b3a92cb079f45feb33a40c65591"}]},{"root":"d2318bfb745c9198ca3e995119569c3ef85e95659dc45ade939398b8f3a23383","refs":[{"period":25706903,"vote_hash_hex":"ba70810ff156d12032d1a1b56d3cd598bcd3813be27abd9aef726ca4991ec94b"}]},{"root":"b07e806de23f0a4b16b6cef287118ab09699f32a0a4d8ffea3d436c9d53782ce","refs":[{"period":25706904,"vote_hash_hex":"843461d82ceb40322942ccf0e0b828e61e1dad32fc8548323c16bb740c50fa30"}]},{"root":"8c7fdbb2b39c01255ead8379e312bc79508558e1d1bd0a4dbe2a570a48fbf08b","refs":[{"period":25706905,"vote_hash_hex":"cdcd1be94894b211c4687df6c4333c4ce0efbd656fbe3f8a8e693fa787630ed9"}]},{"root":"a5fe6be45511742ec6a5b7c508a447507cb34dcfa321beedc81d5ad5e4aa0f8b","refs":[{"period":25706906,"vote_hash_hex":"7f6fe58f7ed5c06026efdf833c0cef88bb5f57d14ab88ae8c140431e384a360c"}]},{"root":"1b8815f01250eeeec908fd7d9093322f21909ae6ee234c9151109f063de9fc38","refs":[{"period":25706907,"vote_hash_hex":"ab2a44877426a09d569c92b8201af2d64ec31d458143c2ddf0179b5daf2a9a67"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706908,"vote_hash_hex":"c85807735fbac122d140414e56a60891229967b7797dd9b1e4435b80b57711da"}]},{"root":"4d39f544cd1555e667884f1cc078410e7a1baf143a487db8174ddc862a525ebc","refs":[{"period":25706909,"vote_hash_hex":"8086ed6e1e3e4253851a017286012808e45f821b5cda13f149b0886e75821b7c"}]},{"root":"5315842400d2c61336a33774e4ddd92eb90cfa19da449dab6c2e7202adbd2821","refs":[{"period":25706910,"vote_hash_hex":"d91f88b1cd69e5e24037643d7b1af3cceaf0077ec1f9312b5f743ed8d87d04b2"}]},{"root":"48240ffcddb5897ff34093c83b3ec9a9460d47290ba211fefae036fd83a107c2","refs":[{"period":25706911,"vote_hash_hex":"39ac3581e5832fab57572586fc80d18eec401659f1be8c01ac12db8938816b73"}]},{"root":"e70dd5ea2193dc4f38ab3d1ae8e8faad7988a0579716d15f79058649973d7964","refs":[{"period":25706912,"vote_hash_hex":"c511223724b325d4ebfd348ce9a647d3c36664527483d346bc8ce7c128cc2bf8"}]},{"root":"8f0c15bca3c202dad12c44f060e074748572ade6a1dba6dfea967b4d47b2a808","refs":[{"period":25706914,"vote_hash_hex":"d18fc2d52952ee69ae32e111a4d27e629f2a3a722fa1516c5e2cacc04915899e"}]},{"root":"5986c863eaab74a42da8642f740f7296d6f326443f4bb883a59c65675c31c56a","refs":[{"period":25706915,"vote_hash_hex":"4fef0505e3b4e40fdfc53b0e39bf9c8d8ce1f8fc1f46a1c219e724a064fe6cff"}]},{"root":"750e6dfcd464b29c1430473ea8338056169734f66c5e213da1cd9eba7566961e","refs":[{"period":25706916,"vote_hash_hex":"367a91271eea2fd9dc34d6448bddf4a67d559d125fe0abab8bc77c8e20005359"}]},{"root":"6389f33e77403f0f92fd654c208b18355f2da19895e08498f9f0aebc61da2075","refs":[{"period":25706917,"vote_hash_hex":"814870a89436a995e5fd02020b8e46c2a0f15e5e9f2a89c72003b6dd06e5c22f"}]},{"root":"3ce52115126f3ff130560bbf477f389159ad89eb7cc543abe1eda1ba38e74366","refs":[{"period":25706918,"vote_hash_hex":"74eeb5b79ffbeca3ce6d903152a0642fadc9978642171be74c3b0107a1d88de7"}]},{"root":"97f12663ea38b8ab1e88572ec16cb992a969529c83b0c5d7e7f19dbec317844c","refs":[{"period":25706919,"vote_hash_hex":"b8ff3499643acd2174308e7623e98502627a32d8e475dc2278a6b6ac0df44f77"}]},{"root":"76f9d6edc45e54c2f705c93b57a9ddc8025240c2678258767a4618477446c59d","refs":[{"period":25706920,"vote_hash_hex":"83b42503c64213106b91b6e3c5b83ff685913887aaf67f6b63af3021ca3b9e74"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706921,"vote_hash_hex":"272c2e416bc08c137bcee508ec47676a03415dd1c8362fc6a91125cd574b9e50"}]},{"root":"0e07b0ab3b770ce3b679796efded272782c82750280a0bd4bcf5697e13233dcc","refs":[{"period":25706922,"vote_hash_hex":"eb8a81a306f149bb243c99cab355f8b16069fbc291da7d5a50976e087e8ccc96"}]},{"root":"8fc69b3a7343817f4c55e8ab7e90680896c45c5bf03d4fd0aa0075c749a6a223","refs":[{"period":25706923,"vote_hash_hex":"3f5cd394c4aad116890b899664024f31aec5f0ec79dcb6177815bed613340398"}]},{"root":"131bcb5151b5e52bbace2653d3a44edf700db6b6337e82a04d70c6a1695ce803","refs":[{"period":25706924,"vote_hash_hex":"341d6a07a14f9b68671b5b863260969114d9ea77ff1c67bcdde24b377e231484"}]},{"root":"4a54404b443f071ae60a07b74dcc601ab375da61f81531e1f7b143fa3c8df222","refs":[{"period":25706925,"vote_hash_hex":"0918a222fa6a21fbf1ff196ebee4164dc3271c60303130fe5f660e7632c87eaa"}]},{"root":"715127dfc215df7d43d63b58cd58a4446edb425bf2e23502932e293ef021d4fa","refs":[{"period":25706926,"vote_hash_hex":"4ed221f548d99d8e17ae568e737eb6505f69e6af52260437bda62ca29d2e30f3"}]},{"root":"04c3266846de8285c05f021bc6fa8730c4ddecd4db10b82c4f9007c8e821d282","refs":[{"period":25706927,"vote_hash_hex":"a6c33facf8741a08acce4f5c3a893a00c641611e7549df5bf84b69b3e5c10e74"}]},{"root":"827e4d2929e8871824a36220b4a59bd75f33f0d2da55294a9ff128c4697671d2","refs":[{"period":25706928,"vote_hash_hex":"f346ea2f6c6ebb7e389c3dfdcfbeddb7412830de7cf753ebb225f7f2a2726dcb"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706929,"vote_hash_hex":"627e922538a7646d69b31d55b32fd83c59b5d2cd43b6abfda4866716e4c0c465"}]},{"root":"112aed825cb2f31d530671bc75a28d90d89c7e3021f6cd11617c6c5924697c34","refs":[{"period":25706930,"vote_hash_hex":"8f7731dfb9ed2fa6488efbbe9218193b3973e877076f6c5b477174be20aeefe6"}]},{"root":"60b89dcf9e9ecbb1c038b381d144fabda53064e6b46a002d9deeece69dad3697","refs":[{"period":25706931,"vote_hash_hex":"e638fd60ef64c0fe0793277337b3b710b347e240dcd219020ad3895bec4cf1df"}]},{"root":"f4c00f1fe89e446c63aa25d8fd80d138fb0eedfba6bb1ce51b63cd2ef2d0dd56","refs":[{"period":25706932,"vote_hash_hex":"a65f77be80ace59bb5dccbc45cd5062b5b8bb9fa2ec4a882effec2ff366075f0"}]},{"root":"9109390dc21e981e3ef34a44fb06359ccff8d9f5da9785ad769d4c451c629853","refs":[{"period":25706933,"vote_hash_hex":"f821d1e29ed0ceb689da7c6b225b481460a3c03d102c2a88dc707f533d497a0d"}]},{"root":"1d9242c2aa421de9d4e5ef8126ab5ecf1ba49f53a72ef19f3243463bb21bc486","refs":[{"period":25706934,"vote_hash_hex":"512d7dd665d289aeab28738d77c507e3d0647eaf6cf7049e5b028da9ce87aede"}]},{"root":"ece3968a30c0b8dedd5792e99d88c426bcff348385f281035eafb80b5f000315","refs":[{"period":25706935,"vote_hash_hex":"2b2601202337716b8bc585f8a262b880e476282f28fe3cda6b55fbe7e3a24247"}]},{"root":"64a4a559c1911b6d98e270d7c17805db3deaf3f4961f9773c3f98ccd136dd139","refs":[{"period":25706936,"vote_hash_hex":"873d392d7339fd9d46fada40c16f4a7dc0f88a92f2c55a0e05c30e586f91dd4c"}]},{"root":"b918017816a1b0ebc4dd3e43bbbb6e0490b9e5c94d33aed4d0cd84359b840145","refs":[{"period":25706937,"vote_hash_hex":"1b3e91c190fdf495ca92cf76ab6c0aec5fb84034788cbf0042cb58c29eeade9b"}]},{"root":"b918017816a1b0ebc4dd3e43bbbb6e0490b9e5c94d33aed4d0cd84359b840145","refs":[{"period":25706938,"vote_hash_hex":"60bc01b29e3545550bf063c5cd4594711b47562b0107a7cb2a5f84bf139757a3"}]},{"root":"54a7068396b9d331e9bf9d3815ae26a96517e2af3596f7d4cf6a004ed5652252","refs":[{"period":25706939,"vote_hash_hex":"203bda6a91152ab39099e465f49f64f6fb56c91c19ae09d31e78a7a238339707"}]},{"root":"3fedb74070694d46637ea9fb65e70060f9ce8ae264a033dc1404872e0ab339ac","refs":[{"period":25706940,"vote_hash_hex":"dd85b4311fb2246cac437f83888d7bc9ef67c019f1fa73d64885a6bb56bfefca"}]},{"root":"51fdb8fa9573a32977862bddf804d4d211ef4a46c242a284d697033c85e1d0c7","refs":[{"period":25706941,"vote_hash_hex":"8f49c5afd553daa175b81df609f4169de9634601bd1df033f410149e136bb2cd"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706942,"vote_hash_hex":"3bde029b2e53b919a560f9577d78c12f099f68333c425e83c0bbbbb938a20166"}]},{"root":"bd6bbc5b4725ee321e43c2ba367adc95deda48f05780a9ec859635ec9d39e8b5","refs":[{"period":25706943,"vote_hash_hex":"af49863be74fa44a926469448d0beaa8e5fdd877b6a291ad03da03ad0b007108"}]},{"root":"b5cee51b7dc57bc68bd2cd5144daefcfca83686170bf226cf2297795371a2f83","refs":[{"period":25706944,"vote_hash_hex":"593a59d8b66404158b37fdc33e906df1eb6def233b2a9dbd8c36776e12bc4bc5"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706945,"vote_hash_hex":"7d6c8d9cbf2417ad059efe04b96b67c970c7dc715d098e6412865e5324d42948"}]},{"root":"10561ec6a95842ceea4b5bbca7879ab192850eca7e62b91fa586e734876cd4d9","refs":[{"period":25706947,"vote_hash_hex":"e12e6e051b67624e09405fd3802f64be39cdae9fb61487603c8d2028b44e1a5c"}]},{"root":"4a31ec5fbb426cd88c29ed42b7428aae41951bb9ca4a617073df5a9e3312a624","refs":[{"period":25706948,"vote_hash_hex":"6a860b73977c0f6518ffb08a8d4883b792f911e1d55f2527a2d8525584ca5cc5"}]}]"#;
const VOTERS: [&str; 10] = [
    "138c1b9baea7d93a981ba694f5318f735c997298",
    "264a30e880d3b53bb2cdeb234e34201ebf9a9965",
    "329d40b5f15cfe4de0e9953d0599d2a39a65fc04",
    "575910424ac437467726542781508b2220b71ce8",
    "6bf44c438f6328640b0bc5d7d7766310cf49610f",
    "a6cec53b4d7920709f05ca1f7ac0d66338d29cb9",
    "c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa",
    "d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef",
    "d6d73e4c65c92787a2bbc80d109c293d8d3477af",
    "f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d",
];
const PROFILES: [&str; 10] = [
    "n4_remaining_vrf_voter_138c1b9baea7d93a981ba694f5318f735c997298_v1",
    "n4_remaining_vrf_voter_264a30e880d3b53bb2cdeb234e34201ebf9a9965_v1",
    "n4_remaining_vrf_voter_329d40b5f15cfe4de0e9953d0599d2a39a65fc04_v1",
    "n4_remaining_vrf_voter_575910424ac437467726542781508b2220b71ce8_v1",
    "n4_remaining_vrf_voter_6bf44c438f6328640b0bc5d7d7766310cf49610f_v1",
    "n4_remaining_vrf_voter_a6cec53b4d7920709f05ca1f7ac0d66338d29cb9_v1",
    "n4_remaining_vrf_voter_c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa_v1",
    "n4_remaining_vrf_voter_d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef_v1",
    "n4_remaining_vrf_voter_d6d73e4c65c92787a2bbc80d109c293d8d3477af_v1",
    "n4_remaining_vrf_voter_f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d_v1",
];
const PRIOR_INDICES: [usize; 10] = [1, 2, 3, 6, 7, 10, 13, 14, 15, 18];
const INDICES: [[usize; 47]; 10] = [
    [
        65, 122, 182, 239, 293, 347, 404, 464, 524, 581, 641, 698, 752, 809, 869, 923, 980, 1037,
        1094, 1154, 1214, 1274, 1331, 1385, 1433, 1490, 1550, 1607, 1664, 1718, 1775, 1835, 1892,
        1946, 2000, 2060, 2117, 2174, 2234, 2288, 2348, 2402, 2462, 2519, 2579, 2627, 2687,
    ],
    [
        68, 125, 185, 242, 296, 350, 407, 467, 527, 584, 644, 701, 755, 812, 872, 926, 983, 1040,
        1097, 1157, 1217, 1277, 1334, 1388, 1436, 1493, 1553, 1610, 1667, 1721, 1778, 1838, 1895,
        1949, 2003, 2063, 2120, 2177, 2237, 2291, 2351, 2405, 2465, 2522, 2582, 2630, 2690,
    ],
    [
        71, 128, 188, 245, 299, 353, 410, 470, 530, 587, 647, 704, 758, 815, 875, 929, 986, 1043,
        1100, 1160, 1220, 1280, 1337, 1391, 1439, 1496, 1556, 1613, 1670, 1724, 1781, 1841, 1898,
        1952, 2006, 2066, 2123, 2180, 2240, 2294, 2354, 2408, 2468, 2525, 2585, 2633, 2693,
    ],
    [
        80, 137, 197, 251, 308, 359, 419, 479, 539, 596, 656, 710, 764, 824, 881, 938, 992, 1049,
        1109, 1169, 1229, 1289, 1346, 1397, 1445, 1505, 1565, 1619, 1679, 1733, 1790, 1847, 1904,
        1958, 2015, 2075, 2132, 2189, 2246, 2303, 2360, 2417, 2474, 2534, 2591, 2642, 2702,
    ],
    [
        83, 140, 200, 254, 311, 362, 422, 482, 542, 599, 659, 713, 767, 827, 884, 941, 995, 1052,
        1112, 1172, 1232, 1292, 1349, 1400, 1448, 1508, 1568, 1622, 1682, 1736, 1793, 1850, 1907,
        1961, 2018, 2078, 2135, 2192, 2249, 2306, 2363, 2420, 2477, 2537, 2594, 2645, 2705,
    ],
    [
        89, 149, 209, 263, 314, 371, 431, 491, 551, 608, 665, 722, 776, 836, 893, 947, 1004, 1061,
        1121, 1181, 1241, 1301, 1352, 1406, 1457, 1517, 1574, 1631, 1688, 1745, 1802, 1859, 1913,
        1967, 2027, 2087, 2141, 2201, 2255, 2315, 2372, 2429, 2486, 2546, 2600, 2654, 2714,
    ],
    [
        98, 158, 218, 272, 323, 380, 440, 500, 560, 617, 674, 731, 785, 845, 902, 956, 1013, 1070,
        1130, 1190, 1250, 1310, 1361, 1412, 1466, 1526, 1583, 1640, 1697, 1754, 1811, 1868, 1922,
        1976, 2036, 2093, 2150, 2210, 2264, 2324, 2381, 2438, 2495, 2555, 2606, 2663, 2723,
    ],
    [
        101, 161, 221, 275, 326, 383, 443, 503, 563, 620, 677, 734, 788, 848, 905, 959, 1016, 1073,
        1133, 1193, 1253, 1313, 1364, 1415, 1469, 1529, 1586, 1643, 1700, 1757, 1814, 1871, 1925,
        1979, 2039, 2096, 2153, 2213, 2267, 2327, 2384, 2441, 2498, 2558, 2609, 2666, 2726,
    ],
    [
        104, 164, 224, 278, 329, 386, 446, 506, 566, 623, 680, 737, 791, 851, 908, 962, 1019, 1076,
        1136, 1196, 1256, 1316, 1367, 1418, 1472, 1532, 1589, 1646, 1703, 1760, 1817, 1874, 1928,
        1982, 2042, 2099, 2156, 2216, 2270, 2330, 2387, 2444, 2501, 2561, 2612, 2669, 2729,
    ],
    [
        113, 173, 230, 287, 338, 395, 455, 515, 572, 632, 689, 743, 800, 860, 914, 971, 1028, 1085,
        1145, 1205, 1265, 1322, 1376, 1424, 1481, 1541, 1598, 1655, 1709, 1769, 1826, 1883, 1937,
        1991, 2051, 2108, 2165, 2225, 2279, 2339, 2396, 2453, 2510, 2570, 2618, 2678, 2738,
    ],
];
impl Profile {
    fn residual(self) -> bool {
        matches!(
            self,
            Self::ResidualFirst | Self::ResidualB92a | Self::ResidualE50b | Self::Residual49f8
        )
    }

    fn voter(self) -> &'static str {
        if self == Self::ResidualFirst {
            RESIDUAL_VOTER
        } else if self == Self::ResidualB92a {
            B92A_VOTER
        } else if self == Self::ResidualE50b {
            E50B_VOTER
        } else if self == Self::Residual49f8 {
            F49_VOTER
        } else {
            VOTERS[self as usize]
        }
    }
    fn name(self) -> &'static str {
        if self == Self::ResidualFirst {
            RESIDUAL_PROFILE
        } else if self == Self::ResidualB92a {
            B92A_PROFILE
        } else if self == Self::ResidualE50b {
            E50B_PROFILE
        } else if self == Self::Residual49f8 {
            F49_PROFILE
        } else {
            PROFILES[self as usize]
        }
    }
    fn prior_index(self) -> usize {
        if self == Self::ResidualFirst {
            0
        } else if self == Self::ResidualB92a {
            12
        } else if self == Self::ResidualE50b {
            16
        } else if self == Self::Residual49f8 {
            5
        } else {
            PRIOR_INDICES[self as usize]
        }
    }
    fn payloads(self) -> usize {
        match self {
            Self::Residual49f8 => 45,
            Self::ResidualFirst | Self::ResidualB92a | Self::ResidualE50b => 46,
            _ => 47,
        }
    }
}
pub(crate) fn select(profile: Profile, rows: &Value) -> Result<Value> {
    if profile.residual() {
        return select_residual(profile, rows);
    }
    let rows = rows.as_array().context("complete voter rows")?;
    ensure!(rows.len() == 2742, "window original count");
    let mut periods = BTreeMap::<&str, BTreeSet<u64>>::new();
    for row in rows {
        if row["semantic_key"] == "fe_vrf_00_04_address" {
            periods
                .entry(row["voter_hex"].as_str().context("window voter")?)
                .or_default()
                .insert(row["delayed_period"].as_u64().context("window period")?);
        }
    }
    let all: BTreeSet<_> = (FIRST..FIRST + 48).collect();
    let candidates: Vec<_> = periods
        .iter()
        .filter(|(_, p)| **p == all)
        .map(|(v, _)| *v)
        .collect();
    ensure!(
        candidates.len() == 10 && candidates[..=profile as usize] == VOTERS[..=profile as usize],
        "fixed complete voter selector"
    );
    let selected: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["voter_hex"] == profile.voter()
                && r["semantic_key"] == "fe_vrf_00_04_address"
                && r["delayed_period"] != FIRST
        })
        .collect();
    ensure!(selected.len() == 47, "complete remaining voter family");
    let mut output = Vec::with_capacity(47);
    for (i, (index, row)) in selected.into_iter().enumerate() {
        let d = FIRST + 1 + i as u64;
        ensure!(
            index == INDICES[profile as usize][i]
                && row["delayed_period"] == d
                && row["qualified"] == false
                && row["value"].is_null()
                && row["proof"].is_null(),
            "window request/order drift"
        );
        let refs = row["source_references"]
            .as_array()
            .context("window vote references")?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|r| r["period"] == d + 7),
            "window source period"
        );
        output.push(json!({"parent_manifest_index":index,"source_row":row}));
    }
    Ok(json!(output))
}
/// Select the fixed partial-period raw request set. The omitted period has no
/// payload; it remains in the authenticated constructor/retention context.
fn select_residual(profile: Profile, rows: &Value) -> Result<Value> {
    let rows = rows.as_array().context("residual original rows")?;
    ensure!(rows.len() == 2742, "residual original count");
    let selected: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["voter_hex"] == profile.voter()
                && r["semantic_key"] == "fe_vrf_00_04_address"
                && r["delayed_period"] != FIRST
        })
        .collect();
    ensure!(selected.len() == profile.payloads(), "fixed residual count");
    let (indices, omitted): (&[usize], usize) = match profile {
        Profile::ResidualFirst => (&RESIDUAL_INDICES, 5),
        Profile::ResidualB92a => (&B92A_INDICES, 36),
        Profile::ResidualE50b => (&E50B_INDICES, 12),
        Profile::Residual49f8 => (&F49_INDICES, 12),
        _ => anyhow::bail!("residual profile required"),
    };
    let bindings: Value = serde_json::from_str(match profile {
        Profile::ResidualE50b => E50B_BINDINGS,
        Profile::Residual49f8 => F49_BINDINGS,
        _ => B92A_BINDINGS,
    })?;
    let mut out = Vec::with_capacity(profile.payloads());
    for (i, (index, row)) in selected.into_iter().enumerate() {
        let delayed_index = if i + 1 < omitted { i + 1 } else { i + 2 };
        let delayed_index = if profile == Profile::Residual49f8 && delayed_index >= 45 {
            delayed_index + 1
        } else {
            delayed_index
        };
        let d = FIRST + delayed_index as u64;
        ensure!(
            index == indices[i]
                && row["delayed_period"] == d
                && row["qualified"] == false
                && row["value"].is_null()
                && row["proof"].is_null(),
            "residual request/order drift"
        );
        let refs = row["source_references"]
            .as_array()
            .context("residual vote references")?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|r| r["period"] == d + 7),
            "residual source period"
        );
        if matches!(
            profile,
            Profile::ResidualB92a | Profile::ResidualE50b | Profile::Residual49f8
        ) {
            ensure!(
                row["source_references"] == bindings[i]["refs"],
                "b92a exact source references"
            );
        }
        out.push(json!({"parent_manifest_index":index,"source_row":row}));
    }
    Ok(json!(out))
}
pub(crate) fn prior(profile: Profile, bytes: &[u8], plan: &ConcreteVrfPlan) -> Result<Value> {
    ensure!(
        bytes.len() == PRIOR_BYTES && admission::sha(bytes) == PRIOR_SHA,
        "prior VRF pin drift"
    );
    let verified = replay_vrf_bundle(bytes, plan, ConcreteWitnessLimits::default())
        .map_err(|e| anyhow::anyhow!("prior VRF proof: {e:?}"))?;
    let index = plan
        .voters()
        .iter()
        .position(|v| hex::encode(v) == profile.voter())
        .context("prior selected voter")?;
    ensure!(index == profile.prior_index(), "prior selected index");
    let values = first::decode_values(&verified.logical)?;
    let value = accepted_prior_value(&verified.logical[index], &verified.physical[index])?;
    if profile == Profile::ResidualFirst {
        ensure!(
            hex::encode(value)
                == "6fabaf3eaa629b97b1dfaf77b9025072a1197299dd90cb19e46038794e91447b"
                && admission::sha(value)
                    == "1840cc6d06879636b32f20952e587930ba6c2cd12a32c48a3c72261574db4862",
            "fixed residual prior binding"
        );
    }
    if profile == Profile::ResidualB92a {
        ensure!(
            hex::encode(value)
                == "04d5cbb4aed88f984596c757d953bd962bf9e1353046b3e28af0aac6366cab93"
                && admission::sha(value)
                    == "f7c60a5e0b5ddeb58bff35973082fea4872f10f0b24552cc131c606240fa00b8",
            "fixed b92a prior binding"
        );
    }
    if profile == Profile::ResidualE50b {
        e50b_prior_binding(value)?;
    }
    if profile == Profile::Residual49f8 {
        f49_prior_binding(value)?;
    }
    Ok(
        json!({"input":{"path":PRIOR_PATH,"bytes":PRIOR_BYTES,"sha256":PRIOR_SHA},
        "actual":native::usage(verified.usage),"semantic_sha256":fields::normalized_digest(&values)?,
        "selected":{"proof_index":51 + index,"identity":native::identity_json(plan.context().delayed[0]),
            "voter_hex":profile.voter(),"value_hex":hex::encode(value),"usable_vrf_key":true}}),
    )
}
// This positive fixed prior binding is required after all71 canonical proofs.
fn e50b_prior_binding(value: &[u8]) -> Result<()> {
    ensure!(
        hex::encode(value) == "d25384032e092d1e74d81fc2819cdade58bae55c144d4a4f47ebf4449fc89a7a"
            && admission::sha(value)
                == "06577b8e723593fd76b1eea39d5aa5338567380bb121d5322390af0400930193",
        "fixed e50b prior binding"
    );
    Ok(())
}
// The sealed e50b tuple descriptor rejects root/key/address substitution.
fn e50b_tuple_binding(index: usize, tuple: ConcreteDiagnosticTuple) -> Result<()> {
    let bindings: Value = serde_json::from_str(E50B_BINDINGS)?;
    ensure!(
        index < 46 && hex::encode(tuple.identity.state_root) == bindings[index]["root"],
        "e50b fixed root binding"
    );
    ensure!(
        hex::encode(tuple.key.0) == E50B_KEY,
        "e50b fixed key binding"
    );
    ensure!(
        hex::encode(tuple.address) == "00000000000000000000000000000000000000fe",
        "e50b fixed address"
    );
    Ok(())
}
// Fixed positive prior and tuple bindings; callers cannot select these values.
fn f49_prior_binding(value: &[u8]) -> Result<()> {
    ensure!(
        hex::encode(value) == "2c9895a89f0d58e2210f21f2129dc74de825a1b7ca03a15c600cda5f4c4a6dc3"
            && admission::sha(value)
                == "7a279476475faccbad75c204c7f20e9cf0d2c0506b278be545c2726aae065c2e",
        "fixed49f8 prior binding"
    );
    Ok(())
}
fn f49_tuple_binding(index: usize, tuple: ConcreteDiagnosticTuple) -> Result<()> {
    let bindings: Value = serde_json::from_str(F49_BINDINGS)?;
    ensure!(
        index < 45 && hex::encode(tuple.identity.state_root) == bindings[index]["root"],
        "fixed49f8 root"
    );
    ensure!(hex::encode(tuple.key.0) == F49_KEY, "fixed49f8 key");
    ensure!(
        hex::encode(tuple.address) == "00000000000000000000000000000000000000fe",
        "fixed49f8 address"
    );
    Ok(())
}
pub(crate) fn accepted_prior_value<'a>(
    logical: &'a ConcreteWitnessLogical,
    physical: &ConcreteWitnessPhysical,
) -> Result<&'a [u8]> {
    let value = match logical {
        ConcreteWitnessLogical::Member(v) if v.len() == 32 && v.iter().any(|b| *b != 0) => v,
        _ => anyhow::bail!("required accepted firstD member unavailable"),
    };
    ensure!(
        matches!(physical,ConcreteWitnessPhysical::Present(v) if v==value),
        "prior physical member mismatch"
    );
    Ok(value)
}
pub(crate) fn canonical_prior() -> Result<Vec<u8>> {
    let p = Path::new(PRIOR_PATH);
    let mut current = std::path::PathBuf::new();
    for part in p.components() {
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "prior symlink"
        );
    }
    ensure!(
        fs::canonicalize(p)? == p && fs::metadata(p)?.is_file(),
        "prior regular file"
    );
    admission::evidence(p, p, PRIOR_SHA, PRIOR_BYTES, PRIOR_BYTES)
}
trait PlanView {
    fn context(&self) -> &ConcreteDiagnosticPlan;
    fn tuples(&self) -> &[ConcreteDiagnosticTuple];
}
impl PlanView for ConcreteVrfVoterPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        self.context()
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        self.tuples()
    }
}
impl PlanView for ConcreteVrfResidualPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        self.context()
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        self.tuples()
    }
}
impl PlanView for ConcreteVrfResidual49f8Plan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        self.context()
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        self.tuples()
    }
}
enum WindowPlan {
    Complete(ConcreteVrfVoterPlan),
    Residual(ConcreteVrfResidualPlan),
    Residual49f8(ConcreteVrfResidual49f8Plan),
}
impl PlanView for WindowPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        match self {
            Self::Complete(p) => p.context(),
            Self::Residual(p) => p.context(),
            Self::Residual49f8(p) => p.context(),
        }
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        match self {
            Self::Complete(p) => p.tuples(),
            Self::Residual(p) => p.tuples(),
            Self::Residual49f8(p) => p.tuples(),
        }
    }
}
fn window_authority(profile: Profile) -> Result<(WindowPlan, Value, Value)> {
    if profile == Profile::Residual49f8 {
        let (p, s, a) = authority_with(profile, |first, _| {
            Ok(ConcreteVrfResidual49f8Plan::new(first)?)
        })?;
        return Ok((WindowPlan::Residual49f8(p), s, a));
    }
    if profile.residual() {
        let (p, s, a) = authority_with(profile, |first, _| {
            Ok(match profile {
                Profile::ResidualFirst => ConcreteVrfResidualPlan::new(first)?,
                Profile::ResidualB92a => ConcreteVrfResidualPlan::new_b92a(first)?,
                Profile::ResidualE50b => ConcreteVrfResidualPlan::new_e50b(first)?,
                _ => unreachable!(),
            })
        })?;
        Ok((WindowPlan::Residual(p), s, a))
    } else {
        let (p, s, a) = authority(profile)?;
        Ok((WindowPlan::Complete(p), s, a))
    }
}
pub(crate) fn authority(profile: Profile) -> Result<(ConcreteVrfVoterPlan, Value, Value)> {
    ensure!(!profile.residual(), "complete profile required");
    authority_with(profile, |first, voter| {
        Ok(ConcreteVrfVoterPlan::new(first, voter)?)
    })
}
fn authority_with<T: PlanView>(
    profile: Profile,
    build: impl FnOnce(ConcreteVrfPlan, [u8; 20]) -> Result<T>,
) -> Result<(T, Value, Value)> {
    let (context, rows, six) = parent_input_handoff::load_authenticated_voter_authority()?;
    let (first_plan, first_selected) = first::selection(context, &rows)?;
    let mut selected = select(profile, &rows)?;
    let prior_index = profile.prior_index();
    ensure!(
        first_selected[prior_index]["parent_manifest_index"] == prior_index * 3 + 2,
        "prior parent index"
    );
    ensure!(
        first_selected[prior_index]["source_row"]["voter_hex"] == profile.voter(),
        "prior voter binding"
    );
    after_prior(
        profile,
        first_plan,
        &canonical_prior()?,
        |first_plan, old_proof| {
            let voter: [u8; 20] = hex::decode(profile.voter())?
                .try_into()
                .map_err(|_| anyhow::anyhow!("fixed voter width"))?;
            let plan = build(first_plan, voter)?;
            ensure!(
                plan.tuples().len() == profile.payloads(),
                "fixed payload count"
            );
            for (i, row) in selected
                .as_array_mut()
                .context("window selection")?
                .iter_mut()
                .enumerate()
            {
                let tuple = plan.tuples()[i];
                let fixed_key = match profile {
                    Profile::Seventh => {
                        Some("fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f")
                    }
                    Profile::Eighth => {
                        Some("d801919ffd29925243c28612983151535ef289be8872b1db4b4b41885167ee42")
                    }
                    Profile::Ninth => {
                        Some("da94f95efae9d2677940e6f37edbad5613af9bdb4d920a54465aae06f871e07d")
                    }
                    Profile::Tenth => {
                        Some("afff6c972f0508d1a892f0fb547e3044bf50ee2df3293d56c78ba7ab8939aebe")
                    }
                    Profile::ResidualFirst => Some(RESIDUAL_KEY),
                    Profile::ResidualB92a => Some(B92A_KEY),
                    Profile::ResidualE50b => Some(E50B_KEY),
                    Profile::Residual49f8 => Some(F49_KEY),
                    _ => None,
                };
                if let Some(key) = fixed_key {
                    ensure!(
                        hex::encode(tuple.key.0) == key,
                        "fixed successor key binding"
                    );
                }
                ensure!(
                    row["source_row"]["delayed_period"] == tuple.identity.period.as_u64(),
                    "authenticated tuple period"
                );
                if profile == Profile::ResidualB92a {
                    let bindings: Value = serde_json::from_str(B92A_BINDINGS)?;
                    ensure!(
                        hex::encode(tuple.identity.state_root) == bindings[i]["root"],
                        "b92a fixed root binding"
                    );
                    ensure!(
                        hex::encode(tuple.address) == "00000000000000000000000000000000000000fe",
                        "b92a fixed address"
                    );
                }
                if profile == Profile::ResidualE50b {
                    e50b_tuple_binding(i, tuple)?;
                }
                if profile == Profile::Residual49f8 {
                    f49_tuple_binding(i, tuple)?;
                }
                row["tuple"] = json!({"identity":native::identity_json(tuple.identity),"address_hex":hex::encode(tuple.address),
            "logical_key_hex":hex::encode(tuple.key.0),"key_preimage_hex":format!("0004{}", profile.voter())});
            }
            Ok((
                plan,
                selected,
                json!({"six_input_owner":six,"prior_first_d":old_proof,
        "raw_artifact_reads":7,"raw_artifact_bytes":6785636}),
            ))
        },
    )
}
/// The continuation reaches the window/open stage only after fresh prior proof.
pub(crate) fn after_prior<T>(
    profile: Profile,
    plan: ConcreteVrfPlan,
    bytes: &[u8],
    next: impl FnOnce(ConcreteVrfPlan, Value) -> Result<T>,
) -> Result<T> {
    let proof = prior(profile, bytes, &plan)?;
    next(plan, proof)
}

pub(crate) fn base(profile: Profile, selected: Value, authority: Value) -> Result<Value> {
    macro_rules! pin {($p:literal)=>{{let b=include_bytes!($p);
        ensure!(b.len()<=1048576,"window source file cap");
        json!({"path":$p,"bytes":b.len(),"sha256":admission::sha(b)})}};}
    let mut sources = first::sources()?
        .as_array()
        .context("shared VRF sources")?
        .clone();
    sources.push(pin!("vrf_shared_window.rs"));
    sources.extend(match profile {
        Profile::First => [
            pin!("bin/live_vrf_voter_window.rs"),
            pin!("bin/replay_vrf_voter_window.rs"),
        ],
        Profile::Second => [
            pin!("bin/live_vrf_next_voter_window.rs"),
            pin!("bin/replay_vrf_next_voter_window.rs"),
        ],
        Profile::Third => [
            pin!("bin/live_vrf_third_voter_window.rs"),
            pin!("bin/replay_vrf_third_voter_window.rs"),
        ],
        Profile::Fourth => [
            pin!("bin/live_vrf_fourth_voter_window.rs"),
            pin!("bin/replay_vrf_fourth_voter_window.rs"),
        ],
        Profile::Fifth => [
            pin!("bin/live_vrf_fifth_voter_window.rs"),
            pin!("bin/replay_vrf_fifth_voter_window.rs"),
        ],
        Profile::Sixth => [
            pin!("bin/live_vrf_sixth_voter_window.rs"),
            pin!("bin/replay_vrf_sixth_voter_window.rs"),
        ],
        Profile::Seventh => [
            pin!("bin/live_vrf_seventh_voter_window.rs"),
            pin!("bin/replay_vrf_seventh_voter_window.rs"),
        ],
        Profile::Eighth => [
            pin!("bin/live_vrf_eighth_voter_window.rs"),
            pin!("bin/replay_vrf_eighth_voter_window.rs"),
        ],
        Profile::Ninth => [
            pin!("bin/live_vrf_ninth_voter_window.rs"),
            pin!("bin/replay_vrf_ninth_voter_window.rs"),
        ],
        Profile::Tenth => [
            pin!("bin/live_vrf_tenth_voter_window.rs"),
            pin!("bin/replay_vrf_tenth_voter_window.rs"),
        ],
        Profile::ResidualFirst => [
            pin!("bin/live_vrf_residual_first.rs"),
            pin!("bin/replay_vrf_residual_first.rs"),
        ],
        Profile::ResidualB92a => [
            pin!("bin/live_vrf_residual_b92a.rs"),
            pin!("bin/replay_vrf_residual_b92a.rs"),
        ],
        Profile::ResidualE50b => [
            pin!("bin/live_vrf_residual_e50b.rs"),
            pin!("bin/replay_vrf_residual_e50b.rs"),
        ],
        Profile::Residual49f8 => [
            pin!("bin/live_vrf_residual_49f8.rs"),
            pin!("bin/replay_vrf_residual_49f8.rs"),
        ],
    });
    ensure!(sources.len() == 22, "window source count");
    let bytes = authority["six_input_owner"]["sources"]
        .as_array()
        .context("owner sources")?
        .iter()
        .chain(&sources)
        .try_fold(0u64, |n, p| {
            n.checked_add(p["bytes"].as_u64().context("source bytes")?)
                .context("source total")
        })?;
    ensure!(bytes <= 4194304, "combined window source cap");
    let mut bounds = native::policy();
    bounds["report_bytes"] = json!(REPORT_CAP);
    bounds["authority_input_bytes"] = json!(6785636);
    bounds["raw_artifacts_exact"] = json!(7);
    bounds["payload_tuples_exact"] = json!(profile.payloads());
    bounds["proofs_exact"] = json!(51 + profile.payloads());
    let digest = fields::normalized_digest(&selected)?;
    Ok(
        json!({"schema":1,"profile":profile.name(),"selection":selected,"selection_sha256":digest,
        "authority":authority,"vrf_sources":sources,"bounds":bounds,
        "qualification":{"vrf_family_qualified":false,"voter_weights_qualified":false,
        "reward_cache_equal":false,"owner_ready":false,"producer_verified":false,"production_routing_authorized":false}}),
    )
}
fn publish(output: &Path, report: &Value) -> Result<()> {
    admission::publish_exclusive(
        output,
        &interval_history::encode_bounded(report, REPORT_CAP)?,
    )
}
fn stopped(report: &mut Value, failure: &rustaxa_storage::ConcreteDiagnosticFailure) {
    let f = native::failure_report(failure);
    report["status"] = json!("stopped");
    report["failure"] = f["failure"].clone();
    report["actual"] = f["actual"].clone();
    report["values"] = Value::Null;
}
/// Preflight repeats raw-seven authentication and prior proof without DB path
/// inspection. It publishes the complete fixed request/source/bound profile.
pub(crate) fn preflight(profile: Profile, output: &Path) -> Result<()> {
    let output = native::output_path(output)?;
    let (plan, selected, authority) = window_authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    report["status"] = json!("authority_inputs_authenticated_no_database_open");
    report["head"] = native::identity_json(plan.context().head);
    report["parent"] = native::identity_json(plan.context().parent);
    report["delayed"] = json!(
        plan.context()
            .delayed
            .iter()
            .copied()
            .map(native::identity_json)
            .collect::<Vec<_>>()
    );
    report["genesis_hash_hex"] = json!(hex::encode(plan.context().genesis_hash));
    report["head_header_sha256"] = json!(hex::encode(plan.context().head_header_sha256));
    publish(&output, &report)
}
/// Capture exactly forty-seven remaining requests after complete prior-proof
/// authority. One failure stops later reads and publishes no accepted prefix.
pub(crate) fn capture(profile: Profile, bundle_output: &Path, report_output: &Path) -> Result<()> {
    let paths = paths::validate(Path::new(native::COPY), bundle_output)?;
    let reports = paths::validate(Path::new(native::COPY), report_output)?;
    ensure!(paths.output != reports.output, "window outputs overlap");
    let (plan, selected, authority) = window_authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    interval_history::encode_bounded(&report, REPORT_CAP)?;
    let captured = match &plan {
        WindowPlan::Complete(p) => capture_vrf_voter_run(
            &paths.application,
            &paths.state,
            p,
            ConcreteWitnessLimits::default(),
        ),
        WindowPlan::Residual(p) => capture_vrf_residual_run(
            &paths.application,
            &paths.state,
            p,
            ConcreteWitnessLimits::default(),
        ),
        WindowPlan::Residual49f8(p) => capture_vrf_residual_49f8_run(
            &paths.application,
            &paths.state,
            p,
            ConcreteWitnessLimits::default(),
        ),
    };
    match captured {
        Err(f) => {
            stopped(&mut report, &f);
            publish(&reports.output, &report)?;
            anyhow::bail!("window capture stopped; no prefix")
        }
        Ok(c) => {
            ensure!(c.bundle.len() <= native::BUNDLE_CAP, "window bundle cap");
            report["status"] = json!("captured_requires_offline_replay");
            report["actual"] = native::usage(c.usage);
            report["bundle"] = json!({"path":paths.output,"bytes":c.bundle.len(),"sha256":admission::sha(&c.bundle)});
            let encoded = interval_history::encode_bounded(&report, REPORT_CAP)?;
            admission::publish_exclusive(&paths.output, &c.bundle)?;
            admission::publish_exclusive(&reports.output, &encoded)
        }
    }
}
/// Offline replay repeats raw-seven authority, consumes every proof and returns
/// all forty-seven canonical values/provenance. No DB input is available.
pub(crate) fn replay(profile: Profile, bundle_input: &Path, report_output: &Path) -> Result<()> {
    let output = native::output_path(report_output)?;
    let input = fs::canonicalize(bundle_input)?;
    admission::outside_db(&input)?;
    ensure!(
        fs::metadata(&input)?.is_file() && input != output,
        "window bundle input"
    );
    let (plan, selected, authority) = window_authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    let bytes = admission::read_capped(fs::File::open(&input)?, native::BUNDLE_CAP)?;
    let result = match &plan {
        WindowPlan::Complete(p) => {
            replay_vrf_voter_bundle(&bytes, p, ConcreteWitnessLimits::default())
                .map(|v| replay_values(v.logical, v.physical).map(|values| (values, v.usage)))
        }
        WindowPlan::Residual(p) => {
            replay_vrf_residual_bundle(&bytes, p, ConcreteWitnessLimits::default())
                .map(|v| replay_values(v.logical, v.physical).map(|values| (values, v.usage)))
        }
        WindowPlan::Residual49f8(p) => {
            replay_vrf_residual_49f8_bundle(&bytes, p, ConcreteWitnessLimits::default())
                .map(|v| replay_values(v.logical, v.physical).map(|values| (values, v.usage)))
        }
    };
    let (values, usage) = match result {
        Ok(v) => v?,
        Err(f) => {
            stopped(&mut report, &f);
            publish(&output, &report)?;
            anyhow::bail!("window replay stopped; no prefix")
        }
    };
    report["status"] = json!(match profile {
        Profile::Residual49f8 => "all_forty_five_residual_vrf_inputs_authenticated",
        Profile::ResidualFirst | Profile::ResidualB92a | Profile::ResidualE50b =>
            "all_forty_six_residual_vrf_inputs_authenticated",
        _ => "all_forty_seven_vrf_inputs_authenticated",
    });
    report["actual"] = native::usage(usage);
    report["bundle"] = json!({"path":input,"bytes":bytes.len(),"sha256":admission::sha(&bytes)});
    report["values"] = values;
    report["qualification"]["vrf_family_qualified"] = json!(true);
    publish(&output, &report)
}
fn replay_values<const N: usize>(
    logical: [ConcreteWitnessLogical; N],
    physical: [ConcreteWitnessPhysical; N],
) -> Result<Value> {
    let mut values = first::decode_values(&logical)?;
    for (i, (value, physical)) in values
        .as_array_mut()
        .context("window values")?
        .iter_mut()
        .zip(physical)
        .enumerate()
    {
        value["proof_index"] = json!(51 + i);
        value["physical"] = match physical {
            ConcreteWitnessPhysical::Present(v) => {
                json!({"kind":"present","bytes":v.len(),"sha256":admission::sha(&v)})
            }
            ConcreteWitnessPhysical::Tombstone => json!({"kind":"tombstone"}),
            ConcreteWitnessPhysical::HistoryUnavailable(id) => {
                json!({"kind":"successful_seek_without_matching_version","identity":native::identity_json(id)})
            }
        };
    }
    Ok(values)
}

/// Authenticate the fixed seventh profile offline and publish its bounded manifest.
pub fn seventh_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Seventh, output)
}
/// Capture the fixed seventh profile once through an independently reviewed guard.
pub fn seventh_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Seventh, bundle, report)
}
/// Consume every seventh proof offline; no database input is available.
pub fn seventh_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Seventh, bundle, report)
}

/// Authenticate the fixed eighth profile offline; errors publish no partial authority.
pub fn eighth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Eighth, output)
}
/// Capture the fixed eighth profile through its independently reviewed one-use guard.
pub fn eighth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Eighth, bundle, report)
}
/// Replay all fixed eighth proofs offline; errors qualify no prefix.
pub fn eighth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Eighth, bundle, report)
}

/// Authenticate the fixed ninth profile offline; errors publish no partial authority.
pub fn ninth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Ninth, output)
}
/// Capture the fixed ninth profile through its independently reviewed one-use guard.
pub fn ninth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Ninth, bundle, report)
}
/// Replay all fixed ninth proofs offline; errors qualify no prefix.
pub fn ninth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Ninth, bundle, report)
}

/// Authenticate the fixed tenth profile offline; errors publish no partial authority.
pub fn tenth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Tenth, output)
}
/// Capture the fixed tenth profile through its independently reviewed one-use guard.
pub fn tenth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Tenth, bundle, report)
}
/// Replay all fixed tenth proofs offline; errors qualify no prefix.
pub fn tenth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Tenth, bundle, report)
}

/// Authenticate six raw files and all 71 prior proofs, then publish the fixed
/// 46-row partial-period manifest at an exclusive outside-DB path. No DB open
/// occurs. Invalid authority or output admission returns an error; no prefix
/// supplies authority. Omitted periods remain in the full retention context.
pub fn residual_first_preflight(output: &Path) -> Result<()> {
    preflight(Profile::ResidualFirst, output)
}
/// Capture the sealed 46-row profile from the fixed read-only copy after exact
/// independent guard READY. Exclusive bundle/report outputs must be distinct
/// and outside the DB. All 97 proofs are required. A diagnostic failure attempts
/// a stopped report and returns an error; publication errors may leave no report.
/// No prefix qualifies and no omitted period is assigned a value.
pub fn residual_first_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::ResidualFirst, bundle, report)
}
/// Replay a bounded RCV3 bundle without DB input and publish all 46 logical
/// values and physical provenance at an exclusive outside-DB path. Fresh raw
/// authentication, the 71-proof prior barrier, all 97 bundle proofs and exact EOF
/// are required. Errors qualify no prefix and attempt a stopped report when
/// possible. Fact acceptance separately requires all 46 positive members.
pub fn residual_first_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::ResidualFirst, bundle, report)
}

/// Authenticate six fixed raw inputs and all 71 fresh prior proofs before the
/// named 46-row b92a constructor. Publish a bounded exclusive outside-DB report;
/// invalid authority or output admission returns an error and no partial authority.
pub fn residual_b92a_preflight(output: &Path) -> Result<()> {
    preflight(Profile::ResidualB92a, output)
}
/// Capture the fixed b92a profile after its independent one-use READY guard.
/// Distinct exclusive outside-DB bundle/report paths are required. Full 48-period
/// retention and all 97 proofs are mandatory; errors qualify no prefix and attempt
/// a stopped report. The omitted period receives no payload or value.
pub fn residual_b92a_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::ResidualB92a, bundle, report)
}
/// Replay the fixed b92a RCV3 profile offline with fresh six-input authentication
/// and all 71 prior proofs. Require all 97 bundle proofs and exact EOF. Publish
/// all 46 values/provenance exclusively outside DB; errors qualify no prefix and
/// attempt a stopped report. Positive-member fact acceptance remains separate.
pub fn residual_b92a_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::ResidualB92a, bundle, report)
}

/// Authenticate the six fixed raw inputs and all71 canonical prior proofs before
/// the named e50b constructor. Publish its fixed46 manifest to an exclusive
/// outside-DB path. Invalid input/output admission fails without partial authority.
pub fn residual_e50b_preflight(output: &Path) -> Result<()> {
    preflight(Profile::ResidualE50b, output)
}
/// Capture the sealed e50b46 profile through its independently reviewed one-use
/// guard. Exclusive outside-DB outputs must be distinct. Full48 retention and97
/// proofs are mandatory; errors qualify no prefix and attempt a stopped report.
/// The omitted period receives no payload, state, absence or zero-value fact.
pub fn residual_e50b_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::ResidualE50b, bundle, report)
}
/// Replay the sealed e50b RCV3 profile offline. Fresh six-input authentication,
/// all71 prior and97 bundle proofs, exact order and EOF are mandatory. Publish
/// all46 values/provenance exclusively outside DB. Errors qualify no prefix and
/// attempt a stopped report; positive-member fact acceptance remains separate.
pub fn residual_e50b_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::ResidualE50b, bundle, report)
}

/// Authenticate fixed raw6 and all71 canonical prior proofs before constructing
/// the named49f8 plan. Publish the fixed45/96 manifest exclusively outside DB.
/// Both omitted identities remain in full48 retention. Invalid authority or
/// output admission returns an error without partial authority or DB open.
pub fn residual_49f8_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Residual49f8, output)
}
/// Capture the fixed49f8 RCV4 profile after its independent one-use guard.
/// Distinct exclusive outside-DB bundle/report outputs are required. All51
/// constructor and45 payload proofs are mandatory. Failure qualifies no prefix
/// and attempts a stopped report; omissions supply no state/absence/zero fact.
pub fn residual_49f8_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Residual49f8, bundle, report)
}
/// Replay fixed45 RCV4 values/provenance offline after raw6 and fresh71 prior.
/// Require all96 proofs, canonical order, strict history and exact EOF. Publish
/// exclusively outside DB. Failure qualifies no prefix and attempts a stopped
/// report; positive-member fact acceptance remains a separate decision.
pub fn residual_49f8_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Residual49f8, bundle, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    // Only reviewed changed storage source pins can differ. Semantic fields
    // remain compared in full; this is an exact source-successor mapping.
    fn storage_successor(path: &str) -> Option<&'static [u8]> {
        match path {
            "../../../../rust/crates/rustaxa-storage/src/lib.rs" => Some(include_bytes!(
                "../../../../rust/crates/rustaxa-storage/src/lib.rs"
            )),
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs" => Some(
                include_bytes!("../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs"),
            ),
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs"
                ))
            }
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs"
                ))
            }
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs"
                ))
            }
            _ => None,
        }
    }
    fn verify_pin(pin: &Value) {
        use sha2::{Digest, Sha256};
        let p = Path::new(pin["path"].as_str().unwrap());
        admission::outside_db(p).unwrap();
        let size = pin["bytes"].as_u64().unwrap();
        assert!(size <= 268435456);
        assert_eq!(fs::metadata(p).unwrap().len(), size);
        let mut file = fs::File::open(p).unwrap();
        let mut hash = Sha256::new();
        let mut buf = [0; 8192];
        let mut count = 0;
        loop {
            let n = file.read(&mut buf).unwrap();
            if n == 0 {
                break;
            };
            count += n as u64;
            assert!(count <= size);
            hash.update(&buf[..n]);
        }
        assert_eq!(count, size);
        assert_eq!(
            hex::encode(hash.finalize()),
            pin["sha256"].as_str().unwrap()
        );
    }
    #[test]
    fn six_saved_windows_have_complete_equivalent_reports() {
        let cases = [
            (
                Profile::First,
                "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-voter-window-replay.json",
                59391,
                "b6278f844e627f5830d56803a5555668aa982b56ce2c81fdc9b0cb677b9bc3e6",
                "vrf_voter_window.rs",
            ),
            (
                Profile::Second,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/vrf-next-voter-window-replay.json",
                59374,
                "0e3b5e18f00209a7799da50565d90bc273ef226c17cc33c5685578255495db42",
                "vrf_next_voter_window.rs",
            ),
            (
                Profile::Third,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/third-voter/vrf-third-voter-window-replay.json",
                59403,
                "a7f94b240fcdf7293475ae0ec7972fb0d1fb72609703db7080b339066de6c59a",
                "vrf_third_voter_window.rs",
            ),
            (
                Profile::Fourth,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/fourth-voter/vrf-fourth-voter-window-replay.json",
                59410,
                "bd099e106c56454bcf94cb6c2fbadbb5d4f14d0acad9b74786d569af75525815",
                "vrf_fourth_voter_window.rs",
            ),
            (
                Profile::Fifth,
                "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/fifth-voter/vrf-fifth-voter-window-replay.json",
                59403,
                "9f3124ada4f7c88dd2c8037ec0ac7c7e87907bcea61d683df8746054d9f39fce",
                "vrf_fifth_voter_window.rs",
            ),
            (
                Profile::Sixth,
                "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/sixth-voter/vrf-sixth-voter-window-replay.json",
                59404,
                "35c05787ece4bb2b38a3cbe2fb3137019cbb34d79fd24c9406b857821d5fe187",
                "vrf_sixth_voter_window.rs",
            ),
            (
                Profile::Seventh,
                "/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/seventh-voter/vrf-seventh-voter-window-replay.json",
                59467,
                "f551c202f8b929616294395d0748f8e269b53352748a8411fd7671a9a37a8c5e",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Eighth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/eighth-voter/vrf-eighth-voter-window-replay.json",
                59470,
                "59280657393129d87ead097ecb1ce967f24e6ce5974ec8f3f926aed328f7631d",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Ninth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/ninth-voter/vrf-ninth-voter-window-replay.json",
                59464,
                "3833e18901c4c633e48962f607248a5850c2ce24e759eca94e37955f2c435d5d",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Tenth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/tenth-voter/vrf-tenth-voter-window-replay.json",
                59464,
                "760aab8712c5047c4efd9a1ee3c7ab518d2dd1af468a4f975e65cfa3e2ff9d90",
                "vrf_shared_window.rs",
            ),
        ];
        let scratch = Path::new("/home/fry/artifacts").join(format!(
            "shared-vrf-equivalence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&scratch).unwrap();
        for (profile, path, size, sha, adapter) in cases {
            let path = Path::new(path);
            let mut old = admission::parse_unique(
                &admission::evidence(path, path, sha, size, REPORT_CAP).unwrap(),
            )
            .unwrap();
            verify_pin(&old["bundle"]);
            verify_pin(&old["authority"]["six_input_owner"]["executable"]);
            let input = Path::new(old["bundle"]["path"].as_str().unwrap());
            let output = scratch.join(format!("{}.json", profile as usize));
            replay(profile, input, &output).unwrap();
            let mut new = admission::parse_unique(
                &admission::read_capped(fs::File::open(&output).unwrap(), REPORT_CAP).unwrap(),
            )
            .unwrap();
            let exe = &new["authority"]["six_input_owner"]["executable"];
            assert_eq!(
                exe["path"],
                json!(fs::canonicalize(std::env::current_exe().unwrap()).unwrap())
            );
            verify_pin(exe);
            old["authority"]["six_input_owner"]["executable"] = exe.clone();
            for (parent, key) in [("", "vrf_sources"), ("authority", "sources")] {
                let (a, b) = if parent.is_empty() {
                    (&mut old[key], &mut new[key])
                } else {
                    (
                        &mut old[parent]["six_input_owner"][key],
                        &mut new[parent]["six_input_owner"][key],
                    )
                };
                let a = a.as_array_mut().unwrap();
                let b = b.as_array_mut().unwrap();
                assert_eq!(a.len(), b.len());
                for (before, after) in a.iter_mut().zip(b.iter()) {
                    let path = before["path"].as_str().unwrap();
                    let expected = if path == "lib.rs" && profile != Profile::Seventh {
                        Some(("lib.rs", include_bytes!("lib.rs").as_slice()))
                    } else if path == adapter {
                        Some((
                            "vrf_shared_window.rs",
                            include_bytes!("vrf_shared_window.rs").as_slice(),
                        ))
                    } else {
                        storage_successor(path).map(|bytes| (path, bytes))
                    };
                    if let Some((name, bytes)) = expected {
                        assert_eq!(
                            *after,
                            json!({"path":name,"bytes":bytes.len(),"sha256":admission::sha(bytes)})
                        );
                        *before = after.clone();
                    } else {
                        assert_eq!(before, after);
                    }
                }
            }
            assert_eq!(old, new, "complete report for {profile:?}");
            assert_eq!(new["values"].as_array().unwrap().len(), 47);
            assert_eq!(new["qualification"]["vrf_family_qualified"], true);
        }
        fs::remove_dir_all(scratch).unwrap();
    }
    #[test]
    fn residual_fixed_selection_prior_and_complete_guard() {
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let selected = select(Profile::ResidualFirst, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 46);
        for i in 0..46 {
            assert_eq!(selected[i]["parent_manifest_index"], RESIDUAL_INDICES[i]);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 5);
        }
        for (field, value) in [
            ("voter_hex", json!(VOTERS[0])),
            ("delayed_period", json!(FIRST + 5)),
            ("qualified", json!(true)),
            ("proof", json!({})),
            ("value", json!("00")),
        ] {
            let mut bad = rows.clone();
            bad[RESIDUAL_INDICES[0]][field] = value;
            assert!(select(Profile::ResidualFirst, &bad).is_err());
        }
        let mut duplicate = rows.clone();
        duplicate[RESIDUAL_INDICES[1]] = duplicate[RESIDUAL_INDICES[0]].clone();
        assert!(select(Profile::ResidualFirst, &duplicate).is_err());
        let mut reorder = rows.clone();
        reorder
            .as_array_mut()
            .unwrap()
            .swap(RESIDUAL_INDICES[0], RESIDUAL_INDICES[1]);
        assert!(select(Profile::ResidualFirst, &reorder).is_err());
        let mut missing = rows.clone();
        missing.as_array_mut().unwrap().remove(RESIDUAL_INDICES[0]);
        assert!(select(Profile::ResidualFirst, &missing).is_err());
        let mut extra = rows.clone();
        extra[65]["voter_hex"] = json!(RESIDUAL_VOTER);
        assert!(select(Profile::ResidualFirst, &extra).is_err());
        let mut refs = rows.clone();
        refs[RESIDUAL_INDICES[0]]["source_references"] = json!([]);
        assert!(select(Profile::ResidualFirst, &refs).is_err());
        assert!(authority(Profile::ResidualFirst).is_err());
        let mut partial_complete = rows.clone();
        partial_complete[INDICES[0][4]]["voter_hex"] = json!(RESIDUAL_VOTER);
        assert!(select(Profile::First, &partial_complete).is_err());
        let (plan, selected, summary) = window_authority(Profile::ResidualFirst).unwrap();
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 51);
        assert_eq!(
            summary["prior_first_d"]["actual"]["admitted_input_bytes"],
            PRIOR_BYTES
        );
        assert_eq!(plan.context().delayed.len(), 48);
        assert_eq!(plan.tuples().len(), 46);
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(hex::encode(t.key.0), RESIDUAL_KEY);
            assert_eq!(
                selected[i]["tuple"]["identity"],
                native::identity_json(t.identity)
            );
        }
        let report = base(Profile::ResidualFirst, selected, summary).unwrap();
        assert_eq!(report["bounds"]["proofs_exact"], 97);
        assert_eq!(report["bounds"]["payload_tuples_exact"], 46);
        assert!(
            report["qualification"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == false)
        );
        let (context, _, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let (first, _) = first::selection(context, &rows).unwrap();
        let mut bad = canonical_prior().unwrap();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(
            after_prior(Profile::ResidualFirst, first, &bad, |_, _| -> Result<()> {
                panic!("bad prior reached residual constructor")
            })
            .is_err()
        );
    }
    #[test]
    fn b92a_fixed_selection_prior_and_full_barrier() {
        let (context, rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let selected = select(Profile::ResidualB92a, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 46);
        for i in 0..46 {
            assert_eq!(selected[i]["parent_manifest_index"], B92A_INDICES[i]);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 36);
        }
        for (field, value) in [
            ("voter_hex", json!(RESIDUAL_VOTER)),
            ("semantic_key", json!("other")),
            ("delayed_period", json!(FIRST + 36)),
            ("qualified", json!(true)),
            ("proof", json!({})),
            ("value", json!("00")),
            ("source_references", json!([])),
            (
                "source_references",
                json!([{"period":FIRST+8,"vote_hash_hex":"00"}]),
            ),
        ] {
            let mut bad = rows.clone();
            bad[B92A_INDICES[0]][field] = value;
            assert!(select(Profile::ResidualB92a, &bad).is_err(), "{field}");
        }
        let mut duplicate = rows.clone();
        duplicate[B92A_INDICES[1]] = duplicate[B92A_INDICES[0]].clone();
        assert!(select(Profile::ResidualB92a, &duplicate).is_err());
        let mut reorder = rows.clone();
        reorder
            .as_array_mut()
            .unwrap()
            .swap(B92A_INDICES[0], B92A_INDICES[1]);
        assert!(select(Profile::ResidualB92a, &reorder).is_err());
        let mut missing = rows.clone();
        missing.as_array_mut().unwrap().remove(B92A_INDICES[0]);
        assert!(select(Profile::ResidualB92a, &missing).is_err());
        let mut extra = rows.clone();
        extra[65]["voter_hex"] = json!(B92A_VOTER);
        assert!(select(Profile::ResidualB92a, &extra).is_err());
        assert!(authority(Profile::ResidualB92a).is_err());
        let (_, prior_selected) = first::selection(context, &rows).unwrap();
        assert_eq!(prior_selected[12]["parent_manifest_index"], 38);
        assert_eq!(
            prior_selected[12]["source_row"]["source_references"],
            json!([{"period":25706901,"vote_hash_hex":"802b4ced9f1e288d16ed761b9dea12ec352872e688624339270354befff1e2d8"}])
        );
        let canonical = canonical_prior().unwrap();
        for index in [0, canonical.len() / 2, canonical.len() - 1] {
            let mut bad = canonical.clone();
            bad[index] ^= 1;
            assert!(
                after_prior(
                    Profile::ResidualB92a,
                    first::selection(
                        parent_input_handoff::load_authenticated_voter_authority()
                            .unwrap()
                            .0,
                        &rows
                    )
                    .unwrap()
                    .0,
                    &bad,
                    |_, _| -> Result<()> { panic!("bad prior reached b92a constructor") }
                )
                .is_err()
            );
        }
        let (plan, selected, summary) = window_authority(Profile::ResidualB92a).unwrap();
        assert_eq!(plan.context().delayed.len(), 48);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 63);
        assert_eq!(
            summary["prior_first_d"]["selected"]["value_hex"],
            "04d5cbb4aed88f984596c757d953bd962bf9e1353046b3e28af0aac6366cab93"
        );
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(hex::encode(t.key.0), B92A_KEY);
            assert_eq!(
                selected[i]["tuple"]["identity"],
                native::identity_json(t.identity)
            );
        }
        let report = base(Profile::ResidualB92a, selected, summary).unwrap();
        assert_eq!(report["bounds"]["proofs_exact"], 97);
        assert_eq!(report["bounds"]["payload_tuples_exact"], 46);
        assert!(
            report["qualification"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == false)
        );
        let bytes = admission::evidence(
            Path::new(FIRST_RESIDUAL_BUNDLE),
            Path::new(FIRST_RESIDUAL_BUNDLE),
            "5c403c68f1cb9f742b309fd293ce192e05f453fb1f3e8c9a960f944c2f045469",
            649381,
            649381,
        )
        .unwrap();
        let WindowPlan::Residual(p) = plan else {
            panic!("residual plan")
        };
        let failure = replay_vrf_residual_bundle(&bytes, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }
    #[test]
    fn e50b_fixed_selection_prior_and_full_barrier() {
        let (context, rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let selected = select(Profile::ResidualE50b, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 46);
        for i in 0..46 {
            assert_eq!(selected[i]["parent_manifest_index"], E50B_INDICES[i]);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 12);
        }
        for (field, value) in [
            ("voter_hex", json!(RESIDUAL_VOTER)),
            ("semantic_key", json!("other")),
            ("delayed_period", json!(FIRST + 12)),
            ("qualified", json!(true)),
            ("proof", json!({})),
            ("value", json!("00")),
            ("source_references", json!([])),
            (
                "source_references",
                json!([{"period":FIRST+8,"vote_hash_hex":"00"}]),
            ),
        ] {
            let mut bad = rows.clone();
            bad[E50B_INDICES[0]][field] = value;
            assert!(select(Profile::ResidualE50b, &bad).is_err(), "{field}");
        }
        let mut duplicate = rows.clone();
        duplicate[E50B_INDICES[1]] = duplicate[E50B_INDICES[0]].clone();
        assert!(select(Profile::ResidualE50b, &duplicate).is_err());
        let mut reorder = rows.clone();
        reorder
            .as_array_mut()
            .unwrap()
            .swap(E50B_INDICES[0], E50B_INDICES[1]);
        assert!(select(Profile::ResidualE50b, &reorder).is_err());
        let mut missing = rows.clone();
        missing.as_array_mut().unwrap().remove(E50B_INDICES[0]);
        assert!(select(Profile::ResidualE50b, &missing).is_err());
        let mut extra = rows.clone();
        extra[65]["voter_hex"] = json!(E50B_VOTER);
        assert!(select(Profile::ResidualE50b, &extra).is_err());
        assert!(authority(Profile::ResidualE50b).is_err());
        let (_, prior_selected) = first::selection(context, &rows).unwrap();
        assert_eq!(prior_selected[16]["parent_manifest_index"], 50);
        assert_eq!(
            prior_selected[16]["source_row"]["source_references"],
            json!([{"period":25706901,"vote_hash_hex":"81d68a6bfeb618caefe42ce49dea66dfd4e5717f1b6bfd660f576118032c1419"}])
        );
        let canonical = canonical_prior().unwrap();
        for index in [0, canonical.len() / 2, canonical.len() - 1] {
            let mut bad = canonical.clone();
            bad[index] ^= 1;
            assert!(
                after_prior(
                    Profile::ResidualE50b,
                    first::selection(
                        parent_input_handoff::load_authenticated_voter_authority()
                            .unwrap()
                            .0,
                        &rows
                    )
                    .unwrap()
                    .0,
                    &bad,
                    |_, _| -> Result<()> { panic!("bad prior reached e50b constructor") }
                )
                .is_err()
            );
        }
        let (plan, selected, summary) = window_authority(Profile::ResidualE50b).unwrap();
        assert_eq!(plan.context().delayed.len(), 48);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 67);
        assert_eq!(
            summary["prior_first_d"]["selected"]["value_hex"],
            "d25384032e092d1e74d81fc2819cdade58bae55c144d4a4f47ebf4449fc89a7a"
        );
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(hex::encode(t.key.0), E50B_KEY);
            assert_eq!(
                selected[i]["tuple"]["identity"],
                native::identity_json(t.identity)
            );
        }
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.identity.state_root[0] ^= 1;
        assert!(e50b_tuple_binding(0, bad_tuple).is_err());
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.key.0[0] ^= 1;
        assert!(e50b_tuple_binding(0, bad_tuple).is_err());
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.address[0] ^= 1;
        assert!(e50b_tuple_binding(0, bad_tuple).is_err());
        assert!(e50b_tuple_binding(46, plan.tuples()[0]).is_err());
        let member =
            hex::decode("d25384032e092d1e74d81fc2819cdade58bae55c144d4a4f47ebf4449fc89a7a")
                .unwrap();
        e50b_prior_binding(&member).unwrap();
        let mut wrong_member = member.clone();
        wrong_member[0] ^= 1;
        assert!(e50b_prior_binding(&wrong_member).is_err());
        assert!(
            accepted_prior_value(
                &ConcreteWitnessLogical::Member(member),
                &ConcreteWitnessPhysical::Present(wrong_member)
            )
            .is_err()
        );
        let report = base(Profile::ResidualE50b, selected, summary).unwrap();
        assert_eq!(report["bounds"]["proofs_exact"], 97);
        assert_eq!(report["bounds"]["payload_tuples_exact"], 46);
        assert!(
            report["qualification"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == false)
        );
        let bytes = admission::evidence(
            Path::new(FIRST_RESIDUAL_BUNDLE),
            Path::new(FIRST_RESIDUAL_BUNDLE),
            "5c403c68f1cb9f742b309fd293ce192e05f453fb1f3e8c9a960f944c2f045469",
            649381,
            649381,
        )
        .unwrap();
        let WindowPlan::Residual(p) = plan else {
            panic!("residual plan")
        };
        let failure = replay_vrf_residual_bundle(&bytes, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(
            failure.error,
            rustaxa_storage::ConcreteWitnessError::Identity
        );
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }
    #[test]
    fn f49_fixed_selection_prior_and_full_barrier() {
        let (context, rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let selected = select(Profile::Residual49f8, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 45);
        for i in 0..45 {
            assert_eq!(selected[i]["parent_manifest_index"], F49_INDICES[i]);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 12);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 45);
        }
        for (field, value) in [
            ("voter_hex", json!(RESIDUAL_VOTER)),
            ("semantic_key", json!("other")),
            ("delayed_period", json!(FIRST + 12)),
            ("qualified", json!(true)),
            ("proof", json!({})),
            ("value", json!("00")),
            ("source_references", json!([])),
            (
                "source_references",
                json!([{"period":FIRST+8,"vote_hash_hex":"00"}]),
            ),
        ] {
            let mut bad = rows.clone();
            bad[F49_INDICES[0]][field] = value;
            assert!(select(Profile::Residual49f8, &bad).is_err(), "{field}");
        }
        let mut duplicate = rows.clone();
        duplicate[F49_INDICES[1]] = duplicate[F49_INDICES[0]].clone();
        assert!(select(Profile::Residual49f8, &duplicate).is_err());
        let mut reorder = rows.clone();
        reorder
            .as_array_mut()
            .unwrap()
            .swap(F49_INDICES[0], F49_INDICES[1]);
        assert!(select(Profile::Residual49f8, &reorder).is_err());
        let mut missing = rows.clone();
        missing.as_array_mut().unwrap().remove(F49_INDICES[0]);
        assert!(select(Profile::Residual49f8, &missing).is_err());
        let mut extra = rows.clone();
        extra[65]["voter_hex"] = json!(F49_VOTER);
        assert!(select(Profile::Residual49f8, &extra).is_err());
        assert!(authority(Profile::Residual49f8).is_err());
        let mut second_omission = rows.clone();
        second_omission[F49_INDICES[0]]["delayed_period"] = json!(FIRST + 45);
        assert!(select(Profile::Residual49f8, &second_omission).is_err());
        let (_, prior_selected) = first::selection(context, &rows).unwrap();
        assert_eq!(prior_selected[5]["parent_manifest_index"], 17);
        assert_eq!(
            prior_selected[5]["source_row"]["source_references"],
            json!([{"period":25706901,"vote_hash_hex":"7ddf0bc56f1ffa5e3d0dcb53ecbc4a159ac2971a7f8b3d6e71081b396a906d1c"}])
        );
        let canonical = canonical_prior().unwrap();
        for index in [0, canonical.len() / 2, canonical.len() - 1] {
            let mut bad = canonical.clone();
            bad[index] ^= 1;
            assert!(
                after_prior(
                    Profile::Residual49f8,
                    first::selection(
                        parent_input_handoff::load_authenticated_voter_authority()
                            .unwrap()
                            .0,
                        &rows
                    )
                    .unwrap()
                    .0,
                    &bad,
                    |_, _| -> Result<()> { panic!("bad prior reached49f8 constructor") }
                )
                .is_err()
            );
        }
        let (plan, selected, summary) = window_authority(Profile::Residual49f8).unwrap();
        assert_eq!(plan.context().delayed.len(), 48);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 56);
        assert_eq!(
            summary["prior_first_d"]["selected"]["value_hex"],
            "2c9895a89f0d58e2210f21f2129dc74de825a1b7ca03a15c600cda5f4c4a6dc3"
        );
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(hex::encode(t.key.0), F49_KEY);
            assert_eq!(
                selected[i]["tuple"]["identity"],
                native::identity_json(t.identity)
            );
        }
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.identity.state_root[0] ^= 1;
        assert!(f49_tuple_binding(0, bad_tuple).is_err());
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.key.0[0] ^= 1;
        assert!(f49_tuple_binding(0, bad_tuple).is_err());
        let mut bad_tuple = plan.tuples()[0];
        bad_tuple.address[0] ^= 1;
        assert!(f49_tuple_binding(0, bad_tuple).is_err());
        assert!(f49_tuple_binding(45, plan.tuples()[0]).is_err());
        let member =
            hex::decode("2c9895a89f0d58e2210f21f2129dc74de825a1b7ca03a15c600cda5f4c4a6dc3")
                .unwrap();
        f49_prior_binding(&member).unwrap();
        let mut wrong_member = member.clone();
        wrong_member[0] ^= 1;
        assert!(f49_prior_binding(&wrong_member).is_err());
        assert!(
            accepted_prior_value(
                &ConcreteWitnessLogical::Member(member),
                &ConcreteWitnessPhysical::Present(wrong_member)
            )
            .is_err()
        );
        let report = base(Profile::Residual49f8, selected, summary).unwrap();
        assert_eq!(report["bounds"]["proofs_exact"], 96);
        assert_eq!(report["bounds"]["payload_tuples_exact"], 45);
        assert!(
            report["qualification"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == false)
        );
        let bytes = admission::evidence(
            Path::new(FIRST_RESIDUAL_BUNDLE),
            Path::new(FIRST_RESIDUAL_BUNDLE),
            "5c403c68f1cb9f742b309fd293ce192e05f453fb1f3e8c9a960f944c2f045469",
            649381,
            649381,
        )
        .unwrap();
        let WindowPlan::Residual49f8(p) = plan else {
            panic!("residual plan")
        };
        let failure = replay_vrf_residual_49f8_bundle(&bytes, &p, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        // RCV3 is a different wire schema. RCV4 rejects its header before
        // tuple identity or payload qualification can begin.
        assert_eq!(failure.error, rustaxa_storage::ConcreteWitnessError::Schema);
        assert_eq!(failure.index, 0);
        assert_eq!(
            failure
                .usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }
    // The saved-report comparison admits only these explicit source-unit mappings.
    // Every other provenance field remains equal to its sealed original.
    fn residual_source_successor(before: &Value, after: &Value) -> Result<Value> {
        let path = before["path"].as_str().context("source pin path")?;
        let expected = match path {
            "vrf_shared_window.rs" => Some(include_bytes!("vrf_shared_window.rs").as_slice()),
            "../../../../rust/crates/rustaxa-storage/src/lib.rs"
            | "../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs"
            | "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs"
            | "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs"
            | "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs" => {
                storage_successor(path)
            }
            _ => None,
        };
        if let Some(bytes) = expected {
            ensure!(
                *after == json!({"path":path,"bytes":bytes.len(),"sha256":admission::sha(bytes)}),
                "exact source successor required"
            );
        } else {
            ensure!(before == after, "unchanged source required");
        }
        Ok(after.clone())
    }
    #[test]
    fn residual_source_mapping_rejects_substitution() {
        let bytes = include_bytes!("vrf_shared_window.rs");
        let exact = json!({"path":"vrf_shared_window.rs","bytes":bytes.len(),"sha256":admission::sha(bytes)});
        residual_source_successor(&exact, &exact).unwrap();
        for (field, value) in [
            ("path", json!("other.rs")),
            ("bytes", json!(0)),
            ("sha256", json!("00")),
        ] {
            let mut bad = exact.clone();
            bad[field] = value;
            assert!(residual_source_successor(&exact, &bad).is_err());
        }
        let unchanged = json!({"path":"lib.rs","bytes":17,"sha256":"fixed"});
        let mut substituted = unchanged.clone();
        substituted["sha256"] = json!("other");
        assert!(residual_source_successor(&unchanged, &substituted).is_err());
    }
    const FIRST_RESIDUAL_BUNDLE: &str = "/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/vrf-residual-first-voter-window.rcv3";
    #[test]
    fn first_residual_saved_bundle_report_is_completely_equal() {
        let path = Path::new(
            "/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/vrf-residual-first-voter-window-replay.json",
        );
        let mut old = admission::parse_unique(
            &admission::evidence(
                path,
                path,
                "c6e2c514ef2d923441d3b121cd69e33be5e51c5cab9792e3ae015ddc4daeb1bb",
                58552,
                REPORT_CAP,
            )
            .unwrap(),
        )
        .unwrap();
        verify_pin(&old["bundle"]);
        verify_pin(&old["authority"]["six_input_owner"]["executable"]);
        let scratch = Path::new("/home/fry/artifacts").join(format!(
            "first-residual-equivalence-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        replay(
            Profile::ResidualFirst,
            Path::new(FIRST_RESIDUAL_BUNDLE),
            &scratch,
        )
        .unwrap();
        let new = admission::parse_unique(
            &admission::read_capped(fs::File::open(&scratch).unwrap(), REPORT_CAP).unwrap(),
        )
        .unwrap();
        let exe = &new["authority"]["six_input_owner"]["executable"];
        assert_eq!(
            exe["path"],
            json!(fs::canonicalize(std::env::current_exe().unwrap()).unwrap())
        );
        verify_pin(exe);
        old["authority"]["six_input_owner"]["executable"] = exe.clone();
        for (parent, key) in [("", "vrf_sources"), ("authority", "sources")] {
            let (a, b) = if parent.is_empty() {
                (&mut old[key], &new[key])
            } else {
                (
                    &mut old[parent]["six_input_owner"][key],
                    &new[parent]["six_input_owner"][key],
                )
            };
            let a = a.as_array_mut().unwrap();
            let b = b.as_array().unwrap();
            assert_eq!(a.len(), b.len());
            for (before, after) in a.iter_mut().zip(b) {
                *before = residual_source_successor(before, after).unwrap();
            }
        }
        assert_eq!(old, new);
        assert_eq!(new["values"].as_array().unwrap().len(), 46);
        fs::remove_file(scratch).unwrap();
    }
    #[test]
    fn second_residual_saved_bundle_report_is_completely_equal() {
        let path = Path::new(
            "/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/vrf-residual-second-voter-window-replay.json",
        );
        let mut old = admission::parse_unique(
            &admission::evidence(
                path,
                path,
                "61d9eee9508222092c3d5595e7b7ec8c7f9b6150dfbc15f23c4bd05cbe15152a",
                58554,
                REPORT_CAP,
            )
            .unwrap(),
        )
        .unwrap();
        verify_pin(&old["bundle"]);
        verify_pin(&old["authority"]["six_input_owner"]["executable"]);
        let scratch = Path::new("/home/fry/artifacts").join(format!(
            "second-residual-equivalence-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        replay(
            Profile::ResidualB92a,
            Path::new(old["bundle"]["path"].as_str().unwrap()),
            &scratch,
        )
        .unwrap();
        let new = admission::parse_unique(
            &admission::read_capped(fs::File::open(&scratch).unwrap(), REPORT_CAP).unwrap(),
        )
        .unwrap();
        let exe = &new["authority"]["six_input_owner"]["executable"];
        assert_eq!(
            exe["path"],
            json!(fs::canonicalize(std::env::current_exe().unwrap()).unwrap())
        );
        verify_pin(exe);
        old["authority"]["six_input_owner"]["executable"] = exe.clone();
        for (parent, key) in [("", "vrf_sources"), ("authority", "sources")] {
            let (a, b) = if parent.is_empty() {
                (&mut old[key], &new[key])
            } else {
                (
                    &mut old[parent]["six_input_owner"][key],
                    &new[parent]["six_input_owner"][key],
                )
            };
            let a = a.as_array_mut().unwrap();
            let b = b.as_array().unwrap();
            assert_eq!(a.len(), b.len());
            for (before, after) in a.iter_mut().zip(b) {
                *before = residual_source_successor(before, after).unwrap();
            }
        }
        assert_eq!(old, new);
        assert_eq!(new["values"].as_array().unwrap().len(), 46);
        fs::remove_file(scratch).unwrap();
    }
    #[test]
    fn third_residual_saved_bundle_report_is_completely_equal() {
        let path = Path::new(
            "/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/vrf-residual-third-voter-window-replay.json",
        );
        let mut old = admission::parse_unique(
            &admission::evidence(
                path,
                path,
                "609bae5bfce834365e89423c10913fa921876192b905b978c0f757976c72ca44",
                58552,
                REPORT_CAP,
            )
            .unwrap(),
        )
        .unwrap();
        verify_pin(&old["bundle"]);
        verify_pin(&old["authority"]["six_input_owner"]["executable"]);
        let scratch = Path::new("/home/fry/artifacts").join(format!(
            "third-residual-equivalence-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        replay(
            Profile::ResidualE50b,
            Path::new(old["bundle"]["path"].as_str().unwrap()),
            &scratch,
        )
        .unwrap();
        let new = admission::parse_unique(
            &admission::read_capped(fs::File::open(&scratch).unwrap(), REPORT_CAP).unwrap(),
        )
        .unwrap();
        let exe = &new["authority"]["six_input_owner"]["executable"];
        assert_eq!(
            exe["path"],
            json!(fs::canonicalize(std::env::current_exe().unwrap()).unwrap())
        );
        verify_pin(exe);
        old["authority"]["six_input_owner"]["executable"] = exe.clone();
        for (parent, key) in [("", "vrf_sources"), ("authority", "sources")] {
            let (a, b) = if parent.is_empty() {
                (&mut old[key], &new[key])
            } else {
                (
                    &mut old[parent]["six_input_owner"][key],
                    &new[parent]["six_input_owner"][key],
                )
            };
            let a = a.as_array_mut().unwrap();
            let b = b.as_array().unwrap();
            assert_eq!(a.len(), b.len());
            for (before, after) in a.iter_mut().zip(b) {
                *before = residual_source_successor(before, after).unwrap();
            }
        }
        assert_eq!(old, new);
        assert_eq!(new["values"].as_array().unwrap().len(), 46);
        fs::remove_file(scratch).unwrap();
    }
    #[test]
    fn seventh_is_fixed_derived_and_reverse_rejected() {
        let (plan, selected, summary) = authority(Profile::Seventh).unwrap();
        assert_eq!(hex::encode(plan.voter()), VOTERS[6]);
        assert_eq!(selected.as_array().unwrap().len(), 47);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 64);
        assert_eq!(
            summary["prior_first_d"]["selected"]["value_hex"],
            "d574a33dae0aa6a0f41a2f7e3dd6d73d1048292bec55f03c36f27c56067c25fd"
        );
        for tuple in plan.tuples() {
            assert_eq!(
                hex::encode(tuple.key.0),
                "fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f"
            );
        }
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        assert_eq!(
            select(Profile::Seventh, &rows).unwrap(),
            select(Profile::Seventh, &rows).unwrap()
        );
        for index in [INDICES[6][0], INDICES[6][46]] {
            let mut wrong = rows.clone();
            wrong[index]["voter_hex"] = json!(VOTERS[5]);
            assert!(select(Profile::Seventh, &wrong).is_err());
            let mut wrong = rows.clone();
            wrong[index]["delayed_period"] = json!(FIRST);
            assert!(select(Profile::Seventh, &wrong).is_err());
        }
        let mut duplicate = rows.clone();
        duplicate[INDICES[6][1]] = duplicate[INDICES[6][0]].clone();
        assert!(select(Profile::Seventh, &duplicate).is_err());
        let p = Path::new(
            "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/sixth-voter/vrf-sixth-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            p,
            p,
            "7b270eb9a7bd023d66a74e7f3cf775e07321ad5361db08ed083756f54cf04d5b",
            678585,
            678585,
        )
        .unwrap();
        let f = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(f.error, rustaxa_storage::ConcreteWitnessError::Identity);
        assert_eq!(f.index, 0);
        assert_eq!(
            f.usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }

    #[test]
    fn fixed_successors_authenticate_prior_and_reject_substitution() {
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let prior_bytes = canonical_prior().unwrap();
        for (profile, key, prior_index) in [
            (
                Profile::Eighth,
                "d801919ffd29925243c28612983151535ef289be8872b1db4b4b41885167ee42",
                14,
            ),
            (
                Profile::Ninth,
                "da94f95efae9d2677940e6f37edbad5613af9bdb4d920a54465aae06f871e07d",
                15,
            ),
            (
                Profile::Tenth,
                "afff6c972f0508d1a892f0fb547e3044bf50ee2df3293d56c78ba7ab8939aebe",
                18,
            ),
        ] {
            let (plan, selected, summary) = authority(profile).unwrap();
            assert_eq!(hex::encode(plan.voter()), profile.voter());
            assert_eq!(
                summary["prior_first_d"]["selected"]["proof_index"],
                51 + prior_index
            );
            assert_eq!(selected.as_array().unwrap().len(), 47);
            for (i, tuple) in plan.tuples().iter().enumerate() {
                assert_eq!(hex::encode(tuple.key.0), key);
                assert_eq!(
                    selected[i]["tuple"]["identity"],
                    native::identity_json(tuple.identity)
                );
                assert_eq!(
                    selected[i]["parent_manifest_index"],
                    INDICES[profile as usize][i]
                );
            }
            assert_eq!(
                select(profile, &rows).unwrap(),
                select(profile, &rows).unwrap()
            );
            for index in [INDICES[profile as usize][0], INDICES[profile as usize][46]] {
                let mut wrong = rows.clone();
                wrong[index]["voter_hex"] = json!(VOTERS[6]);
                assert!(select(profile, &wrong).is_err());
                let mut wrong = rows.clone();
                wrong[index]["delayed_period"] = json!(FIRST);
                assert!(select(profile, &wrong).is_err());
            }
            let mut reordered = rows.clone();
            reordered
                .as_array_mut()
                .unwrap()
                .swap(INDICES[profile as usize][0], INDICES[profile as usize][1]);
            assert!(select(profile, &reordered).is_err());
            let mut duplicate = rows.clone();
            duplicate[INDICES[profile as usize][1]] =
                duplicate[INDICES[profile as usize][0]].clone();
            assert!(select(profile, &duplicate).is_err());
            let mut missing = rows.clone();
            missing
                .as_array_mut()
                .unwrap()
                .remove(INDICES[profile as usize][0]);
            assert!(select(profile, &missing).is_err());
            let (context, _, _) =
                parent_input_handoff::load_authenticated_voter_authority().unwrap();
            let (first_plan, _) = first::selection(context, &rows).unwrap();
            let mut bad_prior = prior_bytes.clone();
            bad_prior[0] ^= 1;
            assert!(
                after_prior(profile, first_plan, &bad_prior, |_, _| -> Result<()> {
                    panic!("prior failure reached window construction")
                })
                .is_err()
            );
            let old_path = Path::new(
                "/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/seventh-voter/vrf-seventh-voter-window.rcv2",
            );
            let old = admission::evidence(
                old_path,
                old_path,
                "a85b207821918e85e687180f94eca13a7e1dab6fc5659cbd4a80d917d476bf9f",
                680089,
                680089,
            )
            .unwrap();
            let failure = replay_vrf_voter_bundle(&old, &plan, ConcreteWitnessLimits::default())
                .err()
                .unwrap();
            assert_eq!(
                failure.error,
                rustaxa_storage::ConcreteWitnessError::Identity
            );
            assert_eq!(failure.index, 0);
            assert_eq!(
                failure
                    .usage
                    .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
                0
            );
        }
    }
}
