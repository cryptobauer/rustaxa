// Declarative finite signed-period actions. Run only through the pinned era harness.
package main

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"math/big"
	"strings"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
)

// eraAction declares inputs, never expected execution results or native state.
// Amount/Value are canonical unsigned decimal uint256 strings. Value defaults
// to0 only for methods without a required deposit; delegate/register require an
// explicit positive Value. Amount is required by undelegation/redelegation and
// is not substituted for Value. Business-invalid amounts/commission/gas remain
// real ABI inputs; only malformed input shapes and widths are rejected here.
//
// Who and CreatedByKey select integrationKeys; Validator/TargetValidator select
// integrationValidators (0..2). A new registered address uses a repeated-byte
// private key RegisterKey (default12), not an integration-validator index.
// Raw/call can address such a validator by exact calldata and a declared To.
// Input/To are canonical lowercase hex without0x. Create treats Input as runtime
// bytes and wraps initRuntime. Raw/call treat Input as exact calldata, including
// the old prepacked claim-all ABI when required. Nothing rewrites that payload.
//
// CreatedNonce is presence-sensitive: for raw/call it chooses the exact CREATE
// destination from integrationKeys[CreatedByKey], mutually exclusive with To.
// For create it may assert the expected address, with CreatedByKey==Who; the
// actual working nonce still comes from the normal signed execution path.
// GasPrice defaults1e9 only when omitted; explicit0 stays0. Gas0 means the
// declared source default500000. Value, custody locks, reward frequency, jail
// duration and commission dates are never shortened or synthesized here.
type eraAction struct {
	Period uint64 `json:"period"`
	Who int `json:"who"`
	Name string `json:"name"`
	Method string `json:"method"`
	Validator int `json:"validator,omitempty"`
	TargetValidator int `json:"target_validator,omitempty"`
	Amount string `json:"amount,omitempty"`
	Value string `json:"value,omitempty"`
	ID uint64 `json:"id,omitempty"`
	Commission uint16 `json:"commission,omitempty"`
	Input string `json:"input,omitempty"`
	To string `json:"to,omitempty"`
	CreatedByKey int `json:"created_by_key,omitempty"`
	CreatedNonce *uint64 `json:"created_nonce,omitempty"`
	GasPrice *uint64 `json:"gas_price,omitempty"`
	Gas uint64 `json:"gas,omitempty"`
	VotePeriod uint64 `json:"vote_period,omitempty"`
	RegisterKey *uint64 `json:"register_key,omitempty"`
}

func eraActionRequire(ok bool, action eraAction, message string) {
	if !ok { panic(fmt.Sprintf("declared action %d/%s: %s", action.Period, action.Name, message)) }
}

func eraActionNumber(text string, required bool, action eraAction, field string) *big.Int {
	if text == "" {
		eraActionRequire(!required, action, field+" must be explicitly declared")
		return new(big.Int)
	}
	value, ok := new(big.Int).SetString(text,10)
	eraActionRequire(ok && value.Sign() >= 0 && value.BitLen() <= 256 && value.String() == text,
		action, field+" must be canonical unsigned decimal uint256")
	return value
}

func eraActionHex(text string, size int, action eraAction, field string) []byte {
	value, err := hex.DecodeString(text)
	eraActionRequire(err == nil && hex.EncodeToString(value) == text && (size < 0 || len(value) == size),
		action, field+" must be canonical lowercase hex at its declared width")
	return value
}

func eraActionTarget(action eraAction) *common.Address {
	if action.CreatedNonce != nil {
		eraActionRequire(action.To == "", action, "CREATE-derived and explicit destinations conflict")
		owner := addressForPrivateKey(integrationKeys[action.CreatedByKey])
		address := crypto.CreateAddress(&owner,new(big.Int).SetUint64(*action.CreatedNonce))
		return &address
	}
	eraActionRequire(action.To != "", action, "raw/call requires To or CreatedNonce")
	address := common.BytesToAddress(eraActionHex(action.To,20,action,"to"))
	return &address
}

