"""Deterministic fixed-profile command builder and single-use sealed-profile guard.

Building a freeze grants no READY or fact acceptance. Independent review must
approve the exact freeze SHA and invocation. No builder database access occurs.
The guard permits exactly three copy metadata records and one read-only child.
All evidence and source pins are explicit; no discovery or recursive expansion.
"""
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time
import types

REPO = Path('/workspaces/rustaxa-evm')
SRC = REPO / 'experiments/evm_feasibility/snapshot_qualifier/src'
COPY = REPO / 'local/evm-state-db/snapshot-litenode-copy'
R3 = Path('/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013')
PREPARATION = R3 / 'seventh-voter-preparation.json'
PREPARATION_SHA = '363766ef6de0dad73bc71095172da1bfac6a87f72b4169e9d3ecc501e218ec00'
VOTER = 'c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa'
PROFILE = f'n4_remaining_vrf_voter_{VOTER}_v1'
KEY = 'fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f'
SHARED_RUN = Path("/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013")
RESIDUAL_RUN = Path('/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014')
PREVIOUS_RUN = Path('/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013')
RESIDUAL_VOTER = '03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b'
RESIDUAL_PROFILE = f'n4_residual_vrf_{RESIDUAL_VOTER}_v1'
RESIDUAL_KEY = '89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b'
RESIDUAL_INDICES = [62,119,179,236,344,401,461,521,578,638,695,749,806,866,920,977,1034,1091,1151,1211,1271,1328,1382,1430,1487,1547,1604,1661,1715,1772,1832,1889,1943,1997,2057,2114,2171,2231,2285,2345,2399,2459,2516,2576,2624,2684]
FIRST_CODE_RECORD = RESIDUAL_RUN/'code-and-gates-freeze.json'
FIRST_COMMAND_RECORD = RESIDUAL_RUN/'residual-first/vrf-residual-first-voter-window-command-freeze.json'
FIRST_OUTCOME_RECORD = RESIDUAL_RUN/'residual-first/fact-outcome-freeze.json'
SECOND_VOTER = 'b92a29912b08b4fa7404b02dfa2b8a971abdeda1'
SECOND_PROFILE = f'n4_residual_vrf_{SECOND_VOTER}_v1'
SECOND_KEY = '92f670429dc4076aa7771488f1512225a3bbf93eb17aa2e37a8cedcd463fff65'
SECOND_INDICES = [95,155,215,269,320,377,437,497,557,614,671,728,782,842,899,953,1010,1067,1127,1187,1247,1307,1358,1409,1463,1523,1580,1637,1694,1751,1808,1865,1919,1973,2033,2147,2207,2261,2321,2378,2435,2492,2552,2603,2660,2720]

