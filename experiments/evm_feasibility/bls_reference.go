// Pinned direct Go oracle for Taraxa's Ficus and Cacti BLS12-381 precompiles.
// The corpus bounds expensive operations: large MSM discount cases use only
// infinity points and zero scalars.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	bls12381 "github.com/consensys/gnark-crypto/ecc/bls12-381"
	"github.com/consensys/gnark-crypto/ecc/bls12-381/fp"
)

type blsRow struct {
	Name        string     `json:"name"`
	Registry    string     `json:"registry"`
	Operation   string     `json:"operation"`
	Address     byte       `json:"address"`
	Input       string     `json:"input"`
	RequiredGas uint64     `json:"required_gas"`
	Output      string     `json:"output"`
	Error       string     `json:"error"`
	Repeat      *blsRepeat `json:"repeat,omitempty"`
}

type blsRepeat struct {
	Element string `json:"element"`
	Count   int    `json:"count"`
}

func blsOracleRow(name, registry, operation string, address byte, input []byte) blsRow {
	var table vm.Precompiles
	switch registry {
	case "ficus":
		table = vm.PrecompiledContractsFicus
	case "cacti":
		table = vm.PrecompiledContractsCacti
	default:
		panic("unknown BLS registry")
	}
	contractAddress := common.BytesToAddress([]byte{address})
	precompile := table.Get(&contractAddress)
	if precompile == nil {
		panic("BLS operation absent from requested registry")
	}
	frame := vm.CallFrame{Input: input}
	output, err := precompile.Run(frame, nil)
	errorText := ""
	if err != nil {
		errorText = err.Error()
	}
	return blsRow{
		Name: name, Registry: registry, Operation: operation, Address: address,
		Input: hex.EncodeToString(input), RequiredGas: precompile.RequiredGas(frame, nil),
		Output: hex.EncodeToString(output), Error: errorText,
	}
}

func repeatedBlsOracleRow(name, registry, operation string, address byte, element []byte, count int) blsRow {
	row := blsOracleRow(name, registry, operation, address, repeated(element, count))
	row.Input = ""
	row.Repeat = &blsRepeat{Element: hex.EncodeToString(element), Count: count}
	return row
}

func clone(input []byte) []byte { return append([]byte(nil), input...) }

func scalar(value *big.Int) []byte {
	result := make([]byte, 32)
	value.FillBytes(result)
	return result
}

func encodeFp(value *fp.Element) []byte {
	result := make([]byte, 64)
	fp.BigEndian.PutElement((*[fp.Bytes]byte)(result[16:]), *value)
	return result
}

func encodeG1(point *bls12381.G1Affine) []byte {
	result := make([]byte, 128)
	copy(result[:64], encodeFp(&point.X))
	copy(result[64:], encodeFp(&point.Y))
	return result
}

func encodeG2(point *bls12381.G2Affine) []byte {
	result := make([]byte, 256)
	copy(result[:64], encodeFp(&point.X.A0))
	copy(result[64:128], encodeFp(&point.X.A1))
	copy(result[128:192], encodeFp(&point.Y.A0))
	copy(result[192:], encodeFp(&point.Y.A1))
	return result
}

func repeated(pair []byte, count int) []byte {
	result := make([]byte, 0, len(pair)*count)
	for i := 0; i < count; i++ {
		result = append(result, pair...)
	}
	return result
}

