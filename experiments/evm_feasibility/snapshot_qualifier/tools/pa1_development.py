"""Fixed40 development recovery tokens; no verifier or physical authority.

This controller reconciles the reviewed finite development reservation manifest.
It cannot initialize history, admit protocol routes, issue READY or capture. The
physical verifier keeps its independently accepted source/route contract.
"""
import os
import hashlib
import pa1_evidence as core
import evidence_records as records
from pa1_fixed40 import RecoveryCounters, BASELINE_HISTORY, _pin_tuple, _pin, graph_charge


class DevelopmentReservations(core.Counters):
    """Fixed40 development-only reservation root; no protocol/capture admission.

    The independently reviewed manifest owns the caps and exact history index.
    migrate() is forbidden on the real root until frozen-source and complete
    preflight acceptance. Tests clone the old exact root. A token retains BOTH
    its full outcome and a distinct correction/revalidation packet. Only a
    terminal, reaped process with authenticated complete outputs can settle it.
    Used read/unit/time/control charges never decrease. Unknown pending work
    blocks new admission, including after restart. This class cannot initialize
    a missing root and never supplies READY, a verifier receipt or a fact.
    """
    LIVE = {}
    SOURCE_COUNT = 96
    SOURCE_BODY = 200704
    EVENT_PARSER_MAX = 37+16777216//256
    TOKEN_PARSER_MAX = 16777216//256
    NAMES = ('setup', 'source', 'nonbinary', 'binary', 'units', 'seconds', 'control')
    HARD = (16777216,4194304,469258398,56455622958,40960,6000,344148)
    INHERITED = (15904417,3274109,33587200,0,8192,1200,0)
    STATE_FIELDS = {'schema','legacy','authority','history','events','generation',
                    'caps','used','pending','chain','closeout','issued'}

    @classmethod
    def initialize(cls, *args, **kwargs):
        raise ValueError('development must reconcile exact existing history')

    @staticmethod
    def authenticate(pin, maximum, *, decoded=False, live_bytes=0):
        """Authenticate the complete regular stream and EOF, never a projection."""
        import stat
        _pin_tuple(pin)
        core.integer(pin['bytes'], maximum, 'development evidence body cap')
        if decoded and maximum > 131072:
            raise ValueError('development decoded body bound')
        core.integer(live_bytes,core.SEMANTIC,'development authenticated caller')
        if live_bytes+(2*pin['bytes'] if decoded else 0)+2*core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('development pre-open caller lifetime')
        fd = os.open(records.canonical_path(pin['path']),os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        raw = bytearray() if decoded else None
        size = 0
        checksum = hashlib.sha256()
        try:
            if not stat.S_ISREG(os.fstat(fd).st_mode):
                raise ValueError('development regular evidence')
            while size < pin['bytes']:
                chunk = os.read(fd,min(core.CHUNK,pin['bytes']-size))
                if not chunk:
                    raise ValueError('development truncated body')
                checksum.update(chunk);size += len(chunk)
                if decoded:
                    raw.extend(chunk)
            if os.read(fd,1) or checksum.hexdigest()!=pin['sha256']:
                raise ValueError('development complete body/EOF/hash')
        finally:
            os.close(fd)
        if not decoded:return size
        core.integer(live_bytes,core.SEMANTIC,'development authenticated caller')
        if (live_bytes+2*len(raw)+128*core._wire_nodes(raw)
                +2*core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('development predecode caller lifetime')
        return records.decode(raw,maximum)


    @classmethod
    def history_page_pins(cls, history):
        """Validate the exact bounded root; return only authenticated page pins."""
        if type(history) is not dict:raise ValueError('development history root type')
        if (history.get('breach_event')!=36
                or type(history.get('effective')) is not list
                or len(history['effective'])!=3
                or history['effective'][1:]!=[0,0]):
            raise ValueError('development complete historical authority')
        core.integer(history['effective'][0],cls.EVENT_PARSER_MAX,'development history event bound')
        if history['effective'][0]<36:raise ValueError('development history floor')
        schema=history.get('schema')
        if schema=='fixed40-exact-development-history-v1':
            if 'pages' in history:raise ValueError('development ambiguous records/pages history')
            return ()
        if (schema!='fixed40-exact-development-history-v2'
                or set(history)!={'schema','admission','breach_event','durable','effective','pages'}
                or type(history['admission']) is not str or not history['admission']
                or type(history['pages']) is not list or not history['pages']):
            raise ValueError('development exact paged history root')
        _pin_tuple(history['durable'])
        if (history['durable']['bytes']!=734
                or history['durable']['sha256']!=RecoveryCounters.LEGACY_SHA):
            raise ValueError('development original durable history identity')
        next_event=9
        pins=[]
        for entry in history['pages']:
            if (type(entry) is not dict or set(entry)!={'first','last','pin'}
                    or type(entry['first']) is not int or type(entry['last']) is not int
                    or entry['first']!=next_event or entry['last']<entry['first']
                    or entry['last']>history['effective'][0]):
                raise ValueError('development ordered history page ranges')
            _pin_tuple(entry['pin'])
            core.integer(entry['pin']['bytes'],16384,'development unchanged history page cap')
            pins.append(entry['pin']);next_event=entry['last']+1
        if next_event!=history['effective'][0]+1:
            raise ValueError('development missing terminal history page')
        return tuple(pins)

    @classmethod
    def history_pages(cls, history, authenticate, *, live_bytes=0):
        """Read every complete page; preserve all original ordered row bodies.

        The caller prepays every page packet before iteration. Each yielded
        list remains live until consumed, then must be released before advance.
        This helper authenticates pages, not their original marker bodies.
        """
        pins=cls.history_page_pins(history)
        next_event=9
        entries=history['pages'] if pins else (None,)
        for entry in entries:
            page=None
            if entry is None:
                rows=history.get('records')
            else:
                page=authenticate(entry['pin'],16384,decoded=True,
                    live_bytes=live_bytes+graph_charge(history))
                if (type(page) is not dict
                        or set(page)!={'schema','records'}
                        or page['schema']!='fixed40-exact-development-history-page-v1'):
                    raise ValueError('development exact history page fields')
                rows=page['records']
            if type(rows) is not list or not rows:
                raise ValueError('development complete history page rows')
            if live_bytes+graph_charge((history,page,rows))+2*core.CHUNK+262144>core.SEMANTIC:
                raise ValueError('development history page caller lifetime')
            for row in rows:
                if (type(row) is not dict or set(row)!={'first','last','pin'}
                        or type(row['first']) is not int or type(row['last']) is not int
                        or row['first']!=next_event or row['last']<row['first']
                        or row['last']>history['effective'][0]):
                    raise ValueError('development missing/duplicate event history')
                _pin_tuple(row['pin'])
                core.integer(row['pin']['bytes'],327680,'development full history marker cap')
                next_event=row['last']+1
            if entry is not None and next_event!=entry['last']+1:
                raise ValueError('development history page row range')
            yield rows
            del rows,page
        if next_event!=history['effective'][0]+1:
            raise ValueError('development missing terminal history')

    @classmethod
    def migrate(cls, directory, *, history_pin, authority, authority_pin, caps, used,
                history_read_charge=None):
        """Reconcile all ordered9..effective records without retrospective admission.

        The caller must first authenticate independent acceptance of the exact
        authority/history/source. Bounds are the reviewed manifest values, not
        arbitrary defaults. Only the same old lock and state path are replaced.
        """
        core.sha(authority)
        config=cls.authenticate(authority_pin,16384,decoded=True)
        if (authority_pin['sha256']!=authority or config!={
                'caps':list(caps),'used':list(used),'history':history_pin,
                'profiles':config.get('profiles'),'development_only':True}):
            raise ValueError('development exact authority-bound resource ledger')
        history=cls.authenticate(history_pin,16384,decoded=True)
        page_pins=cls.history_page_pins(history)
        if page_pins:
            if not callable(history_read_charge):
                raise ValueError('development prepaid full history pages required')
            history_read_charge(page_pins)
        for rows in cls.history_pages(history,cls.authenticate,live_bytes=graph_charge(config)):
            for row in rows:cls.authenticate(row['pin'],327680)
            del rows
        if type(caps) is not tuple or type(used) is not tuple or len(caps)!=7 or len(used)!=7:
            raise ValueError('development exact resource dimensions')
        for i,(n,cap) in enumerate(zip(used,caps)):
            core.integer(cap,cls.HARD[i],'development unchanged hard limit')
            core.integer(n,cap,'development inherited resource')
            if n<cls.INHERITED[i]:raise ValueError('development inherited charge rollback')
        base=core.Counters(directory,
            '200328ad6e5a5153a31a67cd05b0be30da41b060f45712568c7548cf3e6c6a4f',
            BASELINE_HISTORY,trace_io=True)
        with base._locked():
            legacy=base._read();consumed=base._io_trace[-1]
            if (consumed['bytes']!=734 or consumed['sha256']!=RecoveryCounters.LEGACY_SHA
                    or legacy['pending'] is not None or legacy['failures']!=[8,0,0]):
                raise ValueError('development exact consumed old state')
            value=dict(schema=4,legacy=core.canonical(legacy).decode(),authority=authority,
                history=history_pin['sha256'],events=history['effective'],generation=legacy['generation']+1,
                caps=list(caps),used=list(used),pending=[],chain=core.digest(history),closeout=65536,issued=0)
            # This separate migration scope is prepaid, including full IO debit.
            value['used'][2]=core.integer(value['used'][2]+32768,caps[2],'development migration debit')
            cls._check_capacity(value)
            base._write(value)
        return base.io_summary()

    def __init__(self,directory,authority, *, authority_pin, trace_io=True,
                 history_read_charge=None):
        if trace_io is not True:
            raise ValueError('development complete IO tracing required')
        self.authority=core.sha(authority)
        self.authority_pin=authority_pin
        self.config=self.authenticate(authority_pin,16384,decoded=True)
        history=self.authenticate(self.config['history'],16384,decoded=True)
        page_pins=self.history_page_pins(history)
        if page_pins:
            if not callable(history_read_charge):
                raise ValueError('development prepaid full history pages required')
            history_read_charge(page_pins)
        # The old single root contained every row. Authenticate the same complete
        # row information across all pages on every constructor pass.
        for rows in self.history_pages(history,self.authenticate,live_bytes=graph_charge(self.config)):
            del rows
        self.inherited_events=history['effective']
        del history
        if authority_pin['sha256']!=authority:raise ValueError('development exact authority pin')
        super().__init__(directory,authority,BASELINE_HISTORY,trace_io=True)

    def _read(self):
        import stat
        self._io_check(4097)
        raw=self._read_state_raw()
        self._io_trace.append(dict(operation='read',bytes=len(raw),attempted_bytes=4097,
            sha256=hashlib.sha256(raw).hexdigest(),generation=None,authenticated=False))
        value=records.decode(raw,4096)
        if (type(value) is not dict or set(value)!=self.STATE_FIELDS or value['schema']!=4
                or value['authority']!=self.authority):raise ValueError('development state identity')
        legacy=value['legacy']
        if type(legacy) is not str or hashlib.sha256(legacy.encode()).hexdigest()!=RecoveryCounters.LEGACY_SHA:
            raise ValueError('development immutable legacy')
        core.sha(value['history']);core.sha(value['chain'])
        if (value['history']!=self.config['history']['sha256'] or value['events'][1:]!=[0,0]
                or value['events'][0]<self.inherited_events[0]):
            raise ValueError('development event rollback/physical namespace')
        # Each event retains at least one canonical256-byte record in setup.
        # The finite maximum follows from the hard byte bank, not candidates.
        core.integer(value['events'][0],self.EVENT_PARSER_MAX,'development finite event parser bound')
        core.integer(value['generation'],200000,'development generation')
        core.integer(value['issued'],self.TOKEN_PARSER_MAX,'development finite token count')
        if len(value['caps'])!=7 or len(value['used'])!=7:raise ValueError('development resource shape')
        for i,(n,cap) in enumerate(zip(value['used'],value['caps'])):
            core.integer(cap,self.HARD[i],'development hard cap identity')
            core.integer(n,cap,'development consumed debit')
            if cap!=self.config['caps'][i] or n<self.config['used'][i] or n<self.INHERITED[i]:
                raise ValueError('development immutable authority/floor')
        if value['closeout']!=65536:raise ValueError('development closeout identity')
        # Exact4KiB state also bounds pending count independently of this list.
        if type(value['pending']) is not list or len(value['pending'])>4096//256:
            raise ValueError('development bounded pending records')
        if len({row[0] for row in value['pending']})!=len(value['pending']):
            raise ValueError('development duplicate pending identity')
        for row in value['pending']:
            if type(row) is not list or len(row)!=6:raise ValueError('development pending shape')
            token,spec,normal,recovery,generation,phase=row
            if phase not in ('running','recovery-held'):raise ValueError('development pending phase')
            core.sha(token);_pin_tuple(_pin(spec))
            core.integer(generation,value['generation'],'development pending generation')
            for costs in (normal,recovery):
                if type(costs) is not list or len(costs)!=7:raise ValueError('development packet dimensions')
                for n,cap in zip(costs,value['caps']):core.integer(n,cap,'development packet bound')
        self._check_capacity(value)
        self._io_trace[-1].update(authenticated=True,generation=value['generation'])
        return value

    @staticmethod
    def _check_capacity(value):
        for i,cap in enumerate(value['caps']):
            held=sum((row[2][i]+row[3][i]) if i<2 or row[5]=='recovery-held'
                     else row[3][i] for row in value['pending'])
            closeout=value['closeout'] if i==0 else 0
            if value['used'][i]+held+closeout>cap:raise ValueError('development full reservation exhaustion')

    def _descriptor(self,pin):
        spec=self.authenticate(pin,4096,decoded=True)
        required={'identity','slot','route','source','limits','context','version','generation',
                  'normal','recovery','command','cwd','log'}
        if type(spec) is not dict or set(spec)!=required:raise ValueError('development descriptor fields')
        if (spec['slot']!=0 or spec['route']!='development' or spec['limits']!=self.authority
                or type(spec['identity']) is not str or not 0<len(spec['identity'])<=64):
            raise ValueError('development exact slot/route/limits')
        for key in ('source','context','version'):core.sha(spec[key])
        if type(spec['command']) is not list or not spec['command'] or not all(type(x) is str for x in spec['command']):
            raise ValueError('development exact command')
        records.canonical_path(spec['cwd']);records.canonical_path(spec['log'])
        for key in ('normal','recovery'):
            if type(spec[key]) is not list or len(spec[key])!=7:raise ValueError('development exact cost vector')
            for i,n in enumerate(spec[key]):core.integer(n,2**63-1,'development packet debit')
            if spec[key][4]>2048 or spec[key][5]>300 or spec[key][6]!=(0 if key=='recovery' and not any(spec[key]) else 20483):
                raise ValueError('development per-operation/IO limits')
        profile={k:spec[k] for k in ('command','cwd','normal','recovery','source','context','version')}
        if profile not in self.config['profiles']:
            raise ValueError('development command/source/packet not independently reviewed')
        if len(core.canonical(spec))<256:raise ValueError('development mandatory record minimum')
        return spec

    def reserve(self,spec_pin):
        spec=self._descriptor(spec_pin)
        with self._locked():
            value=self._read()
            if any((str(self.directory),row[0]) not in self.LIVE for row in value['pending']):
                raise ValueError('development unknown interrupted pending work')
            if spec['generation']!=value['generation']+1:raise ValueError('development generation reuse')
            token=core.digest((self.authority,spec_pin,spec['generation']))
            if any(row[0]==token for row in value['pending']):raise ValueError('development duplicate reservation')
            normal,recovery=spec['normal'],spec['recovery']
            value['pending'].append([token,list(_pin_tuple(spec_pin)),normal,recovery,spec['generation'],'running'])
            for i in range(2,7):
                # Burn normal charges at its start; keep every future recovery
                # dimension held and unavailable until claim or terminal success.
                value['used'][i]=core.integer(value['used'][i]+normal[i],
                                              value['caps'][i],'development full debit exhaustion')
            value['generation']=core.integer(spec['generation'],200000,'development generation')
            value['issued']=core.integer(value['issued']+1,self.TOKEN_PARSER_MAX,'development finite token count')
            self._check_capacity(value)
            self._write(value)  #4KiB state precheck occurs before any side effect.
            self.LIVE[(str(self.directory),token)]=None
        return token

    def launch(self,token,spec_pin):
        """Start only the exact reviewed command with a bounded pipe supervisor.

        The4096-byte pipe is set BEFORE spawn. Stop at319489 with an adaptive final read, then
        kill/reap and drain the entire pipe, retaining every produced byte up
        to327680. This is an overflow FAILURE, never a truncated successful
        gate. All process-group descendants must exit before settlement.
        """
        import subprocess,fcntl,threading,selectors,signal,time
        spec=self._descriptor(spec_pin)
        key=(str(self.directory),token)
        if key not in self.LIVE or self.LIVE[key] is not None:
            raise ValueError('development duplicate/unknown launch')
        if core.digest((self.authority,spec_pin,spec['generation']))!=token:
            raise ValueError('development wrong launch source/context')
        reader,writer=os.pipe2(os.O_CLOEXEC)
        try:
            fcntl.fcntl(reader,fcntl.F_SETPIPE_SZ,4096)
            if fcntl.fcntl(reader,fcntl.F_GETPIPE_SZ)!=4096:
                raise ValueError('development exact bounded stdout pipe')
            output=open(spec['log'],'xb')
            started=time.monotonic()
            try:
                process=subprocess.Popen(spec['command'],cwd=spec['cwd'],stdout=writer,
                    stderr=subprocess.STDOUT,start_new_session=True)
            except BaseException:
                output.close();raise
        except BaseException:
            os.close(reader);os.close(writer);raise
        os.close(writer);os.set_blocking(reader,False)
        watch=dict(process=process,timeout=False,overflow=False,done=threading.Event(),error=None)
        self.LIVE[key]=watch
        def supervise():
            selector=selectors.DefaultSelector();selector.register(reader,selectors.EVENT_READ)
            size=0;killed=False
            try:
                while True:
                    if not killed and time.monotonic()-started>=spec['normal'][5]:
                        watch['timeout']=True;killed=True
                    if killed:
                        try:os.killpg(process.pid,signal.SIGKILL)
                        except ProcessLookupError:pass
                    selector.select(0.01)
                    # Reach the original strict319488 boundary by one byte,
                    # never by an unchecked4096-byte chunk overshoot.
                    read_size=4096 if killed else min(4096,319489-size)
                    try:chunk=os.read(reader,read_size)
                    except BlockingIOError:continue
                    if not chunk:break
                    output.write(chunk);size+=len(chunk)
                    if size>=319489:
                        watch['overflow']=True;killed=True
                        try:os.killpg(process.pid,signal.SIGKILL)
                        except ProcessLookupError:pass
                try:process.wait(timeout=max(0.001,spec['normal'][5]-(time.monotonic()-started)))
                except subprocess.TimeoutExpired:
                    watch['timeout']=True
                    try:os.killpg(process.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
                    process.wait()
                output.flush();os.fsync(output.fileno())
            except BaseException as error:
                watch['error']=repr(error)
                try:os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError:pass
                process.wait()
            finally:
                selector.close();os.close(reader);output.close();watch['done'].set()
        thread=threading.Thread(target=supervise,daemon=True);watch['thread']=thread;thread.start()
        return process

    def complete(self,token,spec_pin,result_pin):
        import signal
        spec=self._descriptor(spec_pin)
        result=self.authenticate(result_pin,16384,decoded=True)
        required={'token','source','context','version','generation','exit_code','outputs','review','source_bytes','sources','timeout','overflow'}
        if (type(result) is not dict or set(result)!=required or result['token']!=token
                or any(result[k]!=spec[k] for k in ('source','context','version','generation'))):
            raise ValueError('development exact terminal source/context/generation')
        key=(str(self.directory),token);watch=self.LIVE.get(key)
        if not isinstance(watch,dict) or watch['process'].poll() is None:
            raise ValueError('development producer still live or unknown')
        process=watch['process']
        watch['thread'].join(timeout=0.1)
        if not watch['done'].is_set() or watch['error'] is not None:
            raise ValueError('development producer output not terminal')
        try:os.killpg(process.pid,0)
        except ProcessLookupError:pass
        else:raise ValueError('development late producer group still live')
        if result['timeout'] is not watch['timeout'] or result['overflow'] is not watch['overflow']:
            raise ValueError('development immutable monitor outcome mismatch')
        if result['exit_code']!=process.returncode:raise ValueError('development terminal exit mismatch')
        outputs=result['outputs']
        if type(outputs) is not list or len(outputs)>64 or len({q['path'] for q in outputs})!=len(outputs):
            raise ValueError('development complete output inventory')
        if spec['log'] not in [q['path'] for q in outputs]:raise ValueError('development missing full log')
        actual=result_pin['bytes']+spec_pin['bytes']
        for output in outputs:actual+=self.authenticate(output,327680)
        review=self.authenticate(result['review'],32768,decoded=True)
        if review!={'token':token,'source':spec['source'],'context':spec['context'],'complete_inventory':outputs,'obligations_complete':True}:
            raise ValueError('development independent exact inventory review')
        actual+=result['review']['bytes']
        sources=result['sources']
        if type(sources) is not list or len(sources)>self.SOURCE_COUNT or core.digest(sources)!=spec['source']:
            raise ValueError('development exact full source inventory')
        source_bytes=sum(self.authenticate(pin,self.SOURCE_BODY) for pin in sources)
        if source_bytes!=result['source_bytes']:
            raise ValueError('development source-byte settlement mismatch')
        core.integer(source_bytes,spec['normal'][1],'development exact full source bytes')
        failed=process.returncode!=0 or watch['timeout'] or watch['overflow']
        with self._locked():
            value=self._read()
            rows=[row for row in value['pending'] if row[0]==token]
            if len(rows)!=1 or rows[0][1]!=list(_pin_tuple(spec_pin)):
                raise ValueError('development exactly-once terminal token')
            if actual>spec['normal'][0]:raise ValueError('development full output overrun')
            # Success closes only this operation's proven-unused output reserve.
            # Failure transfers its recovery packet; no other producer is touched.
            row=rows[0]
            if not failed:
                value['pending'].remove(row)
            elif not any(row[3]):
                value['pending'].remove(row)
            else:
                row[2]=row[3];row[3]=[0]*7
                row[0]=core.digest((token,'reserved-recovery'));row[5]='recovery-held'
                self.LIVE[(str(self.directory),row[0])]=None
            value['used'][0]+=actual;value['used'][1]+=result['source_bytes']
            if failed:value['events'][0]+=1
            value['generation']=core.integer(value['generation']+1,200000,'development generation')
            value['chain']=core.digest((value['chain'],result_pin,result['review']))
            self._check_capacity(value)
            self._write(value)
            del self.LIVE[key]
        return self.io_summary()

    def claim_recovery(self,failed_token,spec_pin):
        """Bind a named correction to only its already prepaid recovery packet.

        Held recovery is consumed once at its start; no prior charge is refunded. The new descriptor must fit every
        reserved dimension; new recovery beyond that packet needs a separate
        fully reserved operation. This is not retry/capture authority.
        """
        spec=self._descriptor(spec_pin)
        old_token=core.digest((failed_token,'reserved-recovery'))
        with self._locked():
            value=self._read()
            if any((str(self.directory),row[0]) not in self.LIVE for row in value['pending']):
                raise ValueError('development unknown interrupted pending recovery')
            rows=[row for row in value['pending'] if row[0]==old_token]
            if (len(rows)!=1 or (str(self.directory),old_token) not in self.LIVE
                    or spec['generation']!=value['generation']+1):
                raise ValueError('development unknown recovery/generation')
            row=rows[0]
            if row[5]!='recovery-held':raise ValueError('development recovery not held')
            if any(spec['normal'][i]+spec['recovery'][i]>row[2][i] for i in range(7)):
                raise ValueError('development recovery exceeds its own reservation')
            token=core.digest((self.authority,spec_pin,spec['generation']))
            row[:]=[token,list(_pin_tuple(spec_pin)),spec['normal'],spec['recovery'],spec['generation'],'running']
            for i in range(2,7):
                value['used'][i]=core.integer(value['used'][i]+spec['normal'][i],
                    value['caps'][i],'development recovery debit exhaustion')
            value['generation']=core.integer(spec['generation'],200000,'development recovery generation')
            self._check_capacity(value);self._write(value)
            del self.LIVE[(str(self.directory),old_token)]
            self.LIVE[(str(self.directory),token)]=None
        return token

    def admit(self,**kwargs):
        raise ValueError('development reservations cannot admit protocol/physical routes')

    def finish(self,*args,**kwargs):
        raise ValueError('development reservations require exact terminal token settlement')


class CoherentDevelopmentReservations(DevelopmentReservations):
    """Pinned finite engineering history; never a runtime working-limit grant.

    Complete old consumed/held resource prefixes remain in this ledger. New
    source versions and engineering work use the independently accepted pool.
    RuntimeAdmission keeps the runtime legacy floors and prescribed runtime
    reads separate. This class does not grant a binary read or physical launch.
    """
    ROOT = '/home/fry/artifacts/vrf40-development-20261018'
    PLAN_PIN = dict(path=ROOT+'/development-successor-v3.json', bytes=52492,
        sha256='e59a662ef716c613ab29efc7d1f53c9c75df1d891374c6d6af1ac82340076d6c')
    REVIEW_PINS = (
        dict(path=ROOT+'/development-accounting-v3-final.json', bytes=4582,
             sha256='548210c26035520797dd390e4eac40351414afda95d9caebeb96e933d832bd05'),
        dict(path=ROOT+'/development-boundary-v3-final.json', bytes=2815,
             sha256='374ee8ed6e631963c535261642de60859f243a79bffd43b0416074c5b3fffc4c'),
        dict(path=ROOT+'/development-v3-exact-byte-confirmation.json', bytes=1769,
             sha256='569b6644c1e2dafc436f9b5cc2fd44edad942ce3160bf0e45ec2887983fc488b'),
    )
    NAMES = ('retention','source_history','read','binary','units','seconds','control')
    HARD = (3167859430,2634015626,485640248369,446954195380,
            1912286,282720,27693086)
    INHERITED = (67108864,4187018,38020526127+6422528,425374501202,
                 196062+4096,30300+1620,299038)
    SOURCE_COUNT = 101
    SOURCE_BODY = 1048576
    EVENT_PARSER_MAX = 106+HARD[6]//20483
    TOKEN_PARSER_MAX = HARD[6]//20483

    @classmethod
    def check_plan(cls, value):
        """Check the exact full manifest, not caller-selected larger limits."""
        expected = dict(storage=cls.HARD[0],source_history=cls.HARD[1],
            read=cls.HARD[2],units=cls.HARD[4],seconds=cls.HARD[5],
            development_control=cls.HARD[6],new_source_images=627)
        if (value.get('schema')!='fixed40-finite-development-successor-v3'
                or value.get('development_caps')!=expected
                or value.get('obligation_count')!=209):
            raise ValueError('development exact finite successor caps')
        runtime=value.get('runtime_unchanged',{})
        required=dict(setup=16777216,nonbinary_lifecycle=38020585776,
            binary_lifecycle=449080998528,current_source=4194304,
            per_source=1048576,allocation=2097152,framing=2097152,
            saved_gate_seconds=300,single_fixed40_capture=1,
            additional_physical_attempts=0,protected_future37_32=True,
            no37_32_capture_this_round=True)
        if runtime!=required:
            raise ValueError('development cannot substitute runtime working limits')

    @classmethod
    def authenticate_successor(cls, *, read_charge):
        """Charge the exact full read packet before opening any body.

        read_charge is the source-frozen caller's reservation consumer, not a
        receipt that replaces authentication. RuntimeAdmission binds it to its
        prepaid constructor packet; engineering callers retain its full debit.
        """
        packet=(dict(cls.PLAN_PIN),*(dict(p) for p in cls.REVIEW_PINS))
        read_charge(packet)
        value=cls.authenticate(cls.PLAN_PIN,131072,decoded=True)
        cls.check_plan(value)
        for pin in cls.REVIEW_PINS:
            cls.authenticate(pin,32768)
        return value

    @classmethod
    def migrate(cls, *args, read_charge, **kwargs):
        cls.authenticate_successor(read_charge=read_charge)
        read_charge((dict(kwargs['history_pin']),))
        history=cls.authenticate(kwargs['history_pin'],16384,decoded=True)
        if history.get('effective',[0])[0]<106:
            raise ValueError('development successor event floor rollback')
        page_pins=cls.history_page_pins(history)
        if page_pins:read_charge(page_pins)
        # The original first root pass contained all rows. Preserve that full
        # information now spread over pages before the separate base pass.
        for rows in cls.history_pages(history,cls.authenticate):
            del rows
        # Base migration rereads the full root. Preserve both actual passes.
        read_charge((dict(kwargs['history_pin']),))
        return super().migrate(*args,history_read_charge=read_charge,**kwargs)

    def __init__(self, *args, read_charge, **kwargs):
        self.authenticate_successor(read_charge=read_charge)
        super().__init__(*args,history_read_charge=read_charge,**kwargs)


    def _read(self):
        value=super()._read()
        self._io_trace[-1]['authenticated']=False
        if value['events'][0]<106 or self.inherited_events[0]<106:
            raise ValueError('development successor event floor rollback')
        self._io_trace[-1]['authenticated']=True
        return value
