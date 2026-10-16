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

FIFTH_RUN = Path('/home/fry/artifacts/vrf-residual-20261014-current/a8a45')
FIFTH_PROFILE = 'n4_residual_vrf_a8a23a6c96a09c7783b70373d2302fefd943730e_v1'
FIFTH_KEY = '21058ac713cda18837a0c75283e1852e0f0d0e2696643d04d9bcae7df088673a'
FIFTH_INDICES = [92, 152, 212, 266, 317, 374, 434, 494, 554, 611, 668, 725, 779, 839, 896, 950, 1007, 1064, 1124, 1184, 1244, 1304, 1355, 1460, 1520, 1577, 1634, 1691, 1748, 1805, 1862, 1916, 1970, 2030, 2090, 2144, 2204, 2258, 2318, 2375, 2432, 2489, 2549, 2657, 2717]
# Filled only from the final independently accepted finite successor.
FIFTH_CONTRACT = dict(path='/home/fry/artifacts/a8a45-run-01a13f63-20261015/astra/a8a45-successor-contract-v16-split.json', bytes=129858, sha256='737985c907c8b4c38142dfeafba9002983ec740d8694e7b3ab75a911b55ca621')

SIXTH_RUN = Path('/home/fry/artifacts/vrf-round2-01a141ba-20261016/fbb45')
SIXTH_PROFILE = 'n4_residual_vrf_fbb3ee068e491e218d2cda595c23c6e3cc2f00f2_v1'
SIXTH_KEY = 'ec5c494ab92b4655da656eae4a546ee7efab77ffe269fb82adc3bf19c5d1ac73'
SIXTH_INDICES = [116, 176, 233, 290, 341, 398, 458, 518, 575, 635, 692, 746, 803, 863, 917, 974, 1031, 1088, 1148, 1208, 1268, 1325, 1379, 1427, 1484, 1544, 1601, 1658, 1712, 1829, 1886, 1940, 1994, 2054, 2111, 2168, 2228, 2282, 2342, 2456, 2513, 2573, 2621, 2681, 2741]
SIXTH_CONTRACT = dict(path='/home/fry/artifacts/vrf-round2-01a141ba-20261016/astra/fbb45-successor-contract-v14-tuples.json', bytes=118933, sha256='f831786029d53c7cf86600198ed15ae8cddc643d6063083f844ee95a320d86be')
SIXTH_PREDECESSOR_SOURCES = json.loads('[{"bytes":29289,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_input_handoff.rs","sha256":"7845e2a972ec0c550c9c16c2039bde1ea453c4da6625ccb7a142daf6aa29f7f8"},{"bytes":371,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/parent_input_handoff.rs","sha256":"9b28884952fccdb3252cdca30465875e65e04d2c313098fb5d8ca2579c729d38"},{"bytes":699,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/lib.rs","sha256":"6139fd14946594ee82a46759aba6587ecdc9ee285aacbbe3c6bea59ccd1429d0"},{"bytes":22289,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/native_witness.rs","sha256":"c24c75d79d5c793fab6a4c349e20bce7aebbcf3867d6b15af56b6a011b689a4d"},{"bytes":27968,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/delayed_headers.rs","sha256":"5c7ff91e502733d4ca63357dc6a97bda10bc7d8e8438a4f208e0373afc8eb63d"},{"bytes":11245,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/interval_history.rs","sha256":"44e1430d4a0206325de12a7c804f964760b6d261b83f1954185761ca2f3be6e4"},{"bytes":19542,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/head_history.rs","sha256":"cc0ea9b27712d67e75ec46b6a858dbfcdeb65affce048339e3ad6cfcb624379b"},{"bytes":23298,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_history.rs","sha256":"46786d43ff10d076d425c23f901b149e462b139e138ee253852bbe7a3554319b"},{"bytes":44873,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_reward_fields.rs","sha256":"a5c2beb6543affb2b8ddf2ef7aed5505b338f477050179ee71df46c6758f38f7"},{"bytes":2443,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/reward_votes.rs","sha256":"9854994cf3cd49677b132c26095d7a01b2add8e5370a64f90ed75337879a577c"},{"bytes":5070,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/paths.rs","sha256":"a2db7e497bc4a21f11344a3365a72b3cd2dce43887cedb9df862228392595b7a"},{"bytes":4863,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/parent_cache_fields/admission.rs","sha256":"d247a52dd9f848db7454e5c1f4159692e1e5fc275332c071196fafe573f9d091"},{"bytes":16255,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/aspen_policy.rs","sha256":"7df98bad2b42ddef270c5eacbe20d8ce6a549cfa0201e232ce82a3b8747692ff"},{"bytes":4723,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/aspen_policy_sources.rs","sha256":"dc8771a33d1102221bcc8b2ec6be151a2aeb9a3b8a3624dcf0662590ebef7e9a"},{"bytes":901,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/Cargo.toml","sha256":"63b643eca00b29108eef0b70fdcdfddb278dc6a22a08ab295f84c6e4a49304cd"},{"bytes":86577,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/Cargo.lock","sha256":"d70f0f669944ad25f5fe779a5b23d1916ae1a98c8882b5ca0447214d1ab9bd54"},{"bytes":22269,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness.rs","sha256":"ec47bb56f197d9c02ff2a4c4ef1f1f4f58a781ce1a14586a8e1a417fe90b93e4"},{"bytes":56841,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs","sha256":"450f97ef56d0341ee5570e4560aef03a6a3eda4aea20b66aca66d775d3f57e40"},{"bytes":11855,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs","sha256":"4d927a1c7aeb5dfdb9b81c917f0e5196f987dfae51b36543389813cf83fad726"},{"bytes":12599,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_binary.rs","sha256":"1d5db25980cae3ef4b85e7220144f8388ecab67a82069a4b02b0822cca3dbd84"},{"bytes":33298,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/physical_node.rs","sha256":"55d3096128c68e1949f3d96870f65d99b6206c1ca8092c5302c6a1bb0d39f75b"},{"bytes":8359,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/codec.rs","sha256":"9a74a1fa46f7a9efb6ffd0d7002e888e681b867cc83080da593b477b06dd3def"},{"bytes":9000,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-consensus/src/final_chain/native_session/vote_inputs.rs","sha256":"bcd0e0a20e5176ee9480b6422e94a03c9d35a6bb7f8404579c791bc52678dcff"},{"bytes":65589,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs","sha256":"cd5670f1ea93167b1ef4075be6b52e81d9a35d9ad2823da98f3c6d1a6531a38d"},{"bytes":17348,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-types/src/codec/rlp/final_chain.rs","sha256":"c3c3ba662b2052cc6bbaabe7ea782a599f6291ac0fcad5936e35946744713451"},{"bytes":14719,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_witness.rs","sha256":"6b27f9c4650fb89e2132f1f17b905a34b6b21aa2418762bc0ee54a3efd5fdfc8"},{"bytes":668,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_witness.rs","sha256":"ec9f048a77243b27ac64af4839621a277b586860d6aed4966ba58f7b5db277b8"},{"bytes":495,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_witness.rs","sha256":"01ebc98eaf56db60f54755f768ab79fd02b0673bb8f1e8c6697b4bb539d0c332"},{"bytes":3456,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/lib.rs","sha256":"07d607a2482ab94824f96adb6b0c858c18eb806db5ab9b20aa2447a7bc34acbc"},{"bytes":2634,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/mod.rs","sha256":"85876f79f9848f3c6976e138cedab20d1993dd0304a728f19110693276ee183c"},{"bytes":43628,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/db.rs","sha256":"7adabcea4874a81b656b85876feb3a2ab32d695740d7a76fc44b1479d21b409b"},{"bytes":14648,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/config.rs","sha256":"ab02fb7f695cc19be49e25f46ac78f99a00c76e7a0eb7292c962d80a6d9f6c5b"},{"bytes":55304,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/final_chain.rs","sha256":"c996c560415b9d8fc34e50f2bcfe3473186f3766a4169fd6a9a6f91869cab52f"},{"bytes":25673,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/metadata.rs","sha256":"e43e17b855a5e7d018b088c085006ad002e127eb68c79b54bddaafab1cf54ab6"},{"bytes":2891,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_rlp.rs","sha256":"264022b34bd07033037ccead6fe020b336ff3340f15bb40ec1ce0b3e0293787f"},{"bytes":93524,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db.rs","sha256":"d6093ac53cde9751045d1e69bad5e88a94c97124ec7b931e1ecf2a69f50ce6a5"},{"bytes":10001,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_bounded.rs","sha256":"542952d508b01b14f99fd0e47f7cb71d76c7d5545788b5b1d5e66916df45189f"},{"bytes":4343,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/bounded_read.rs","sha256":"72926fe19ad6398d1b9bc78d3c6b9422cb81e856bbf11d8658bc8406cc245008"},{"bytes":21648,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_iterator.rs","sha256":"833ab9ed87042f10408ed168d5043b6466e51b98447d7d3bf9a85bc66590d879"},{"bytes":174740,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_options.rs","sha256":"524ad9b6477a5bdc6ae5f111fe916a6749fe5f091d8cf7d1eaabb3e03a60e484"},{"bytes":2148,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_pinnable_slice.rs","sha256":"3714970b72c44f72fc07db463f0dcf8a339a6eb4c6c93496d9fd6571e641c63c"},{"bytes":6725,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/column_family.rs","sha256":"8566584605a812c6f092951a4a50ff24ef69d7c73c9236983aabab2787dccee0"},{"bytes":8737,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/ffi_util.rs","sha256":"541f1e0698392c83f283cea634d637d9b38dcaaee2cfccfcf85f4558228df561"},{"bytes":6397,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_voter_window.rs","sha256":"19e3b21ab0c4d14599494d719c572bbc2c1ace05eb98c89b8614bc58f614135c"},{"bytes":659,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_voter_window.rs","sha256":"2b9c9eb8a5b8f437e1959dfd0339392aa8b97e1837a49bc8aaad143c84aae6aa"},{"bytes":493,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_voter_window.rs","sha256":"7045b9c061255214dbf01a2dcb0020e5ebd6fc3d802ee6ba76ad0a5fb7539263"},{"bytes":105007,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_live_tests.rs","sha256":"25cc80a704e0cef8bb2e5859451e1d181bda7f4dbaa8701754152b7f3fccb77f"},{"bytes":298,"path":"/workspaces/rustaxa-evm/rust/Cargo.toml","sha256":"83999ded4fb16bbc16c6fbefd9e2806238a6d2ef06ec47b70528c62d0b4b5e05"},{"bytes":98358,"path":"/workspaces/rustaxa-evm/rust/Cargo.lock","sha256":"43b97efd6509c6373de57f89f832b31de70795fcad303316d56cbdc684a81741"},{"bytes":149,"path":"/workspaces/rustaxa-evm/.githooks/pre-commit","sha256":"07fe760100863ed1f069f69fc8afa7a026b81515bd37964d9e2eb60091bf3967"},{"bytes":7587,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_next_voter_window.rs","sha256":"79fe8336bb56b25e48c60fc1e97800f0a5c3153f30199e4d3148f77d88ce3f00"},{"bytes":674,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_next_voter_window.rs","sha256":"1884ac77ca8ea5c68b4db57f28e26548113ba7a293f8efe29264aeb3730e7edc"},{"bytes":503,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_next_voter_window.rs","sha256":"3e5398958169a691f13bdb389e777aa18ac1af5b109c363d93ffb9ded5fdedd8"},{"bytes":9789,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_third_voter_window.rs","sha256":"4a9ff909c2d45c516407e837f48b3bde8a5a0005c6faa87bd7a294c1c8187c2c"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_third_voter_window.rs","sha256":"177adb0fc5c18f4500e4e1d6bb88e336cc1932a035c683dee49ed1b293a55cf1"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_third_voter_window.rs","sha256":"677e9254e1dd64d64b217affebf310484ed1118b5070e2885f17eb54e8608bad"},{"bytes":10757,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_fourth_voter_window.rs","sha256":"c5c8eb81f668a4a7c10fc90852f3471636f9388f9f632082798fbb4f0dd70139"},{"bytes":680,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_fourth_voter_window.rs","sha256":"64bb93f43b8fbe90820b041dc4caaf15020e19fa1f184126c004189645b9c601"},{"bytes":507,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_fourth_voter_window.rs","sha256":"a23150bceacd3b6327697082f891cbf597b852afdc8ad2bd032294725ad24c03"},{"bytes":11710,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_fifth_voter_window.rs","sha256":"6fd1195c5de10cd7c273006f5abfb303386d01941488ba22e9dc1f095d0db4a3"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_fifth_voter_window.rs","sha256":"58a8ad971a420049ec0dd002668428f537a6ec95345a744188b91c49e0a359b5"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_fifth_voter_window.rs","sha256":"67d27b53247fd938d697f1205617f9e76702f70e67dd98ed1b191f27f95bc49c"},{"bytes":12668,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_sixth_voter_window.rs","sha256":"f3288ca42262699084bc3be21912f75fd8f40535de2a3372c7ebbcd3a32bbba7"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_sixth_voter_window.rs","sha256":"f4047ae71635a880484981469f7d38465e07dc648c4eefba145ff7401c3ac5bf"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_sixth_voter_window.rs","sha256":"fb227bf3cc8da1a5fa0693877965bf51e63bfa492c60405b160cd642498acedc"},{"bytes":133727,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_shared_window.rs","sha256":"d188dbf5bb52823f8fcc9feb0e3b1f43491bac44d1077240b893d6ace6bd8165"},{"bytes":708,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_seventh_voter_window.rs","sha256":"b40015f8402f104d8c9faaf38b6fe6f988c28e5227ea50dc201ceaa634342642"},{"bytes":533,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_seventh_voter_window.rs","sha256":"b5d740091ba8a9abcdd6aef9caaa5a02b6ab8c226029494b384f4d7961ca6888"},{"bytes":705,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_eighth_voter_window.rs","sha256":"85e1c5041d1ef41aa61d54600436aae0b5ae7ff0884e2ae6c0f897770d04969f"},{"bytes":508,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_eighth_voter_window.rs","sha256":"c05e8a30dac7baa5cf887f15d1bfcfd6f4eb9a9fad406e6a344bb75635c68818"},{"bytes":679,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_ninth_voter_window.rs","sha256":"45b0bb6f4ea21831cc29b816bd65e028e0ee7791665bd01d570dc856efab8b46"},{"bytes":506,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_ninth_voter_window.rs","sha256":"838acbc22eab208afe5dbcbdf8f53c21b4b3ef88db2e822697eda153a8ccc5e8"},{"bytes":679,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_tenth_voter_window.rs","sha256":"0936202430bdc381a8afa2f369d21c6d9ec878f96c489877bacc5b15c7078c3e"},{"bytes":506,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_tenth_voter_window.rs","sha256":"5cc82b4de8aa31c929de970d14def7f41e130c4a34e0d25034b6b96c888b1815"},{"bytes":736,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_first.rs","sha256":"7b1b728763a5398eeba76233852b62483d49f6bed2c76aefa6266355d65e6de1"},{"bytes":524,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_first.rs","sha256":"01b9eb0841083e5d2dab4960fc9cc46a4337f4a0204be4990fbd9dbc2da5756b"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_b92a.rs","sha256":"fcf690f75e33a91e3f2a28b16a4bc82a2ce7f9bf2f92c84738b836a8c9c7fa26"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_b92a.rs","sha256":"4c00a9202869bea49a0b24b4cfbd592118e5a5e5e41070cc3f794adf090cf9b1"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_e50b.rs","sha256":"ae6cabd86f07fa0540cc5787cbade8ddce8a6a8c4bcf85fad65644049573b42d"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_e50b.rs","sha256":"09bd08c4a7987d8bc45679179e2c5d71042f08fd88823e62580ad867e104aafa"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_49f8.rs","sha256":"516ee437c70509545b35d5213b25c097a977d7ba4e62e53db992f1ac4592cb71"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_49f8.rs","sha256":"c78838bc270a895f0f43c327bddff46819c97f5c98ca66611511c3041e480b62"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_a8a2.rs","sha256":"9f854f7a5fd89202cf587fdfede8566e9abc26cfb949e8f27fc7909470a31fba"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_a8a2.rs","sha256":"ff14e82d71955b89b9f1383259fa073d59a12772933d530ee9c0e216bf2a2a87"}]')
SIXTH_COMPARISON_PROFILES = [{'profile': 'native-current', 'before': {'path': '/home/fry/artifacts/n4-binding-resume-20261011/native-live-replay-frame-cost.json', 'bytes': 10911, 'sha256': '2ccf155e2a851cd52161f090bcd3cc76b09f212a44647ba8f76b7db0b7511fbb'}, 'after': {'path': '/home/fry/artifacts/vrf-run-01a141ba-20261015/fbb45/compatibility/native-current.json', 'bytes': 10945, 'sha256': '4c4c045bb090749c8c83acab2fd851999473dfa1b2820ab496cc1fbc0730ae06'}, 'fields': ['/source_profile/compiled/0', '/source_profile/compiled/10', '/source_profile/compiled/11', '/source_profile/compiled/12', '/source_profile/executable'], 'executable': {'path': '/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/target/debug/replay_native_witness', 'bytes': 192510384, 'sha256': 'abfe04a0550b421c14fa80d585feee30a53727eca09c944c9e986a964f9d432e'}}, {'profile': 'first-d-current', 'before': {'path': '/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d-independent-replay.json', 'bytes': 32805, 'sha256': '8bd32c28356e85944a37a900c95e8ae3cb7cbc7f942f27cc2769257b6be36854'}, 'after': {'path': '/home/fry/artifacts/vrf-run-01a141ba-20261015/fbb45/compatibility/first-d-current.json', 'bytes': 32813, 'sha256': 'b1c09f0c1bacfad76764f40df66b71b8a9f5e8c97eac79c4151f6b87dc68b765'}, 'fields': ['/authority/executable', '/authority/sources/2', '/authority/sources/16', '/authority/sources/17', '/authority/sources/18', '/vrf_sources/0', '/vrf_sources/3', '/vrf_sources/4', '/vrf_sources/5'], 'executable': {'path': '/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/target/debug/replay_vrf_witness', 'bytes': 194465976, 'sha256': 'b55496742d36ed6b6e5c6fce1af14fe151a44a9bef59d438d36469810c272094'}}]
SIXTH_FAILURE_UTF8 = [31427,'a55c5aff1010f733eb68ea8248fba6e33c6085ca366dcc7f099c218051eb43fa']
SIXTH_COMPARISON_ARGV = ['/usr/bin/python3','-c',"import sys;sys.path.insert(0,'/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/tools');import bounded_vrf;bounded_vrf.emit_fifth_comparison(bounded_vrf.SIXTH_PROFILE)"]