func main() {
	_, _, g1Generator, g2Generator := bls12381.Generators()
	g1 := encodeG1(&g1Generator)
	g2 := encodeG2(&g2Generator)
	g1Infinity := make([]byte, 128)
	g2Infinity := make([]byte, 256)

	g1Negative := new(bls12381.G1Affine).Neg(&g1Generator)
	g2Negative := new(bls12381.G2Affine).Neg(&g2Generator)
	g1Neg := encodeG1(g1Negative)
	g2Neg := encodeG2(g2Negative)

	// Scan nonzero x coordinates for a deterministic on-curve G1 point outside
	// the prime subgroup. Avoid the low-order (0,2) point because it can hide
	// errors in GLV scalar-boundary witnesses.
	var g1WrongSubgroupPoint bls12381.G1Affine
	foundG1WrongSubgroup := false
	for candidate := uint64(1); candidate < 1024; candidate++ {
		var x, rhs, y fp.Element
		x.SetUint64(candidate)
		rhs.Square(&x).Mul(&rhs, &x).Add(&rhs, new(fp.Element).SetUint64(4))
		if y.Sqrt(&rhs) == nil {
			continue
		}
		point := bls12381.G1Affine{X: x, Y: y}
		if point.IsOnCurve() && !point.IsInSubGroup() {
			g1WrongSubgroupPoint = point
			foundG1WrongSubgroup = true
			break
		}
	}
	if !foundG1WrongSubgroup {
		panic("failed to construct G1 non-subgroup point")
	}
	g1WrongSubgroup := encodeG1(&g1WrongSubgroupPoint)

	var two fp.Element
	two.SetUint64(2)

	// Scan small real x coordinates for the first square on the G2 twist. The
	// first accepted candidate is deterministic and outside the prime subgroup.
	var twistB bls12381.E2
	twistB.A0.SetUint64(4)
	twistB.A1.SetUint64(4)
	var g2WrongSubgroupPoint bls12381.G2Affine
	foundG2WrongSubgroup := false
	for candidate := uint64(0); candidate < 1024; candidate++ {
		var x, rhs, y bls12381.E2
		x.A0.SetUint64(candidate)
		rhs.Square(&x).Mul(&rhs, &x).Add(&rhs, &twistB)
		if y.Sqrt(&rhs) == nil {
			continue
		}
		point := bls12381.G2Affine{X: x, Y: y}
		if point.IsOnCurve() && !point.IsInSubGroup() {
			g2WrongSubgroupPoint = point
			foundG2WrongSubgroup = true
			break
		}
	}
	if !foundG2WrongSubgroup {
		panic("failed to construct G2 non-subgroup point")
	}
	g2WrongSubgroup := encodeG2(&g2WrongSubgroupPoint)

	fieldModulus := fp.Modulus()
	nonCanonicalFp := make([]byte, 64)
	fieldModulus.FillBytes(nonCanonicalFp[16:])
	badTopFp := make([]byte, 64)
	badTopFp[0] = 1
	badG1Top := append(clone(badTopFp), make([]byte, 64)...)
	badG1Field := append(clone(nonCanonicalFp), make([]byte, 64)...)
	badG1FieldThenTop := append(clone(nonCanonicalFp), badTopFp...)
	badG1Curve := make([]byte, 128)
	badG1Curve[127] = 1
	badG2Top := append(clone(badTopFp), make([]byte, 192)...)
	badG2Field := append(clone(nonCanonicalFp), make([]byte, 192)...)
	badG2FieldThenTop := append(append(clone(nonCanonicalFp), badTopFp...), make([]byte, 128)...)
	badG2Curve := make([]byte, 256)
	badG2Curve[255] = 1

	rows := []blsRow{}
	subgroupOrder := new(big.Int)
	if _, ok := subgroupOrder.SetString("52435875175126190479447740508185965837690552500527637822603658699938581184513", 10); !ok {
		panic("subgroup order")
	}
	orderMinusOne := new(big.Int).Sub(new(big.Int).Set(subgroupOrder), big.NewInt(1))
	orderPlusOne := new(big.Int).Add(new(big.Int).Set(subgroupOrder), big.NewInt(1))
	twiceOrder := new(big.Int).Mul(new(big.Int).Set(subgroupOrder), big.NewInt(2))
	twiceOrderMinusOne := new(big.Int).Sub(new(big.Int).Set(twiceOrder), big.NewInt(1))
	twiceOrderPlusOne := new(big.Int).Add(new(big.Int).Set(twiceOrder), big.NewInt(1))
	maxScalar := new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 256), big.NewInt(1))
	add := func(name, operation string, ficusAddress byte, cactiAddress *byte, input []byte) {
		rows = append(rows, blsOracleRow(name, "ficus", operation, ficusAddress, input))
		if cactiAddress != nil {
			rows = append(rows, blsOracleRow(name, "cacti", operation, *cactiAddress, input))
		}
	}
	addRepeated := func(name, operation string, ficusAddress byte, cactiAddress *byte, element []byte, count int) {
		rows = append(rows, repeatedBlsOracleRow(name, "ficus", operation, ficusAddress, element, count))
		if cactiAddress != nil {
			rows = append(rows, repeatedBlsOracleRow(name, "cacti", operation, *cactiAddress, element, count))
		}
	}
	address := func(value byte) *byte { return &value }

	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g1-add-generators", append(clone(g1), g1...)},
		{"g1-add-generator-infinity", append(clone(g1), g1Infinity...)},
		{"g1-add-infinities", make([]byte, 256)},
		{"g1-add-non-subgroup", append(clone(g1WrongSubgroup), g1Infinity...)},
		{"g1-add-empty", nil},
		{"g1-add-short", make([]byte, 255)},
		{"g1-add-long", make([]byte, 257)},
		{"g1-add-bad-top", append(clone(badG1Top), g1Infinity...)},
		{"g1-add-noncanonical-field", append(clone(badG1Field), g1Infinity...)},
		{"g1-add-field-before-later-top", append(clone(badG1FieldThenTop), g1Infinity...)},
		{"g1-add-off-curve", append(clone(badG1Curve), g1Infinity...)},
		{"g1-add-error-order", append(clone(badG1Curve), badG1Top...)},
	} {
		add(item.name, "g1_add", 0x0b, address(0x0b), item.input)
	}

	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g1-mul-generator-two", append(clone(g1), scalar(big.NewInt(2))...)},
		{"g1-mul-generator-order-plus-one", append(clone(g1), scalar(orderPlusOne)...)},
		{"g1-mul-infinity-max", append(clone(g1Infinity), scalar(maxScalar)...)},
		{"g1-mul-non-subgroup-one", append(clone(g1WrongSubgroup), scalar(big.NewInt(1))...)},
		{"g1-mul-non-subgroup-two", append(clone(g1WrongSubgroup), scalar(big.NewInt(2))...)},
		{"g1-mul-non-subgroup-order-minus-one", append(clone(g1WrongSubgroup), scalar(orderMinusOne)...)},
		{"g1-mul-non-subgroup-order", append(clone(g1WrongSubgroup), scalar(subgroupOrder)...)},
		{"g1-mul-non-subgroup-order-plus-one", append(clone(g1WrongSubgroup), scalar(orderPlusOne)...)},
		{"g1-mul-non-subgroup-twice-order-minus-one", append(clone(g1WrongSubgroup), scalar(twiceOrderMinusOne)...)},
		{"g1-mul-non-subgroup-twice-order", append(clone(g1WrongSubgroup), scalar(twiceOrder)...)},
		{"g1-mul-non-subgroup-twice-order-plus-one", append(clone(g1WrongSubgroup), scalar(twiceOrderPlusOne)...)},
		{"g1-mul-non-subgroup-max", append(clone(g1WrongSubgroup), scalar(maxScalar)...)},
		{"g1-mul-empty", nil},
		{"g1-mul-short", make([]byte, 159)},
		{"g1-mul-long", make([]byte, 161)},
		{"g1-mul-bad-top", append(clone(badG1Top), scalar(big.NewInt(1))...)},
		{"g1-mul-noncanonical-field", append(clone(badG1Field), scalar(big.NewInt(1))...)},
		{"g1-mul-off-curve", append(clone(badG1Curve), scalar(big.NewInt(1))...)},
	} {
		add(item.name, "g1_mul", 0x0c, nil, item.input)
	}

	g1PairZero := append(clone(g1Infinity), make([]byte, 32)...)
	g1PairOne := append(clone(g1), scalar(big.NewInt(1))...)
	g1PairTwo := append(clone(g1), scalar(big.NewInt(2))...)
	g1PairWrongSubgroup := append(clone(g1WrongSubgroup), scalar(big.NewInt(1))...)
	g1PairWrongSubgroupHigh := append(clone(g1WrongSubgroup), scalar(orderMinusOne)...)
	g1PairOrderPlusOne := append(clone(g1), scalar(orderPlusOne)...)
	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g1-multiexp-one", g1PairOne},
		{"g1-multiexp-two-pairs", append(clone(g1PairOne), g1PairTwo...)},
		{"g1-multiexp-scalar-reduction", g1PairOrderPlusOne},
		{"g1-multiexp-non-subgroup-one", g1PairWrongSubgroup},
		{"g1-multiexp-non-subgroup-high", g1PairWrongSubgroupHigh},
		{"g1-multiexp-mixed-subgroup-non-subgroup-high", append(clone(g1PairTwo), g1PairWrongSubgroupHigh...)},
		{"g1-multiexp-empty", nil},
		{"g1-multiexp-short", make([]byte, 159)},
		{"g1-multiexp-trailing", make([]byte, 161)},
		{"g1-multiexp-bad-top", append(clone(badG1Top), scalar(big.NewInt(1))...)},
		{"g1-multiexp-noncanonical-field", append(clone(badG1Field), scalar(big.NewInt(1))...)},
		{"g1-multiexp-off-curve", append(clone(badG1Curve), scalar(big.NewInt(1))...)},
	} {
		add(item.name, "g1_multiexp", 0x0d, address(0x0c), item.input)
	}
	addRepeated("g1-multiexp-k128-discount", "g1_multiexp", 0x0d, address(0x0c), g1PairZero, 128)
	addRepeated("g1-multiexp-k129-cap", "g1_multiexp", 0x0d, address(0x0c), g1PairZero, 129)

	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g2-add-generators", append(clone(g2), g2...)},
		{"g2-add-generator-infinity", append(clone(g2), g2Infinity...)},
		{"g2-add-infinities", make([]byte, 512)},
		{"g2-add-non-subgroup", append(clone(g2WrongSubgroup), g2Infinity...)},
		{"g2-add-empty", nil},
		{"g2-add-short", make([]byte, 511)},
		{"g2-add-long", make([]byte, 513)},
		{"g2-add-bad-top", append(clone(badG2Top), g2Infinity...)},
		{"g2-add-noncanonical-field", append(clone(badG2Field), g2Infinity...)},
		{"g2-add-field-before-later-top", append(clone(badG2FieldThenTop), g2Infinity...)},
		{"g2-add-off-curve", append(clone(badG2Curve), g2Infinity...)},
		{"g2-add-error-order", append(clone(badG2Curve), badG2Top...)},
	} {
		add(item.name, "g2_add", 0x0e, address(0x0d), item.input)
	}

	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g2-mul-generator-two", append(clone(g2), scalar(big.NewInt(2))...)},
		{"g2-mul-generator-order-plus-one", append(clone(g2), scalar(orderPlusOne)...)},
		{"g2-mul-infinity-max", append(clone(g2Infinity), scalar(maxScalar)...)},
		{"g2-mul-non-subgroup-one", append(clone(g2WrongSubgroup), scalar(big.NewInt(1))...)},
		{"g2-mul-non-subgroup-two", append(clone(g2WrongSubgroup), scalar(big.NewInt(2))...)},
		{"g2-mul-non-subgroup-order-minus-one", append(clone(g2WrongSubgroup), scalar(orderMinusOne)...)},
		{"g2-mul-non-subgroup-order", append(clone(g2WrongSubgroup), scalar(subgroupOrder)...)},
		{"g2-mul-non-subgroup-order-plus-one", append(clone(g2WrongSubgroup), scalar(orderPlusOne)...)},
		{"g2-mul-non-subgroup-twice-order-minus-one", append(clone(g2WrongSubgroup), scalar(twiceOrderMinusOne)...)},
		{"g2-mul-non-subgroup-twice-order", append(clone(g2WrongSubgroup), scalar(twiceOrder)...)},
		{"g2-mul-non-subgroup-twice-order-plus-one", append(clone(g2WrongSubgroup), scalar(twiceOrderPlusOne)...)},
		{"g2-mul-non-subgroup-max", append(clone(g2WrongSubgroup), scalar(maxScalar)...)},
		{"g2-mul-empty", nil},
		{"g2-mul-short", make([]byte, 287)},
		{"g2-mul-long", make([]byte, 289)},
		{"g2-mul-bad-top", append(clone(badG2Top), scalar(big.NewInt(1))...)},
		{"g2-mul-noncanonical-field", append(clone(badG2Field), scalar(big.NewInt(1))...)},
		{"g2-mul-off-curve", append(clone(badG2Curve), scalar(big.NewInt(1))...)},
	} {
		add(item.name, "g2_mul", 0x0f, nil, item.input)
	}

	g2PairZero := append(clone(g2Infinity), make([]byte, 32)...)
	g2PairOne := append(clone(g2), scalar(big.NewInt(1))...)
	g2PairTwo := append(clone(g2), scalar(big.NewInt(2))...)
	g2PairWrongSubgroup := append(clone(g2WrongSubgroup), scalar(big.NewInt(1))...)
	g2PairWrongSubgroupHigh := append(clone(g2WrongSubgroup), scalar(orderMinusOne)...)
	g2PairOrderPlusOne := append(clone(g2), scalar(orderPlusOne)...)
	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"g2-multiexp-one", g2PairOne},
		{"g2-multiexp-two-pairs", append(clone(g2PairOne), g2PairTwo...)},
		{"g2-multiexp-scalar-reduction", g2PairOrderPlusOne},
		{"g2-multiexp-non-subgroup-one", g2PairWrongSubgroup},
		{"g2-multiexp-non-subgroup-high", g2PairWrongSubgroupHigh},
		{"g2-multiexp-mixed-subgroup-non-subgroup-high", append(clone(g2PairTwo), g2PairWrongSubgroupHigh...)},
		{"g2-multiexp-empty", nil},
		{"g2-multiexp-short", make([]byte, 287)},
		{"g2-multiexp-trailing", make([]byte, 289)},
		{"g2-multiexp-bad-top", append(clone(badG2Top), scalar(big.NewInt(1))...)},
		{"g2-multiexp-noncanonical-field", append(clone(badG2Field), scalar(big.NewInt(1))...)},
		{"g2-multiexp-off-curve", append(clone(badG2Curve), scalar(big.NewInt(1))...)},
	} {
		add(item.name, "g2_multiexp", 0x10, address(0x0e), item.input)
	}
	addRepeated("g2-multiexp-k128-discount", "g2_multiexp", 0x10, address(0x0e), g2PairZero, 128)
	addRepeated("g2-multiexp-k129-cap", "g2_multiexp", 0x10, address(0x0e), g2PairZero, 129)

	validPair := append(clone(g1), g2...)
	negatedProduct := append(append(clone(validPair), g1Neg...), g2...)
	negatedG2Product := append(append(clone(validPair), g1...), g2Neg...)
	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"pairing-one-false", validPair},
		{"pairing-negated-g1-true", negatedProduct},
		{"pairing-negated-g2-true", negatedG2Product},
		{"pairing-infinity-true", make([]byte, 384)},
		{"pairing-empty", nil},
		{"pairing-short", make([]byte, 383)},
		{"pairing-trailing", make([]byte, 385)},
		{"pairing-g1-non-subgroup", append(clone(g1WrongSubgroup), g2...)},
		{"pairing-g2-non-subgroup", append(clone(g1), g2WrongSubgroup...)},
		{"pairing-g1-bad-top", append(clone(badG1Top), g2...)},
		{"pairing-g2-bad-top", append(clone(g1), badG2Top...)},
		{"pairing-g1-noncanonical-field", append(clone(badG1Field), g2...)},
		{"pairing-g2-noncanonical-field", append(clone(g1), badG2Field...)},
		{"pairing-g1-off-curve", append(clone(badG1Curve), g2...)},
		{"pairing-g2-off-curve", append(clone(g1), badG2Curve...)},
		{"pairing-g1-off-curve-before-g2-top", append(clone(badG1Curve), badG2Top...)},
		{"pairing-g2-top-before-g1-subgroup", append(clone(g1WrongSubgroup), badG2Top...)},
		{"pairing-first-subgroup-before-later-top", append(append(clone(g1WrongSubgroup), g2...), append(clone(g1), badG2Top...)...)},
	} {
		add(item.name, "pairing", 0x11, address(0x0f), item.input)
	}

	var fieldOne fp.Element
	fieldOne.SetOne()
	mapG1One := encodeFp(&fieldOne)
	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"map-g1-zero", make([]byte, 64)},
		{"map-g1-one", mapG1One},
		{"map-g1-empty", nil},
		{"map-g1-short", make([]byte, 63)},
		{"map-g1-long", make([]byte, 65)},
		{"map-g1-bad-top", badTopFp},
		{"map-g1-noncanonical-field", nonCanonicalFp},
	} {
		add(item.name, "map_g1", 0x12, address(0x10), item.input)
	}

	mapG2OneTwo := append(clone(mapG1One), encodeFp(&two)...)
	for _, item := range []struct {
		name  string
		input []byte
	}{
		{"map-g2-zero", make([]byte, 128)},
		{"map-g2-one-two", mapG2OneTwo},
		{"map-g2-empty", nil},
		{"map-g2-short", make([]byte, 127)},
		{"map-g2-long", make([]byte, 129)},
		{"map-g2-first-bad-top", append(clone(badTopFp), make([]byte, 64)...)},
		{"map-g2-second-bad-top", append(make([]byte, 64), badTopFp...)},
		{"map-g2-first-noncanonical", append(clone(nonCanonicalFp), make([]byte, 64)...)},
		{"map-g2-second-noncanonical", append(make([]byte, 64), nonCanonicalFp...)},
		{"map-g2-first-field-before-second-top", append(clone(nonCanonicalFp), badTopFp...)},
	} {
		add(item.name, "map_g2", 0x13, address(0x11), item.input)
	}

	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"bls": rows}); err != nil {
		panic(err)
	}
}
