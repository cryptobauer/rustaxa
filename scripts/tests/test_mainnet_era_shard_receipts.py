"""Authenticate actual receipt bytes before the existing full shard validation."""
import copy
import gzip
import json
import os
from pathlib import Path
import sys
import unittest
from unittest import mock
import uuid

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'experiments/evm_feasibility'))
import mainnet_era_reference as era
import mainnet_era_estimate_reference as estimate
from test_mainnet_era_shards import fixture


class ReceiptAuthentication(unittest.TestCase):
    def setUp(self):
        self.root=Path(os.environ['RUN_ARTIFACT_DIR'])/('receipt-contract-'+uuid.uuid4().hex)
        self.root.mkdir();self.corpus=self.root/'corpus';self.corpus.mkdir()
        self.scenario,shards=fixture();self.binding={'source':'current-pinned-source'}
        self.index=self.root/'receipts.json';self.pins=[]
        parent=self.root/'parent';parent.mkdir()
        self.launch=parent/'launch.json'
        source=parent/'source.json';source.write_bytes(b'[]')
        self.source=source
        source_pin={'path':str(source),'sha256':era.digest(source.read_bytes())}
        self.write(self.launch,{'argv':['single-shard'],'source_manifest':source_pin})
        self.result=parent/'source-result.json'
        self.write(self.result,{'argv':['single-shard'],'source_manifest':source_pin,'source_stable':True,'child_exit':0})
        (parent/'capture').mkdir();self.outer=parent/'capture/status.json'
        self.write(self.outer,{'capture_complete':True,'child_exit':0,'runner_exit':0,'outcome':'exited'})
        objects=self.root/'source-objects';objects.mkdir()
        producer=b'pinned effective Go producer';hashed=era.digest(producer);(objects/hashed).write_bytes(producer)
        input_raw=b'actual input';self.input_hash=era.digest(input_raw);(objects/self.input_hash).write_bytes(input_raw)
        self.sources={'cmd/mainnet_era/producer.go':{'sha256':hashed,'bytes':len(producer)},'cmd/mainnet_era/input.json':{'sha256':self.input_hash,'bytes':len(input_raw)}}
        self.patchers=[mock.patch.object(era,'CORPUS',self.corpus),mock.patch.object(estimate,'CORPUS',self.corpus),mock.patch.object(era,'cutover_context',return_value={'schema':2}),mock.patch.dict(os.environ,{'RUSTAXA_ERA_LAUNCH_MANIFEST':str(self.launch)})]
        for patcher in self.patchers:patcher.start();self.addCleanup(patcher.stop)
        for label,pin in era.REFERENCES.items():
            for selector,original in zip(era.selectors(self.scenario),shards):
                data=copy.deepcopy(original);data['reference']=pin;data['input_sha256']=self.input_hash
                directory=self.root/(label+'-'+str(len(self.pins)));directory.mkdir()
                argv=['go','run',label]
                self.write(directory/'command.json',{'reference_pin':pin,'selector':selector,'phase':'captured','argv':argv,'base_archive_sha256':'a'*64,'base_archive_bytes':100,'effective_sources':self.sources})
                body=json.dumps(data).encode()+b'\n';encoded=gzip.compress(body,mtime=0)
                (directory/'stdout.gz').write_bytes(encoded)
                err=b'real compiler stderr';err_encoded=gzip.compress(err,mtime=0);(directory/'stderr.gz').write_bytes(err_encoded)
                status={'capture_complete':True,'child_exit':0,'argv':argv,'outcome':'exited','launched':True,'runner_error':None,'streams':{'stdout':{'file':'stdout.gz','gzip_sha256':era.digest(encoded),'sha256':era.digest(body),'bytes':len(body)},'stderr':{'file':'stderr.gz','gzip_sha256':era.digest(err_encoded),'sha256':era.digest(err),'bytes':len(err)}}}
                self.write(directory/'capture.json',status)
                command=json.loads((directory/'command.json').read_bytes());command.update(status);self.write(directory/'command.json',command)
                receipt=era.shard_receipt(data,directory,label,self.scenario,selector,0,self.binding,self.root)
                self.pins.append({'path':str(receipt),'sha256':era.digest(receipt.read_bytes())})
        self.write(self.index,self.pins)

    def write(self,path,value):path.write_text(json.dumps(value))

    def load(self):return era.load_shard_receipts(self.index,self.binding,self.scenario,self.root)

    def mutate_receipt(self,mutate):
        path=Path(self.pins[0]['path']);value=json.loads(path.read_bytes());mutate(value)
        self.write(path,value);self.pins[0]['sha256']=era.digest(path.read_bytes());self.write(self.index,self.pins)

    def test_complete_two_pin_receipts_use_existing_assembler(self):
        groups=self.load()
        for label,pin in era.REFERENCES.items():
            result=era.assemble([row[1] for row in groups[(label,0)]],pin,self.scenario)
            self.assertEqual(len(result['periods']),4)
            self.assertEqual(len(result['api_cases']),21)
        self.assertEqual(list(self.corpus.iterdir()),[])

    def test_assembler_writes_only_task_owned_output(self):
        groups=self.load();scheduler=self.root/'scheduler.json';scheduler.write_bytes(b'{}')
        with mock.patch.object(sys,'argv',['mainnet_era_reference.py','--scenario','unit','--assemble-shards',str(self.index)]),mock.patch.object(era,'scenarios',return_value=[self.scenario]),mock.patch.object(era,'schedule_for',return_value=[]),mock.patch.object(era,'scheduler_path',return_value=scheduler),mock.patch.object(era,'load_shard_receipts',return_value=groups),mock.patch.object(era,'comparison_digest',return_value='plumbing-test-only'),mock.patch.dict(os.environ,{'RUN_ARTIFACT_DIR':str(self.root)}):
            era.main()
        self.assertEqual(list(self.corpus.iterdir()),[])
        manifests=list(self.root.glob('era-assembly-*/unit.manifest.json'))
        self.assertEqual(len(manifests),1)
        value=json.loads(manifests[0].read_bytes())
        self.assertEqual(set(value['streams']),set(era.REFERENCES))

    def test_missing_pin_or_duplicate_receipt_rejected(self):
        for pins in (self.pins[:-5],self.pins+[self.pins[0]]):
            self.write(self.index,pins)
            with self.assertRaises(ValueError):self.load()

    def test_receipt_hash_and_source_binding_rejected(self):
        saved=list(self.pins);bad=copy.deepcopy(saved);bad[0]['sha256']='0'*64;self.write(self.index,bad)
        with self.assertRaises(ValueError):self.load()
        self.write(self.index,saved)
        self.mutate_receipt(lambda value:value.__setitem__('binding',{'source':'changed'}))
        with self.assertRaises(ValueError):self.load()

    def test_failed_or_unknown_parent_never_qualifies(self):
        value=json.loads(self.result.read_bytes())
        for change in ({'child_exit':1},{'source_stable':False},{'argv':['other-command']}):
            self.write(self.result,{**value,**change})
            with self.assertRaises(ValueError):self.load()
        self.result.unlink()
        with self.assertRaises(FileNotFoundError):self.load()

    def test_actual_stream_tampering_rejected(self):
        receipt=json.loads(Path(self.pins[0]['path']).read_bytes())
        (Path(receipt['command']['path']).parent/'stdout.gz').write_bytes(gzip.compress(b'changed output\n'))
        with self.assertRaises(ValueError):self.load()

    def test_stderr_and_parent_manifest_tampering_rejected(self):
        receipt=json.loads(Path(self.pins[0]['path']).read_bytes())
        err=Path(receipt['command']['path']).parent/'stderr.gz';original=err.read_bytes()
        err.write_bytes(gzip.compress(b'tampered compiler stderr'))
        with self.assertRaises(ValueError):self.load()
        err.write_bytes(original);self.source.write_bytes(b'["tampered source"]')
        with self.assertRaises(ValueError):self.load()

    def test_raw_input_digest_is_bound_to_effective_object(self):
        def change(value):
            root=Path(value['stream']['root']);directory=Path(value['command']['path']).parent
            data=json.loads(gzip.decompress((root/value['stream']['file']).read_bytes()));data['input_sha256']='e'*64
            body=json.dumps(data).encode()+b'\n';encoded=gzip.compress(body,mtime=0);(directory/'stdout.gz').write_bytes(encoded)
            status=json.loads((directory/'capture.json').read_bytes());status['streams']['stdout']={'file':'stdout.gz','gzip_sha256':era.digest(encoded),'sha256':era.digest(body),'bytes':len(body)}
            self.write(directory/'capture.json',status);value['capture']['sha256']=era.digest((directory/'capture.json').read_bytes())
            command=json.loads((directory/'command.json').read_bytes());command.update(status);self.write(directory/'command.json',command);value['command']['sha256']=era.digest((directory/'command.json').read_bytes())
            stream=era.retain(data,directory,root);stream['root']=str(root);value['stream']=stream
        self.mutate_receipt(change)
        with self.assertRaisesRegex(ValueError,'raw input/effective'):self.load()

    def test_effective_producer_or_archive_disagreement_rejected(self):
        def change(value):
            path=Path(value['command']['path']);command=json.loads(path.read_bytes())
            command['base_archive_sha256']='b'*64
            self.write(path,command);value['command']['sha256']=era.digest(path.read_bytes())
        self.mutate_receipt(change)
        with self.assertRaises(ValueError):self.load()

    def test_missing_or_corrupted_source_object_rejected(self):
        hashed=next(iter(self.sources.values()))['sha256'];path=self.root/'source-objects'/hashed
        path.write_bytes(b'corrupt')
        with self.assertRaises(ValueError):self.load()
        path.unlink()
        with self.assertRaises(FileNotFoundError):self.load()

    def test_outer_overflow_or_timeout_cannot_qualify_completed_child(self):
        for outcome in ('output_limit','timeout'):
            self.write(self.outer,{'capture_complete':False,'child_exit':0,'runner_exit':125,'outcome':outcome})
            with self.assertRaises(ValueError):self.load()

    def test_unrelated_or_failed_producer_rejected(self):
        def change(value):
            path=Path(value['capture']['path']);status=json.loads(path.read_bytes());status['child_exit']=9
            self.write(path,status);value['capture']['sha256']=era.digest(path.read_bytes())
        self.mutate_receipt(change)
        with self.assertRaises(ValueError):self.load()

    def test_forged_retained_stream_cannot_replace_raw_producer(self):
        def change(value):
            root=Path(value['stream']['root'])
            data=json.loads(gzip.decompress((root/value['stream']['file']).read_bytes()));data['input_sha256']='forged'
            stream=era.retain(data,Path(value['stream']['capture']),root);stream['root']=str(root)
            value['stream']=stream
        self.mutate_receipt(change)
        with self.assertRaises(ValueError):self.load()

    def test_receipt_symlink_and_corpus_escape_rejected(self):
        original=self.pins[0]['path'];link=self.root/'receipt-link.json';link.symlink_to(original)
        self.pins[0]['path']=str(link);self.write(self.index,self.pins)
        with self.assertRaises(ValueError):self.load()
        self.pins[0]['path']=original
        self.mutate_receipt(lambda value:value['stream'].__setitem__('file','../outside.json.gz'))
        with self.assertRaises(ValueError):self.load()


