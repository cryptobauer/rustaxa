#!/usr/bin/env python3
"""Reproduce only the accepted immutable Cacti producer instance, not new eras."""
import hashlib
import json
import os
from pathlib import Path
import mainnet_era_reference as reference
from mainnet_era_compare import comparison_digest

run=Path('/home/fry/artifacts/mainnet-era-20261026-01a179de')
path=reference.CORPUS/'cacti.manifest.json'
raw=path.read_bytes(); manifest=json.loads(raw)
# The second exact identity removes only three license-comment trailing spaces.
# The frozen manifest retains its original support identity and all semantic pins.
assert hashlib.sha256((reference.HERE/'mainnet_era_compare.py').read_bytes()).hexdigest() in {
    '9de6110dcc9816cb8436c1ac585497c59375aef085d18bfd8e8a7e2398fee1a6',
    'b51faffca52afaa25c863a5459e5b817aa40d808966624c4d3179ecd8ef4aa8f',
}
assert os.environ['RUN_ARTIFACT_DIR']==str(run)
scenario=manifest['scenario']; schedule=reference.schedule_for(scenario,run)
result={'scope':'exact retained Cacti effective Go sources/inputs/flags/selectors; current runner only','input_manifest_sha256':hashlib.sha256(raw).hexdigest(),'streams':{},'commands':[]}
for label,pin in manifest['references'].items():
    shards=[]
    for selected in manifest['shards'][label]['0']:
        command_path=Path(selected['stream']['capture'])/'command.json'
        frozen=json.loads(command_path.read_bytes())
        value,directory=reference.run(pin,label,scenario,schedule,run,0,selected['selector'],frozen=frozen)
        reference.validate_shard(value,pin,scenario,selected['selector'],0)
        shards.append(value)
        result['commands'].append({'original':str(command_path),'original_sha256':hashlib.sha256(command_path.read_bytes()).hexdigest(),'fresh':str(directory/'command.json'),'stream':reference.retain(value,directory)})
    assembled=reference.assemble(shards,pin,scenario,0)
    identity=comparison_digest(assembled)
    assert identity==manifest['comparison']['streams'][label],(label,'exact same-pin comparison identity')
    result['streams'][label]={'comparison':identity,'stream':reference.retain(assembled)}
encoded=(json.dumps(result,sort_keys=True,indent=2)+'\n').encode()
target=run/('frozen-cacti-reproduction-'+hashlib.sha256(encoded).hexdigest()+'.json')
with target.open('xb') as output: output.write(encoded)
print('Exact retained Cacti producer reproduced at both pins:',target)
