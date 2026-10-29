"""Fixed40 development token regressions on exact cloned legacy state."""
import unittest
import hashlib
import json
import tempfile
import os
from pathlib import Path
import pa1_fixed40 as fixed
import pa1_development as development

class DevelopmentReservationTests(unittest.TestCase):
    def setUp(self):
        import subprocess
        self.subprocess=subprocess
        root=Path(os.environ.get('DEVELOPMENT_TEST_ARTIFACT_ROOT',
            '/home/fry/artifacts/vrf40-development-20261018/test-fixtures'))
        root.mkdir(parents=True,exist_ok=True)
        self.directory=Path(tempfile.mkdtemp(dir=root,prefix=self._testMethodName+'-'))
        (self.directory/'NONHISTORICAL.json').write_text(json.dumps(dict(
            scope='Synthetic unit-test clone only; never original history or runtime authority',
            test=self._testMethodName)))
        (self.directory/'lock').touch()
        (self.directory/'state.json').write_bytes((fixed.ROOT/'offline-fit-counters/state.json').read_bytes())
        self.history=Path('/home/fry/artifacts/vrf40-admission-01a14b66-20261018/history-index.json')
        self.caps=development.DevelopmentReservations.HARD
        self.used=(15904417,3274109,33587200,0,8192,1200,0)
        costs=[8192,0,131072,0,16,2,20483]
        profiles=[]
        for code,normal,recovery in [
            ('pass',costs,costs),
            ('import os,time; os.close(1); os.close(2); time.sleep(2)',
             [8192,0,131072,0,16,1,20483],costs),
            ('pass',costs,[0]*7),
            ('import time; time.sleep(0.4)',costs,costs),
            ('import time; time.sleep(0.5)',costs,costs),
            ('pass',[16777216,0,131072,0,16,2,20483],costs),
            ('import time; time.sleep(2)',[8192,0,131072,0,16,1,20483],costs),
            ('print("x"*400000)',[376832,0,524288,0,16,2,20483],
             [376832,0,524288,0,16,2,20483]),
            ('import os; os.write(1,b"x"*319488)',[376832,0,524288,0,16,2,20483],
             [376832,0,524288,0,16,2,20483]),
            ('import os; os.write(1,b"x"*319489)',[376832,0,524288,0,16,2,20483],
             [376832,0,524288,0,16,2,20483])]:
            profiles.append(dict(command=['python3','-c',code],cwd=str(self.directory),
                normal=normal,recovery=recovery,source=fixed.core.digest([]),context='b'*64,version='c'*64))
        config=dict(caps=list(self.caps),used=list(self.used),history=self.pin(self.history),
            profiles=profiles,development_only=True)
        path=self.directory/'reviewed-authority.json';path.write_bytes(fixed.core.canonical(config))
        self.authority_pin=self.pin(path);self.authority=self.authority_pin['sha256']
        self.migrate()
        self.serial=0;self.processes=[]
        self.addCleanup(self.clean_processes)

    def clean_processes(self):
        import os,signal
        for process in self.processes:
            if process.poll() is None:
                os.killpg(process.pid,signal.SIGKILL);process.wait()
        for key in list(development.DevelopmentReservations.LIVE):
            if key[0]==str(self.directory):del development.DevelopmentReservations.LIVE[key]

    @staticmethod
    def pin(path):
        raw=Path(path).read_bytes()
        return dict(path=str(path),bytes=len(raw),sha256=hashlib.sha256(raw).hexdigest())

    def migrate(self,history=None):
        return development.DevelopmentReservations.migrate(self.directory,
            history_pin=self.pin(history or self.history),authority=self.authority,authority_pin=self.authority_pin,
            caps=self.caps,used=self.used)

    def state(self):return json.loads((self.directory/'state.json').read_bytes())

    def book(self):return development.DevelopmentReservations(self.directory,self.authority,authority_pin=self.authority_pin)

    def spec(self,code='pass',**changes):
        self.serial+=1
        costs=[8192,0,131072,0,16,2,20483]
        value=dict(identity='fixture-'+str(self.serial),slot=0,route='development',
            source=fixed.core.digest([]),limits=self.authority,context='b'*64,version='c'*64,
            generation=self.state()['generation']+1,normal=costs,recovery=costs,
            command=['python3','-c',code],cwd=str(self.directory),
            log=str(self.directory/f'log-{self.serial}'))
        value.update(changes)
        path=self.directory/f'spec-{self.serial}.json';path.write_bytes(fixed.core.canonical(value))
        return self.pin(path),value

    def launch(self,book,pin):
        token=book.reserve(pin);process=book.launch(token,pin);self.processes.append(process)
        return token,process

    def result(self,token,spec,process,**changes):
        inventory=[self.pin(spec['log'])]
        review=dict(token=token,source=spec['source'],context=spec['context'],complete_inventory=inventory,obligations_complete=True)
        rp=self.directory/f'review-{token}.json';rp.write_bytes(fixed.core.canonical(review))
        result=dict(token=token,source=spec['source'],context=spec['context'],version=spec['version'],
            generation=spec['generation'],exit_code=process.poll(),outputs=inventory,
            review=self.pin(rp),source_bytes=0,sources=[],
            timeout=development.DevelopmentReservations.LIVE[(str(self.directory),token)]['timeout'],
            overflow=development.DevelopmentReservations.LIVE[(str(self.directory),token)]['overflow'])
        result.update(changes)
        path=self.directory/f'result-{token}.json';path.write_bytes(fixed.core.canonical(result))
        return self.pin(path)

    def test_two_inflight_and_correction_keep_late_timeout_full_packet(self):
        import os,signal
        first=self.book();p1,s1=self.spec('import time; time.sleep(0.4)')
        t1,c1=self.launch(first,p1)
        second=self.book();p2,s2=self.spec('import time; time.sleep(0.5)')
        t2,c2=self.launch(second,p2)
        correction=self.book();pc,sc=self.spec();tc,cc=self.launch(correction,pc);cc.wait(timeout=2)
        correction.complete(tc,pc,self.result(tc,sc,cc))
        state=self.state();self.assertEqual(len(state['pending']),2)
        self.assertEqual({row[0] for row in state['pending']},{t1,t2})
        # The late timeout occurs AFTER correction consumed its own resources.
        os.killpg(c1.pid,signal.SIGKILL);c1.wait(timeout=2)
        first.complete(t1,p1,self.result(t1,s1,c1))
        state=self.state();self.assertEqual(state['events'],[38,0,0])
        recovery=next(row for row in state['pending'] if row[0]==fixed.core.digest((t1,'reserved-recovery')))
        self.assertEqual(recovery[2],s1['recovery']);self.assertEqual(recovery[3],[0]*7)
        still=next(row for row in state['pending'] if row[0]==t2)
        self.assertEqual(still[2],s2['normal']);self.assertEqual(still[3],s2['recovery'])
        c2.wait(timeout=2);second.complete(t2,p2,self.result(t2,s2,c2))
        self.assertLessEqual(first.io_summary()['attempted_bytes'],20483)
        self.assertEqual(self.state()['legacy'],fixed.core.canonical(json.loads((fixed.ROOT/'offline-fit-counters/state.json').read_bytes())).decode())
        burned=self.state()['used'][2:]
        recovery_book=self.book();pr,sr=self.spec(recovery=[0]*7)
        tr=recovery_book.claim_recovery(t1,pr);cr=recovery_book.launch(tr,pr);self.processes.append(cr)
        cr.wait(timeout=2);recovery_book.complete(tr,pr,self.result(tr,sr,cr))
        self.assertEqual(self.state()['pending'],[])
        self.assertEqual(self.state()['used'][2:],
            [n+s1['recovery'][i+2] for i,n in enumerate(burned)])


    def test_exhaustion_before_launch_and_closeout_stays_reserved(self):
        book=self.book();pin,spec=self.spec(normal=[16777216,0,131072,0,16,2,20483])
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'exhaustion'):book.reserve(pin)
        self.assertFalse(Path(spec['log']).exists())
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        self.assertEqual(self.state()['closeout'],65536)

    def test_duplicate_completion_and_generation_reuse_do_not_write(self):
        book=self.book();pin,spec=self.spec();token,process=self.launch(book,pin);process.wait(timeout=2)
        result=self.result(token,spec,process);book.complete(token,pin,result)
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaises(ValueError):book.complete(token,pin,result)
        fresh=self.book()
        with self.assertRaisesRegex(ValueError,'generation reuse'):fresh.reserve(pin)
        self.assertEqual((self.directory/'state.json').read_bytes(),before)

    def test_restart_unknown_pending_retains_both_packets(self):
        book=self.book();pin,spec=self.spec();token=book.reserve(pin)
        del development.DevelopmentReservations.LIVE[(str(self.directory),token)]
        fresh=self.book();other,_=self.spec();before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'interrupted'):fresh.reserve(other)
        self.assertEqual((self.directory/'state.json').read_bytes(),before)

    def test_wrong_slot_source_context_and_live_terminal_reject(self):
        book=self.book();bad,_=self.spec(slot=1)
        with self.assertRaisesRegex(ValueError,'slot/route'):book.reserve(bad)
        pin,spec=self.spec('import time; time.sleep(0.5)');token,process=self.launch(book,pin)
        before=(self.directory/'state.json').read_bytes()
        result=self.result(token,spec,process,context='d'*64)
        with self.assertRaisesRegex(ValueError,'source/context'):book.complete(token,pin,result)
        result=self.result(token,spec,process)
        with self.assertRaisesRegex(ValueError,'still live'):book.complete(token,pin,result)
        altered=dict(spec,source='e'*64);path=self.directory/'wrong-source.json';path.write_bytes(fixed.core.canonical(altered))
        other=self.book()
        with self.assertRaisesRegex(ValueError,'not independently reviewed'):other.launch(token,self.pin(path))
        self.assertEqual((self.directory/'state.json').read_bytes(),before)

    def test_atomic_concurrent_same_generation_only_one_reservation(self):
        from concurrent.futures import ThreadPoolExecutor
        p1,_=self.spec();p2,_=self.spec();books=[self.book(),self.book()]
        def attempt(index):
            try:return books[index].reserve([p1,p2][index])
            except ValueError:return None
        with ThreadPoolExecutor(max_workers=2) as pool:results=list(pool.map(attempt,[0,1]))
        self.assertEqual(sum(value is not None for value in results),1)
        self.assertEqual(len(self.state()['pending']),1)

    def test_missing_history_and_changed_legacy_fail_before_write(self):
        # Fresh clone; remove one full intervening event, never substitute8for36.
        old=(fixed.ROOT/'offline-fit-counters/state.json').read_bytes()
        (self.directory/'state.json').write_bytes(old)
        history=json.loads(self.history.read_bytes());history['records'].pop(0)
        path=self.directory/'missing-history.json';path.write_bytes(fixed.core.canonical(history))
        config=json.loads(Path(self.authority_pin['path']).read_bytes());config['history']=self.pin(path)
        ap=self.directory/'missing-history-authority.json';ap.write_bytes(fixed.core.canonical(config))
        original=(self.authority,self.authority_pin)
        self.authority_pin=self.pin(ap);self.authority=self.authority_pin['sha256']
        with self.assertRaisesRegex(ValueError,'missing/duplicate'):self.migrate(path)
        self.authority,self.authority_pin=original
        self.assertEqual((self.directory/'state.json').read_bytes(),old)
        (self.directory/'state.json').write_bytes(old+b' ')
        with self.assertRaisesRegex(ValueError,'consumed old'):self.migrate()
        self.assertEqual((self.directory/'state.json').read_bytes(),old+b' ')

    def check_exact_output_boundary(self,size,overflow):
        costs=[376832,0,524288,0,16,2,20483]
        book=self.book()
        pin,spec=self.spec('import os; os.write(1,b"x"*'+str(size)+')',
                           normal=costs,recovery=costs)
        token,process=self.launch(book,pin)
        process.wait(timeout=3)
        watch=development.DevelopmentReservations.LIVE[(str(self.directory),token)]
        watch['thread'].join(timeout=1)
        self.assertTrue(watch['done'].is_set())
        self.assertIsNone(watch['error'])
        self.assertEqual(watch['overflow'],overflow)
        self.assertFalse(watch['timeout'])
        self.assertEqual(Path(spec['log']).read_bytes(),b'x'*size)
        book.complete(token,pin,self.result(token,spec,process))
        self.assertEqual(bool(self.state()['pending']),overflow)

    def test_exact_output_below_stop_boundary_is_complete(self):
        self.check_exact_output_boundary(319488,False)

    def test_exact_first_overflow_byte_is_retained(self):
        self.check_exact_output_boundary(319489,True)

    def test_producer_deadline_and_complete_overflow_body_are_enforced(self):
        for code,costs in [
                ('import time; time.sleep(2)',[8192,0,131072,0,16,1,20483]),
                ('print("x"*400000)',[376832,0,524288,0,16,2,20483])]:
            book=self.book()
            recovery=costs if costs[0]==376832 else [8192,0,131072,0,16,2,20483]
            pin,spec=self.spec(code,normal=costs,recovery=recovery)
            token,process=self.launch(book,pin)
            process.wait(timeout=3)
            watch=development.DevelopmentReservations.LIVE[(str(self.directory),token)]
            watch['thread'].join(timeout=1)
            self.assertTrue(watch['done'].is_set());self.assertIsNone(watch['error'])
            self.assertTrue(watch['timeout'] if code.startswith('import') else watch['overflow'])
            self.assertLessEqual(Path(spec['log']).stat().st_size,327680)
            if code.startswith('print'):
                self.assertGreater(Path(spec['log']).stat().st_size,319488)
            # Failure retains its recovery reservation; a different next full
            # operation may not consume it. Use a fresh fixture for the pair.
            book.complete(token,pin,self.result(token,spec,process))
            self.assertTrue(self.state()['pending'])
            if code.startswith('import'):
                self.clean_processes()
                (self.directory/'state.json').write_bytes((fixed.ROOT/'offline-fit-counters/state.json').read_bytes())
                self.migrate()

    def test_authority_floor_packet_and_source_settlement_cannot_be_forged(self):
        old=(fixed.ROOT/'offline-fit-counters/state.json').read_bytes()
        (self.directory/'state.json').write_bytes(old)
        config=json.loads(Path(self.authority_pin['path']).read_bytes());config['used']=[0]*7
        ap=self.directory/'zero-floor.json';ap.write_bytes(fixed.core.canonical(config));pin=self.pin(ap)
        with self.assertRaisesRegex(ValueError,'charge rollback'):
            development.DevelopmentReservations.migrate(self.directory,history_pin=self.pin(self.history),
                authority=pin['sha256'],authority_pin=pin,caps=self.caps,used=(0,)*7)
        self.assertEqual((self.directory/'state.json').read_bytes(),old)
        self.migrate();book=self.book();bad,spec=self.spec(normal=[0,0,0,0,0,0,20483])
        with self.assertRaisesRegex(ValueError,'not independently reviewed'):book.reserve(bad)
        self.assertFalse(Path(spec['log']).exists())
        pin,spec=self.spec();token,process=self.launch(book,pin);process.wait(timeout=2)
        result=self.result(token,spec,process,source_bytes=1)
        before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'source-byte'):book.complete(token,pin,result)
        self.assertEqual((self.directory/'state.json').read_bytes(),before)

    def test_stdout_eof_does_not_waive_deadline_and_event_floor(self):
        book=self.book();pin,spec=self.spec(
            'import os,time; os.close(1); os.close(2); time.sleep(2)',
            normal=[8192,0,131072,0,16,1,20483])
        token,process=self.launch(book,pin);process.wait(timeout=3)
        watch=development.DevelopmentReservations.LIVE[(str(self.directory),token)]
        watch['thread'].join(timeout=1);self.assertTrue(watch['timeout'])
        result=self.result(token,spec,process,timeout=False)
        with self.assertRaisesRegex(ValueError,'monitor outcome'):book.complete(token,pin,result)
        book.complete(token,pin,self.result(token,spec,process))
        state=self.state();state['events'][0]=36
        (self.directory/'state.json').write_bytes(fixed.core.canonical(state))
        with self.assertRaisesRegex(ValueError,'event rollback'):self.book()

    def test_held_recovery_blocks_other_work_and_only_unused_capacity_closes(self):
        old=(fixed.ROOT/'offline-fit-counters/state.json').read_bytes()
        (self.directory/'state.json').write_bytes(old)
        config=json.loads(Path(self.authority_pin['path']).read_bytes())
        config['caps'][2]=self.used[2]+32768+3*131072
        path=self.directory/'tight-read-bank.json';path.write_bytes(fixed.core.canonical(config))
        self.authority_pin=self.pin(path);self.authority=self.authority_pin['sha256']
        self.caps=tuple(config['caps']);self.migrate()
        first=self.book();pin,spec=self.spec();token,process=self.launch(first,pin)
        after=self.state();self.assertEqual(after['used'][2],self.used[2]+32768+131072)
        second=self.book();other,unused=self.spec();before=(self.directory/'state.json').read_bytes()
        with self.assertRaisesRegex(ValueError,'reservation exhaustion'):second.reserve(other)
        self.assertEqual((self.directory/'state.json').read_bytes(),before)
        self.assertFalse(Path(unused['log']).exists())
        process.wait(timeout=2);first.complete(token,pin,self.result(token,spec,process))
        self.assertEqual(self.state()['used'][2],after['used'][2])
        self.assertEqual(self.state()['pending'],[])
        # Success closed unused recovery but did not credit any consumed read.
        third=self.book();next_pin,_=self.spec();third.reserve(next_pin)
        self.assertEqual(self.state()['used'][2],after['used'][2]+131072)

    def test_state_and_numeric_boundaries_fail_closed(self):
        state=self.state();state['legacy']+='x'*4096
        (self.directory/'state.json').write_bytes(fixed.core.canonical(state))
        with self.assertRaises(ValueError):self.book()
        self.assertGreater(len((self.directory/'state.json').read_bytes()),4096)
        self.assertEqual(development.DevelopmentReservations.HARD[0:2],(16777216,4194304))



if __name__ == '__main__':
    unittest.main()