THIRD_VOTER = 'e50b5452b2e8435404dbe06e6a05410c47b7583d'
THIRD_PROFILE = f'n4_residual_vrf_{THIRD_VOTER}_v1'
THIRD_KEY = '97c2d7a6a0dc3ae78ab1fe9147bd5db10088db1c01850d9b8bce75ae287361d2'
THIRD_INDICES = [107, 167, 227, 281, 332, 389, 449, 509, 569, 626, 683, 794, 854, 911, 965, 1022, 1079, 1139, 1199, 1259, 1319, 1370, 1421, 1475, 1535, 1592, 1649, 1706, 1763, 1820, 1877, 1931, 1985, 2045, 2102, 2159, 2219, 2273, 2333, 2390, 2447, 2504, 2564, 2615, 2672, 2732]
THIRD_EVIDENCE_TAIL = ['/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-all-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-all-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-focused-first.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-focused-first.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-quality.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/qualifier-quality.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/tool-residual-positive-final-2.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/tool-residual-positive-final-2.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-bridge-build.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-bridge-build.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-bridge-tests.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/storage-bridge-tests.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/canonical-parity.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/canonical-parity.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/native-compatibility-corrected.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/native-compatibility-corrected.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/first-d-compatibility-replay.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/first-d-compatibility-replay.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/native-first-complete-equivalence-final.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/native-first-complete-equivalence-final.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/argument-rejection.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/argument-rejection.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/fast-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/fast-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/actual-hook-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/actual-hook-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/code-acceptance-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/validation-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/source-commit.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/source-commit.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/source-hook-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/source-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/binary-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/preparation-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/storage-validation-current.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/tool-source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/residual-authority-review-v2.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/e50b46-independent-map-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-third/e50b46-candidate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-third/e50b46-structural-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-third/e50b46-pins.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-map/build_residual_map_corrected_v1.py', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-map/residual-424-corrected-v1.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/vrf-residual-second-voter-window-command-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/fact-outcome-freeze.json', '/home/fry/artifacts/vrf-residual-preparation-20261014/manifest.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/third-result-equivalence.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/third-result-equivalence.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/third-correction-record.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/third-command-closure-review.md']
THIRD_EVIDENCE_CAPS = [16384, 131544, 16384, 67436, 16384, 130554, 16384, 65536, 16384, 92706, 16384, 81324, 16384, 65536, 16384, 65536, 16384, 65536, 16384, 65536, 16384, 65536, 16384, 65536, 16384, 583136, 16384, 582060, 131072, 131072, 131072, 1048576, 131072, 131072, 131072, 131072, 131072, 131072, 131072, 131072, 131072, 131072, 41913, 923, 1487, 14706, 437589, 34625, 92988, 7371, 1893, 131072, 131072, 131072, 131072]
SECOND_HISTORY_PINS = [{'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/code-and-gates-freeze.json', 'bytes': 34625, 'sha256': 'fccb5d38776b207ea74c3bb360f5c220586a181c9f592d81a83ae6da3cfbc03b'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/vrf-residual-second-voter-window-command-freeze.json', 'bytes': 92988, 'sha256': 'd5db0ec359003df0141f8f5468e418f2fe7d3a2f6b679b658abdadea6aba6725'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/fact-outcome-freeze.json', 'bytes': 7371, 'sha256': '0466b004e68244355bb51bd7feb4fe202123d4361cfc2dbd608602cee6c4b386'}]
THIRD_CORRECTION_PIN = dict(path=str(RESIDUAL_RUN/'third-source/third-correction-record.json'), bytes=126549, sha256='c82bc82761b014e365bc8eb7870775f5b937d2362b2e80277b1806cb1a5e9874')
THIRD_AUTHORITY_PIN = dict(path=str(RESIDUAL_RUN/'third-source/residual-authority-review-v2.md'), bytes=10473, sha256='3cf3a409417250c152a675cbc9e0a2a33fa1406d3a4d4cf6e1f5e91074e0e842')

FOURTH_VOTER = '49f85534bbcf461d44fd3fc96feecf9abb6d38ad'
FOURTH_PROFILE = f'n4_residual_vrf_{FOURTH_VOTER}_v1'
FOURTH_KEY = '446a40c15b86d1effc8e92cf6bb7db72c56131bd9491c6a6bcb70cd0c0bac9f2'
FOURTH_INDICES = [77, 134, 194, 248, 305, 356, 416, 476, 536, 593, 653, 761, 821, 878, 935, 989, 1046, 1106, 1166, 1226, 1286, 1343, 1394, 1442, 1502, 1562, 1616, 1676, 1730, 1787, 1844, 1901, 1955, 2012, 2072, 2129, 2186, 2243, 2300, 2357, 2414, 2471, 2531, 2639, 2699]
FOURTH_EVIDENCE_TAIL = ['/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-all-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-all-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-focused-corrected.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-focused-corrected.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-quality-corrected.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/qualifier-quality-corrected.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/tool-residual-positive-final-3.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/tool-residual-positive-final-3.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-bridge-build.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-bridge-build.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-bridge-tests.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/storage-bridge-tests.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/canonical-parity.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/canonical-parity.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/native-compatibility-corrected.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/native-compatibility-corrected.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/first-d-compatibility-replay.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/first-d-compatibility-replay.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/native-first-complete-equivalence-final.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/native-first-complete-equivalence-final.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/argument-rejection.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/argument-rejection.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/fast-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/fast-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/actual-hook-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-fourth/actual-hook-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/code-acceptance-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/validation-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/source-commit.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/source-commit.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/source-hook-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/source-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/binary-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/preparation-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/storage-validation-current.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/tool-source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/residual-authority-review-45.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/independent-45-map-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-candidate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-pins.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-failure-history.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/fourth-result-equivalence.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/fourth-result-equivalence.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/fourth-correction-record.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/fourth-command-closure-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/vrf-residual-third-voter-window-command-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/fact-outcome-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-finite-projection-history-successor.json']
FOURTH_EVIDENCE_CAPS = [16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 131072, 65536, 65536, 327680, 16384, 131072, 16384, 131072, 65536, 131072, 131072, 16384, 131072, 131072, 41550, 4670, 1341, 2606, 16384, 327680, 131072, 131072, 36042, 92897, 131072, 1048576]
FOURTH_HISTORY_PINS = [{'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/code-and-gates-freeze.json', 'bytes': 36042, 'sha256': 'b42c90053574348079ae5038098231a329d85638d84247139c8ba3cbed138271'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/vrf-residual-third-voter-window-command-freeze.json', 'bytes': 92897, 'sha256': '16e2656fcbc9155fa114755357928926d5a7b50e5b514724c0d9af12eca6a715'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/fact-outcome-freeze.json', 'bytes': 7342, 'sha256': 'c5ff42f1428d74ac208ee647b6e10568d80944e7b9b1a6016280de8e7bddcbc0'}]
FOURTH_ENVELOPE_PIN = {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-finite-projection-history.json', 'bytes': 448801, 'sha256': 'd8e3d5d93da58ff548978874f9089002bcefbe74e1793991f6c7a6b7ac1be071'}
FOURTH_ENVELOPE_MEMBERS = [{'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-list/projection-v1.json', 'bytes': 172884, 'sha256': '57fbf2a431ab253710dadf281226c246e1504c3c5dbaa4d6bffbf18d3042e8b0'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-list/projection-v2.json', 'bytes': 179390, 'sha256': '7fa23bcba9fac817f75f7f751c73f68c764a384cfd14080b318c36b6541ede82'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-authority/design-checkpoint.md', 'bytes': 11530, 'sha256': 'ecc0e7a4bd723a95eb7d2ff8b3588cb2b2db538b87e7549bbfaef19d9967920f'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-authority/evidence-conditional-review.md', 'bytes': 7883, 'sha256': '30c59203e30528694157c4fa2577a8d05185ce2dba31c92afea444546a14279a'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-list/projection-v2-correction-evidence.json', 'bytes': 4278, 'sha256': 'c5305799a28ddc6da9c928e0aec2655366f773a2bb186c473142e57f50cf8d15'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-authority/evidence-v2-review.md', 'bytes': 11007, 'sha256': '2407e5b08552735b82ca61808c3d3d17abde7371903d3c2a3fe33e010b1db609'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-list/v2-first-failed-invocation.json', 'bytes': 36362, 'sha256': '44e8478cee341796082928f1291089ac3434d689e844e32deb991787b776041e'}]
FOURTH_AUTHORITY_PIN = {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/residual-authority-review-45.md', 'bytes': 124048, 'sha256': '911be93ffa8f72a38359afd43b90fa6520253919c47b0d6c5aad923adbbfd3d3'}
FOURTH_CORRECTION_PIN = {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/fourth-correction-record.json', 'bytes': 111165, 'sha256': '10a467bad6a5e83beaa6c8e2d292237303f14429d9f82e1a8cd0d4e8af400e8f'}

FOURTH_SUCCESSOR_ENVELOPE_PIN = {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-finite-projection-history-successor.json', 'bytes': 749065, 'sha256': 'cfadd1196bebcab7db7b78070325acc2983c493f3664c4bc4dc37ece2b18dd60'}
FOURTH_SUCCESSOR_MEMBERS = [{'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-finite-projection-history.json', 'bytes': 448801, 'sha256': 'd8e3d5d93da58ff548978874f9089002bcefbe74e1793991f6c7a6b7ac1be071'}, {'path': '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-source/tool-test-handoff.json', 'bytes': 82937, 'sha256': 'c56d4424337896e287431668bd3496b65c2e6d4c9025e149d8724d3986c0cd00'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/review/fourth-qualifier-first-failure-full.json', 'bytes': 159238, 'sha256': '572646575e119fc191ec788eae42b28a6abe670aa15af0542dbce092f58d6341'}, {'path': '/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/next-45-authority/first-source-failure-retention-addendum.md', 'bytes': 7780, 'sha256': '02d5b6f7c672ce352f9e99280d704f9e111820e222391a765b6a5b0ab61db17d'}]
RESIDUAL_PROFILES = {RESIDUAL_PROFILE, SECOND_PROFILE, THIRD_PROFILE, FOURTH_PROFILE}
SECOND_EVIDENCE_TAIL = ['/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/vrf-residual-first-voter-window-command-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/fact-outcome-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-all-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-all-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-focused-first.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-focused-first.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-quality.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/qualifier-quality.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/tool-residual-positive-final-2.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/tool-residual-positive-final-2.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-package.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-package.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-bridge-build.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-bridge-build.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-bridge-tests.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/storage-bridge-tests.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/canonical-parity.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/canonical-parity.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/native-compatibility-corrected.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/native-compatibility-corrected.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/first-d-compatibility-replay.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/first-d-compatibility-replay.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/native-first-complete-equivalence-final.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/native-first-complete-equivalence-final.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/argument-rejection.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/argument-rejection.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/fast-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/fast-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/actual-hook-gate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-second/actual-hook-gate.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/code-acceptance-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/validation-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/source-commit.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/source-commit.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/source-hook-summary.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/source-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/binary-successor-mapping.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/preparation-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/storage-validation-current.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/code-and-gates-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/tool-source-freeze.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/residual-authority-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/b92a46-independent-map-review.md', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-next/b92a46-spec-candidate.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-next/b92a46-structural-comparison.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-next/b92a46-pins.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-map/build_residual_map_corrected_v1.py', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/luna-map/residual-424-corrected-v1.json', '/home/fry/artifacts/vrf-residual-preparation-20261014/manifest.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/instance-spec.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-first/vrf-residual-first-voter-preparation.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/first-result-equivalence.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/first-result-equivalence.log', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/second-correction-record.json', '/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/second-source/second-command-closure-review.md']

RESIDUAL_PREFIXES = [str(REPO),
    '/home/fry/artifacts/n4-binding-resume-20261011',
    '/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010',
    '/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2',
    '/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011',
    '/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011',
    '/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012', str(R3), str(SHARED_RUN), str(PREVIOUS_RUN),
    '/home/fry/artifacts/vrf-shared-trial-preparation-20261013',
    '/home/fry/artifacts/vrf-successor-preparation-20261013', str(RESIDUAL_RUN),
    '/home/fry/artifacts/vrf-residual-preparation-20261014']
RESIDUAL_GATES = [
    'qualifier-all-package','qualifier-focused-first','qualifier-quality',
    'tool-residual-positive-final-2','storage-package','storage-bridge-build',
    'storage-bridge-tests','canonical-parity','native-compatibility-corrected',
    'first-d-compatibility-replay','native-first-complete-equivalence-final',
    'argument-rejection','fast-gate','actual-hook-gate']
SUCCESSORS = {
    "n4_remaining_vrf_voter_d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef_v1": dict(ordinal='eighth', voter='d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef', key='d801919ffd29925243c28612983151535ef289be8872b1db4b4b41885167ee42', prior_index=14, indices=[101, 161, 221, 275, 326, 383, 443, 503, 563, 620, 677, 734, 788, 848, 905, 959, 1016, 1073, 1133, 1193, 1253, 1313, 1364, 1415, 1469, 1529, 1586, 1643, 1700, 1757, 1814, 1871, 1925, 1979, 2039, 2096, 2153, 2213, 2267, 2327, 2384, 2441, 2498, 2558, 2609, 2666, 2726]),
    "n4_remaining_vrf_voter_d6d73e4c65c92787a2bbc80d109c293d8d3477af_v1": dict(ordinal='ninth', voter='d6d73e4c65c92787a2bbc80d109c293d8d3477af', key='da94f95efae9d2677940e6f37edbad5613af9bdb4d920a54465aae06f871e07d', prior_index=15, indices=[104, 164, 224, 278, 329, 386, 446, 506, 566, 623, 680, 737, 791, 851, 908, 962, 1019, 1076, 1136, 1196, 1256, 1316, 1367, 1418, 1472, 1532, 1589, 1646, 1703, 1760, 1817, 1874, 1928, 1982, 2042, 2099, 2156, 2216, 2270, 2330, 2387, 2444, 2501, 2561, 2612, 2669, 2729]),
    "n4_remaining_vrf_voter_f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d_v1": dict(ordinal='tenth', voter='f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d', key='afff6c972f0508d1a892f0fb547e3044bf50ee2df3293d56c78ba7ab8939aebe', prior_index=18, indices=[113, 173, 230, 287, 338, 395, 455, 515, 572, 632, 689, 743, 800, 860, 914, 971, 1028, 1085, 1145, 1205, 1265, 1322, 1376, 1424, 1481, 1541, 1598, 1655, 1709, 1769, 1826, 1883, 1937, 1991, 2051, 2108, 2165, 2225, 2279, 2339, 2396, 2453, 2510, 2570, 2618, 2678, 2738]),
}

def sealed_profile(profile):
    """Select only a fixed reviewed identity; unknown profiles supply no authority."""
    if profile == PROFILE:
        return dict(ordinal='seventh', voter=VOTER, key=KEY, prior_index=13)
    if profile == RESIDUAL_PROFILE:
        return dict(ordinal='residual-first', voter=RESIDUAL_VOTER, key=RESIDUAL_KEY,
                    prior_index=0, indices=RESIDUAL_INDICES)
    if profile == SECOND_PROFILE:
        return dict(ordinal='residual-second', voter=SECOND_VOTER, key=SECOND_KEY,
                    prior_index=12, indices=SECOND_INDICES)
    if profile == THIRD_PROFILE:
        return dict(ordinal='residual-third', voter=THIRD_VOTER, key=THIRD_KEY,
                    prior_index=16, indices=THIRD_INDICES)
    if profile == FOURTH_PROFILE:
        return dict(ordinal='residual-fourth', voter=FOURTH_VOTER, key=FOURTH_KEY,
                    prior_index=5, indices=FOURTH_INDICES)
    if profile not in SUCCESSORS:
        raise ValueError('unknown sealed profile')
    return SUCCESSORS[profile]


def expand_residual_evidence(value, profile=RESIDUAL_PROFILE):
    """Expand the reviewed explicit prefix representation once, without I/O.

    Order and identical duplicate occurrences survive. A conflicting repeated
    path or invalid type/lexical suffix rejects before any row file inspection.
    Count/body charges remain those of the fully expanded freeze.
    """
    if not isinstance(value, dict) or set(value) != {'prefixes', 'pins'} or value['prefixes'] != RESIDUAL_PREFIXES:
        raise ValueError('fixed residual evidence prefixes')
    rows = value['pins']
    # The fixed residual closures use 76/78/80 owner sources and two tools.
    # Two binaries, preflight, preparation and spec make 83/85/87 occurrences.
    if profile not in RESIDUAL_PROFILES:
        raise ValueError('fixed residual evidence profile')
    direct = 89 if profile == FOURTH_PROFILE else 87 if profile == THIRD_PROFILE else 85 if profile == SECOND_PROFILE else 83
    if not isinstance(rows, list) or len(rows) + direct > PIN_COUNT:
        raise ValueError('expanded evidence count')
    result, seen = [], {}
    for row in rows:
        if not isinstance(row, list) or len(row) != 4:
            raise ValueError('compressed pin shape')
        index, suffix, size, sha = row
        if type(index) is not int or not 0 <= index < len(RESIDUAL_PREFIXES) or type(size) is not int or not 0 <= size <= FILE_CAP:
            raise ValueError('compressed pin integer bounds')
        if not isinstance(suffix, str) or not suffix or '\\' in suffix or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in suffix):
            raise ValueError('compressed pin suffix')
        if any(part in ('', '.', '..', 'AI.md') for part in suffix.split('/')):
            raise ValueError('compressed pin lexical escape')
        if not isinstance(sha, str) or len(sha) != 64 or any(c not in '0123456789abcdef' for c in sha):
            raise ValueError('compressed pin SHA')
        path = str(outside_database(RESIDUAL_PREFIXES[index] + '/' + suffix))
        pin = dict(path=path, bytes=size, sha256=sha)
        if path in seen and seen[path] != pin:
            raise ValueError('conflicting repeated evidence pin')
        seen[path] = pin
        result.append(pin)
    return result

ENV = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8'}
METADATA_CAP = 65536
FREEZE_CAP = 131072
SOURCE_CAP = 1048576
SOURCE_TOTAL = 4194304
FILE_CAP = 268435456
PIN_TOTAL = 471859200
PIN_COUNT = 400
SINK_CAP = 65536
RESULT_CAP = 16384
TIMEOUT = 300
HISTORICAL_MAP = Path('/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/startup-pin-map.json')
HISTORICAL_MAP_SHA = 'bd5beab4dbd66c67d242d410c072540cf753dc3831771aa20f6f190a890a332b'
FOURTH_RECORD_CONTRACT = dict(
    path='/home/fry/artifacts/n4-evidence-01a13c53-b487-7dc2-954a-ba9af3ad695d-20261014/astra/contract-manifest-v7-final.json',
    bytes=132703, sha256='05cb4625567bf6f722a3aee20818fe0824aa8f8e6cbc9bb08b23035e8e722017')

# The seven reviewed successor paths retain their exact positions and caps.
for _slot, _name in [(28, 'code-acceptance-review-v5.md'),
                     (30, 'source-commit-v5.json'),
                     (31, 'source-commit-v5.log'),
                     (32, 'source-hook-summary-v5.json'),
                     (37, 'code-and-gates-freeze-v7.json'),
                     (46, 'fourth-result-equivalence-summary-v5.json'),
                     (49, 'fourth-command-closure-review-v7.md')]:
    FOURTH_EVIDENCE_TAIL[_slot] = str(RESIDUAL_RUN/'fourth-source'/_name)


def fourth_record_contract():
    """Read only the independently accepted fixed roster, without ancestry walks."""
    pin_file(FOURTH_RECORD_CONTRACT['path'], FOURTH_RECORD_CONTRACT, 262144)
    return read_json(FOURTH_RECORD_CONTRACT['path'], 262144)


def verify_fourth_records(evidence):
    """Authenticate all three finite reference records and every child occurrence.

    Bootstrap the shared verifier from its exact frozen child source bytes. No
    mutable import lookup occurs. Return complete repeated byte/source charges;
    any missing, changed, aliased or extra reference raises before physical reads.
    """
    contract = fourth_record_contract()
    code_path = contract['pending_validation_rows']['placement']
    by_path = {row['path']: row for row in evidence}
    pin_file(code_path, by_path[code_path], FREEZE_CAP)
    code = read_json(code_path, FREEZE_CAP)
    pending = code['record_tool_pins']
    definition = contract['pending_validation_rows']
    if not isinstance(pending, list) or len(pending) != 8:
        raise ValueError('fixed record tooling roster')
    for row, role, path, cap in zip(pending, definition['roles'], definition['paths'], definition['caps']):
        if set(row) != {'role', 'path', 'bytes', 'sha256'} or row['role'] != role or row['path'] != path or type(row['bytes']) is not int or not 0 <= row['bytes'] <= cap:
            raise ValueError('fixed record tooling row')
    for frozen in contract.get('frozen_validation_rows', []):
        required = {key: frozen[key] for key in ('role', 'path', 'bytes', 'sha256')}
        if required not in pending:
            raise ValueError('immutable first gate failure pin')
    source_pin = {key: pending[0][key] for key in ('path', 'bytes', 'sha256')}
    if source_pin['path'] != str(Path(__file__).resolve().with_name('evidence_records.py')):
        raise ValueError('fixed shared verifier source')
    pin_file(source_pin['path'], source_pin, 65536)
    source_path = regular_path(source_pin['path'])
    before = source_path.lstat()
    fd = os.open(source_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        initial = os.fstat(fd)
        raw = os.read(fd, 65537)
        after = source_path.lstat()
        fields = ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
        if any(getattr(before, key) != getattr(initial, key) or getattr(initial, key) != getattr(after, key) for key in fields):
            raise ValueError('shared verifier source identity drift')
    finally:
        os.close(fd)
    if len(raw) != source_pin['bytes'] or hashlib.sha256(raw).hexdigest() != source_pin['sha256']:
        raise ValueError('shared verifier loaded bytes drift')
    module = types.ModuleType('frozen_evidence_records')
    module.__file__ = source_pin['path']
    exec(compile(raw, source_pin['path'], 'exec'), module.__dict__)
    identities = {}
    total = count = streamed = 0
    for placement in contract['reference_placements']:
        destination = placement['destination']
        pin_file(destination, by_path[destination], FREEZE_CAP)
        value = read_json(destination, FREEZE_CAP)
        if destination == code_path:
            value = value['evidence_record']
        expected = list(placement['members'])
        if destination == code_path:
            expected += pending + contract['instruction_reference_rows']
        result = module.verify(value, destination, FOURTH_RECORD_CONTRACT, expected, identities)
        count += result['reference_count']
        total += result['referenced_bytes']
        streamed += result['streamed_bytes']
        if count > 64 or total > 1048576:
            raise ValueError('all record child count/byte cap')
    if count != 61:
        raise ValueError('exact finite record child coverage')
    summary_spec = contract['equivalence_summary']
    full_rows = [row for row in code['evidence_record']['references']
                 if row['role'] == summary_spec['full_record_role']]
    if len(full_rows) != 1:
        raise ValueError('unique full equivalence body role')
    full_pin = {key: full_rows[0][key] for key in ('path', 'bytes', 'sha256')}
    # The body has already been authenticated as its one semantic occurrence.
    # Recheck bytes on the bounded parse pass; report that additional read.
    if type(full_pin['bytes']) is not int or not 0 <= full_pin['bytes'] <= METADATA_CAP:
        raise ValueError('full equivalence parse admission')
    with regular_path(full_pin['path']).open('rb') as stream:
        full_raw = stream.read(METADATA_CAP + 1)
    if len(full_raw) != full_pin['bytes'] or hashlib.sha256(full_raw).hexdigest() != full_pin['sha256']:
        raise ValueError('full equivalence parse drift')
    full = module.decode(full_raw, METADATA_CAP)
    summary_path = summary_spec['path']
    pin_file(summary_path, by_path[summary_path], summary_spec['cap_bytes'])
    summary = read_json(summary_path, summary_spec['cap_bytes'])
    verify_equivalence_summary(summary, full)
    source_bytes = pending[0]['bytes'] + pending[1]['bytes']
    return dict(reference_count=count, referenced_bytes=total,
                streamed_bytes=streamed, semantic_read_bytes=len(full_raw),
                source_bytes=source_bytes)


def equivalence_summary(full):
    """Project every original semantic field; reference the sole full log child."""
    if not isinstance(full, dict) or not isinstance(full.get('full_gate_output'), str):
        raise ValueError('full equivalence record shape')
    output = full['full_gate_output'].encode('utf-8')
    return dict(schema='n4-result-equivalence-summary-v1',
                full_record_role='full-result-equivalence',
                projection={key: value for key, value in full.items() if key != 'full_gate_output'},
                omitted_output=dict(bytes=len(output), sha256=hashlib.sha256(output).hexdigest()),
                accepted=False)


def verify_equivalence_summary(summary, full):
    """Reject all mismatched fields before accepting a bounded projection."""
    expected = equivalence_summary(full)
    if not isinstance(summary, dict) or set(summary) != set(expected) or summary.get('accepted') is not False or summary != expected:
        raise ValueError('exact full equivalence projection')
    # JSON bool/int equality must not make an invented zero/false acceptable.
    if type(summary['omitted_output'].get('bytes')) is not int or encoded(summary, 16384) != encoded(expected, 16384):
        raise ValueError('equivalence projection scalar identity')



def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate JSON key: {key}')
        result[key] = value
    return result


def outside_database(path):
    if any(part in ('.', '..') for part in str(path).split('/')):
        raise ValueError('lexical path alias prohibited')
    path = Path(path)
    if not path.is_absolute() or path.name == 'AI.md':
        raise ValueError('absolute non-AI path required')
    if path.is_relative_to(COPY) or path.is_relative_to(REPO / 'data'):
        raise ValueError('database path prohibited')
    return path


def regular_path(path):
    path = outside_database(path)
    current = Path(path.anchor)
    for part in path.parts[1:]:
        current /= part
        if current.is_symlink():
            raise ValueError('symlink path prohibited')
    if path.resolve(strict=True) != path or not path.is_file():
        raise ValueError('canonical regular file required')
    return path


def pin_file(path, expected=None, cap=FILE_CAP):
    path = regular_path(path)
    size = path.stat().st_size
    if size > cap:
        raise ValueError('file cap exceeded')
    digest = hashlib.sha256()
    count = 0
    with path.open('rb') as stream:
        while block := stream.read(min(8192, size - count + 1)):
            count += len(block)
            if count > size:
                raise ValueError('file grew while hashing')
            digest.update(block)
    result = dict(path=str(path), bytes=count, sha256=digest.hexdigest())
    if count != size or (expected is not None and result != expected):
        raise ValueError(f'pin drift: {path}')
    return result


def read_json(path, cap=METADATA_CAP):
    path = regular_path(path)
    with path.open('rb') as stream:
        data = stream.read(cap + 1)
    if len(data) > cap:
        raise ValueError('JSON cap exceeded')
    return json.loads(data, object_pairs_hook=unique_object)


def historical_map(path):
    if Path(path) != HISTORICAL_MAP:
        raise ValueError('historical exception path')
    pin_file(path, dict(path=str(path), bytes=87841, sha256=HISTORICAL_MAP_SHA))
    return read_json(path, 87841)


def encoded(value, cap):
    data = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    if len(data) > cap:
        raise ValueError('encoded metadata cap exceeded')
    return data


def exclusive(path, data):
    path = outside_database(path)
    if path.parent.resolve(strict=True) != path.parent:
        raise ValueError('output parent alias')
    with path.open('xb') as stream:
        stream.write(data)


def expected_source_paths(profile=None):
    path = R3 / 'sixth-voter/source-freeze.json'
    pin_file(path, dict(path=str(path), bytes=13765,
                       sha256='669add9d030f189bc7a6b68023d1a63aa3ac39186bb12e45984805230cea1962'))
    legacy = read_json(path)
    paths = [row['path'] for row in legacy]
    if len(paths) != 65 or len(set(paths)) != 65:
        raise ValueError('legacy source closure')
    result = paths + [str(SRC / name) for name in [
        'vrf_shared_window.rs', 'bin/live_vrf_seventh_voter_window.rs',
        'bin/replay_vrf_seventh_voter_window.rs']]+[str(SRC / f'bin/{kind}_vrf_{ordinal}_voter_window.rs') for ordinal in ['eighth','ninth','tenth'] for kind in ['live','replay']]
    if profile in RESIDUAL_PROFILES:
        result += [str(SRC / f'bin/{kind}_vrf_residual_first.rs') for kind in ['live','replay']]
    if profile in {SECOND_PROFILE, THIRD_PROFILE, FOURTH_PROFILE}:
        result += [str(SRC / f'bin/{kind}_vrf_residual_b92a.rs') for kind in ['live','replay']]
    if profile in {THIRD_PROFILE, FOURTH_PROFILE}:
        result += [str(SRC / f'bin/{kind}_vrf_residual_e50b.rs') for kind in ['live','replay']]
    if profile == FOURTH_PROFILE:
        result += [str(SRC / f'bin/{kind}_vrf_residual_49f8.rs') for kind in ['live','replay']]
    return result


def validate_preflight(report, preparation=None):
    pin_file(PREPARATION, dict(path=str(PREPARATION), bytes=60570, sha256=PREPARATION_SHA))
    common_prepared = read_json(PREPARATION)
    fixed = sealed_profile(report['profile'])
    residual = report['profile'] in RESIDUAL_PROFILES
    count = 45 if report['profile'] == FOURTH_PROFILE else 46 if residual else 47
    voter, key = fixed['voter'], fixed['key']
    prepared = common_prepared if preparation is None else read_json(preparation)
    baseline = read_json(R3 / 'sixth-voter/vrf-sixth-voter-window-preflight.json', FREEZE_CAP)
    if report['status'] != 'authority_inputs_authenticated_no_database_open':
        raise ValueError('fixed profile preflight')
    selected = report['selection']
    if len(selected) != count or len(prepared['rows']) != count:
        raise ValueError('complete fixed selection')
    for i, (actual, expected) in enumerate(zip(selected, prepared['rows'])):
        if actual['parent_manifest_index'] != expected['original_parent_manifest_index'] or actual['source_row'] != expected['source_row']:
            raise ValueError('authenticated row binding')
        tuple_ = actual['tuple']
        if tuple_ != dict(identity=expected['identity'], address_hex=expected['account_hex'],
                          logical_key_hex=key, key_preimage_hex=f'0004{voter}'):
            raise ValueError('authenticated tuple binding')
        omitted = 12 if report['profile'] == THIRD_PROFILE else 36 if report['profile'] == SECOND_PROFILE else 5
        delayed_index = ([x for x in range(1, 48) if x not in (12, 45)][i]
                         if report['profile'] == FOURTH_PROFILE else
                         (i + 1 if i < omitted - 1 else i + 2) if residual else i + 1)
        if tuple_['identity']['period'] != 25706894 + delayed_index:
            raise ValueError('tuple order')
    authority = report['authority']
    if [r['path'] for r in authority['six_input_owner']['sources']] != [
            r['path'] for r in baseline['authority']['six_input_owner']['sources']]:
        raise ValueError('fixed embedded owner source closure')
    if authority['raw_artifact_reads'] != 7 or authority['raw_artifact_bytes'] != 6785636:
        raise ValueError('raw authority bounds')
    if authority['six_input_owner']['inputs'] != common_prepared['raw_authority_inputs'][:6]:
        raise ValueError('raw authority pins')
    for key in ['input', 'actual', 'semantic_sha256']:
        if authority['prior_first_d'][key] != baseline['authority']['prior_first_d'][key]:
            raise ValueError('complete prior proof provenance')
    expected_bounds = dict(baseline['bounds'], payload_tuples_exact=count, proofs_exact=51+count)
    if report['bounds'] != expected_bounds or authority['six_input_owner']['bounds'] != baseline['authority']['six_input_owner']['bounds']:
        raise ValueError('all thirteen ceilings and admission bounds')
    prior = authority['prior_first_d']['selected']
    expected = prepared['prior_first_d']
    if prior != dict(proof_index=51+fixed['prior_index'], identity=expected['identity'], voter_hex=voter,
                     value_hex=expected['accepted_value']['value_hex'], usable_vrf_key=True):
        raise ValueError('complete prior binding')
    if report['profile'] in SUCCESSORS or residual:
        if [r['parent_manifest_index'] for r in selected] != fixed['indices']:
            raise ValueError('sealed successor row indices')
        if expected['selection_index'] != fixed['prior_index'] or expected['parent_manifest_index'] != fixed['prior_index']*3+2:
            raise ValueError('sealed successor prior indices')
    residual_value = ('2c9895a89f0d58e2210f21f2129dc74de825a1b7ca03a15c600cda5f4c4a6dc3'
                      if report['profile'] == FOURTH_PROFILE else 'd25384032e092d1e74d81fc2819cdade58bae55c144d4a4f47ebf4449fc89a7a'
                      if report['profile'] == THIRD_PROFILE else '04d5cbb4aed88f984596c757d953bd962bf9e1353046b3e28af0aac6366cab93'
                      if report['profile'] == SECOND_PROFILE else
                      '6fabaf3eaa629b97b1dfaf77b9025072a1197299dd90cb19e46038794e91447b')
    if residual and (prior['value_hex'] != residual_value
                     or prior['identity'] != baseline['authority']['prior_first_d']['selected']['identity']):
        raise ValueError('fixed residual prior value/root')
    if report['bounds']['payload_tuples_exact'] != count or report['bounds']['proofs_exact'] != 51+count:
        raise ValueError('proof scope')
    if any(report['qualification'].values()):
        raise ValueError('preflight cannot qualify facts')


def validate_residual_history(evidence, profile=RESIDUAL_PROFILE):
    """Keep the full inherited sequence; substitute only four reviewed tuples.

    First reads the fixed predecessor spec/manifest and archived predecessors.
    Second retains the exact accepted first command/outcome records and checks
    the six named immutable originals. No recursive expansion occurs. The
    mandatory 233 inherited occurrences retain their order and multiplicity;
    the second requires its fixed 57-path tail; the third requires its named
    55-path tail with only the two exact historical input representations.
    """
    old_spec = PREVIOUS_RUN/'eighth-voter/instance-spec.json'
    pin_file(old_spec, dict(path=str(old_spec),bytes=65434,
                           sha256='66d8e9d89808af7d42632df0be37016da31a7a8ba21b80e14354d24a97897abd'))
    manifest = Path(RESIDUAL_PREFIXES[13])/'manifest.json'
    pin_file(manifest,dict(path=str(manifest),bytes=1893,
                         sha256='e094ad41fdae2a5ae0edf9d1b0eed9839532d2ab4a8e5dd1e03b99ac67bc2565'))
    mapping = read_json(manifest)['authorized_instruction_paths']
    archives = {
        str(REPO/'doc/evm_research/n4_next_10point_plan.md'):'3-n4_next_10point_plan.md',
        str(REPO/'next_executable_slice_prompt.md'):'6-next_executable_slice_prompt.md',
        str(REPO/'doc/evm_research/n6_restart_checkpoint.md'):'4-n6_restart_checkpoint.md',
        str(REPO/'doc/evm_research/n6_remaining_branch_queue.md'):'5-n6_remaining_branch_queue.md'}
    replacements = {}
    for item in mapping:
        predecessor = dict(item['preparation_predecessor'],path=item['path'])
        successor = {k:item[k] for k in ['path','bytes','sha256']}
        archive = PREVIOUS_RUN/'frozen-instructions'/archives[item['path']]
        archive_pin = dict(predecessor,path=str(archive))
        if archive_pin not in evidence and profile not in {SECOND_PROFILE, THIRD_PROFILE, FOURTH_PROFILE}:
            raise ValueError('missing archived instruction predecessor')
        pin_file(archive,archive_pin)
        replacements[item['path']] = (predecessor,successor)
    expected = []
    for pin in read_json(old_spec)['evidence']:
        if pin['path'] in replacements:
            predecessor,successor = replacements[pin['path']]
            if pin != predecessor:
                raise ValueError('instruction predecessor drift')
            expected.append(successor)
        else:
            expected.append(pin)
    if profile == FOURTH_PROFILE:
        # Only this profile has the separately accepted seven-row successor.
        # Keep historical logical pins in their immutable records and verify
        # their fixed archive bodies before using the authorized current rows.
        for item in fourth_record_contract()['instruction_mapping']:
            index = item['inherited_index']
            historical = item['historical_archive']
            pin_file(historical['path'], historical)
            before = dict(historical, path=item['logical_path'])
            if expected[index] != before:
                raise ValueError('fixed historical instruction mapping')
            current = item['authorized_current']
            pin_file(current['path'], current)
            expected[index] = current
    if len(expected) != 233 or evidence[:233] != expected:
        raise ValueError('full inherited evidence sequence')
    if profile == FOURTH_PROFILE:
        _validate_fourth_history(evidence)
    if profile == THIRD_PROFILE:
        _validate_third_history(evidence)
    if profile == SECOND_PROFILE:
        # Named six-record successor representation: no recursive expansion or
        # removal from the mandatory 233 historical occurrence sequence.
        pin_file(FIRST_CODE_RECORD, dict(path=str(FIRST_CODE_RECORD), bytes=32274,
                 sha256='7ec9fa0a8ca4e813537cdadcbae9ff7fdbc9d08e543c8b08f7e8de246bc07e3b'))
        first_command = FIRST_COMMAND_RECORD
        first_outcome = FIRST_OUTCOME_RECORD
        pin_file(first_command, dict(path=str(first_command), bytes=92315,
                 sha256='6d5e43bb1f64bcece4804b1b81d047c633b407dace111c3c8d86f2651ec9d61b'))
        pin_file(first_outcome, dict(path=str(first_outcome), bytes=12440,
                 sha256='04badad6795a837ee5a9c2e313d4854f5dfd40eb850236a498bdaca281c13019'))
        first = read_json(first_command, FREEZE_CAP)
        if first['pin_count'] != 375 or first['pinned_file_bytes'] != 418519918:
            raise ValueError('first historical command scope')
        retained = read_json(first_outcome)
        if retained['command_freeze'] != pin_file(first_command):
            raise ValueError('first accepted outcome parent')
        for index in [319, 320, 366, 367, 368, 369]:
            old = first['pins'][index]
            pin_file(old['path'], old)
            if old in evidence[233:]:
                raise ValueError('unreviewed six-record successor representation')
        if len(evidence) != 290 or [r['path'] for r in evidence[233:]] != SECOND_EVIDENCE_TAIL:
            raise ValueError('exact second evidence sequence')



def _validate_fourth_envelope():
    """Retain exactly the base envelope and two complete first source failures.

    This one fixed four-member successor remains an ordinary 1 MiB source.
    Validate all full original bodies, then the known seven-member base only.
    No generic recursion, metadata-cap increase or extra occurrence is allowed.
    Any shape, path, pin, duplicate-key or extraction mismatch raises before use.
    """
    pin_file(FOURTH_SUCCESSOR_ENVELOPE_PIN['path'], FOURTH_SUCCESSOR_ENVELOPE_PIN, 1048576)
    with Path(FOURTH_SUCCESSOR_ENVELOPE_PIN['path']).open('rb') as stream:
        body = stream.read(1048577)
    if len(body) > 1048576 or len(body) != FOURTH_SUCCESSOR_ENVELOPE_PIN['bytes'] or hashlib.sha256(body).hexdigest() != FOURTH_SUCCESSOR_ENVELOPE_PIN['sha256']:
        raise ValueError('fixed fourth successor admission')
    value = json.loads(body, object_pairs_hook=unique_object)
    if set(value) != {'schema_id', 'artifact_role', 'members'} or value['schema_id'] != 'n4-fixed-45-first-source-failure-successor-v1' or len(value['members']) != 4:
        raise ValueError('fixed fourth successor shape')
    for member, expected in zip(value['members'], FOURTH_SUCCESSOR_MEMBERS):
        if set(member) != {'original_absolute_path', 'bytes', 'sha256', 'body_utf8'} or member['original_absolute_path'] != expected['path'] or member['bytes'] != expected['bytes'] or member['sha256'] != expected['sha256'] or not isinstance(member['body_utf8'], str):
            raise ValueError('fixed fourth successor member')
        decoded = member['body_utf8'].encode('utf-8')
        if len(decoded) != expected['bytes'] or hashlib.sha256(decoded).hexdigest() != expected['sha256']:
            raise ValueError('fixed fourth successor full-body extraction')
        pin_file(expected['path'], expected)
        # Exact full bytes are checked, not a JSON reserialization of originals.
        with Path(expected['path']).open('rb') as stream:
            original = stream.read(expected['bytes'] + 1)
        if original != decoded:
            raise ValueError('fixed fourth successor original bytes')
    # Only this immutable named base is admitted at the one permitted level.
    _validate_fourth_base_envelope()


def _validate_fourth_base_envelope():
    """Authenticate exactly seven full bodies at the named 1 MiB source role.

    This decoder accepts only the fixed ordinary evidence file, never enlarged
    metadata admission or recursively discovered members. Pin, size, duplicate
    key, shape, path or decoded-body mismatch raises before command use.
    """
    pin_file(FOURTH_ENVELOPE_PIN['path'], FOURTH_ENVELOPE_PIN, 1048576)
    with Path(FOURTH_ENVELOPE_PIN['path']).open('rb') as stream:
        body = stream.read(1048577)
    if len(body) > 1048576 or len(body) != FOURTH_ENVELOPE_PIN['bytes'] or hashlib.sha256(body).hexdigest() != FOURTH_ENVELOPE_PIN['sha256']:
        raise ValueError('fixed fourth envelope admission')
    value = json.loads(body, object_pairs_hook=unique_object)
    if set(value) != {'schema_id', 'artifact_role', 'extraction_provenance', 'members'} or value['schema_id'] != 'n4-fixed-45-finite-projection-history-v1' or len(value['members']) != 7:
        raise ValueError('fixed fourth envelope shape')
    for member, expected in zip(value['members'], FOURTH_ENVELOPE_MEMBERS):
        if set(member) != {'original_absolute_path', 'bytes', 'sha256', 'body_utf8'} or {k: member[k] for k in ('bytes', 'sha256')} != {k: expected[k] for k in ('bytes', 'sha256')} or member['original_absolute_path'] != expected['path'] or not isinstance(member['body_utf8'], str):
            raise ValueError('fixed fourth envelope member')
        decoded = member['body_utf8'].encode('utf-8')
        if len(decoded) != expected['bytes'] or hashlib.sha256(decoded).hexdigest() != expected['sha256']:
            raise ValueError('fixed fourth full-body extraction')
        pin_file(expected['path'], expected)


def _validate_fourth_history(evidence):
    """Retain 233 ordered occurrences and only the two named map representations.

    Authenticate the three accepted third records, their exact parent relation
    and map/helper members 363/364. Keep the fixed 54-body tail, all caps and the
    complete seven-member projection history; reject drift without recursion.
    """
    if len(evidence) != 287 or [r['path'] for r in evidence[233:]] != FOURTH_EVIDENCE_TAIL:
        raise ValueError('exact fourth evidence sequence')
    for row, cap in zip(evidence[233:], FOURTH_EVIDENCE_CAPS):
        if row['bytes'] > cap:
            raise ValueError('strict fourth evidence body cap')
    for row in FOURTH_HISTORY_PINS + [FOURTH_SUCCESSOR_ENVELOPE_PIN, FOURTH_AUTHORITY_PIN, FOURTH_CORRECTION_PIN]:
        pin_file(row['path'], row)
        if row not in evidence[233:]:
            raise ValueError('fourth fixed history membership')
    command = read_json(FOURTH_HISTORY_PINS[1]['path'], FREEZE_CAP)
    seal = read_json(FOURTH_HISTORY_PINS[2]['path'])
    if command['pin_count'] != 375 or command['pinned_file_bytes'] != 419290696 or seal['command_freeze'] != FOURTH_HISTORY_PINS[1] or seal['total_occurrences_with_fixed_parent'] != 399 or seal['total_bytes_with_fixed_parent'] != 420538339 or seal['source_commit'] != command['source_commit'] or FOURTH_HISTORY_PINS[0] not in command['pins']:
        raise ValueError('third accepted fixed history relation')
    for index, expected in zip((363, 364), (
        dict(path=str(RESIDUAL_RUN/'luna-map/build_residual_map_corrected_v1.py'), bytes=14706, sha256='7c35487e939e94156a1a0e7bd3f14dba846ccf1004e908cce89d0302ab255e07'),
        dict(path=str(RESIDUAL_RUN/'luna-map/residual-424-corrected-v1.json'), bytes=437589, sha256='57d27bbd664c0188d96c0d3377a85432c2c2190f8763039c53f125a6ae520291'))):
        if command['pins'][index] != expected or expected in evidence[233:]:
            raise ValueError('exact fourth map/helper representation')
        pin_file(expected['path'], expected)
    _validate_fourth_envelope()


def _validate_third_history(evidence):
    """Validate only the named third historical representation and fixed tail.

    Preserve every inherited occurrence. Authenticate the three immutable second
    records, their exact first-history members and two original input members.
    Charge records once at their reviewed positions; never discover or expand
    ancestry. Unknown tails, forged same-size records and excess bodies reject.
    """
    if len(evidence) != 288 or [r['path'] for r in evidence[233:]] != THIRD_EVIDENCE_TAIL:
        raise ValueError('exact third evidence sequence')
    for row, cap in zip(evidence[233:], THIRD_EVIDENCE_CAPS):
        if row['bytes'] > cap:
            raise ValueError('strict third evidence body cap')
    for pin in SECOND_HISTORY_PINS + [THIRD_CORRECTION_PIN, THIRD_AUTHORITY_PIN]:
        pin_file(pin['path'], pin)
        if pin not in evidence[233:]:
            raise ValueError('third immutable history/correction membership')
    command = read_json(SECOND_HISTORY_PINS[1]['path'], FREEZE_CAP)
    outcome = read_json(SECOND_HISTORY_PINS[2]['path'])
    if command['pin_count'] != 375 or command['pinned_file_bytes'] != 418995822 \
            or outcome['command_freeze'] != SECOND_HISTORY_PINS[1] \
            or outcome['total_occurrences_with_fixed_parent'] != 399 \
            or outcome['total_bytes_with_fixed_parent'] != 420013586:
        raise ValueError('second accepted historical scope')
    first_pins = [dict(path=str(FIRST_CODE_RECORD), bytes=32274,
        sha256='7ec9fa0a8ca4e813537cdadcbae9ff7fdbc9d08e543c8b08f7e8de246bc07e3b'),
        dict(path=str(FIRST_COMMAND_RECORD), bytes=92315,
        sha256='6d5e43bb1f64bcece4804b1b81d047c633b407dace111c3c8d86f2651ec9d61b'),
        dict(path=str(FIRST_OUTCOME_RECORD), bytes=12440,
        sha256='04badad6795a837ee5a9c2e313d4854f5dfd40eb850236a498bdaca281c13019')]
    for index, pin in zip([316,317,318], first_pins):
        if command['pins'][index] != pin:
            raise ValueError('fixed first history inside second command')
        pin_file(pin['path'], pin)
    # Preserve the existing six-record relationship explicitly. This is the
    # fixed accepted first command, never a recursive ancestry traversal.
    first_command = read_json(FIRST_COMMAND_RECORD, FREEZE_CAP)
    if first_command['pin_count'] != 375 or first_command['pinned_file_bytes'] != 418519918:
        raise ValueError('first accepted historical scope')
    for index in [319,320,366,367,368,369]:
        original = first_command['pins'][index]
        pin_file(original['path'], original)
        if original in evidence[233:]:
            raise ValueError('unreviewed six-record successor representation')
    omitted = [dict(path=str(FIRST_COMMAND_RECORD.parent/'instance-spec.json'), bytes=52511,
        sha256='c9355181f75a48976a4a858de71966c0e8e77dad0a5b51a97b6a1b5f18772f7f'),
        dict(path=str(FIRST_COMMAND_RECORD.parent/'vrf-residual-first-voter-preparation.json'), bytes=35410,
        sha256='438ca3651e81f955016d705108791eaac6603054927a678f14fbaa02b9f9cdb9')]
    for index, pin in zip([367,368], omitted):
        if command['pins'][index] != pin or pin in evidence[233:]:
            raise ValueError('exact two third historical input representations')
        pin_file(pin['path'], pin)


def _validate_third_fit(pins, binaries):
    """Keep the third fixed pair at 420 MiB and each executable at 210 MiB.

    All repeated body occurrences remain charged under the 30 MiB other-body
    bound. No file reads or acceptance occur; excess inputs raise ValueError.
    """
    if any(row['bytes'] > 220200960 for row in binaries):
        raise ValueError('strict third executable body cap')
    _validate_second_fit(pins, binaries)


def _validate_second_fit(pins, binaries):
    """Enforce the named stricter second-profile body limits after pin validation.

    Inputs are the complete charged occurrence list and its two fixed binary
    entries. Duplicate occurrences remain charged. Reject either limit with
    ValueError; no file inspection, publication or qualification occurs here.
    """
    binary_bytes = sum(row['bytes'] for row in binaries)
    other_bytes = sum(row['bytes'] for row in pins) - binary_bytes
    if binary_bytes > 440401920 or other_bytes > 31457280:
        raise ValueError('strict second command fit')


def build(spec_path):
    """Return deterministic bytes after all explicit pins and sealed fields pass.

    The instance spec, freeze and invocation need independent review. This
    function never opens, scans or stats a database path and publishes nothing.
    """
    spec = read_json(spec_path)
    pin_file(spec['preflight']['path'], spec['preflight'], FREEZE_CAP)
    report = read_json(spec['preflight']['path'], FREEZE_CAP)
    fixed = sealed_profile(report['profile'])
    residual = report['profile'] in RESIDUAL_PROFILES
    ordinal = fixed['ordinal']
    fields = {'schema', 'run', 'source_commit', 'sources', 'tool_sources', 'binaries', 'preflight', 'evidence', 'copy_metadata'}
    if report['profile'] in SUCCESSORS or residual:
        fields.add('preparation')
    if set(spec) != fields or spec['schema'] != 1:
        raise ValueError('instance spec schema')
    run = outside_database(spec['run'])
    if not run.is_relative_to(Path('/home/fry/artifacts')) or run.resolve(strict=True) != run:
        raise ValueError('persistent canonical run required')
    if residual and run != RESIDUAL_RUN / ordinal:
        raise ValueError('fixed residual run')
    evidence = expand_residual_evidence(spec['evidence'], report['profile']) if residual else spec['evidence']
    if not residual and not isinstance(evidence, list):
        raise ValueError('complete profile evidence representation')
    sources = spec['sources']
    source_count = 82 if report['profile'] == FOURTH_PROFILE else 80 if report['profile'] == THIRD_PROFILE else 78 if report['profile'] == SECOND_PROFILE else 76 if residual else 74
    if [row['path'] for row in sources] != expected_source_paths(report['profile']) or len(sources) != source_count:
        raise ValueError('exact successor source closure')
    expected_tools = [str(Path(__file__).resolve()), str(Path(__file__).resolve().with_name('test_bounded_vrf.py'))]
    if [row['path'] for row in spec['tool_sources']] != expected_tools:
        raise ValueError('exact executable tool closure')
    source_total = 0
    for row in sources + spec['tool_sources']:
        pin_file(row['path'], row, SOURCE_CAP)
        source_total += row['bytes']
        if source_total > SOURCE_TOTAL:
            raise ValueError('complete source aggregate cap')
    shared_freeze = (PREVIOUS_RUN if residual else SHARED_RUN) / 'source-freeze.json'
    pin_file(shared_freeze, dict(path=str(shared_freeze), bytes=15861 if residual else 14455,
             sha256='af2315a94cbb1236c35b3c4ecf0622fe33ee1d9511ce726d8c612a00d85e2644' if residual else '51d3654490a4024c5a4479d383401e943f0cc773af62ef6c1d4204de82e12ee3'))
    legacy = read_json(shared_freeze)
    changed = {str(SRC / 'vrf_shared_window.rs')}
    # These additive storage successors also affect the old embedded provenance.
    # Their full old-profile equivalence and exact mappings require source review.
    changed |= {str(REPO/'rust/crates/rustaxa-storage/src'/name) for name in [
            'lib.rs','concrete_state/mod.rs','concrete_state/witness.rs',
            'concrete_state/witness_live.rs','concrete_state/witness_bundle.rs','concrete_state/witness_live_tests.rs']}
    for before, after in zip(legacy, sources):
        if before['path'] not in changed and before != after:
            raise ValueError('undeclared owner source drift')
    binaries = spec['binaries']
    if [row['path'] for row in binaries] != [str(run / f'{kind}-vrf-{ordinal}-voter-window-v1') for kind in ['live', 'replay']]:
        raise ValueError('fixed executable names')
    pin_file(spec['preflight']['path'], spec['preflight'], FREEZE_CAP)
    preparation = None
    if report['profile'] in SUCCESSORS or residual:
        preparation = run / f'vrf-{ordinal}-voter-preparation.json'
        if spec['preparation']['path'] != str(preparation):
            raise ValueError('fixed preparation path')
        pin_file(preparation, spec['preparation'], METADATA_CAP)
    validate_preflight(report, preparation)
    if report['authority']['six_input_owner']['executable'] != binaries[0]:
        raise ValueError('fresh preflight executable binding')
    source_map = {row['path']: row for row in sources}
    report_sources = report['vrf_sources'] + report['authority']['six_input_owner']['sources']
    if len(report['vrf_sources']) != 22 or len(report['authority']['six_input_owner']['sources']) != 25:
        raise ValueError('embedded provenance counts')
    baseline_path = SHARED_RUN / 'seventh-voter/vrf-seventh-voter-window-preflight.json'
    pin_file(baseline_path, dict(path=str(baseline_path), bytes=52027,
             sha256='2f54b63e18764830e7286e5b6349ef51dc9b65330edb4769f5fb8938e5f86dc8'))
    baseline_report = read_json(baseline_path, FREEZE_CAP)
    baseline_sources = baseline_report['vrf_sources']
    expected_report_paths = [r['path'] for r in baseline_sources[:19]] + ['vrf_shared_window.rs'] + [
        f'bin/{kind}_vrf_residual_49f8.rs' if report['profile'] == FOURTH_PROFILE else f'bin/{kind}_vrf_residual_e50b.rs' if report['profile'] == THIRD_PROFILE else f'bin/{kind}_vrf_residual_b92a.rs' if report['profile'] == SECOND_PROFILE else f'bin/{kind}_vrf_residual_first.rs' if residual else f'bin/{kind}_vrf_{ordinal}_voter_window.rs' for kind in ['live', 'replay']]
    if [r['path'] for r in report['vrf_sources']] != expected_report_paths:
        raise ValueError('fixed embedded profile source closure')
    if [r['path'] for r in report['authority']['six_input_owner']['sources']] != [
            r['path'] for r in baseline_report['authority']['six_input_owner']['sources']]:
        raise ValueError('fixed embedded owner source closure')
    report_total = 0
    for row in report_sources:
        actual = dict(row, path=str((SRC / row['path']).resolve()))
        if source_map.get(actual['path']) != actual:
            raise ValueError('embedded source successor pin')
        report_total += row['bytes']
        if report_total > SOURCE_TOTAL:
            raise ValueError('report aggregate source cap')
    if len(spec['source_commit']) != 40 or any(c not in '0123456789abcdef' for c in spec['source_commit']):
        raise ValueError('source HEAD shape')
    pins = sources + spec['tool_sources'] + binaries + [spec['preflight']] + evidence + ([spec['preparation']] if preparation is not None else []) + [pin_file(spec_path, cap=METADATA_CAP)]
    if len(pins) > PIN_COUNT:
        raise ValueError('command pin count')
    if report['profile'] == FOURTH_PROFILE:
        _validate_third_fit(pins, binaries)
    elif report['profile'] == THIRD_PROFILE:
        _validate_third_fit(pins, binaries)
    elif report['profile'] == SECOND_PROFILE:
        _validate_second_fit(pins, binaries)
    total = 0
    for row in pins:
        pin_file(row['path'], row)
        total += row['bytes']
        if total > PIN_TOTAL:
            raise ValueError('command pin aggregate')
    if report['profile'] == FOURTH_PROFILE:
        children = verify_fourth_records(evidence)
        total += children['referenced_bytes']
        if total > PIN_TOTAL or total - sum(row['bytes'] for row in binaries) > 31457280:
            raise ValueError('command plus repeated child byte cap')
        if source_total + children['source_bytes'] > SOURCE_TOTAL:
            raise ValueError('source plus shared verifier cap')
    evidence_paths = {row['path'] for row in evidence}
    required = [run.parent / name for name in [
        'common-authority-review.md', 'common-authority-amendment.md', 'code-acceptance-review.md', 'validation-summary.json',
        'storage-validation-reuse.json', 'source-commit.json', 'source-commit.log',
        'source-hook-summary.json', 'startup-reconciliation.json', 'instruction-successor-freeze.json']]
    if report['profile'] in SUCCESSORS or residual:
        required = [p for p in required if p.name not in {'common-authority-review.md', 'common-authority-amendment.md'}]
    required += [R3 / name for name in ['round3-closeout-freeze.json',
        'frozen-sixth-source-manifest.json', 'sixth-voter/source-freeze.json',
        'sixth-voter/vrf-sixth-voter-window-preflight.json', 'sixth-voter/family-final-review.md',
        'seventh-voter/dependencies-closed.json', 'seventh-voter/builder-draft-dependencies-closed.json']]
    required += [PREPARATION]
    if report['profile'] in SUCCESSORS or residual:
        # Historical evidence stays at its accepted path, never a retargeted copy.
        required += [SHARED_RUN / name for name in [
            'common-authority-review.md', 'common-authority-amendment.md',
            'code-acceptance-review.md', 'validation-summary.json',
            'storage-validation-reuse.json', 'source-commit.json', 'source-commit.log',
            'source-hook-summary.json', 'startup-reconciliation.json',
            'instruction-successor-freeze.json', 'command-ready-review.md',
            'seventh-voter/family-final-review.md', 'fact-outcome-freeze.json',
            'final-closeout-freeze.json', 'source-freeze.json',
            'seventh-voter/binary-freeze.json',
            'seventh-voter/vrf-seventh-voter-window-command-freeze.json',
            'seventh-voter/exact-commands.json',
            'seventh-voter/vrf-seventh-voter-window-preflight.json',
            'seventh-voter/vrf-seventh-voter-window.rcv2',
            'seventh-voter/vrf-seventh-voter-window-replay.json']]
        required += [(PREVIOUS_RUN if residual else run.parent) / name for name in [
            'successor-authority-amendment.md', 'luna-map.md', 'luna-rows.json',
            'source-successor-mapping.json', 'binary-successor-mapping.json', 'preparation-comparison.json']]
        required += [preparation]

    if residual:
        validate_residual_history(evidence, report['profile'])
        required = [p for p in required if p != run.parent / 'storage-validation-reuse.json']
        if report['profile'] in {SECOND_PROFILE, THIRD_PROFILE, FOURTH_PROFILE}:
            required = [Path(row['path']) for row in evidence]
        else:
            required += [RESIDUAL_RUN/name for name in [
                'residual-authority-review.md','residual-metadata-encoding-review.md',
                'sol-review/map-acceptance.md','luna-map/residual-424-corrected-v1.json',
                'luna-map/provenance-corrected-v1.json','luna-map/integrity-corrected-v1.json',
                'luna-map/build_residual_map_corrected_v1.py','source-successor-mapping.json',
                'binary-successor-mapping.json','preparation-comparison.json',
                'storage-validation-current.json','code-and-gates-freeze.json']]
            required += [Path(RESIDUAL_PREFIXES[13])/'manifest.json',
                         PREVIOUS_RUN/'eighth-voter/instance-spec.json']
            required += [RESIDUAL_RUN/(name+suffix) for name in RESIDUAL_GATES for suffix in ['.json','.log']]
            required += [RESIDUAL_RUN/'storage/correction-test.json',
                         RESIDUAL_RUN/'storage/correction-test.log',
                         RESIDUAL_RUN/'source-freeze.json',RESIDUAL_RUN/'tool-source-freeze.json',
                         RESIDUAL_RUN/'residual-authority-draft.json']
    if not {str(path) for path in required}.issubset(evidence_paths):
        raise ValueError('missing mandatory review/history evidence')
    if [row['path'] for row in spec['copy_metadata']] != [str(COPY), str(COPY/'db/db'), str(COPY/'db/state_db')]:
        raise ValueError('fixed copy metadata paths')
    for row in spec['copy_metadata']:
        if set(row) != {'path', 'device', 'inode', 'mode', 'mtime_ns'}:
            raise ValueError('copy metadata shape')
    extension = 'rcv4' if report['profile'] == FOURTH_PROFILE else 'rcv3' if residual else 'rcv2'
    command = [binaries[0]['path'], '--bundle-output', str(run/f'vrf-{ordinal}-voter-window.{extension}'),
               '--report-output', str(run/f'vrf-{ordinal}-voter-window-capture.json')]
    return encoded(dict(schema=1, profile=report['profile'], run=str(run), source_commit=spec['source_commit'],
                        cwd=str(REPO), environment=ENV, capture_command=command,
                        pins=pins, pin_count=len(pins), pinned_file_bytes=total,
                        source_count=source_count, complete_source_bytes=source_total,
                        embedded_source_bytes=report_total,
                        copy_metadata=spec['copy_metadata'], accepted=False), FREEZE_CAP)


def verify_files(freeze):
    if freeze['pin_count'] != len(freeze['pins']) or len(freeze['pins']) > PIN_COUNT:
        raise ValueError('frozen pin count')
    total = 0
    for row in freeze['pins']:
        pin_file(row['path'], row)
        total += row['bytes']
        if total > PIN_TOTAL:
            raise ValueError('frozen total cap')
    if freeze.get('profile') == FOURTH_PROFILE:
        # 82 sources +2 tools +2 binaries +preflight precede the287 evidence.
        children = verify_fourth_records(freeze['pins'][87:374])
        total += children['referenced_bytes']
        binaries = freeze['pins'][84:86]
        if total > PIN_TOTAL or total - sum(row['bytes'] for row in binaries) > 31457280:
            raise ValueError('frozen repeated child cap')
        if freeze['complete_source_bytes'] + children['source_bytes'] > SOURCE_TOTAL:
            raise ValueError('frozen shared verifier source cap')
    if total != freeze['pinned_file_bytes']:
        raise ValueError('frozen total drift')


def copy_metadata(expected):
    if [row['path'] for row in expected] != [str(COPY), str(COPY/'db/db'), str(COPY/'db/state_db')]:
        raise ValueError('copy paths')
    actual = []
    for row in expected:
        path = Path(row['path'])
        if path.is_symlink() or path.resolve() != path:
            raise ValueError('copy alias')
        stat = path.stat()
        item = dict(path=str(path), device=stat.st_dev, inode=stat.st_ino, mode=stat.st_mode, mtime_ns=stat.st_mtime_ns)
        if item != row:
            raise ValueError('copy metadata drift')
        actual.append(item)
    return actual


def run_bounded(command, env, log, timeout_seconds=TIMEOUT):
    """Bound child output and the full deadline, including wait after pipe EOF."""
    written = 0
    overflow = False
    timed_out = False
    deadline = time.monotonic() + timeout_seconds
    with Path(log).open('xb') as output:
        process = subprocess.Popen(command, cwd=REPO, env=env, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=0)
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    timed_out = True
                    break
                data = os.read(process.stdout.fileno(), min(8192, SINK_CAP-written+1))
                if not data:
                    break
                capacity = SINK_CAP - written
                output.write(data[:capacity])
                written += min(len(data), capacity)
                if len(data) > capacity:
                    overflow = True
                    break
        finally:
            selector.close()
            process.stdout.close()
            if overflow or timed_out:
                process.kill()
            try:
                child_exit = process.wait(timeout=max(0.001, deadline-time.monotonic()))
            except subprocess.TimeoutExpired:
                timed_out = True
                process.kill()
                child_exit = process.wait()
    return dict(exit=2 if overflow or timed_out else child_exit, child_exit=child_exit,
                output_overflow=overflow, wall_timeout=timed_out, log_bytes=written)


def capture(freeze_path, expected_sha):
    freeze_path = regular_path(freeze_path)
    with freeze_path.open('rb') as stream:
        frozen_bytes = stream.read(FREEZE_CAP + 1)
    if len(frozen_bytes) > FREEZE_CAP or hashlib.sha256(frozen_bytes).hexdigest() != expected_sha:
        raise ValueError('reviewed freeze SHA/cap drift')
    # Parse precisely the bounded bytes whose SHA was authenticated above.
    freeze = json.loads(frozen_bytes, object_pairs_hook=unique_object)
    expected_fields = {'schema', 'profile', 'run', 'source_commit', 'cwd', 'environment',
                       'capture_command', 'pins', 'pin_count', 'pinned_file_bytes', 'source_count',
                       'complete_source_bytes', 'embedded_source_bytes', 'copy_metadata', 'accepted'}
    if not isinstance(freeze, dict) or set(freeze) != expected_fields or freeze['accepted'] is not False:
        raise ValueError('trusted freeze schema')
    run = outside_database(freeze['run'])
    if freeze.get('schema') != 1 or freeze.get('profile') not in {PROFILE, *SUCCESSORS, *RESIDUAL_PROFILES} or run != freeze_path.parent \
            or not run.is_relative_to(Path('/home/fry/artifacts')) or run.resolve(strict=True) != run:
        raise ValueError('trusted freeze/run binding')
    fixed = sealed_profile(freeze['profile'])
    ordinal = fixed['ordinal']
    residual = freeze['profile'] in RESIDUAL_PROFILES
    if residual and run != RESIDUAL_RUN/ordinal:
        raise ValueError('fixed residual guard run')
    # Trusted admission ends here. All later failures permanently consume this
    # attempt, including offline pin, HEAD, output and metadata failures.
    exclusive(run/f'vrf-{ordinal}-voter-window-attempt.json', encoded(
        dict(command=freeze['capture_command'], source_commit=freeze['source_commit'],
             freeze_sha256=expected_sha), RESULT_CAP))
    verify_files(freeze)
    spec_pin = freeze['pins'][-1]
    if build(Path(spec_pin['path'])) != frozen_bytes:
        raise ValueError('freeze not deterministic builder output')
    if freeze['profile'] not in {PROFILE, *SUCCESSORS, *RESIDUAL_PROFILES} or freeze['cwd'] != str(REPO) or freeze['environment'] != ENV or Path.cwd() != REPO:
        raise ValueError('exact invocation binding')
    head = subprocess.check_output(['/usr/bin/git', 'rev-parse', 'HEAD'], cwd=REPO, env=ENV).decode().strip()
    if head != freeze['source_commit']:
        raise ValueError('source HEAD drift')
    outputs = [f'vrf-{ordinal}-voter-window{suffix}' for suffix in ['.rcv4' if freeze['profile'] == FOURTH_PROFILE else '.rcv3' if residual else '.rcv2', '-capture.json', '-live.log', '-live-result.json']]
    if any(os.path.lexists(run/name) for name in outputs):
        raise ValueError('single-use output collision')
    before = copy_metadata(freeze['copy_metadata'])
    start = time.monotonic()
    result = run_bounded(freeze['capture_command'], ENV, run/outputs[2])
    record = dict(command=freeze['capture_command'], **result, seconds=round(time.monotonic()-start, 2), before=before, accepted=False)
    try:
        record['after'] = copy_metadata(freeze['copy_metadata'])
        verify_files(freeze)
    finally:
        exclusive(run/outputs[3], encoded(record, RESULT_CAP))
    print(json.dumps(record))
    return result['exit']


def main():
    if len(sys.argv) == 4 and sys.argv[1] == 'build':
        exclusive(Path(sys.argv[3]), build(Path(sys.argv[2])))
        return 0
    if len(sys.argv) == 4 and sys.argv[1] == 'capture':
        return capture(Path(sys.argv[2]), sys.argv[3])
    raise ValueError('usage: bounded_vrf.py build SPEC FREEZE | capture FREEZE REVIEWED_SHA256')


if __name__ == '__main__':
    sys.exit(main())