def sealed_profile(profile):
    if profile == SIXTH_PROFILE:
        return dict(ordinal="fbb45", voter="fbb3ee068e491e218d2cda595c23c6e3cc2f00f2", key=SIXTH_KEY, prior_index=19, indices=SIXTH_INDICES)

    """Select only a fixed reviewed identity; unknown profiles supply no authority."""
    if profile == FIFTH_PROFILE:
        return dict(ordinal='a8a45', voter='a8a23a6c96a09c7783b70373d2302fefd943730e', key=FIFTH_KEY, prior_index=11, indices=FIFTH_INDICES)
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


def record_verifier(source_pin):
    """Load the exact pinned shared verifier bytes without mutable imports."""
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
    return module


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
    module = record_verifier(source_pin)
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


def pin_file(path, expected=None, cap=FILE_CAP, _body=None):
    if _body is not None and (type(_body) is not bytearray or _body or cap > 65536):
        raise ValueError('bounded retained pin buffer')
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
            if _body is not None:
                _body.extend(block)
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
    if profile == SIXTH_PROFILE:
        return expected_source_paths(FIFTH_PROFILE) + [str(SRC / f"bin/{kind}_vrf_residual_fbb3.rs") for kind in ["live","replay"]]
    if profile == FIFTH_PROFILE:
        return expected_source_paths(FOURTH_PROFILE) + [str(SRC / f'bin/{kind}_vrf_residual_a8a2.rs') for kind in ['live','replay']]
    return result