class SchedulerAuthority(unittest.TestCase):
    def test_registered_selector_rejects_bool_float_or_arbitrary_subranges(self):
        scenario,_=fixture()
        self.assertTrue(era.registered_selector(scenario,{'kind':'api','first':0,'end':7},0))
        for first,end,stop in [(False,7,0),(0.0,7,0),(0,7,False),(0,6,0),(0,7,1)]:
            self.assertFalse(era.registered_selector(scenario,{'kind':'api','first':first,'end':end},stop))

    def test_old_external_scheduler_stays_rejected(self):
        root=Path(os.environ['RUN_ARTIFACT_DIR'])
        with mock.patch.dict(os.environ,{'RUN_SCHEDULER_MANIFEST':str(root/'external.json')},clear=True):
            with self.assertRaises(ValueError):era.scheduler_path(root/'new-run')

    def test_external_scheduler_requires_exact_launch_extra_pin(self):
        root=Path(os.environ['RUN_ARTIFACT_DIR'])/('scheduler-contract-'+uuid.uuid4().hex);root.mkdir()
        scheduler=root/'scheduler.json';scheduler.write_bytes(b'{}')
        pin={'path':str(scheduler),'sha256':era.digest(scheduler.read_bytes())}
        launch=root/'launch.json';launch.write_text(json.dumps({'extra_inputs':[pin]}))
        with mock.patch.dict(os.environ,{'RUN_SCHEDULER_MANIFEST':str(scheduler),'RUSTAXA_ERA_LAUNCH_MANIFEST':str(launch)}),mock.patch.object(era,'cutover_context',return_value={'scheduler_input':pin}):
            self.assertEqual(era.scheduler_path(root/'new-run'),scheduler)
            launch.write_text(json.dumps({'extra_inputs':[]}))
            with self.assertRaises(ValueError):era.scheduler_path(root/'new-run')


if __name__=='__main__':unittest.main()