// eraDeclaredTransactions preserves the declared order within a period. It
// validates the entire input plan on every selection, so an invalid action in a
// later period cannot be hidden by selecting an earlier period. Signing, nonce
// selection, admission, native execution and receipt observation remain owned
// by eraSigned/eraRun and the actual Go transition. No JSON fallback is used.
func eraDeclaredTransactions(actions []eraAction, p uint64) []eraTransaction {
	seen := map[struct{ Period uint64; Name string }]bool{}
	rows := []eraTransaction{}
	for _, action := range actions {
		eraActionRequire(action.Period > 0 && action.Name != "" && strings.TrimSpace(action.Name) == action.Name,
			action, "positive execution period and nonempty exact name required")
		identity := struct{ Period uint64; Name string }{action.Period,action.Name}
		eraActionRequire(!seen[identity],action,"duplicate period/name")
		seen[identity] = true
		eraActionRequire(action.Who >= 0 && action.Who < len(integrationKeys),action,"invalid signer key index")
		eraActionRequire(action.CreatedByKey >= 0 && action.CreatedByKey < len(integrationKeys),action,"invalid CREATE owner key index")
		eraActionRequire(action.Validator >= 0 && action.Validator < len(integrationValidators),action,"invalid validator index")
		eraActionRequire(action.TargetValidator >= 0 && action.TargetValidator < len(integrationValidators),action,"invalid destination validator index")
		amountRequired := action.Method == "undelegate" || action.Method == "undelegateV2" || action.Method == "reDelegate"
		amount := eraActionNumber(action.Amount,amountRequired,action,"amount")
		depositRequired := action.Method == "delegate" || action.Method == "register"
		value := eraActionNumber(action.Value,depositRequired,action,"value")
		if depositRequired { eraActionRequire(value.Sign() > 0,action,"delegate/register deposit must be explicitly positive") }
		if action.Method == "delegate" && action.Amount != "" {
			eraActionRequire(amount.Cmp(value) == 0,action,"delegate Amount differs from its payable Value")
		} else if !amountRequired {
			eraActionRequire(action.Amount == "" || amount.Sign() == 0,action,"unused nonzero amount")
		}
		input := eraActionHex(action.Input,-1,action,"input")
		if action.To != "" { eraActionHex(action.To,20,action,"to") }
		isRaw := action.Method == "raw" || action.Method == "call"
		isCreate := action.Method == "create"
		eraActionRequire(isRaw || isCreate || (action.Input == "" && action.To == "" && action.CreatedNonce == nil),
			action,"native ABI method cannot silently override raw/create fields")
		if isRaw { eraActionTarget(action) }
		if isCreate {
			eraActionRequire(action.To == "" && len(input) <= 0xffff,action,"create destination/runtime bounds")
			if action.CreatedNonce != nil { eraActionRequire(action.CreatedByKey == action.Who,action,"create expected address owner differs from signer") }
		}
		if action.CreatedNonce == nil { eraActionRequire(action.CreatedByKey == 0,action,"CREATE owner requires an explicit CreatedNonce") }
		registerKey := uint64(12)
		if action.RegisterKey != nil {
			eraActionRequire(action.Method == "register",action,"register key outside registration method")
			registerKey = *action.RegisterKey
		}
		eraActionRequire(registerKey > 0 && registerKey < 255,action,"invalid repeated-byte registration private key")
		if action.Method == "slash" {
			eraActionRequire(action.VotePeriod > 0 && action.VotePeriod < action.Period,action,"proof must bind a declared earlier positive vote period")
		} else { eraActionRequire(action.VotePeriod == 0,action,"vote period outside slash method") }
		switch action.Method {
		case "delegate", "undelegate", "undelegateV2", "confirm", "confirmUndelegate", "confirmV2", "confirmUndelegateV2",
			"cancel", "cancelUndelegate", "cancelV2", "cancelUndelegateV2", "reDelegate", "claimRewards",
			"commission", "claimCommissionRewards", "claimAll", "claimAllRewards", "setCommission",
			"getEligibleVotes", "getJailBlock", "getJailedValidators", "slash", "register", "create", "call", "raw":
		default:
			eraActionRequire(false,action,"unknown method")
		}
		if action.Period != p { continue }
		a, b := integrationValidators[action.Validator], integrationValidators[action.TargetValidator]
		to := dpos.ContractAddress()
		var expectedContract *common.Address
		switch action.Method {
		case "delegate": input = integrationPack(false,"delegate",a)
		case "undelegate": input = integrationPack(false,"undelegate",a,amount)
		case "undelegateV2": input = integrationPack(false,"undelegateV2",a,amount)
		case "confirm", "confirmUndelegate": input = integrationPack(false,"confirmUndelegate",a)
		case "confirmV2", "confirmUndelegateV2": input = integrationPack(false,"confirmUndelegateV2",a,action.ID)
		case "cancel", "cancelUndelegate": input = integrationPack(false,"cancelUndelegate",a)
		case "cancelV2", "cancelUndelegateV2": input = integrationPack(false,"cancelUndelegateV2",a,action.ID)
		case "reDelegate": input = integrationPack(false,"reDelegate",a,b,amount)
		case "claimRewards": input = integrationPack(false,"claimRewards",a)
		case "commission", "claimCommissionRewards": input = integrationPack(false,"claimCommissionRewards",a)
		case "claimAll", "claimAllRewards": input = integrationPack(false,"claimAllRewards")
		case "setCommission": input = integrationPack(false,"setCommission",a,action.Commission)
		case "getEligibleVotes": input = integrationPack(false,"getValidatorEligibleVotesCount",a)
		case "getJailBlock": to = slashing.ContractAddress(); input = integrationPack(true,"getJailBlock",a)
		case "getJailedValidators": to = slashing.ContractAddress(); input = integrationPack(true,"getJailedValidators")
		case "slash":
			to = slashing.ContractAddress()
			input = integrationPack(true,"commitDoubleVotingProof",eraVote(action.Validator,1,action.VotePeriod),eraVote(action.Validator,2,action.VotePeriod))
		case "register":
			privateKey := bytes.Repeat([]byte{byte(registerKey)},32)
			address := addressForPrivateKey(privateKey)
			proof, err := secp256k1.Sign(crypto.Keccak256(address[:]),privateKey)
			eraActionRequire(err == nil && len(proof) == 65 && proof[64] <= 1,action,"registration proof signing failed")
			proof[64] += 27
			input = integrationPack(false,"registerValidator",address,proof,bytes.Repeat([]byte{0x77},32),action.Commission,"declared historical validator","")
		case "create":
			to = nil
			input = initRuntime(input)
			if action.CreatedNonce != nil {
				owner := addressForPrivateKey(integrationKeys[action.CreatedByKey])
				address := crypto.CreateAddress(&owner,new(big.Int).SetUint64(*action.CreatedNonce))
				expectedContract = &address
			}
		case "raw", "call": to = eraActionTarget(action)
		}
		signer := addressForPrivateKey(integrationKeys[action.Who])
		spec := fullSpec(action.Name,0,signer,integrationKeys[action.Who],to,0,input)
		spec.GasPrice = 1_000_000_000
		if action.GasPrice != nil { spec.GasPrice = *action.GasPrice }
		spec.Gas = action.Gas
		if spec.Gas == 0 { spec.Gas = 500_000 }
		spec.ExpectedContract = expectedContract
		rows = append(rows,eraTransaction{spec,new(big.Int).Set(value)})
	}
	return rows
}