def validate_preflight(report, preparation=None):
    pin_file(PREPARATION, dict(path=str(PREPARATION), bytes=60570, sha256=PREPARATION_SHA))
    common_prepared = read_json(PREPARATION)
    fixed = sealed_profile(report['profile'])
    residual = report['profile'] in RESIDUAL_PROFILES or report['profile'] in {FIFTH_PROFILE, SIXTH_PROFILE}
    count = 45 if report['profile'] in {FOURTH_PROFILE, FIFTH_PROFILE, SIXTH_PROFILE} else 46 if residual else 47
    voter, key = fixed['voter'], fixed['key']
    prepared = common_prepared if preparation is None else read_json(preparation)
    baseline = read_json(R3 / 'sixth-voter/vrf-sixth-voter-window-preflight.json', FREEZE_CAP)
    if report['status'] != 'authority_inputs_authenticated_no_database_open':
        raise ValueError('fixed profile preflight')
    selected = report['selection']
    if report['profile'] in {FIFTH_PROFILE, SIXTH_PROFILE}:
        validate_fifth_selection(selected, prepared, fifth_contract(report['profile']))
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
        delayed_index = ([x for x in range(1, 48) if x not in (30, 41)][i]
                         if report['profile'] == SIXTH_PROFILE else
                         [x for x in range(1, 48) if x not in (24, 45)][i]
                         if report['profile'] == FIFTH_PROFILE else
                         [x for x in range(1, 48) if x not in (12, 45)][i]
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
    residual_value = ('8baa75fd5cb33fb6a75c42caa6e17158cf58023103a8dc9e14eaad618d3220fd' if report['profile'] == SIXTH_PROFILE else 'c3fe453a6ee0a2230034b3f908ba24c4733b3a9c9d059d11df0e8fdaaaf7b48a'
                      if report['profile'] in {FIFTH_PROFILE, SIXTH_PROFILE} else '2c9895a89f0d58e2210f21f2129dc74de825a1b7ca03a15c600cda5f4c4a6dc3'
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


FIFTH_PREDECESSOR_SOURCES = json.loads('[{"bytes":29289,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_input_handoff.rs","sha256":"7845e2a972ec0c550c9c16c2039bde1ea453c4da6625ccb7a142daf6aa29f7f8"},{"bytes":371,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/parent_input_handoff.rs","sha256":"9b28884952fccdb3252cdca30465875e65e04d2c313098fb5d8ca2579c729d38"},{"bytes":699,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/lib.rs","sha256":"6139fd14946594ee82a46759aba6587ecdc9ee285aacbbe3c6bea59ccd1429d0"},{"bytes":22289,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/native_witness.rs","sha256":"c24c75d79d5c793fab6a4c349e20bce7aebbcf3867d6b15af56b6a011b689a4d"},{"bytes":27968,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/delayed_headers.rs","sha256":"5c7ff91e502733d4ca63357dc6a97bda10bc7d8e8438a4f208e0373afc8eb63d"},{"bytes":11245,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/interval_history.rs","sha256":"44e1430d4a0206325de12a7c804f964760b6d261b83f1954185761ca2f3be6e4"},{"bytes":19542,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/head_history.rs","sha256":"cc0ea9b27712d67e75ec46b6a858dbfcdeb65affce048339e3ad6cfcb624379b"},{"bytes":23298,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_history.rs","sha256":"46786d43ff10d076d425c23f901b149e462b139e138ee253852bbe7a3554319b"},{"bytes":44873,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/parent_reward_fields.rs","sha256":"a5c2beb6543affb2b8ddf2ef7aed5505b338f477050179ee71df46c6758f38f7"},{"bytes":2443,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/reward_votes.rs","sha256":"9854994cf3cd49677b132c26095d7a01b2add8e5370a64f90ed75337879a577c"},{"bytes":5070,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/paths.rs","sha256":"a2db7e497bc4a21f11344a3365a72b3cd2dce43887cedb9df862228392595b7a"},{"bytes":4863,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/parent_cache_fields/admission.rs","sha256":"d247a52dd9f848db7454e5c1f4159692e1e5fc275332c071196fafe573f9d091"},{"bytes":16255,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/aspen_policy.rs","sha256":"7df98bad2b42ddef270c5eacbe20d8ce6a549cfa0201e232ce82a3b8747692ff"},{"bytes":4723,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/aspen_policy_sources.rs","sha256":"dc8771a33d1102221bcc8b2ec6be151a2aeb9a3b8a3624dcf0662590ebef7e9a"},{"bytes":901,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/Cargo.toml","sha256":"63b643eca00b29108eef0b70fdcdfddb278dc6a22a08ab295f84c6e4a49304cd"},{"bytes":86577,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/Cargo.lock","sha256":"d70f0f669944ad25f5fe779a5b23d1916ae1a98c8882b5ca0447214d1ab9bd54"},{"bytes":22269,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness.rs","sha256":"ec47bb56f197d9c02ff2a4c4ef1f1f4f58a781ce1a14586a8e1a417fe90b93e4"},{"bytes":55693,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs","sha256":"16d0cb679064f514cb414245e01ef6bda4a87519fbfb8455bb56744c6ff746b6"},{"bytes":11855,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs","sha256":"4d927a1c7aeb5dfdb9b81c917f0e5196f987dfae51b36543389813cf83fad726"},{"bytes":12599,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_binary.rs","sha256":"1d5db25980cae3ef4b85e7220144f8388ecab67a82069a4b02b0822cca3dbd84"},{"bytes":33298,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/physical_node.rs","sha256":"55d3096128c68e1949f3d96870f65d99b6206c1ca8092c5302c6a1bb0d39f75b"},{"bytes":8359,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/codec.rs","sha256":"9a74a1fa46f7a9efb6ffd0d7002e888e681b867cc83080da593b477b06dd3def"},{"bytes":9000,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-consensus/src/final_chain/native_session/vote_inputs.rs","sha256":"bcd0e0a20e5176ee9480b6422e94a03c9d35a6bb7f8404579c791bc52678dcff"},{"bytes":65589,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs","sha256":"cd5670f1ea93167b1ef4075be6b52e81d9a35d9ad2823da98f3c6d1a6531a38d"},{"bytes":17348,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-types/src/codec/rlp/final_chain.rs","sha256":"c3c3ba662b2052cc6bbaabe7ea782a599f6291ac0fcad5936e35946744713451"},{"bytes":14719,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_witness.rs","sha256":"6b27f9c4650fb89e2132f1f17b905a34b6b21aa2418762bc0ee54a3efd5fdfc8"},{"bytes":668,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_witness.rs","sha256":"ec9f048a77243b27ac64af4839621a277b586860d6aed4966ba58f7b5db277b8"},{"bytes":495,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_witness.rs","sha256":"01ebc98eaf56db60f54755f768ab79fd02b0673bb8f1e8c6697b4bb539d0c332"},{"bytes":3456,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/lib.rs","sha256":"07d607a2482ab94824f96adb6b0c858c18eb806db5ab9b20aa2447a7bc34acbc"},{"bytes":2634,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/mod.rs","sha256":"85876f79f9848f3c6976e138cedab20d1993dd0304a728f19110693276ee183c"},{"bytes":43628,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/db.rs","sha256":"7adabcea4874a81b656b85876feb3a2ab32d695740d7a76fc44b1479d21b409b"},{"bytes":14648,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/config.rs","sha256":"ab02fb7f695cc19be49e25f46ac78f99a00c76e7a0eb7292c962d80a6d9f6c5b"},{"bytes":55304,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/final_chain.rs","sha256":"c996c560415b9d8fc34e50f2bcfe3473186f3766a4169fd6a9a6f91869cab52f"},{"bytes":25673,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/metadata.rs","sha256":"e43e17b855a5e7d018b088c085006ad002e127eb68c79b54bddaafab1cf54ab6"},{"bytes":2891,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_rlp.rs","sha256":"264022b34bd07033037ccead6fe020b336ff3340f15bb40ec1ce0b3e0293787f"},{"bytes":93524,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db.rs","sha256":"d6093ac53cde9751045d1e69bad5e88a94c97124ec7b931e1ecf2a69f50ce6a5"},{"bytes":10001,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_bounded.rs","sha256":"542952d508b01b14f99fd0e47f7cb71d76c7d5545788b5b1d5e66916df45189f"},{"bytes":4343,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/bounded_read.rs","sha256":"72926fe19ad6398d1b9bc78d3c6b9422cb81e856bbf11d8658bc8406cc245008"},{"bytes":21648,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_iterator.rs","sha256":"833ab9ed87042f10408ed168d5043b6466e51b98447d7d3bf9a85bc66590d879"},{"bytes":174740,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_options.rs","sha256":"524ad9b6477a5bdc6ae5f111fe916a6749fe5f091d8cf7d1eaabb3e03a60e484"},{"bytes":2148,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/db_pinnable_slice.rs","sha256":"3714970b72c44f72fc07db463f0dcf8a339a6eb4c6c93496d9fd6571e641c63c"},{"bytes":6725,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/column_family.rs","sha256":"8566584605a812c6f092951a4a50ff24ef69d7c73c9236983aabab2787dccee0"},{"bytes":8737,"path":"/workspaces/rustaxa-evm/rust/vendor/rocksdb/src/ffi_util.rs","sha256":"541f1e0698392c83f283cea634d637d9b38dcaaee2cfccfcf85f4558228df561"},{"bytes":6397,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_voter_window.rs","sha256":"19e3b21ab0c4d14599494d719c572bbc2c1ace05eb98c89b8614bc58f614135c"},{"bytes":659,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_voter_window.rs","sha256":"2b9c9eb8a5b8f437e1959dfd0339392aa8b97e1837a49bc8aaad143c84aae6aa"},{"bytes":493,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_voter_window.rs","sha256":"7045b9c061255214dbf01a2dcb0020e5ebd6fc3d802ee6ba76ad0a5fb7539263"},{"bytes":102209,"path":"/workspaces/rustaxa-evm/rust/crates/rustaxa-storage/src/concrete_state/witness_live_tests.rs","sha256":"19e753eefc9a4bf7aca993b424adbed390e4c77f7eb227202047533145c0ddea"},{"bytes":298,"path":"/workspaces/rustaxa-evm/rust/Cargo.toml","sha256":"83999ded4fb16bbc16c6fbefd9e2806238a6d2ef06ec47b70528c62d0b4b5e05"},{"bytes":98358,"path":"/workspaces/rustaxa-evm/rust/Cargo.lock","sha256":"43b97efd6509c6373de57f89f832b31de70795fcad303316d56cbdc684a81741"},{"bytes":149,"path":"/workspaces/rustaxa-evm/.githooks/pre-commit","sha256":"07fe760100863ed1f069f69fc8afa7a026b81515bd37964d9e2eb60091bf3967"},{"bytes":7587,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_next_voter_window.rs","sha256":"79fe8336bb56b25e48c60fc1e97800f0a5c3153f30199e4d3148f77d88ce3f00"},{"bytes":674,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_next_voter_window.rs","sha256":"1884ac77ca8ea5c68b4db57f28e26548113ba7a293f8efe29264aeb3730e7edc"},{"bytes":503,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_next_voter_window.rs","sha256":"3e5398958169a691f13bdb389e777aa18ac1af5b109c363d93ffb9ded5fdedd8"},{"bytes":9789,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_third_voter_window.rs","sha256":"4a9ff909c2d45c516407e837f48b3bde8a5a0005c6faa87bd7a294c1c8187c2c"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_third_voter_window.rs","sha256":"177adb0fc5c18f4500e4e1d6bb88e336cc1932a035c683dee49ed1b293a55cf1"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_third_voter_window.rs","sha256":"677e9254e1dd64d64b217affebf310484ed1118b5070e2885f17eb54e8608bad"},{"bytes":10757,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_fourth_voter_window.rs","sha256":"c5c8eb81f668a4a7c10fc90852f3471636f9388f9f632082798fbb4f0dd70139"},{"bytes":680,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_fourth_voter_window.rs","sha256":"64bb93f43b8fbe90820b041dc4caaf15020e19fa1f184126c004189645b9c601"},{"bytes":507,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_fourth_voter_window.rs","sha256":"a23150bceacd3b6327697082f891cbf597b852afdc8ad2bd032294725ad24c03"},{"bytes":11710,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_fifth_voter_window.rs","sha256":"6fd1195c5de10cd7c273006f5abfb303386d01941488ba22e9dc1f095d0db4a3"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_fifth_voter_window.rs","sha256":"58a8ad971a420049ec0dd002668428f537a6ec95345a744188b91c49e0a359b5"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_fifth_voter_window.rs","sha256":"67d27b53247fd938d697f1205617f9e76702f70e67dd98ed1b191f27f95bc49c"},{"bytes":12668,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_sixth_voter_window.rs","sha256":"f3288ca42262699084bc3be21912f75fd8f40535de2a3372c7ebbcd3a32bbba7"},{"bytes":677,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_sixth_voter_window.rs","sha256":"f4047ae71635a880484981469f7d38465e07dc648c4eefba145ff7401c3ac5bf"},{"bytes":505,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_sixth_voter_window.rs","sha256":"fb227bf3cc8da1a5fa0693877965bf51e63bfa492c60405b160cd642498acedc"},{"bytes":117638,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_shared_window.rs","sha256":"da67a7fa580a3189fb4c3e402e036f89fb1abba4f1278168e11cda12cc9b4a4e"},{"bytes":708,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_seventh_voter_window.rs","sha256":"b40015f8402f104d8c9faaf38b6fe6f988c28e5227ea50dc201ceaa634342642"},{"bytes":533,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_seventh_voter_window.rs","sha256":"b5d740091ba8a9abcdd6aef9caaa5a02b6ab8c226029494b384f4d7961ca6888"},{"bytes":705,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_eighth_voter_window.rs","sha256":"85e1c5041d1ef41aa61d54600436aae0b5ae7ff0884e2ae6c0f897770d04969f"},{"bytes":508,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_eighth_voter_window.rs","sha256":"c05e8a30dac7baa5cf887f15d1bfcfd6f4eb9a9fad406e6a344bb75635c68818"},{"bytes":679,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_ninth_voter_window.rs","sha256":"45b0bb6f4ea21831cc29b816bd65e028e0ee7791665bd01d570dc856efab8b46"},{"bytes":506,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_ninth_voter_window.rs","sha256":"838acbc22eab208afe5dbcbdf8f53c21b4b3ef88db2e822697eda153a8ccc5e8"},{"bytes":679,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_tenth_voter_window.rs","sha256":"0936202430bdc381a8afa2f369d21c6d9ec878f96c489877bacc5b15c7078c3e"},{"bytes":506,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_tenth_voter_window.rs","sha256":"5cc82b4de8aa31c929de970d14def7f41e130c4a34e0d25034b6b96c888b1815"},{"bytes":736,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_first.rs","sha256":"7b1b728763a5398eeba76233852b62483d49f6bed2c76aefa6266355d65e6de1"},{"bytes":524,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_first.rs","sha256":"01b9eb0841083e5d2dab4960fc9cc46a4337f4a0204be4990fbd9dbc2da5756b"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_b92a.rs","sha256":"fcf690f75e33a91e3f2a28b16a4bc82a2ce7f9bf2f92c84738b836a8c9c7fa26"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_b92a.rs","sha256":"4c00a9202869bea49a0b24b4cfbd592118e5a5e5e41070cc3f794adf090cf9b1"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_e50b.rs","sha256":"ae6cabd86f07fa0540cc5787cbade8ddce8a6a8c4bcf85fad65644049573b42d"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_e50b.rs","sha256":"09bd08c4a7987d8bc45679179e2c5d71042f08fd88823e62580ad867e104aafa"},{"bytes":733,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/live_vrf_residual_49f8.rs","sha256":"516ee437c70509545b35d5213b25c097a977d7ba4e62e53db992f1ac4592cb71"},{"bytes":522,"path":"/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/bin/replay_vrf_residual_49f8.rs","sha256":"c78838bc270a895f0f43c327bddff46819c97f5c98ca66611511c3041e480b62"}]')

def read_fifth_body(path, cap):
    """Bound the byte read before any parse, including post-pin file growth."""
    with regular_path(path).open('rb') as stream:
        raw=stream.read(cap+1)
    if len(raw)>cap:raise ValueError('fixed fifth body read cap')
    return raw



def finite_profile(profile):
    """Return one of two sealed descriptors; reject all other input.

    Specs cannot supply these fields. Each fixed descriptor binds complete
    source, evidence order, full parity bodies and exclusive outputs.
    """
    if profile == FIFTH_PROFILE:
        return dict(run=FIFTH_RUN, contract=FIFTH_CONTRACT,
                    predecessor=FIFTH_PREDECESSOR_SOURCES,
                    comparison=FIFTH_COMPARISON_PROFILES, argv=FIFTH_COMPARISON_ARGV,
                    source_count=84, evidence_start=89, evidence_count=285,
                    stem='a8a45', cli='a8a2', tests=FIFTH_EQUIVALENCE_TESTS,
                    equality_field='all14_unchanged_profiles_equal')
    if profile == SIXTH_PROFILE:
        if any(v is None for v in (SIXTH_CONTRACT,SIXTH_PREDECESSOR_SOURCES,SIXTH_COMPARISON_PROFILES)):
            raise ValueError('fixed sixth descriptor not settled')
        return dict(run=SIXTH_RUN, contract=SIXTH_CONTRACT,
                    predecessor=SIXTH_PREDECESSOR_SOURCES,
                    comparison=SIXTH_COMPARISON_PROFILES, argv=SIXTH_COMPARISON_ARGV,
                    source_count=86, evidence_start=91, evidence_count=283,
                    stem='fbb45', cli='fbb3',
                    tests=FIFTH_EQUIVALENCE_TESTS+['fifth_residual_saved_bundle_report_is_completely_equal'],
                    equality_field='all15_unchanged_profiles_equal')
    raise ValueError('unknown finite profile')


SIXTH_INVENTORY_FIELDS = ('group', 'index', 'path', 'bytes', 'sha256',
                          'cap_bytes', 'pending', 'schema_id')


def sixth_inventory_shape(value):
    """Bound the authenticated authority graph before and after row expansion.

    Use the existing 8192-node, depth8 and 2MiB allocation model. Reserve the
    original manifest and stream buffers; reject excess or non-JSON scalars.
    This local check performs no I/O and follows no paths.
    """
    nodes = retained = 0
    def visit(item, depth):
        nonlocal nodes, retained
        nodes += 1
        retained += 128
        if nodes > 8192 or depth > 8:
            raise ValueError('sixth inventory node/depth cap')
        if isinstance(item, str):
            retained += len(item.encode('utf-8'))
        elif isinstance(item, dict):
            for key, child in item.items():
                visit(key, depth + 1)
                visit(child, depth + 1)
        elif isinstance(item, list):
            for child in item:
                visit(child, depth + 1)
        elif item is not None and type(item) not in (bool, int):
            raise ValueError('sixth inventory scalar type')
        if retained > 2097152 - 262144 - 65536:
            raise ValueError('sixth inventory retained cap')
    visit(value, 0)


def expand_sixth_inventory(contract):
    """Expand only the sealed sixth authority's two fixed eight-cell lists.

    The caller authenticates the complete 131072-byte-bounded authority first.
    Require the compiled header, counts, scalar types and canonical lexical
    paths. Null alone omits a field; zero and false retain their JSON identity.
    Replace rows in-place, preserving order, multiplicity and all byte charges.
    No filesystem operation or authority selection occurs here. Any malformed
    cell or excess decoded graph raises ValueError before command construction.
    """
    header = dict(schema='n4-fbb45-fixed-inventory-tuples-v1',
                  fields=list(SIXTH_INVENTORY_FIELDS), command_count=376,
                  outcome_count=24, null_means_absent=True)
    if not isinstance(contract, dict) or contract.get('inventory_encoding') != header \
            or contract['inventory_encoding'].get('null_means_absent') is not True \
            or type(contract['inventory_encoding'].get('command_count')) is not int \
            or type(contract['inventory_encoding'].get('outcome_count')) is not int \
            or contract.get('schema') != 'n4-fbb45-finite-successor-v2':
        raise ValueError('sixth inventory encoding header')
    sixth_inventory_shape(contract)
    for name, count in [('command_inventory', 376), ('outcome_inventory', 24)]:
        rows = contract.get(name)
        if not isinstance(rows, list) or len(rows) != count:
            raise ValueError('sixth inventory exact count')
        for index, row in enumerate(rows):
            if not isinstance(row, list) or len(row) != 8:
                raise ValueError('sixth inventory eight cells')
            group, ordinal, path, size, digest, cap, pending, schema = row
            if not isinstance(group, str) or not group or type(ordinal) is not int or not 0 <= ordinal < 376:
                raise ValueError('sixth inventory role/index')
            if not isinstance(path, str) or len(path.encode('utf-8')) > 4096 \
                    or not path.startswith('/') or '\\' in path \
                    or any(ord(c) < 32 or ord(c) == 127 for c in path) \
                    or any(p in ('', '.', '..', 'AI.md') for p in path.split('/')[1:]):
                raise ValueError('sixth inventory canonical path')
            outside_database(path)
            if pending is not None and type(pending) is not bool:
                raise ValueError('sixth inventory pending type')
            if cap is not None and (type(cap) is not int or not 0 <= cap <= PIN_TOTAL):
                raise ValueError('sixth inventory cap type')
            if pending is True:
                if size is not None or digest is not None or cap is None \
                        or not isinstance(schema, str) or schema not in {f's{i}' for i in range(27)}:
                    raise ValueError('sixth inventory pending fields')
            elif type(size) is not int or not 0 <= size <= PIN_TOTAL \
                    or (cap is not None and size > cap) or schema is not None \
                    or not isinstance(digest, str) or len(digest) != 64 \
                    or any(c not in '0123456789abcdef' for c in digest):
                raise ValueError('sixth inventory immutable fields')
            rows[index] = {key: value for key, value in zip(SIXTH_INVENTORY_FIELDS, row)
                           if value is not None}
    sixth_inventory_shape(contract)
    return contract


def fifth_contract(profile=FIFTH_PROFILE):
    """Authenticate the finite authority within 131072 bytes and its own role.

    Return the parsed pinned contract. Reject byte, path, descriptor or role
    drift with ValueError; do not walk historical contracts or open a database.
    """
    fixed = finite_profile(profile)
    pin_file(fixed['contract']['path'], fixed['contract'], 131072)
    contract = read_json(fixed['contract']['path'], 131072)
    if profile == SIXTH_PROFILE:
        contract = expand_sixth_inventory(contract)
    authority_roles = [r for r in contract['command_inventory']
                       if r['group'] == 'current_tail54' and r['index'] == 40]
    if len(authority_roles) != 1 or authority_roles[0]['path'] != fixed['contract']['path'] or authority_roles[0]['cap_bytes'] != 131072:
        raise ValueError('current finite authority role binding')
    descriptor = json.dumps(fixed['predecessor'],sort_keys=True,separators=(',',':'),ensure_ascii=True).encode('utf-8')
    expected = contract['sealed_source_predecessor']
    if len(fixed['predecessor']) != expected['count'] or len(descriptor) != expected['canonical_bytes'] or hashlib.sha256(descriptor).hexdigest() != expected['canonical_sha256']:
        raise ValueError('sealed source predecessor descriptor')
    comparison=json.dumps({('FIFTH_COMPARISON_PROFILES' if profile == FIFTH_PROFILE else 'SIXTH_COMPARISON_PROFILES'):fixed['comparison'],('FIFTH_COMPARISON_ARGV' if profile == FIFTH_PROFILE else 'SIXTH_COMPARISON_ARGV'):fixed['argv']},sort_keys=True,separators=(',',':'),ensure_ascii=True).encode('utf-8')
    if hashlib.sha256(comparison).hexdigest() != contract['comparison_descriptor_sha256']:
        raise ValueError('sealed current comparison descriptor')
    if profile == SIXTH_PROFILE and contract.get('s7_failure_utf8') != SIXTH_FAILURE_UTF8:
        raise ValueError('sealed sixth full failure descriptor')
    return contract


def verify_finite_inventory(pins, expected, profile=FIFTH_PROFILE, retained_summary=None):
    """Check every ordered role, including repeats; pending means fixed path/cap.

    Immutable rows additionally bind exact bytes and digest. This helper grants
    no authority for an arbitrary roster: callers supply the sealed manifest.
    """
    if not isinstance(pins, list) or len(pins) != len(expected):
        raise ValueError('finite inventory count')
    summary = None
    for actual, required in zip(pins, expected):
        if set(actual) != {'path', 'bytes', 'sha256'} or actual['path'] != required['path']:
            raise ValueError('finite inventory order/path')
        if type(actual['bytes']) is not int or not 0 <= actual['bytes'] <= required.get('cap_bytes',required.get('bytes')):
            raise ValueError('finite inventory byte cap')
        if not required.get('pending', False) and actual != {k: required[k] for k in ('path','bytes','sha256')}:
            raise ValueError('immutable finite role drift')
        cap=required.get('cap_bytes',required.get('bytes'))
        if profile == SIXTH_PROFILE and actual['path'] == str(SIXTH_RUN.parent/'source/validation-summary.json'):
            if summary is not None or cap != 65536:
                raise ValueError('one fixed sixth summary role')
            if retained_summary is None:
                buffer=bytearray()
                pin_file(actual['path'],actual,cap,_body=buffer)
                summary=bytes(buffer)
            else:
                if not isinstance(retained_summary,bytes) or len(retained_summary)>cap or len(retained_summary)!=actual['bytes'] or hashlib.sha256(retained_summary).hexdigest()!=actual['sha256']:
                    raise ValueError('retained sixth inventory byte pin')
                summary=retained_summary
        else:
            pin_file(actual['path'], actual, cap)
    return summary


def validate_fifth_selection(selected, prepared, contract):
    """Bind all pending preflight/preparation data to the immutable candidate."""
    candidate_pin = contract['candidate']
    pin_file(candidate_pin['path'],candidate_pin,41550)
    candidate = read_json(candidate_pin['path'])
    expected = []
    for row in candidate['tuples']:
        source = dict(delayed_period=row['delayed_period'],proof=None,qualified=False,
                      semantic_key='fe_vrf_00_04_address',source_references=row['source_references'],
                      value=None,voter_hex=row['voter_hex'])
        tuple_ = dict(identity=dict(period=row['delayed_period'],state_root_hex=row['state_root_hex']),
                      address_hex=row['account_hex'],logical_key_hex=row['logical_key_hex'],key_preimage_hex=row['key_preimage_hex'])
        expected.append(dict(parent_manifest_index=row['parent_manifest_index'],source_row=source,tuple=tuple_))
    rows = [dict(original_parent_manifest_index=r['parent_manifest_index'],source_row=r['source_row'],identity=r['tuple']['identity'],account_hex=r['tuple']['address_hex']) for r in expected]
    if len(expected) != 45 or selected != expected or prepared['rows'] != rows:
        raise ValueError('immutable fifth candidate selection')


def fifth_read_charges(pins, children, source_total, profile=FIFTH_PROFILE):
    """Charge every direct, repeated-child and extra semantic parse read."""
    fixed = finite_profile(profile)
    total = sum(r['bytes'] for r in pins)+children['referenced_bytes']+children['semantic_read_bytes']
    binaries = sum(r['bytes'] for r in pins[fixed['source_count']+2:fixed['source_count']+4])
    if total > PIN_TOTAL or total-binaries > 31457280 or source_total+children['source_bytes'] > SOURCE_TOTAL:
        raise ValueError('sealed fifth aggregate caps')
    return total


FIFTH_EQUIVALENCE_TESTS = [
    'six_saved_windows_have_complete_equivalent_reports',
    'seventh_is_fixed_derived_and_reverse_rejected',
    'fixed_successors_authenticate_prior_and_reject_substitution',
    'first_residual_saved_bundle_report_is_completely_equal',
    'second_residual_saved_bundle_report_is_completely_equal',
    'third_residual_saved_bundle_report_is_completely_equal',
    'fourth_residual_saved_bundle_report_is_completely_equal',
]


FIFTH_COMPARISON_PROFILES = [{'profile': 'native-current', 'before': {'path': '/home/fry/artifacts/n4-binding-resume-20261011/native-live-replay-frame-cost.json', 'bytes': 10911, 'sha256': '2ccf155e2a851cd52161f090bcd3cc76b09f212a44647ba8f76b7db0b7511fbb'}, 'after': {'path': '/home/fry/artifacts/vrf-residual-20261014-current/source/compatibility/native-current.json', 'bytes': 10945, 'sha256': '6cc2a18103fb1ed1e1e1826fdee07edb70c6e4d8e2c9c61ffa1e556363e80377'}, 'fields': ['/source_profile/compiled/0', '/source_profile/compiled/10', '/source_profile/compiled/11', '/source_profile/compiled/12', '/source_profile/executable'], 'executable': {'bytes': 192505688, 'path': '/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/target/debug/replay_native_witness', 'sha256': 'a588690a16ad0d988b7cfdb4760bb4d091e20922bd0e7816436161a34c818def'}}, {'profile': 'first-d-current', 'before': {'path': '/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d-independent-replay.json', 'bytes': 32805, 'sha256': '8bd32c28356e85944a37a900c95e8ae3cb7cbc7f942f27cc2769257b6be36854'}, 'after': {'path': '/home/fry/artifacts/vrf-residual-20261014-current/source/compatibility/first-d-current.json', 'bytes': 32813, 'sha256': 'c0f8597049363d641a20df01e60c73c9eda2c710d9b975d6468a45146277def7'}, 'fields': ['/authority/executable', '/authority/sources/2', '/authority/sources/16', '/authority/sources/17', '/authority/sources/18', '/vrf_sources/0', '/vrf_sources/3', '/vrf_sources/4', '/vrf_sources/5'], 'executable': {'bytes': 194461280, 'path': '/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/target/debug/replay_vrf_witness', 'sha256': '05ed77937c3bb64f43fad3f360b4e83fa9dfb73b0ffb3d8808fc534c7baab77b'}}]
FIFTH_COMPARISON_ARGV = ['/usr/bin/python3','-c',"import sys;sys.path.insert(0,'/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/tools');import bounded_vrf;bounded_vrf.emit_fifth_comparison()"]

def verify_fifth_comparison(payload, source_pins, tool_pin, module=None, profile=FIFTH_PROFILE):
    """Authenticate both whole reports and apply only the finite pin substitutions.

    No report path is followed. Exact UTF8 body pins and fixed mapping positions
    are required before complete JSON equality; no boolean supplies proof.
    """
    fixed = finite_profile(profile)
    if module is None:
        source = next(r for p in fifth_contract(profile)['reference_placements'] for r in p['references'] if r['role'] == 'record-builder-source')
        module = record_verifier({k:source[k] for k in ('path','bytes','sha256')})
    module.bounded_shape(payload)
    if set(payload) != {'schema','tool_source','pairs'} or type(payload['schema']) is not int or payload['schema'] != 1 or payload['tool_source'] != tool_pin or not isinstance(payload['pairs'],list) or len(payload['pairs']) != 2:
        raise ValueError('finite current comparator schema')
    source_map = {r['path']:r for r in source_pins}
    for pair, fixed in zip(payload['pairs'],fixed['comparison']):
        if set(pair) != {'profile','before','after','before_body_utf8','after_body_utf8','mappings'} or pair['profile'] != fixed['profile'] or pair['before'] != fixed['before'] or pair['after'] != fixed['after'] or [r.get('field') for r in pair['mappings']] != fixed['fields']:
            raise ValueError('finite current comparator identities')
        values = []
        for side in ('before','after'):
            body = pair[side+'_body_utf8']
            if not isinstance(body,str):raise ValueError('full report UTF8 body')
            raw = body.encode('utf-8'); pin = fixed[side]
            if len(raw) != pin['bytes'] or hashlib.sha256(raw).hexdigest() != pin['sha256']:
                raise ValueError('full current report body pin')
            if len(raw) > METADATA_CAP:raise ValueError('full report parse cap')
            values.append(module.decode(raw,METADATA_CAP))
        before,after = values
        def locate(value,field):
            parts=field.strip('/').split('/')
            for part in parts[:-1]:value=value[int(part)] if isinstance(value,list) else value[part]
            return value,int(parts[-1]) if isinstance(value,list) else parts[-1]
        for mapping in pair['mappings']:
            if set(mapping) != {'field','predecessor','successor'}:raise ValueError('finite current mapping schema')
            a,ak=locate(before,mapping['field']);b,bk=locate(after,mapping['field'])
            if mapping['predecessor'] != a[ak] or mapping['successor'] != b[bk]:raise ValueError('finite current mapping body')
            if mapping['field'].endswith('/executable'):
                expected=fixed['executable']
            else:
                old=a[ak]
                path=str((SRC/old['path']).resolve())
                if path not in source_map:raise ValueError('finite current owner source')
                expected=dict(source_map[path],path=old['path'])
                if b[bk]['path'] != old['path']:raise ValueError('finite current source rename')
            if b[bk] != expected:raise ValueError('finite current source/binary pin')
            a[ak]=expected
        if before != after:raise ValueError('complete current report inequality')
    return True


def emit_fifth_comparison(profile=FIFTH_PROFILE):
    """Emit the two bounded complete offline pairs under the fixed inline argv."""
    fixed = finite_profile(profile)
    source_pins=[pin_file(path,cap=SOURCE_CAP) for path in expected_source_paths(profile)]
    tool=pin_file(Path(__file__).resolve(),cap=SOURCE_CAP)
    pairs=[]
    for fixed in fixed['comparison']:
        bodies={}
        for side in ('before','after'):
            pin=fixed[side];pin_file(pin['path'],pin,METADATA_CAP)
            bodies[side]=read_fifth_body(pin['path'],METADATA_CAP).decode('utf-8')
        before=json.loads(bodies['before'],object_pairs_hook=unique_object)
        after=json.loads(bodies['after'],object_pairs_hook=unique_object)
        mappings=[]
        for field in fixed['fields']:
            def at(value):
                for part in field.strip('/').split('/'):value=value[int(part)] if isinstance(value,list) else value[part]
                return value
            mappings.append(dict(field=field,predecessor=at(before),successor=at(after)))
        pairs.append(dict(profile=fixed['profile'],before=fixed['before'],after=fixed['after'],before_body_utf8=bodies['before'],after_body_utf8=bodies['after'],mappings=mappings))
    payload=dict(schema=1,tool_source=tool,pairs=pairs)
    verify_fifth_comparison(payload,source_pins,tool,profile=profile)
    raw=(json.dumps(payload,sort_keys=True,separators=(',',':'))+'\n').encode('utf-8')
    if len(raw)>327680:raise ValueError('full current comparator output cap')
    sys.stdout.buffer.write(raw)


def verify_fifth_gate_result(result, log_pin, command):
    """Require the exact current invocation, success exit and full output pin."""
    if type(result.get('exit_code')) is not int or result.get('exit_code') != 0 or result.get('log') != log_pin or result.get('command') != command:
        raise ValueError('current complete equivalence gate binding')


def verify_fifth_equivalence(evidence, contract, module, profile=FIFTH_PROFILE):
    """Bind current full-report parity to exact source and complete gate outputs."""
    fixed = finite_profile(profile)
    tail = {r['index']:r['path'] for r in contract['command_inventory'] if r['group'] == 'current_tail54'}
    by_path = {r['path']:r for r in evidence}
    summary = read_json(tail[46],16384)
    fields = {'schema','source_freeze','gate_results','gate_logs','required_exact_tests','matched_tests',fixed['equality_field'],'native_and_first_d_complete_equal','accepted'}
    expected_results = [by_path[tail[i]] for i in (0,20)]
    expected_logs = [by_path[tail[i]] for i in (1,21,47)]
    if set(summary) != fields or type(summary['schema']) is not int or summary['schema'] != 1 or summary['accepted'] is not False or summary['source_freeze'] != by_path[tail[38]] or summary['gate_results'] != expected_results or summary['gate_logs'] != expected_logs or summary['required_exact_tests'] != fixed['tests'] or type(summary['matched_tests']) is not int or summary['matched_tests'] != len(fixed['tests']) or summary[fixed['equality_field']] is not True or summary['native_and_first_d_complete_equal'] is not True:
        raise ValueError('current complete equivalence summary')
    commands=[['rtk','proxy','cargo','test','--manifest-path','experiments/evm_feasibility/snapshot_qualifier/Cargo.toml','--lib','--bins'],['rtk','proxy',*fixed['argv']]]
    for result_pin,log_pin,command in zip(expected_results,expected_logs[:2],commands):
        result = read_json(result_pin['path'],16384)
        verify_fifth_gate_result(result,log_pin,command)
    source_pins=read_json(tail[38],FREEZE_CAP)
    tools=read_json(tail[39],METADATA_CAP)
    raw=read_fifth_body(tail[21],FREEZE_CAP)
    payload=module.decode(raw,FREEZE_CAP)
    verify_fifth_comparison(payload,source_pins,tools[0],module,profile)
    # All original assertions compare entire reports, including logical/physical
    # results. These exact selectors cover ten complete and four residual profiles.
    output = read_fifth_body(tail[1],327680)
    for name in fixed['tests']:
        if ('test vrf_shared_window::tests::'+name+' ... ok').encode('ascii') not in output:
            raise ValueError('nonzero complete equivalence selector')
    if by_path[tail[1]]['bytes'] != by_path[tail[47]]['bytes'] or by_path[tail[1]]['sha256'] != by_path[tail[47]]['sha256']:
        raise ValueError('current full equivalence output copy')


def verify_sixth_summary(value):
    """Authenticate the complete fixed failure envelope already in role29.

    Check exact UTF8 length and digest without nested parsing or path reads.
    Missing, truncated or substituted evidence rejects the entire instance.
    """
    body=value.get('s7_failure_utf8') if isinstance(value,dict) else None
    if not isinstance(body,str):
        raise ValueError('sixth complete failure body')
    raw=body.encode('utf-8')
    if len(raw) != SIXTH_FAILURE_UTF8[0] or hashlib.sha256(raw).hexdigest() != SIXTH_FAILURE_UTF8[1]:
        raise ValueError('sixth complete failure byte pin')


def verify_sixth_summary_buffer(raw, pin, module):
    """Decode only the bytes retained by direct role29 authentication.

    This function has no filesystem operation and cannot supply a fallback read.
    Require exact byte identity before parsing and full fixed-body verification.
    """
    if not isinstance(raw,bytes) or len(raw)>65536 or len(raw)!=pin['bytes'] or hashlib.sha256(raw).hexdigest()!=pin['sha256']:
        raise ValueError('sixth summary byte pin')
    verify_sixth_summary(module.decode(raw,65536))


def verify_fifth_records(evidence, profile=FIFTH_PROFILE, summary_bytes=None):
    """Verify the three finite successor placements and each repeated charge."""
    fixed = finite_profile(profile)
    contract = fifth_contract(profile)
    placements = contract['reference_placements']
    sources = [r for p in placements for r in p['references'] if r['role'] == 'record-builder-source']
    if len(sources) != 1:
        raise ValueError('one fixed shared verifier source')
    module = record_verifier({k: sources[0][k] for k in ('path','bytes','sha256')})
    if profile == SIXTH_PROFILE:
        summary=next(r for r in contract['command_inventory'] if r['group']=='current_tail54' and r['index']==29)
        summary_pin=next(r for r in evidence if r['path']==summary['path'])
        verify_sixth_summary_buffer(summary_bytes,summary_pin,module)
    identities = {}
    total = count = streamed = 0
    by_path = {r['path']:r for r in evidence}
    for placement in placements:
        destination = placement['destination']
        pin_file(destination, by_path[destination], FREEZE_CAP)
        value = module.decode(read_fifth_body(destination,FREEZE_CAP))
        # Each successor is the shared record itself, with no hidden envelope.
        result = module.verify(value, destination, placement['common_contract'], placement['references'], identities)
        count += result['reference_count']; total += result['referenced_bytes']; streamed += result['streamed_bytes']
    if count > 64 or total > 1048576:
        raise ValueError('finite child count/byte cap')
    expected_count = sum(1+len(p['references']) for p in placements)
    expected_bytes = sum(p['common_contract']['bytes']+sum(r['bytes'] for r in p['references']) for p in placements)
    if (count,total) != (expected_count,expected_bytes):
        raise ValueError('complete finite child coverage')
    full = [r for p in placements for r in p['references'] if r['role'] == 'full-result-equivalence']
    if len(full) != 1 or full[0]['bytes'] != 45182:
        raise ValueError('historical semantic parse role')
    raw = read_fifth_body(full[0]['path'],METADATA_CAP)
    if len(raw) != full[0]['bytes'] or hashlib.sha256(raw).hexdigest() != full[0]['sha256']:
        raise ValueError('historical semantic parse drift')
    historical = module.decode(raw, METADATA_CAP)
    parse = contract['historical_equivalence_parse']
    if full[0] != dict(role=parse['role'], **parse['pin']) or set(historical) != set(parse['exact_keys']):
        raise ValueError('historical projection schema')
    if historical['schema'] != 1 or historical['matched_tests'] != 11 or historical['new_facts'] != 0 or any(historical[k] is not True for k in ('first_second_third_and_ten_complete_equal','only_reviewed_provenance_changes_allowed')):
        raise ValueError('historical projection facts')
    native = historical['native_and_first_d_complete_equality']
    if not isinstance(native,list) or [r.get('profile') for r in native] != ['native-current','first-d-current'] or any(r.get('complete_equal_except_exact_pin_mappings') is not True for r in native):
        raise ValueError('historical native and first-D equality')
    output = historical['full_gate_output']
    projection = {k:v for k,v in historical.items() if k != 'full_gate_output'}
    if not isinstance(output,str) or len(output.encode('utf-8')) != parse['output_bytes'] or hashlib.sha256(output.encode('utf-8')).hexdigest() != parse['output_sha256'] or hashlib.sha256(json.dumps(projection,sort_keys=True,separators=(',',':'),ensure_ascii=True).encode('utf-8')).hexdigest() != parse['projection_canonical_sha256']:
        raise ValueError('historical complete projection drift')
    correction_spec = contract['correction_record48']
    correction_path = next(r['path'] for r in contract['command_inventory'] if r['group'] == 'current_tail54' and r['index'] == 48)
    correction = module.decode(read_fifth_body(correction_path,FREEZE_CAP))
    if set(correction) != set(correction_spec['fields']) or correction['schema'] != correction_spec['schema'] or correction['accepted'] is not False or not isinstance(correction['findings'],str) or len(correction['findings'].encode('utf-8')) > correction_spec['findings_utf8_cap'] or len(correction['failures']) != len(correction_spec['failures']):
        raise ValueError('finite current correction schema')
    for actual, required in zip(correction['failures'],correction_spec['failures']):
        if set(actual) != set(correction_spec['failure_fields']) or {k:actual[k] for k in required} != required or not isinstance(actual['body_utf8'],str):
            raise ValueError('finite current correction role')
        body = actual['body_utf8'].encode('utf-8')
        if len(body) != required['bytes'] or hashlib.sha256(body).hexdigest() != required['sha256']:
            raise ValueError('finite current full failure body')
    verify_fifth_equivalence(evidence,contract,module,profile)
    source_bytes = sum(r['bytes'] for p in placements for r in p['references']
                       if r['role'] in {'record-builder-source','record-builder-tests'})
    return dict(reference_count=count,referenced_bytes=total,streamed_bytes=streamed,
                semantic_read_bytes=len(raw),source_bytes=source_bytes)



def expand_fifth_evidence(value, contract):
    """Expand exactly 285 ordered byte/digest pairs with authority-owned paths."""
    start = 91 if contract['shape']['voter'] == sealed_profile(SIXTH_PROFILE)['voter'] else 89
    expected = contract['command_inventory'][start:374]
    if not isinstance(value, list) or len(value) != 374-start or len(expected) != 374-start:
        raise ValueError('fixed fifth compact count')
    result = []
    for pair, required in zip(value, expected):
        if not isinstance(pair,list) or len(pair) != 2 or type(pair[0]) is not int or not 0 <= pair[0] <= required.get('cap_bytes',required.get('bytes')) or not isinstance(pair[1],str) or len(pair[1]) != 64 or any(c not in '0123456789abcdef' for c in pair[1]):
            raise ValueError('fixed fifth compact row')
        result.append(dict(path=required['path'],bytes=pair[0],sha256=pair[1]))
    return result


def publish_fifth_records(findings, profile=FIFTH_PROFILE):
    """Publish only the three reviewed reference records and finite correction.

    Each body is authenticated by the shared deterministic builder. Outputs are
    exclusive at authority-owned paths. Findings supply no source acceptance.
    """
    fixed = finite_profile(profile)
    contract = fifth_contract(profile)
    if not isinstance(findings,str) or len(findings.encode('utf-8')) > contract['correction_record48']['findings_utf8_cap']:
        raise ValueError('fixed current findings cap')
    sources = [r for p in contract['reference_placements'] for r in p['references'] if r['role'] == 'record-builder-source']
    module = record_verifier({k:sources[0][k] for k in ('path','bytes','sha256')})
    for placement in contract['reference_placements']:
        destination = placement['destination']
        raw = module.build(destination,placement['common_contract'],findings,placement['references'])
        module.publish(destination,raw,destination,placement['common_contract'],placement['references'])
    definition = contract['correction_record48']
    failures = []
    for required in definition['failures']:
        pin_file(required['path'],{k:required[k] for k in ('path','bytes','sha256')},FREEZE_CAP)
        body = read_fifth_body(required['path'],FREEZE_CAP)
        failures.append(dict(required,body_utf8=body.decode('utf-8')))
    correction = dict(schema=definition['schema'],failures=failures,findings=findings,accepted=False)
    destination = next(r['path'] for r in contract['command_inventory'] if r['group'] == 'current_tail54' and r['index'] == 48)
    if profile == SIXTH_PROFILE:
        # The reviewed sixth representation changes only outer whitespace.
        # Every embedded failure string and findings byte remains complete.
        body=(json.dumps(correction,ensure_ascii=False,sort_keys=True,separators=(',',':'))+'\n').encode('utf-8')
        if len(body)>FREEZE_CAP:
            raise ValueError('sixth complete correction byte cap')
    else:
        body=encoded(correction,FREEZE_CAP)
    exclusive(Path(destination),body)


def fifth_spec_from_roster(source_commit, metadata, profile=FIFTH_PROFILE):
    """Assemble exact current pins after all fixed evidence exists.

    This publishes nothing. Every row is authenticated against the one sealed
    roster; full build validation must still precede READY and capture.
    """
    fixed = finite_profile(profile)
    contract = fifth_contract(profile)
    pins=[]
    summary_bytes=None
    for row in contract['command_inventory'][:-1]:
        if profile == SIXTH_PROFILE and row['path'] == str(SIXTH_RUN.parent/'source/validation-summary.json'):
            buffer=bytearray()
            pins.append(pin_file(row['path'],cap=row.get('cap_bytes',row.get('bytes')),_body=buffer))
            summary_bytes=bytes(buffer)
        else:
            pins.append(pin_file(row['path'],cap=row.get('cap_bytes',row.get('bytes'))))
    verify_finite_inventory(pins,contract['command_inventory'][:-1],profile,summary_bytes)
    return dict(schema=1,run=str(fixed['run']),source_commit=source_commit,
                sources=pins[:fixed['source_count']],tool_sources=pins[fixed['source_count']:fixed['source_count']+2],binaries=pins[fixed['source_count']+2:fixed['source_count']+4],
                preflight=pins[fixed['source_count']+4],evidence=[[r['bytes'],r['sha256']] for r in pins[fixed['evidence_start']:374]],
                preparation=pins[374],copy_metadata=metadata)


def build_fifth(spec_path, spec, report):
    """Build one of the two sealed fixed45 instances through the shared verifier.

    All command roles, source closure, embedded pins, full preflight, repeated
    evidence charges and fixed outputs must pass before complete bytes return.
    No database path is opened or inspected here. Missing evidence rejects all.
    """
    profile = report['profile']
    fixed = finite_profile(profile)
    contract = fifth_contract(profile)
    fields = {'schema','run','source_commit','sources','tool_sources','binaries','preflight','evidence','copy_metadata','preparation'}
    if set(spec) != fields or type(spec['schema']) is not int or spec['schema'] != 1 or spec['run'] != str(fixed['run']):
        raise ValueError('sealed fifth spec schema/run')
    run = outside_database(spec['run'])
    if run.resolve(strict=True) != fixed['run']:
        raise ValueError('sealed fifth canonical run')
    if [r['path'] for r in spec['sources']] != expected_source_paths(profile):
        raise ValueError('sealed fifth source closure')
    if [r['path'] for r in spec['tool_sources']] != [str(Path(__file__).resolve()),str(Path(__file__).resolve().with_name('test_bounded_vrf.py'))]:
        raise ValueError('sealed fifth tool closure')
    evidence = expand_fifth_evidence(spec['evidence'], contract)
    pins = spec['sources']+spec['tool_sources']+spec['binaries']+[spec['preflight']]+evidence+[spec['preparation'],pin_file(spec_path,cap=METADATA_CAP)]
    summary_bytes=verify_finite_inventory(pins, contract['command_inventory'],profile)
    if len(pins) != 376:
        raise ValueError('sealed fifth command count')
    # Earlier source pins stay immutable except this explicitly reviewed delta.
    old = fixed['predecessor']
    changed = {str(SRC/'vrf_shared_window.rs'),str(REPO/'rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs'),str(REPO/'rust/crates/rustaxa-storage/src/concrete_state/witness_live_tests.rs')}
    if len(old) != fixed['source_count']-2 or len({r['path'] for r in old}) != len(old) or [r['path'] for r in old] != [r['path'] for r in spec['sources'][:len(old)]]:
        raise ValueError('complete fifth predecessor coverage')
    for before, after in zip(old,spec['sources']):
        if before['path'] not in changed and before != after:
            raise ValueError('undeclared fifth source drift')
    if read_json(next(r['path'] for r in contract['command_inventory'] if r['group'] == 'current_tail54' and r['index'] == 38),FREEZE_CAP) != spec['sources'] or read_json(next(r['path'] for r in contract['command_inventory'] if r['group'] == 'current_tail54' and r['index'] == 39),METADATA_CAP) != spec['tool_sources']:
        raise ValueError('complete fifth source freeze drift')
    validate_preflight(report, spec['preparation']['path'])
    embedded = report['vrf_sources']+report['authority']['six_input_owner']['sources']
    if len(report['vrf_sources']) != 22 or len(report['authority']['six_input_owner']['sources']) != 25:
        raise ValueError('sealed fifth embedded counts')
    baseline = read_json(SHARED_RUN/'seventh-voter/vrf-seventh-voter-window-preflight.json',FREEZE_CAP)
    expected_paths = [r['path'] for r in baseline['vrf_sources'][:19]]+['vrf_shared_window.rs',f'bin/live_vrf_residual_{fixed["cli"]}.rs' ,f'bin/replay_vrf_residual_{fixed["cli"]}.rs' ]
    if [r['path'] for r in report['vrf_sources']] != expected_paths:
        raise ValueError('sealed fifth embedded source order')
    source_map = {r['path']:r for r in spec['sources']}
    for row in embedded:
        actual = dict(row,path=str((SRC/row['path']).resolve()))
        if source_map.get(actual['path']) != actual:
            raise ValueError('sealed fifth embedded source pin')
    if report['authority']['six_input_owner']['executable'] != spec['binaries'][0]:
        raise ValueError('sealed fifth executable binding')
    source_total = sum(r['bytes'] for r in spec['sources']+spec['tool_sources'])
    children = verify_fifth_records(evidence,profile,summary_bytes)
    total = fifth_read_charges(pins,children,source_total,profile)
    if len(spec['source_commit']) != 40 or any(c not in '0123456789abcdef' for c in spec['source_commit']):
        raise ValueError('sealed fifth source HEAD')
    if [r['path'] for r in spec['copy_metadata']] != [str(COPY),str(COPY/'db/db'),str(COPY/'db/state_db')] or any(set(r) != {'path','device','inode','mode','mtime_ns'} for r in spec['copy_metadata']):
        raise ValueError('sealed fifth metadata')
    command = [spec['binaries'][0]['path'],'--bundle-output',str(run/(fixed['stem']+'.rcv4')),'--report-output',str(run/(fixed['stem']+'-capture.json'))]
    return encoded(dict(schema=1,profile=profile,run=str(run),source_commit=spec['source_commit'],cwd=str(REPO),environment=ENV,capture_command=command,pins=pins,pin_count=len(pins),pinned_file_bytes=total,source_count=fixed['source_count'],complete_source_bytes=source_total,embedded_source_bytes=sum(r['bytes'] for r in embedded),copy_metadata=spec['copy_metadata'],accepted=False),FREEZE_CAP)


def build(spec_path):
    """Return deterministic bytes after all explicit pins and sealed fields pass.

    The instance spec, freeze and invocation need independent review. This
    function never opens, scans or stats a database path and publishes nothing.
    """
    spec = read_json(spec_path)
    pin_file(spec['preflight']['path'], spec['preflight'], FREEZE_CAP)
    report = read_json(spec['preflight']['path'], FREEZE_CAP)
    fixed = sealed_profile(report['profile'])
    if report['profile'] in {FIFTH_PROFILE, SIXTH_PROFILE}:
        return build_fifth(spec_path, spec, report)
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
        if freeze.get('profile') != SIXTH_PROFILE or row['path'] != str(SIXTH_RUN.parent/'source/validation-summary.json'):
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
    if freeze.get('profile') in {FIFTH_PROFILE, SIXTH_PROFILE}:
        profile = freeze['profile']
        fixed = finite_profile(profile)
        summary_bytes=verify_finite_inventory(freeze['pins'], fifth_contract(profile)['command_inventory'],profile)
        children = verify_fifth_records(freeze['pins'][fixed['evidence_start']:374],profile,summary_bytes)
        total = fifth_read_charges(freeze['pins'],children,freeze['complete_source_bytes'],profile)
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
    if freeze.get('schema') != 1 or freeze.get('profile') not in {PROFILE, FIFTH_PROFILE, SIXTH_PROFILE, *SUCCESSORS, *RESIDUAL_PROFILES} or run != freeze_path.parent \
            or not run.is_relative_to(Path('/home/fry/artifacts')) or run.resolve(strict=True) != run:
        raise ValueError('trusted freeze/run binding')
    fixed = sealed_profile(freeze['profile'])
    ordinal = fixed['ordinal']
    residual = freeze['profile'] in RESIDUAL_PROFILES
    if (residual and run != RESIDUAL_RUN/ordinal) or (freeze['profile'] in {FIFTH_PROFILE,SIXTH_PROFILE} and run != finite_profile(freeze['profile'])['run']):
        raise ValueError('fixed residual guard run')
    # Trusted admission ends here. All later failures permanently consume this
    # attempt, including offline pin, HEAD, output and metadata failures.
    stem = finite_profile(freeze['profile'])['stem'] if freeze['profile'] in {FIFTH_PROFILE,SIXTH_PROFILE} else f'vrf-{ordinal}-voter-window'
    exclusive(run/f'{stem}-attempt.json', encoded(
        dict(command=freeze['capture_command'], source_commit=freeze['source_commit'],
             freeze_sha256=expected_sha), RESULT_CAP))
    verify_files(freeze)
    spec_pin = freeze['pins'][-1]
    if build(Path(spec_pin['path'])) != frozen_bytes:
        raise ValueError('freeze not deterministic builder output')
    if freeze['profile'] not in {PROFILE, FIFTH_PROFILE, SIXTH_PROFILE, *SUCCESSORS, *RESIDUAL_PROFILES} or freeze['cwd'] != str(REPO) or freeze['environment'] != ENV or Path.cwd() != REPO:
        raise ValueError('exact invocation binding')
    head = subprocess.check_output(['/usr/bin/git', 'rev-parse', 'HEAD'], cwd=REPO, env=ENV).decode().strip()
    if head != freeze['source_commit']:
        raise ValueError('source HEAD drift')
    outputs = [f'{stem}{suffix}' for suffix in ['.rcv4' if freeze['profile'] in {FOURTH_PROFILE, FIFTH_PROFILE, SIXTH_PROFILE} else '.rcv3' if residual else '.rcv2', '-capture.json', '-live.log', '-live-result.json']]
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
