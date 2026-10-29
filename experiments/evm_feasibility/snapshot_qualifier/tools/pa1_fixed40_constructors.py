"""Fixed40 concrete output constructors; no acceptance or process launch.

These constructors consume actual completed results. Reserved future outputs
remain unaccepted until their real producer and semantic consumer run.
"""
import math
import re
import hashlib
from datetime import datetime, timezone
import json
import pa1_evidence as core
import pa1_fixed40 as fixed
import evidence_records as records


def gate_result(*, command, cwd, exit_code, finished_utc, seconds, log):
    """Encode a completed bounded command with its full authenticated log pin."""
    if (type(command) is not list or not command
            or any(type(x) is not str or not x for x in command)
            or type(exit_code) is not int or type(finished_utc) is not str
            or type(seconds) not in (int,float) or not math.isfinite(seconds)
            or not 0<=seconds<=300):
        raise ValueError('fixed40 exact completed gate result')
    records.canonical_path(cwd)
    fixed._pin_tuple(log)
    core.integer(log['bytes'],327680,'fixed40 complete gate log bound')
    value=dict(command=list(command),cwd=cwd,exit_code=exit_code,
               finished_utc=finished_utc,seconds=seconds,log=dict(log))
    raw=core.canonical(value)
    if len(raw)>16384:raise ValueError('fixed40 gate result serialization')
    return raw


def source_freeze(sources, *, tools=False):
    """Encode the exact ordered full current101-source closure as90+11 rows."""
    expected=11 if tools else 90
    if type(sources) is not list or len(sources)!=expected:
        raise ValueError('fixed40 complete ordered source roster')
    if len({x['path'] for x in sources})!=len(sources):
        raise ValueError('fixed40 duplicate source path')
    for pin in sources:
        fixed._pin_tuple(pin)
        core.integer(pin['bytes'],records.BODY_CAP,'fixed40 individual source')
    raw=core.canonical(sources)
    if len(raw)>(16384 if tools else 131072):
        raise ValueError('fixed40 complete source roster serialization')
    return raw


# Closed fixed40 role IDs and immutable V5 maxima. The original mapping is
# reauthenticated by the compiler and every prescribed route; this table grants
# no authority to omit that read or any complete body.
ROLE_MAXIMA = (16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 16384, 327680, 131072, 65536, 65536, 327680, 16384, 131072, 16384, 131072, 65536, 131072, 131072, 16384, 131072, 131072, 41550, 4670, 1341, 2606, 16384, 327680, 131072, 131072, 36042, 92897, 131072, 1048576, 65536, 65536, 65536, 131072, 65536, 16384, 131072, 16384, 16384, 16384, 16384, 65536, 16384, 65536, 1048576, 65536, 16384, 65536, 131072, 16384, 65536, 131072, 131072, 16384, 131072, 131072, 16384)
VALIDATION_RESULTS = tuple(range(0,28,2))
LOG_ROLES = tuple(range(1,28,2))+(31,47,65,67,71,74)
REFERENCE_RECORD_ROLES = (28,37,49)


def _role_maximum(role, maxima):
    if (type(role) is not int or not 0<=role<81
            or type(maxima) is not tuple or maxima!=ROLE_MAXIMA):
        raise ValueError('fixed40 exact role and full81 maximum vector')
    return maxima[role]


def complete_log(role, raw, *, maxima):
    """Retain a complete producer log unchanged, including non-UTF8 bytes.

    Protocol reports still use their prescribed decoder. A byte log has no
    passing verdict; consumers must inspect the real command result and full
    required test lines. This function neither truncates nor publishes.
    """
    maximum=_role_maximum(role,maxima)
    if role not in LOG_ROLES or type(raw) is not bytes or len(raw)>maximum:
        raise ValueError('fixed40 full role-bound log')
    return raw


def validation_result(role, *, maxima, source_snapshot, command_record, live_bytes, **completed):
    """Construct a fresh gate with exact pre-run source and command evidence.

    Reused gates retain their exact accepted original body and separate scope
    proof. They cannot be relabelled as fresh by this constructor.
    """
    maximum=_role_maximum(role,maxima)
    if role not in VALIDATION_RESULTS:
        raise ValueError('fixed40 validation result role')
    fixed._pin_tuple(source_snapshot);fixed._pin_tuple(command_record)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 gate constructor caller')
    if (live_bytes+fixed.graph_charge((source_snapshot,command_record,completed))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 gate constructor caller lifetime')
    value=json.loads(gate_result(**completed))
    value.update(role=role,source_snapshot=dict(source_snapshot),
                 command_record=dict(command_record),
                 cache_status='FRESH_EXECUTION',cache_provenance=None)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete validation result cap')
    return raw


def acceptance_record(role, *, maxima, compiled, destination, findings,
                      unit, declarations, identities, live_bytes):
    """Build the complete wrapper through the prepaid original core unit.

    The existing approved physical aliases remain explicit. Every original
    contract/child stream is consumed, with inode checks and a charged parse.
    No direct path read or receipt substitutes for an original body.
    """
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    maximum=_role_maximum(role,maxima)
    if role not in REFERENCE_RECORD_ROLES or type(findings) is not str:
        raise ValueError('fixed40 complete acceptance wrapper role')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 record constructor caller')
    placement=compiled['context']['reference_placements'][
        REFERENCE_RECORD_ROLES.index(role)]
    if role==28 and not findings.startswith(compiled['context']['source28_prefix']):
        raise ValueError('fixed40 mandatory full source28 findings prefix')
    expected=placement['references'];contract=placement['common_contract']
    children=[dict(role='trusted-contract',**contract),*expected]
    if len(declarations)!=len(children):
        raise ValueError('fixed40 complete constructor child declarations')
    for child,declaration in zip(children,declarations):
        occurrence,physical,kind,mode=declaration
        logical={k:child[k] for k in ('path','bytes','sha256')}
        if mode!='bytes' or kind not in ('source','evidence'):
            raise ValueError('fixed40 original constructor stream mode')
        if tuple(physical)!=fixed._pin_tuple(logical):
            aliases=[a for a in compiled['aliases']
                if a['occurrence']==occurrence and a['logical']==logical
                and fixed._pin_tuple(a['physical'])==tuple(physical)]
            if len(aliases)!=1:
                raise ValueError('fixed40 exact accepted constructor physical alias')
    # Reserve the complete future wrapper/parse before allocating its graph.
    if (owner+live_bytes+fixed.graph_charge((placement,findings,children,declarations))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 wrapper constructor caller lifetime')
    value=dict(schema=records.SCHEMA,destination=destination,ledger='validation',
        contract=contract,references=expected,findings=findings,accepted=False)
    records.bounded_shape(value)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 full reference wrapper cap')
    del value
    if (owner+live_bytes+fixed.graph_charge((raw,placement,children,declarations))
            +2*len(raw)+128*core._wire_nodes(raw)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 wrapper constructor predecode lifetime')
    operation=1024+role
    if unit.invocation.operations[operation]>=8:
        raise ValueError('fixed40 ninth wrapper constructor parse')
    unit.invocation.operations[operation]+=1
    value=records.decode(raw)
    position=0
    original=records.authenticate
    def consume(row,identities=None):
        nonlocal position
        if position>=len(children) or row!=children[position]:
            raise ValueError('fixed40 constructor original ordered child semantics')
        occurrence,physical,kind,mode=declarations[position]
        unit.consume(occurrence,fixed._pin(physical),kind=kind,
                     live_bytes=live_bytes+fixed.graph_charge((raw,value)))
        identity=unit.last_file_identity;location=physical[0]
        if identity in seen and seen[identity]!=location:
            raise ValueError('fixed40 constructor original inode/path alias')
        seen[identity]=location;position+=1
        return row['bytes']
    seen=identities
    records.authenticate=consume
    try:records.verify(value,destination,contract,expected,identities=identities)
    finally:records.authenticate=original
    if position!=len(children):
        raise ValueError('fixed40 complete constructor child semantics')
    return raw



def current_source_role(role, all_sources, *, maxima, live_bytes):
    """Build the exact90+11 source roles from one complete101-pin closure."""
    maximum=_role_maximum(role,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 source constructor caller')
    if role not in (38,39) or type(all_sources) is not list or len(all_sources)!=101:
        raise ValueError('fixed40 complete101 source role')
    if len({x['path'] for x in all_sources})!=101:
        raise ValueError('fixed40 duplicate current source')
    for p in all_sources:
        fixed._pin_tuple(p)
        core.integer(p['bytes'],records.BODY_CAP,'fixed40 current individual source')
    if sum(x['bytes'] for x in all_sources)>core.SOURCE:
        raise ValueError('fixed40 current source aggregate')
    if (live_bytes+fixed.graph_charge(all_sources)+3*maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 source constructor caller lifetime')
    raw=source_freeze(all_sources[90:] if role==39 else all_sources[:90],tools=role==39)
    if len(raw)>maximum:raise ValueError('fixed40 source role cap')
    return raw


def constructed_role_coverage():
    """Report implemented constructors; never call partial coverage complete."""
    return tuple(sorted((*VALIDATION_RESULTS,*LOG_ROLES,
                         *REFERENCE_RECORD_ROLES,29,30,32,33,34,35,36,38,39,40,41,42,43,44,45,46,48,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,66,68,69,70,72,73,75,76,77,78,79,80)))


def candidate_pins(*, maxima, candidate, full_tuple_map, map_acceptance,
                   prior_bundle, prior_report, source_freeze, live_bytes):
    """Bind every complete candidate/context prerequisite without acceptance."""
    maximum=_role_maximum(43,maxima)
    values=(candidate,full_tuple_map,map_acceptance,prior_bundle,prior_report,source_freeze)
    for value in values:fixed._pin_tuple(value)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 candidate pin caller')
    if live_bytes+fixed.graph_charge(values)+3*maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 candidate pin caller lifetime')
    value=dict(schema=1,candidate=dict(candidate),map=dict(full_tuple_map),
        map_acceptance=dict(map_acceptance),prior_bundle=dict(prior_bundle),
        prior_report=dict(prior_report),source_freeze=dict(source_freeze),accepted=False)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete candidate pin constructor')
    return raw


def selected40_candidate(*, maxima, slot, mapped_rows, prior_report,
                         map_pin, map_acceptance, prior_bundle, prior_report_pin,
                         accepted_seal, accepted_review, live_bytes):
    """Construct the full fixed40 candidate from authenticated caller inputs.

    Callers must retain the full map/report read and parse charges. This pure
    constructor grants no read waiver or source/fact acceptance. All40 map
    bindings survive directly. The original full20 first-D reference contract
    remains mandatory, including its full body reads and exact reconstruction.
    """
    maximum=_role_maximum(42,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 candidate caller')
    pins=(map_pin,map_acceptance,prior_bundle,prior_report_pin,accepted_seal,accepted_review)
    for pin in pins:fixed._pin_tuple(pin)
    if (live_bytes+fixed.graph_charge((slot,mapped_rows,prior_report,pins))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 candidate caller lifetime')
    if (slot.get('voter')!='e94679197f925cb953920594ec7ec4c66fc8763b'
            or slot.get('payloads')!=40 or slot.get('full_proofs')!=91
            or (slot.get('prior_one'),slot.get('prior_zero'),slot.get('prior_proof'))!=(18,17,68)
            or len(slot.get('rows',()))!=40 or type(mapped_rows) is not list
            or len(mapped_rows)!=40):
        raise ValueError('fixed40 complete slot and map count')
    fields=('period','state_root_hex','account_hex','logical_key_hex','key_preimage_hex','voter_hex')
    tuples=[];periods=[];roots=[]
    for original,mapped in zip(slot['rows'],mapped_rows):
        if (set(original)!=set(('full_tuple','parent_manifest_index','source_references'))
                or set(original['full_tuple'])!=set(fields)
                or any(mapped.get(k)!=original[k] for k in original)
                or type(mapped.get('request_index')) is not int or mapped['request_index']<0):
            raise ValueError('fixed40 full tuple/source/parent/request binding')
        row=original['full_tuple'];period=row['period']
        if (type(period) is not int or row['voter_hex']!=slot['voter']
                or row['account_hex']!='00000000000000000000000000000000000000fe'
                or row['key_preimage_hex']!='0004'+slot['voter']
                or row['logical_key_hex']!='8b1ace38daea48c3738d2841802efffca8b51e3c108416b1625370dec2f5a4d4'
                or len(row['state_root_hex'])!=64
                or not original['source_references']
                or any(ref.get('period')!=period+7 for ref in original['source_references'])):
            raise ValueError('fixed40 exact period/key/source contract')
        bytes.fromhex(row['state_root_hex'])
        periods.append(period)
        if row['state_root_hex'] not in roots:roots.append(row['state_root_hex'])
        tuples.append(dict(parent_manifest_index=original['parent_manifest_index'],
            request_index=mapped['request_index'],voter_hex=row['voter_hex'],
            delayed_period=period,**{k:row[k] for k in fields if k not in ('period','voter_hex')},
            source_references=original['source_references']))
    omissions=(3,9,15,22,24,29,45)
    expected=[25706894+i for i in range(1,48) if i not in omissions]
    if periods!=expected or len({x['request_index'] for x in tuples})!=40:
        raise ValueError('fixed40 exact40 window/request identities')
    selections=prior_report.get('selection');values=prior_report.get('values')
    if (prior_report.get('status')!='all_twenty_vrf_inputs_authenticated'
            or type(selections) is not list or type(values) is not list
            or len(selections)!=20 or len(values)!=20
            or prior_report.get('bounds',{}).get('proofs_exact')!=71):
        raise ValueError('fixed40 full20 first-D context and71 proofs')
    prior=[dict(selection_index_1_based=i+1,**selection,**value)
           for i,(selection,value) in enumerate(zip(selections,values))]
    chosen=prior[17]
    if (chosen['parent_manifest_index']!=53 or chosen['proof_index']!=68
            or chosen['source_row']['voter_hex']!=slot['voter']
            or chosen['source_row']['delayed_period']!=25706894
            or chosen['logical']!='member' or chosen['usable_vrf_key'] is not True
            or chosen['value_hex']!='31905ba6089466e9b8aabd880d54b4a80f4d38ba9e9db942a4f64aa68f9caeb5'):
        raise ValueError('fixed40 exact selected first-D prior')
    first=tuples[0]
    value=dict(schema='conditional-next-vrf-candidate-v1',
        status='CANDIDATE_ONLY_805_BASELINE_NO_RUNTIME_ACCEPTANCE',
        conditional_basis=dict(status='PREVIOUS_805_SEAL',seal=accepted_seal,
            independent_acceptance=accepted_review,baseline_accepted=805,baseline_residual=109),
        voter_hex=slot['voter'],payload_count=40,
        omitted_delayed_window_indices_1_based=list(omissions),
        omitted_delayed_periods=[25706894+i for i in omissions],period_span=[periods[0],periods[-1]],
        account_hex=first['account_hex'],key_preimage_hex=first['key_preimage_hex'],
        logical_key_hex=first['logical_key_hex'],tuple_fields=list(fields),
        prior_context=dict(prior_bundle_pin=prior_bundle,prior_report_pin=prior_report_pin,
            selection_count=20,selected_prior=chosen,
            all_20_full_source_bindings_reference=dict(report_pin=prior_report_pin,count=20,
                exact_reconstruction='For i=0..19 merge {selection_index_1_based:i+1}, report.selection[i], report.values[i]; preserve all keys/values. Verify all20 and fresh71proofs, not only selected entry.',
                existing_authority_role_only=True,additional_read_or_charge=False),
            fresh_prior_proofs=71),
        coverage=dict(original_requests=914,accepted_a8a45_tuples=718,
            residual_before_fbb45=196,fbb45_count=45,remaining_after_fbb45_acceptance=151,
            previous_7f82_candidate_count=42,remaining_after_previous_7f82_acceptance=109,
            baseline_accepted=805,baseline_residual=109,candidate_count=40,
            remaining_after_candidate=69,conditional_remaining=69,
            independent_raw_map_check_pin=map_acceptance,
            remaining_voter_counts={'70b7ffd8269f116f7df6a46edd993263abd3eeb0':37,
                                    '414d739c4dccfe47d475cc08274b9e735a7941bd':32}),
        minimal_changed_assumptions=[
            'voter changes to '+slot['voter'],
            'payload count40; omitted delayed indices3,9,15,22,24,29,45',
            'full constructor51 plus payload40 proofs; fresh prior71 proofs unchanged',
            'selected first-D prior18 / parent53 / proof68; all20 original full bindings retained',
            'all40 full tuple/source/request/parent bindings and all distinct roots retained'],
        normalization_basis=map_pin,full_tuples=tuples,state_roots_hex=roots,
        accepted=False)
    if (live_bytes+fixed.graph_charge((slot,mapped_rows,prior_report,pins,value))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 candidate serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 full candidate role42 cap')
    return raw

def candidate_preparation(*, maxima, candidate, candidate_pin, prior_report, live_bytes):
    """Build all40 preparation rows and the exact selected first-D context."""
    maximum=_role_maximum(55,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 preparation caller')
    if (live_bytes+fixed.graph_charge((candidate,prior_report,candidate_pin))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 preparation caller lifetime')
    if (len(prior_report.get('selection',()))!=20 or len(prior_report.get('values',()))!=20):
        raise ValueError('fixed40 preparation full20 prior context')
    selection=prior_report['selection'][17];value=prior_report['values'][17]
    prior=dict(index_base=0,selection_index=17,parent_manifest_index=selection['parent_manifest_index'],
               identity=selection['tuple']['identity'],accepted_value=value)
    rows=[]
    for row in candidate.get('full_tuples',()):
        rows.append(dict(original_parent_manifest_index=row['parent_manifest_index'],
            account_hex=row['account_hex'],
            identity=dict(period=row['delayed_period'],state_root_hex=row['state_root_hex']),
            source_row=dict(delayed_period=row['delayed_period'],proof=None,qualified=False,
                semantic_key='fe_vrf_00_04_address',source_references=row['source_references'],
                value=None,voter_hex=row['voter_hex'])))
    prepared=dict(prior_first_d=prior,rows=rows)
    compare_candidate_preparation(maxima=maxima,candidate=candidate,candidate_pin=candidate_pin,
        preparation=prepared,live_bytes=live_bytes+fixed.graph_charge(prior_report))
    if (live_bytes+fixed.graph_charge((candidate,prior_report,prepared))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 preparation serialization lifetime')
    raw=core.canonical(prepared)
    if len(raw)>maximum:raise ValueError('fixed40 full preparation cap')
    return raw


def compare_candidate_preparation(*, maxima, candidate, candidate_pin, preparation, live_bytes):
    """Compare every source/parent/full-tuple binding, with no acceptance grant."""
    maximum=_role_maximum(44,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 candidate comparison caller')
    fixed._pin_tuple(candidate_pin)
    if (live_bytes+fixed.graph_charge((candidate,candidate_pin,preparation))
            +3*ROLE_MAXIMA[42]+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 candidate comparison caller lifetime')
    raw=core.canonical(candidate)
    if len(raw)!=candidate_pin['bytes'] or hashlib.sha256(raw).hexdigest()!=candidate_pin['sha256']:
        raise ValueError('fixed40 exact candidate body pin')
    del raw
    omitted=[3,9,15,22,24,29,45]
    if (candidate.get('payload_count')!=40 or len(candidate.get('full_tuples',()))!=40
            or candidate.get('voter_hex')!='e94679197f925cb953920594ec7ec4c66fc8763b'
            or candidate.get('omitted_delayed_window_indices_1_based')!=omitted
            or len(preparation.get('rows',()))!=40):
        raise ValueError('fixed40 complete candidate comparison count/window')
    for index,(row,prepared) in enumerate(zip(candidate['full_tuples'],preparation['rows'])):
        period=[25706894+i for i in range(1,48) if i not in omitted][index]
        expected=dict(original_parent_manifest_index=row['parent_manifest_index'],
            account_hex=row['account_hex'],
            identity=dict(period=period,state_root_hex=row['state_root_hex']),
            source_row=dict(delayed_period=period,proof=None,qualified=False,
                semantic_key='fe_vrf_00_04_address',source_references=row['source_references'],
                value=None,voter_hex=candidate['voter_hex']))
        if (prepared!=expected or row['delayed_period']!=period
                or row['voter_hex']!=candidate['voter_hex']
                or row['account_hex']!='00000000000000000000000000000000000000fe'
                or row['key_preimage_hex']!='0004'+candidate['voter_hex']
                or row['logical_key_hex']!='8b1ace38daea48c3738d2841802efffca8b51e3c108416b1625370dec2f5a4d4'):
            raise ValueError('fixed40 all40 exact preparation tuple/source binding')
    prior=preparation.get('prior_first_d',{})
    chosen=candidate['prior_context']['selected_prior']
    if (prior.get('index_base')!=0 or prior.get('selection_index')!=17
            or prior.get('parent_manifest_index')!=53
            or prior.get('identity')!=chosen['tuple']['identity']
            or prior.get('accepted_value')!={k:chosen[k] for k in
                ('logical','physical','proof_index','usable_vrf_key','value_hex')}
            or chosen['proof_index']!=68 or chosen['selection_index_1_based']!=18):
        raise ValueError('fixed40 exact preparation prior binding')
    value=dict(schema=1,candidate=candidate_pin,accepted=False,ready=False,
        exact_candidate_selection_equal=True,selection_count=40,constructor_proofs=51,
        bundle_proofs=91,fresh_prior_proofs=71,omitted_delayed_indices=omitted,
        selected_prior_one_index=18,selected_prior_zero_index=17,selected_prior_proof_index=68)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 comparison role44 cap')
    return raw

def saved_equivalence(*, maxima, acceptance_raw, old_source_raw, current_sources,
                      source_freeze, gate_result_pin, gate_log_pin, live_bytes):
    """Reuse only the exact independently accepted ten-test/19-report scope.

    Full accepted log/report bodies and their read charges remain mandatory.
    Changed Python tools receive no gate or source acceptance from this record.
    """
    maximum=_role_maximum(46,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 saved equivalence caller')
    if type(acceptance_raw) is not bytes or type(old_source_raw) is not bytes:
        raise ValueError('fixed40 exact accepted equivalence bodies')
    if (live_bytes+fixed.graph_charge((acceptance_raw,old_source_raw,current_sources))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 saved equivalence caller lifetime')
    if (len(acceptance_raw)!=2821 or hashlib.sha256(acceptance_raw).hexdigest()!=
            '1bc1edd4924538f692faa892d4cd8badba547db7b2ce773530c5eb8cb340e07f'
            or len(old_source_raw)!=17908 or hashlib.sha256(old_source_raw).hexdigest()!=
            'be283dccbab3448bb727749b83615ac3326cbfe4ae0cb44772d5a43d6baf48ae'):
        raise ValueError('fixed40 exact accepted equivalence source scope')
    for pin in (source_freeze,gate_result_pin,gate_log_pin):fixed._pin_tuple(pin)
    if (source_freeze['bytes']!=len(old_source_raw)
            or source_freeze['sha256']!=hashlib.sha256(old_source_raw).hexdigest()):
        raise ValueError('fixed40 exact accepted source roster pin')
    if (live_bytes+fixed.graph_charge((acceptance_raw,old_source_raw,current_sources))
            +3*(len(old_source_raw)+len(acceptance_raw))
            +160*(core._wire_nodes(old_source_raw)+core._wire_nodes(acceptance_raw))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 saved equivalence predecode lifetime')
    # This is an exact pinned administrative acceptance record with a finite
    # duration scalar, not a protocol report. Keep the protocol decoder strict.
    acceptance=json.loads(acceptance_raw,object_pairs_hook=records.unique_object)
    sources=records.decode(old_source_raw)
    if (type(current_sources) is not list or len(current_sources)!=90
            or sources!=current_sources):
        raise ValueError('fixed40 unchanged complete90 Rust/Cargo source scope')
    for pin in sources:fixed._pin_tuple(pin)
    result=acceptance['result'];log=acceptance['full_log']
    if (acceptance['accepted'] is not True or result['exit_code']!=0
            or result['timeout'] or result['overflow'] or not result['seconds']<=300
            or acceptance['reports']['count']!=19
            or len(acceptance['success_roster'])!=10
            or set(acceptance['success_roster'])!=set(fixed.SAVED_TESTS)
            or any(gate_result_pin[k]!=result[k] for k in ('bytes','sha256'))
            or any(gate_log_pin[k]!=log[k] for k in ('bytes','sha256'))):
        raise ValueError('fixed40 complete unchanged ten-test/19-report gate')
    value=dict(schema=1,accepted=False,ready=False,source_freeze=source_freeze,
        all16_unchanged_profiles_equal=True,native_and_first_d_complete_equal=True,
        matched_tests=10,required_exact_tests=list(fixed.SAVED_TESTS),
        gate_results=[gate_result_pin],gate_logs=[gate_log_pin],
        reports=acceptance['reports'],status='REUSED_EXACT_ACCEPTED_COMPONENT',
        accepted_component=dict(path='/home/fry/artifacts/vrf40-admission-01a14b66-20261018/native-ten-frozen-acceptance.json',
            bytes=2821,sha256='1bc1edd4924538f692faa892d4cd8badba547db7b2ce773530c5eb8cb340e07f'),
        source_scope='exact ordered90 Rust/Cargo inputs only',
        whole_current_python_gate_reused=False,new_facts=0)
    if (live_bytes+fixed.graph_charge((acceptance_raw,old_source_raw,current_sources,
            acceptance,sources,value))+2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 saved equivalence serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 saved equivalence role46 cap')
    return raw


def prospective_saved_equivalence(*, maxima, source_freeze, tool_source_freeze,
                                  command_record, log, report_paths,
                                  historical_acceptance, historical_source_freeze,
                                  unit, declarations, live_bytes):
    """Keep the current TEN19 obligation separate from exact historical reuse."""
    maximum=_role_maximum(46,maxima)
    pins=(source_freeze,tool_source_freeze,command_record,log,
          historical_acceptance,historical_source_freeze)
    if (historical_acceptance!=fixed.HISTORICAL_TEN19_ACCEPTANCE
            or historical_source_freeze!=fixed.HISTORICAL_TEN19_SOURCES
            or type(report_paths) is not list or len(report_paths)!=19
            or len(set(report_paths))!=19):
        raise ValueError('fixed40 complete current/historical TEN19 obligations')
    for path in report_paths:records.canonical_path(path)
    inputs=(pins,report_paths,declarations)
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    if live_bytes+owner+fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 complete pending TEN19 lifetime')
    _prospective_reads(unit,pins,declarations,live_bytes+fixed.graph_charge(inputs))
    value=dict(schema=PROSPECTIVE_MODE,status=PROSPECTIVE_MODE,accepted=False,
        ready=False,new_facts=0,source_freeze=source_freeze,
        tool_source_freeze=tool_source_freeze,
        all16_unchanged_profiles_equal=False,native_and_first_d_complete_equal=False,
        matched_tests=0,required_exact_tests=list(fixed.SAVED_TESTS),
        gate_results=[command_record],gate_logs=[log],
        reports=dict(count=19,maximum=1327104,individual_maximum=65536,paths=report_paths),
        deadline_seconds=300,whole_current_python_gate_reused=False,
        historical_component=dict(acceptance=historical_acceptance,
                                  source_freeze=historical_source_freeze),
        source_scope='current90 Rust/Cargo; actual ten-test/19-report gate mandatory',
        current_gate_state='PROSPECTIVE_NOT_EXECUTED_NOT_ACCEPTED')
    return _prospective_output(46,value,inputs,maxima,owner+live_bytes)

QUOTA_BUDGET_IDENTITIES={
    '7e34dd313147e89d77e1b83f7be9df34862f3ace36ea1aa26af193e909fc1d2d': '/home/fry/artifacts/vrf40-role72-recovery-20261019/quota-budget.json',
    '1f415b9ee4384aa68d6d162908bfadfe8b97ff13e516a0a9e6b70c13762f03f0':
        '/home/fry/artifacts/vrf40-complete-fit-20261019/quota-budget.json',
    '96fe0a94a661bbc664a683c913f582ef3aa4b388c4d6a44316ea22df09da6afc':
        '/home/fry/artifacts/vrf40-retention-20261018/quota-budget.json',
    'ce7a390c9422d3fefa7d7062ddbffffa304e4818dbc788154aa5cce97a0c161a':
        '/home/fry/artifacts/vrf40-integration-20261018-round202/quota-budget-round202.json',
    '61ff66f23f1ee3dab7b6eeb5ff8024f82f0a71fb37157ac748671f712ce0b0b2':
        '/home/fry/artifacts/vrf40-integration-20261018-round211/quota-budget-round211.json',
    '012743d416233309378c8eb188755cd4dbffdecbb183c4dc4eb9e9921373431f':
        '/home/fry/artifacts/vrf40-integration-20261018-round211/quota-budget-source91-20261019.json',
    'a78ace94ec73a8c7e5bfcf4092d9baa0adcbac95101ed28921d553b17ff90fbc':
        '/home/fry/artifacts/vrf40-writer-20261023-round147/quota-budget.json',
    '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4':
        '/home/fry/artifacts/vrf40-guard-20261023-round148/quota-budget.json'}
QUOTA_BUDGET_WIDTHS={
    'a78ace94ec73a8c7e5bfcf4092d9baa0adcbac95101ed28921d553b17ff90fbc':277,
    '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4':277}
QUOTA_BUDGET_RESET_TOLERANCES={
    'a78ace94ec73a8c7e5bfcf4092d9baa0adcbac95101ed28921d553b17ff90fbc':10,
    '7d9040fe6560e7120fcdd9d363e0f0cf69382b569220b8c55b52a9f672b8caa4':10}

# Round150 is a separate explicit ten-point grant; older identities stay intact.
QUOTA_BUDGET_IDENTITIES['6c5931f6a92e48e3240a86035a7564a12ec3084995d36ca2bf41be429756bfc2']='/home/fry/artifacts/vrf40-current160-20261023-round150/quota-budget.json'
QUOTA_BUDGET_WIDTHS['6c5931f6a92e48e3240a86035a7564a12ec3084995d36ca2bf41be429756bfc2']=278
QUOTA_BUDGET_RESET_TOLERANCES['6c5931f6a92e48e3240a86035a7564a12ec3084995d36ca2bf41be429756bfc2']=10

# Round151 adds only this exact saved grant; all earlier identities stay valid.
QUOTA_BUDGET_IDENTITIES['21c08e37f50d35f95494477558cc6eb94d693db13b6934fd8a395971067dc5ae'] = '/home/fry/artifacts/vrf40-generation-20261024-round151/quota-budget.json'
QUOTA_BUDGET_WIDTHS['21c08e37f50d35f95494477558cc6eb94d693db13b6934fd8a395971067dc5ae'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['21c08e37f50d35f95494477558cc6eb94d693db13b6934fd8a395971067dc5ae'] = 10


def quota_snapshot(raw, *, maxima, budget_raw, live_bytes, now_utc):
    """Retain the complete fresh saved-budget observation without resetting it."""
    maximum=_role_maximum(80,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 quota caller')
    budget_sha=hashlib.sha256(budget_raw).hexdigest() if type(budget_raw) is bytes else None
    if (type(raw) is not bytes or type(budget_raw) is not bytes
            or len(raw)>maximum or len(budget_raw)!=QUOTA_BUDGET_WIDTHS.get(budget_sha,278)
            or budget_sha not in QUOTA_BUDGET_IDENTITIES):
        raise ValueError('fixed40 exact saved quota budget')
    tolerance=QUOTA_BUDGET_RESET_TOLERANCES.get(budget_sha,2)
    if (live_bytes+fixed.graph_charge((raw,budget_raw))
            +3*(len(raw)+len(budget_raw))
            +160*(core._wire_nodes(raw)+core._wire_nodes(budget_raw))
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 quota caller lifetime')
    # Quota telemetry is administrative JSON with finite decimal percentages.
    budget=json.loads(budget_raw,object_pairs_hook=records.unique_object)
    value=json.loads(raw,object_pairs_hook=records.unique_object)
    fields={'age_seconds','baseline_observed_utc','baseline_remaining_percent',
        'baseline_resets_at','baseline_session_id','baseline_used_percent','budget_file',
        'budget_initialized','consume_percent','consumed_percent','consumed_percentage_points',
        'effort','floor_remaining_percent','log','model','observed_utc','reason',
        'remaining_percent','reset_drift_seconds','reset_tolerance_seconds','resets_at',
        'session_id','status','used_percent'}
    if set(value)!=fields:raise ValueError('fixed40 complete quota telemetry')
    numeric=('age_seconds','baseline_remaining_percent','baseline_used_percent','consume_percent',
        'consumed_percent','consumed_percentage_points','floor_remaining_percent',
        'remaining_percent','reset_drift_seconds','reset_tolerance_seconds','used_percent')
    if any(type(value[k]) not in (int,float) or not math.isfinite(value[k]) for k in numeric):
        raise ValueError('fixed40 finite quota telemetry')
    if (value['budget_initialized'] is not False
            or value['budget_file']!=QUOTA_BUDGET_IDENTITIES[budget_sha]
            or value['baseline_session_id']!=budget['session_id']
            or value['session_id']!=budget['session_id']
            or value['baseline_observed_utc']!=budget['observed_utc']
            or value['baseline_remaining_percent']!=budget['baseline_remaining_percent']
            or value['baseline_used_percent']!=budget['baseline_used_percent']
            or value['baseline_resets_at']!=budget['resets_at']
            or (tolerance==2 and value['resets_at']!=budget['resets_at'])
            or value['consume_percent']!=budget['consume_percent']
            or value['floor_remaining_percent']!=budget['floor_remaining_percent']):
        raise ValueError('fixed40 saved baseline/floor cannot change')
    now=datetime.fromisoformat(now_utc)
    observed=datetime.fromisoformat(value['observed_utc'])
    if (now.tzinfo is None or observed.tzinfo is None
            or now.utcoffset()!=timezone.utc.utcoffset(now)
            or observed.utcoffset()!=timezone.utc.utcoffset(observed)
            or not 0<=(now-observed).total_seconds()<=300):
        raise ValueError('fixed40 quota observation absolute freshness')
    if tolerance==10 and (
            type(value['resets_at']) is not int
            or value['reset_drift_seconds']!=value['resets_at']-budget['resets_at']
            or abs(value['resets_at']-budget['resets_at'])>tolerance
            or now.timestamp()>=budget['resets_at']
            or value['used_percent']<budget['baseline_used_percent']
            or value['remaining_percent']>budget['baseline_remaining_percent']):
        raise ValueError('fixed40 anchored saved quota window and usage rollback')
    if (not 0<=value['age_seconds']<=300 or not 0<=value['remaining_percent']<=100
            or value['used_percent']!=100-value['remaining_percent']
            or value['consumed_percentage_points']!=budget['baseline_remaining_percent']-value['remaining_percent']
            or value['consumed_percent']!=value['consumed_percentage_points']
            or value['reset_tolerance_seconds']!=tolerance
            or abs(value['reset_drift_seconds'])>tolerance
            or value['status'] not in ('allow_start','do_not_start')
            or (value['status']=='allow_start' and value['remaining_percent']<=budget['floor_remaining_percent'])):
        raise ValueError('fixed40 fresh quota and floor arithmetic')
    return raw

HISTORICAL_CONTEXT_PINS = {
    45: dict(path='/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/fourth-map/45-failure-history.json',
        bytes=2606,sha256='758b2206d21711cdd15c0d37b1d504dd9311ec0522dbe6d750bc08ebbf1d0a31'),
    50: dict(path='/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/third-source/code-and-gates-freeze.json',
        bytes=36042,sha256='b42c90053574348079ae5038098231a329d85638d84247139c8ba3cbed138271'),
    51: dict(path='/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/vrf-residual-third-voter-window-command-freeze.json',
        bytes=92897,sha256='16e2656fcbc9155fa114755357928926d5a7b50e5b514724c0d9af12eca6a715'),
    52: dict(path='/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/residual-third/fact-outcome-freeze.json',
        bytes=7342,sha256='c5ff42f1428d74ac208ee647b6e10568d80944e7b9b1a6016280de8e7bddcbc0'),
    53: dict(path='/home/fry/artifacts/vrf-run-01a141ba-20261015/luna/offline-full-tuple-map.json',
        bytes=202728,sha256='43ffbffd48d1d287b2c8c46e25a07456b43a2bcbc099312a6c64cf57b16b56c8')}
HISTORICAL_CONTEXT_SCOPE=dict(
    path='/home/fry/artifacts/vrf40-development-20261018/historical45-50-53-context-scope-review.json',
    bytes=7685,sha256='f38aff1727ecec8bbb55d63b62430f006e05122d590eff0d2f5ba334d7704161')


def historical_context_body(role, raw, *, maxima, unit, scope_declaration,
                            body_declaration, live_bytes):
    """Copy only five reviewed immutable historical bodies, with full reads.

    Both the exact scope review and the original body are streamed through
    prepaid core declarations. The future role keeps every original decoder
    and semantic consumer obligation; this byte constructor discharges none.
    Original values remain historical and confer no current acceptance.
    """
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    maximum=_role_maximum(role,maxima)
    if role not in HISTORICAL_CONTEXT_PINS or type(raw) is not bytes:
        raise ValueError('fixed40 reviewed historical context role')
    original=HISTORICAL_CONTEXT_PINS[role]
    core.integer(live_bytes,core.SEMANTIC,'fixed40 historical context caller')
    if len(raw)>maximum or len(raw)!=original['bytes']:
        raise ValueError('fixed40 complete historical context body')
    if (owner+live_bytes+fixed.graph_charge((raw,scope_declaration,body_declaration))
            +2*len(raw)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 historical context caller lifetime')
    for declaration,pin in ((scope_declaration,HISTORICAL_CONTEXT_SCOPE),
                            (body_declaration,original)):
        occurrence,physical,kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(pin) or kind!='evidence' or mode!='bytes':
            raise ValueError('fixed40 exact historical constructor declarations')
    # Full read/hash/EOF charges apply even for identical copies and repeats.
    for declaration,pin in ((scope_declaration,HISTORICAL_CONTEXT_SCOPE),
                            (body_declaration,original)):
        unit.consume(declaration[0],dict(pin),kind='evidence',
                     live_bytes=live_bytes+fixed.graph_charge(raw))
    if hashlib.sha256(raw).hexdigest()!=original['sha256']:
        raise ValueError('fixed40 historical context body changed')
    return raw


def runtime_command_result(role, *, maxima, command, exit_code, child_exit,
                           seconds, log, wall_timeout, output_overflow,
                           live_bytes, cwd=None, environment=None, freeze=None,
                           ready=None, binary=None, bundle=None, before=None,
                           after=None):
    """Encode complete terminal results for four fixed runtime command roles.

    This pure constructor cannot run a command, grant READY or accept facts.
    Exact commands, guard state, sources, full logs and before/after metadata
    still require their original independent consumers and physical checks.
    """
    maximum=_role_maximum(role,maxima)
    if role not in (64,66,70,73):
        raise ValueError('fixed40 exact runtime command result role')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 runtime result caller')
    if (type(command) is not list or not command
            or any(type(arg) is not str or not arg for arg in command)
            or type(exit_code) is not int or type(child_exit) is not int
            or type(seconds) not in (int,float) or not math.isfinite(seconds)
            or not 0<=seconds<=300 or type(wall_timeout) is not bool
            or type(output_overflow) is not bool):
        raise ValueError('fixed40 complete terminal runtime result')
    fixed._pin_tuple(log)
    core.integer(log['bytes'],65536,'fixed40 runtime full paired log')
    arguments=(command,log,cwd,environment,freeze,ready,binary,bundle,before,after)
    if (live_bytes+fixed.graph_charge(arguments)+3*maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 runtime result caller lifetime')
    value=dict(schema=1,accepted=False,command=command,exit=exit_code,
        child_exit=child_exit,seconds=seconds,log_bytes=log['bytes'],
        wall_timeout=wall_timeout,output_overflow=output_overflow)
    if role==66:
        del value['schema']
        if any(arg is not None for arg in (cwd,environment,freeze,ready,binary,bundle)):
            raise ValueError('fixed40 live result exact original fields')
        expected_paths=['/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy'+suffix
                        for suffix in ('','/db/db','/db/state_db')]
        for metadata in (before,after):
            if type(metadata) is not list or len(metadata)!=3:
                raise ValueError('fixed40 complete three-path copy metadata')
            for row,path in zip(metadata,expected_paths):
                if (type(row) is not dict or set(row)!=
                        {'path','device','inode','mode','mtime_ns'} or row['path']!=path
                        or any(type(row[k]) is not int or row[k]<0
                               for k in ('device','inode','mode','mtime_ns'))):
                    raise ValueError('fixed40 exact complete copy metadata')
        # Preserve differences as failure evidence; equality is a later gate.
        value.update(before=before,after=after)
    else:
        if before is not None or after is not None:
            raise ValueError('fixed40 command role metadata fields')
        records.canonical_path(cwd)
        if (type(environment) is not dict or set(environment)!={'LANG','LC_ALL','PATH'}
                or any(type(v) is not str or not v for v in environment.values())):
            raise ValueError('fixed40 exact command environment')
        value.update(cwd=cwd,environment=environment,log=log,new_facts=0)
        if role==64:
            fixed._pin_tuple(freeze);fixed._pin_tuple(ready)
            if binary is not None or bundle is not None:
                raise ValueError('fixed40 guard result original fields')
            value.update(freeze=freeze,ready=ready)
        elif role==70:
            fixed._pin_tuple(binary);fixed._pin_tuple(bundle)
            if freeze is not None or ready is not None:
                raise ValueError('fixed40 lead replay result original fields')
            value.update(binary=binary,bundle=bundle)
        else:
            for pin in (freeze,ready,binary,bundle):fixed._pin_tuple(pin)
            value.update(freeze=freeze,ready=ready,replay_binary=binary,input=bundle)
    if (live_bytes+fixed.graph_charge((arguments,value))+2*maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 runtime result serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete runtime result cap')
    return raw

def binary_successor_mapping(*, maxima, binaries, embedded_executable,
                             preflight, source_freeze, live_bytes):
    """Bind the actual current pair to its complete preflight/source records.

    Pins are metadata inputs. Required full binary/source reads and current
    build identity checks remain separate; this record grants no acceptance.
    """
    maximum=_role_maximum(34,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 binary mapping caller')
    if (type(binaries) is not list or len(binaries)!=2
            or binaries[0]['path']==binaries[1]['path']):
        raise ValueError('fixed40 exact two current binary paths')
    for pin in (*binaries,embedded_executable,preflight,source_freeze):fixed._pin_tuple(pin)
    if (any(not 0<pin['bytes']<=core.BINARY_FILE for pin in binaries)
            or sum(pin['bytes'] for pin in binaries)>core.BINARY_PAIR):
        raise ValueError('fixed40 unchanged current binary pair limits')
    if embedded_executable!=binaries[0]:
        raise ValueError('fixed40 complete preflight embedded executable binding')
    if (live_bytes+fixed.graph_charge((binaries,embedded_executable,preflight,source_freeze))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 binary mapping caller lifetime')
    value=dict(schema=1,accepted=False,binaries=binaries,embedded_executable=embedded_executable,
               preflight=preflight,source_freeze=source_freeze)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 binary mapping role34 cap')
    return raw

def _normal_commit_value(raw, pin, *, live_bytes):
    """Decode one complete actual normal-hook command record, never launch it."""
    fixed._pin_tuple(pin)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 actual commit caller')
    if (type(raw) is not bytes or len(raw)>65536 or len(raw)!=pin['bytes']
            or hashlib.sha256(raw).hexdigest()!=pin['sha256']):
        raise ValueError('fixed40 exact actual commit result body')
    if (live_bytes+fixed.graph_charge(raw)+3*len(raw)+160*core._wire_nodes(raw)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 actual commit predecode lifetime')
    value=records.decode(raw)
    expected={'schema','command','exit_code','hooks_path','log','no_push','normal_hook',
              'owned_sources','parent','source_acceptance','source_commit'}
    if (type(value) is not dict or set(value)!=expected or type(value['schema']) is not int
            or value['schema']!=1 or type(value['exit_code']) is not int
            or value['exit_code']!=0 or value['hooks_path']!='.githooks'
            or value['normal_hook'] is not True or value['no_push'] is not True
            or type(value['command']) is not list or len(value['command'])!=6
            or value['command'][:5]!=['rtk','proxy','git','commit','-m']
            or type(value['command'][5]) is not str or not value['command'][5]
            or type(value['parent']) is not str
            or not re.fullmatch('[0-9a-f]{40}',value['parent'])
            or type(value['source_commit']) is not str
            or not re.fullmatch('[0-9a-f]{40}',value['source_commit'])):
        raise ValueError('fixed40 exact normal-hook local commit record')
    fixed._pin_tuple(value['log']);fixed._pin_tuple(value['source_acceptance'])
    core.integer(value['log']['bytes'],327680,'fixed40 complete normal hook log')
    sources=value['owned_sources']
    if (type(sources) is not list or not 1<=len(sources)<=101
            or len({p['path'] for p in sources})!=len(sources)):
        raise ValueError('fixed40 owned committed source roster')
    for source in sources:
        fixed._pin_tuple(source)
        core.integer(source['bytes'],records.BODY_CAP,'fixed40 owned source body')
    if sum(p['bytes'] for p in sources)>core.SOURCE:
        raise ValueError('fixed40 owned source aggregate')
    return value


def source_commit_record(*, maxima, actual_raw, actual_pin, source_commit,
                         source_acceptance, log, owned_sources, live_bytes):
    """Bind a completed local commit; frozen-source/full-log checks stay required."""
    maximum=_role_maximum(30,maxima)
    fixed._pin_tuple(source_acceptance);fixed._pin_tuple(log)
    if (live_bytes+fixed.graph_charge((source_acceptance,log,owned_sources))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 source commit constructor lifetime')
    actual=_normal_commit_value(actual_raw,actual_pin,
        live_bytes=live_bytes+fixed.graph_charge((source_acceptance,log,owned_sources)))
    if (actual['source_commit']!=source_commit or actual['source_acceptance']!=source_acceptance
            or actual['log']!=log or actual['owned_sources']!=owned_sources):
        raise ValueError('fixed40 actual commit source/log/acceptance binding')
    value=dict(schema=1,source_commit=source_commit,source_acceptance=source_acceptance,
               actual_commit=actual_pin,log=log,no_push=True)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 source commit role30 cap')
    return raw


def normal_hook_summary(*, maxima, actual_raw, actual_pin, full_log, live_bytes):
    """Count complete passing Rust test summaries in the actual normal-hook log.

    The full hook log and actual command record remain mandatory evidence.
    This summary grants neither source acceptance nor permission to commit.
    """
    maximum=_role_maximum(32,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 hook summary caller')
    if type(full_log) is not bytes or len(full_log)>327680:
        raise ValueError('fixed40 complete hook log body')
    actual=_normal_commit_value(actual_raw,actual_pin,
        live_bytes=live_bytes+fixed.graph_charge(full_log))
    if (len(full_log)!=actual['log']['bytes']
            or hashlib.sha256(full_log).hexdigest()!=actual['log']['sha256']):
        raise ValueError('fixed40 exact complete normal-hook log')
    # Reserve capture-copy and integer workspace before scanning an arbitrarily
    # long bounded digit run. Every live input and decoded record stays charged.
    if (live_bytes+fixed.graph_charge((actual_raw,actual,full_log))
            +2*len(full_log)+2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 hook capture/int pre-scan lifetime')
    # Byte scanning retains all original output. No replacement/rewrite occurs.
    matched=0;start=0
    pattern=re.compile(rb'test result: ok[.] ([0-9]+) passed; 0 failed;.*')
    while start<len(full_log):
        stop=full_log.find(b'\n',start)
        if stop<0:stop=len(full_log)
        end=stop-1 if stop>start and full_log[stop-1]==13 else stop
        match=pattern.fullmatch(full_log,start,end)
        if match:matched+=int(match.group(1))
        start=stop+1
    if matched<=0:raise ValueError('fixed40 complete normal-hook passing test evidence')
    value=dict(schema=1,source_commit=actual['source_commit'],exit_code=0,bypassed=False,
               matched_tests=matched,log=actual['log'])
    if (live_bytes+fixed.graph_charge((actual_raw,actual,full_log,value))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 normal hook summary serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 normal hook role32 cap')
    return raw

def validation_summary(*, maxima, gates, source_commit, source_acceptance,
                       source_freeze, tool_source_freeze, source_commit_record,
                       actual_hook, hook_summary, complete_offline_fit_review,
                       authority, bridge_scope, cache_scope, live_bytes):
    """Retain the complete ordered14-gate and source/fit/hook evidence bindings.

    Every referenced full result/log/review remains mandatory. Reused component
    bodies keep original provenance through the exact cache-scope record.
    This summary cannot mark physical readiness or accept new facts.
    """
    maximum=_role_maximum(29,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 validation summary caller')
    refs=dict(source_acceptance=source_acceptance,source_freeze=source_freeze,
        tool_source_freeze=tool_source_freeze,source_commit_record=source_commit_record,
        actual_hook=actual_hook,hook_summary=hook_summary,
        complete_offline_fit_review=complete_offline_fit_review,authority=authority,
        bridge_scope=bridge_scope,cache_scope=cache_scope)
    for pin in refs.values():fixed._pin_tuple(pin)
    if type(source_commit) is not str or not re.fullmatch('[0-9a-f]{40}',source_commit):
        raise ValueError('fixed40 validation exact source commit')
    if type(gates) is not list or len(gates)!=14:
        raise ValueError('fixed40 all14 ordered gate bindings')
    for index,gate in zip(VALIDATION_RESULTS,gates):
        if (type(gate) is not dict or set(gate)!={'index','result','log'}
                or type(gate['index']) is not int or gate['index']!=index):
            raise ValueError('fixed40 exact gate order and complete fields')
        fixed._pin_tuple(gate['result']);fixed._pin_tuple(gate['log'])
        core.integer(gate['result']['bytes'],16384,'fixed40 complete gate result')
        core.integer(gate['log']['bytes'],327680,'fixed40 complete gate log')
    if (live_bytes+fixed.graph_charge((gates,refs,source_commit))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 validation summary caller lifetime')
    value=dict(schema=1,source_commit=source_commit,gates=gates,**refs,
               captured=False,new_facts=0,physical_ready=False)
    if (live_bytes+fixed.graph_charge((gates,refs,value))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 validation summary serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete validation summary cap')
    return raw

INSTRUCTION_PATHS=(
    '/workspaces/rustaxa-evm/doc/evm_research/n4_next_10point_plan.md',
    '/workspaces/rustaxa-evm/next_executable_slice_prompt.md',
    '/workspaces/rustaxa-evm/doc/evm_research/n4_next_run_checkpoint.md',
    '/workspaces/rustaxa-evm/doc/evm_research/n6_restart_checkpoint.md',
    '/workspaces/rustaxa-evm/doc/evm_research/n6_remaining_branch_queue.md',
    '/workspaces/rustaxa-evm/doc/evm_research/n4_evidence_record_successor.md',
)


def instruction_preparation_comparison(*, maxima, instructions, active_user_scope,
                                       live_bytes):
    """Preserve all six current/authority/preparation observations as role35.

    Every pin still requires its full original authentication and semantic read.
    Pin equality is an observation. It grants no instruction or runtime authority.
    The active scope must come from the authenticated current user authorization.
    """
    maximum=_role_maximum(35,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 instruction comparison caller')
    if (type(active_user_scope) is not str or not active_user_scope
            or len(active_user_scope)>8192
            or type(instructions) is not list or len(instructions)!=6):
        raise ValueError('fixed40 complete six instruction observations')
    if (live_bytes+fixed.graph_charge((instructions,active_user_scope))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 instruction comparison caller lifetime')
    if len(active_user_scope.encode('utf-8'))>8192:
        raise ValueError('fixed40 instruction scope UTF8 byte cap')
    rows=[]
    for path,row in zip(INSTRUCTION_PATHS,instructions):
        if (type(row) is not dict
                or set(row)!={'current','authority_observation','preparation_predecessor'}):
            raise ValueError('fixed40 complete instruction observation fields')
        for pin in row.values():
            fixed._pin_tuple(pin)
            core.integer(pin['bytes'],records.BODY_CAP,'fixed40 instruction source body')
        if row['current']['path']!=path:
            raise ValueError('fixed40 exact ordered instruction paths')
        rows.append(dict(**row,exact_authority_pin_equal=(
            row['current']==row['authority_observation'])))
    value=dict(schema=1,accepted=False,ready=False,
        status='CURRENT_INSTRUCTION_OBSERVATION_PENDING_REVIEW',
        active_user_scope=active_user_scope,instructions=rows)
    if (live_bytes+fixed.graph_charge((instructions,value))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 instruction comparison serialization lifetime')
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete instruction comparison cap')
    return raw

def physical_attempt_marker(*, maxima, live_executable, bundle_path, report_path,
                            freeze, source_commit, live_bytes):
    """Serialize role63 bytes without publishing or consuming an attempt.

    The runtime guard must authenticate all frozen bodies, independent READY,
    exact commands and collision absence before exclusive durable publication.
    These bytes alone cannot authorize a launch or discharge any required read.
    """
    maximum=_role_maximum(63,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 attempt marker caller')
    fixed._pin_tuple(live_executable);fixed._pin_tuple(freeze)
    if not 0<live_executable['bytes']<=core.BINARY_FILE:
        raise ValueError('fixed40 attempt marker executable size')
    records.canonical_path(bundle_path);records.canonical_path(report_path)
    if len({live_executable['path'],bundle_path,report_path,freeze['path']})!=4:
        raise ValueError('fixed40 attempt marker distinct frozen paths')
    if type(source_commit) is not str or not re.fullmatch('[0-9a-f]{40}',source_commit):
        raise ValueError('fixed40 attempt marker exact source commit')
    if (live_bytes+fixed.graph_charge((live_executable,bundle_path,report_path,
                                      freeze,source_commit))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 attempt marker caller lifetime')
    command=[live_executable['path'],'--bundle-output',bundle_path,
             '--report-output',report_path]
    value=dict(command=command,freeze_sha256=freeze['sha256'],source_commit=source_commit)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete attempt marker cap')
    return raw

def _correction_string_wire_size(value, maximum):
    # Exact ensure_ascii=True JSON string width, without an encoded copy.
    size=2
    for character in value:
        ordinal=ord(character)
        if character in ('"',chr(92)) or character in ('\b','\f','\n','\r','\t'):
            size+=2
        elif ordinal<32 or 127<=ordinal<=65535:size+=6
        elif ordinal>65535:size+=12
        else:size+=1
        if size>maximum:return size
    return size


def current_correction_record(*, maxima, failures, live_bytes):
    """Retain every caller-supplied current failure body as complete UTF8.

    The full frozen manifest must prove this list covers every required failure.
    Loss records remain separately mandatory; missing output cannot be replaced
    by this constructor. Failed commands never become validation evidence.
    """
    maximum=_role_maximum(48,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 correction record caller')
    if type(failures) is not list or len(failures)>2048:
        raise ValueError('fixed40 complete finite current failure list')
    if (live_bytes+fixed.graph_charge(failures)+3*maximum+core.CHUNK+262144
            >core.SEMANTIC):
        raise ValueError('fixed40 correction record caller lifetime')
    findings=('Fit preparation only. Prior failure bodies remain in immutable history. '
              'Current source acceptance, hook, READY and capture are pending.')
    wire_size=len(core.canonical(dict(schema='n4-fixed40-finite-current-corrections-v1',
        accepted=False,failures=[],findings=findings)))
    empty_row_size=len(core.canonical(dict(role='',path='',bytes=0,sha256='',body_utf8='')))-1
    paths=set()
    for row in failures:
        if (type(row) is not dict
                or set(row)!={'role','path','bytes','sha256','body_utf8'}
                or type(row['role']) is not str or not row['role']
                or len(row['role'])>256 or type(row['body_utf8']) is not str
                or len(row['body_utf8'])>maximum):
            raise ValueError('fixed40 full current failure fields')
        fixed._pin_tuple({k:row[k] for k in ('path','bytes','sha256')})
        if row['path'] in paths:raise ValueError('fixed40 duplicate current failure path')
        row_size=empty_row_size+len(str(row['bytes']))-1
        row_size+=sum(_correction_string_wire_size(row[k],maximum)-2
                      for k in ('role','path','sha256','body_utf8'))
        wire_size+=row_size+(1 if paths else 0)
        if wire_size>maximum:raise ValueError('fixed40 complete current correction record cap')
        paths.add(row['path'])
        if (live_bytes+fixed.graph_charge((failures,paths))+4*len(row['body_utf8'])
                +core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 correction UTF8 caller lifetime')
        raw=row['body_utf8'].encode('utf-8')
        if len(raw)!=row['bytes'] or hashlib.sha256(raw).hexdigest()!=row['sha256']:
            raise ValueError('fixed40 full original failure body identity')
        del raw
    value=dict(schema='n4-fixed40-finite-current-corrections-v1',accepted=False,
               failures=failures,findings=findings)
    if (live_bytes+fixed.graph_charge((failures,value,paths))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 correction record serialization lifetime')
    raw=core.canonical(value)
    if len(raw)!=wire_size or len(raw)>maximum:
        raise ValueError('fixed40 complete current correction record cap')
    return raw


FIXED40_PROFILE='n4_residual_vrf_e94679197f925cb953920594ec7ec4c66fc8763b_v1'
PROSPECTIVE_MODE='NONHISTORICAL_PROSPECTIVE_FIT_ONLY'
FIXED40_ENVIRONMENT={'LANG':'C.UTF-8','LC_ALL':'C.UTF-8','PATH':'/usr/bin:/bin'}
NATIVE_RESOURCE_LIMITS=(('KeyBytes',64),('RowBytes',1048576),
    ('RawCopyBytes',67108864),('CanonicalBytes',67108864),
    ('EncodedBytes',134217728),('Operations',100000),('Visits',100000),
    ('Records',100000),('FrameInspections',300000),
    ('ConstructorIdentities',49),('Registrations',51),
    ('DistinctIdentities',50),('PayloadTuples',96))
INDEPENDENT_VERIFICATION_FIELDS=frozenset(('accepted','binary_count','bundle',
    'canonical_reports_byte_equal','capture_decision','capture_report','constructor_proofs',
    'current_proofs','exact_eof_verified','freeze','fresh_prior_proofs',
    'full_database_preservation_proven','guarded_capture','immutable_extra_source_basis',
    'independent_log','independent_report','independent_result','lead_report','limits',
    'live_result','marker','metadata_before_after_equal','new_facts','omitted_delayed_indices',
    'payload_proofs','prior_payload_bindings','prior_usage','profile','proof_basis','qualification',
    'raw_artifact_bytes','raw_artifact_count','raw_inputs','ready','replay_binary','replay_usage',
    'requested_model','resource_limits','reviewer_provenance','run','runtime_identity','schema',
    'selected_prior','selection_delayed_indices','selection_sha256','source_commit','source_count',
    'source_observations','source_occurrence_bytes','source_occurrences','status','tool_count','values','verified'))


def _native_usage(usage):
    if (type(usage) is not dict or set(usage)!={'admitted_input_bytes','resources'}
            or type(usage['admitted_input_bytes']) is not int
            or not 0<=usage['admitted_input_bytes']<=134217728
            or type(usage['resources']) is not list or len(usage['resources'])!=13):
        raise ValueError('fixed40 complete native usage')
    for row,(name,limit) in zip(usage['resources'],NATIVE_RESOURCE_LIMITS):
        if (type(row) is not dict or set(row)!={'resource','used'}
                or row['resource']!=name or type(row['used']) is not int
                or not 0<=row['used']<=limit):
            raise ValueError('fixed40 unchanged thirteen native usage bounds')


def _constructor_lifetime(role, maxima, inputs, live_bytes):
    """Reserve the caller, input graph, complete output and encoding scratch."""
    maximum=_role_maximum(role,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 concrete constructor caller')
    if (live_bytes+2*fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144
            >core.SEMANTIC):
        raise ValueError('fixed40 concrete constructor caller lifetime')
    return maximum


def _constructor_output(value, maximum):
    records.bounded_shape(value)
    raw=core.canonical(value)
    if len(raw)>maximum:raise ValueError('fixed40 complete constructor output cap')
    return raw


def _prospective_output(role,value,inputs,maxima,full_caller):
    """Encode a new complete capacity model; never encode production evidence."""
    maximum=_role_maximum(role,maxima)
    records.bounded_shape(value)
    encoder=json.JSONEncoder(sort_keys=True,separators=(',',':'),ensure_ascii=True,allow_nan=False)
    width=1+sum(len(piece) for piece in encoder.iterencode(value))
    if width>maximum:raise ValueError('fixed40 complete prospective role cap')
    if (full_caller+fixed.graph_charge((inputs,value))+maximum+2*width
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective serialization preallocation')
    raw=core.canonical(value)
    if len(raw)!=width:raise ValueError('fixed40 prospective exact encoding width')
    # The closed field graph is the specimen. Whitespace cannot stand in for
    # a maximum semantic graph or a future output that does not yet exist.
    return raw


def _prospective_reads(unit,pins,declarations,external_live):
    if len(pins)!=len(declarations):
        raise ValueError('fixed40 complete prospective input declarations')
    for pin,declaration in zip(pins,declarations):
        fixed._pin_tuple(pin)
        occurrence,physical,kind,mode=declaration
        if (tuple(physical)!=fixed._pin_tuple(pin) or kind not in ('source','evidence')
                or mode!='bytes'):
            raise ValueError('fixed40 exact prospective input declaration')
    for pin,declaration in zip(pins,declarations):
        occurrence,physical,kind,mode=declaration
        # Unit already adds the owner metadata; only external growth goes here.
        unit.consume(occurrence,pin,kind=kind,live_bytes=external_live)


def prospective_validation_result(role,*,maxima,source_snapshot,command_record,
                                  command,cwd,log,unit,declarations,live_bytes):
    """Model every original gate field without claiming a completed command.

    The full available snapshot, planned-command body and prospective log are
    authenticated inputs. Current-source singleton units remain mandatory.
    The distinctive cache status and impossible completion timestamp prevent
    this specimen from serving as a fresh result or accepted reused gate.
    """
    maximum=_role_maximum(role,maxima)
    if role not in VALIDATION_RESULTS:raise ValueError('fixed40 prospective gate role')
    _command_argv(command);records.canonical_path(cwd)
    inputs=(source_snapshot,command_record,command,cwd,log,declarations)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 prospective gate external caller')
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    if full+2*fixed.graph_charge(inputs)+maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 prospective gate input preallocation')
    _prospective_reads(unit,(source_snapshot,command_record,log),declarations,
        live_bytes+fixed.graph_charge(inputs))
    core.integer(log['bytes'],ROLE_MAXIMA[role+1],'fixed40 prospective complete gate log')
    value=dict(command=list(command),cwd=cwd,exit_code=2147483647,
        finished_utc=PROSPECTIVE_MODE,seconds=300,log=dict(log),role=role,
        source_snapshot=dict(source_snapshot),command_record=dict(command_record),
        cache_status=PROSPECTIVE_MODE,cache_provenance=None)
    return _prospective_output(role,value,inputs,maxima,full)


def prospective_log(role,*,maxima,command,live_bytes):
    """Produce complete nonhistorical log records, with no execution claims."""
    maximum=_role_maximum(role,maxima)
    if role not in LOG_ROLES:raise ValueError('fixed40 prospective log role')
    _command_argv(command)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 prospective log caller')
    if live_bytes+fixed.graph_charge(command)+maximum+16384+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 prospective log preallocation')
    prefix=core.canonical(dict(mode=PROSPECTIVE_MODE,command=command,executed=False))
    record=core.canonical(dict(mode=PROSPECTIVE_MODE,executed=False,
        event='capacity_observation',value=2147483647))
    if len(prefix)>maximum:raise ValueError('fixed40 prospective log prefix cap')
    count=(maximum-len(prefix))//len(record)
    # Each byte belongs to a complete record. The unused suffix is not emitted
    # or counted as semantic maximum coverage.
    return prefix+record*count


def _source_lists(sources, tool_sources):
    if (type(sources) is not list or len(sources)!=90
            or type(tool_sources) is not list or len(tool_sources)!=11):
        raise ValueError('fixed40 complete90 Rust and11 tool sources')
    seen=set();total=0
    for row in (*sources,*tool_sources):
        fixed._pin_tuple(row)
        core.integer(row['bytes'],1048576,'fixed40 source file cap')
        if row['path'] in seen:raise ValueError('fixed40 duplicate source path')
        seen.add(row['path']);total+=row['bytes']
    if total>core.SOURCE:raise ValueError('fixed40 complete source aggregate')
    return total


def _copy_rows(rows):
    prefix='/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy'
    if type(rows) is not list or len(rows)!=3:
        raise ValueError('fixed40 complete copy metadata')
    for row,suffix in zip(rows,('','/db/db','/db/state_db')):
        if (type(row) is not dict or set(row)!={'path','device','inode','mode','mtime_ns'}
                or row['path']!=prefix+suffix
                or any(type(row[k]) is not int or row[k]<0
                       for k in ('device','inode','mode','mtime_ns'))):
            raise ValueError('fixed40 exact copy metadata')


def _command_argv(command):
    if (type(command) is not list or not command
            or any(type(arg) is not str or not arg or '\x00' in arg for arg in command)):
        raise ValueError('fixed40 exact command argv')


def instance_spec(role, *, maxima, run, source_commit, sources, tool_sources,
                  binaries, evidence, preflight, preparation, copy_metadata,
                  live_bytes):
    """Build both complete identical instance placements from actual pins.

    Evidence stays ordered and includes every repeated original occurrence.
    Pin authentication, original-body semantics and runtime admission remain
    mandatory caller work. This constructor performs no file or database I/O.
    """
    if role not in (56,58):raise ValueError('fixed40 instance placement role')
    maximum=_constructor_lifetime(role,maxima,
        (run,source_commit,sources,tool_sources,binaries,evidence,preflight,
         preparation,copy_metadata),live_bytes)
    records.canonical_path(run)
    if type(source_commit) is not str or re.fullmatch('[0-9a-f]{40}',source_commit) is None:
        raise ValueError('fixed40 complete source commit')
    _source_lists(sources,tool_sources);_copy_rows(copy_metadata)
    if type(binaries) is not list or len(binaries)!=2:
        raise ValueError('fixed40 immutable binary pair')
    for row in binaries:fixed._executable_value(row)
    if binaries[0]['path']==binaries[1]['path']:
        raise ValueError('fixed40 distinct binary pair')
    fixed._pin_tuple(preflight);fixed._pin_tuple(preparation)
    if type(evidence) is not list or not evidence:
        raise ValueError('fixed40 complete ordered evidence')
    # Full path/order authority remains in the complete command freeze. Keep
    # the original compact instance representation without dropping repeats.
    for row in evidence:fixed._pin_tuple(row)
    return _constructor_output(dict(schema=1,run=run,source_commit=source_commit,
        sources=sources,tool_sources=tool_sources,binaries=binaries,
        evidence=[[p['bytes'],p['sha256']] for p in evidence],
        preflight=preflight,preparation=preparation,
        copy_metadata=copy_metadata),maximum)


def command_freeze(*, maxima, run, source_commit, sources, tool_sources,
                   binaries, preflight, evidence, preparation, instance,
                   capture_command, copy_metadata, embedded_source_bytes,
                   live_bytes):
    """Construct the original complete freeze fields in actual full-pin order.

    No evidence pin can replace its mandatory full body read or semantic
    route. The caller must authenticate this complete closure before launch.
    """
    maximum=_constructor_lifetime(57,maxima,
        (run,source_commit,sources,tool_sources,binaries,preflight,evidence,
         preparation,instance,capture_command,copy_metadata),live_bytes)
    records.canonical_path(run)
    if type(source_commit) is not str or re.fullmatch('[0-9a-f]{40}',source_commit) is None:
        raise ValueError('fixed40 complete source commit')
    source_bytes=_source_lists(sources,tool_sources);_copy_rows(copy_metadata)
    if type(binaries) is not list or len(binaries)!=2:
        raise ValueError('fixed40 immutable binary pair')
    for pin in binaries:fixed._executable_value(pin)
    if binaries[0]['path']==binaries[1]['path']:
        raise ValueError('fixed40 distinct binary pair')
    if type(evidence) is not list or not evidence:
        raise ValueError('fixed40 complete ordered evidence')
    for pin in (preflight,*evidence,preparation,instance):fixed._pin_tuple(pin)
    _command_argv(capture_command)
    if (len(capture_command)!=5 or capture_command[0]!=binaries[0]['path']
            or capture_command[1::2]!=['--bundle-output','--report-output']):
        raise ValueError('fixed40 exact native capture command')
    for path in capture_command[2::2]:records.canonical_path(path)
    if capture_command[2]==capture_command[4]:
        raise ValueError('fixed40 distinct native capture outputs')
    core.integer(embedded_source_bytes,core.SOURCE,'fixed40 embedded sources')
    pins=[*sources,*tool_sources,*binaries,preflight,*evidence,preparation,instance]
    return _constructor_output(dict(schema=1,profile=FIXED40_PROFILE,run=run,
        source_commit=source_commit,cwd='/workspaces/rustaxa-evm',
        environment=dict(FIXED40_ENVIRONMENT),capture_command=capture_command,
        pins=pins,pin_count=len(pins),pinned_file_bytes=sum(p['bytes'] for p in pins),
        source_count=90,complete_source_bytes=source_bytes,
        embedded_source_bytes=embedded_source_bytes,copy_metadata=copy_metadata,
        accepted=False),maximum)


def exact_commands(*, maxima, freeze, freeze_pin, build, capture, lead_replay,
                   independent_replay, outcomes, live_bytes):
    """Record all original command fields without granting command READY."""
    maximum=_constructor_lifetime(59,maxima,
        (freeze,freeze_pin,build,capture,lead_replay,independent_replay,outcomes),live_bytes)
    fixed._pin_tuple(freeze_pin)
    fields={'schema','profile','run','source_commit','cwd','environment',
        'capture_command','pins','pin_count','pinned_file_bytes','source_count',
        'complete_source_bytes','embedded_source_bytes','copy_metadata','accepted'}
    if (type(freeze) is not dict or set(freeze)!=fields
            or freeze['profile']!=FIXED40_PROFILE or freeze['accepted'] is not False
            or freeze['source_count']!=90 or freeze['pin_count']!=len(freeze['pins'])
            or freeze['environment']!=FIXED40_ENVIRONMENT):
        raise ValueError('fixed40 complete command freeze')
    if (type(outcomes) is not list or len(outcomes)!=24
            or sum(row['cap_bytes'] for row in outcomes)!=sum(ROLE_MAXIMA[57:])):
        raise ValueError('fixed40 complete24 outcome reservation')
    for index,row in enumerate(outcomes):
        if (type(row) is not dict or set(row)!=
                {'cap_bytes','group','index','path','schema_id'}
                or row['group']!='outcome24' or row['index']!=index
                or row['cap_bytes']!=ROLE_MAXIMA[57+index]):
            raise ValueError('fixed40 exact outcome maximum/order')
        records.canonical_path(row['path'])
    for argv in (build,capture,lead_replay,independent_replay):_command_argv(argv)
    pins=freeze['pins'];binaries=pins[101:103]
    if len(binaries)!=2:raise ValueError('fixed40 complete frozen binary pair')
    return _constructor_output(dict(schema=1,profile=FIXED40_PROFILE,
        run=freeze['run'],source_commit=freeze['source_commit'],
        cwd=freeze['cwd'],environment=freeze['environment'],freeze=freeze_pin,
        build=build,capture=capture,inner_capture=freeze['capture_command'],
        lead_replay=lead_replay,independent_replay=independent_replay,
        binaries=binaries,binary_count=2,source_count=90,tool_count=11,
        pin_count=freeze['pin_count'],outcomes=outcomes,outcome_count=24,
        outcome_full_reservation=sum(ROLE_MAXIMA[57:]),deadline_seconds=300,
        result_bytes=16384,sink_bytes=65536,
        source_canonical_sha256=hashlib.sha256(core.canonical(pins[:90])).hexdigest(),
        captured=False,ready=False),maximum)


def native_bundle(raw, *, maxima, bundle_pin, unit, declaration, live_bytes):
    """Bind the complete native RCV6 output; never synthesize witness proofs.

    Native replay must still verify all91 proofs, the fresh71-proof prior and
    exact EOF. The header and SHA checks here grant no proof acceptance.
    """
    maximum=_role_maximum(68,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 native bundle caller')
    fixed._pin_tuple(bundle_pin)
    if (type(raw) is not bytes or len(raw)>maximum or raw[:4]!=b'RCV6'
            or len(raw)!=bundle_pin['bytes']
            or hashlib.sha256(raw).hexdigest()!=bundle_pin['sha256']):
        raise ValueError('fixed40 complete native RCV6 bundle')
    if (live_bytes+fixed.graph_charge((raw,bundle_pin,declaration))
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 native bundle caller lifetime')
    occurrence,physical,kind,mode=declaration
    if (tuple(physical)!=fixed._pin_tuple(bundle_pin)
            or kind!='evidence' or mode!='bytes'):
        raise ValueError('fixed40 native bundle original declaration')
    unit.consume(occurrence,bundle_pin,kind=kind,
                 live_bytes=live_bytes+fixed.graph_charge(raw))
    return raw


def prospective_native_bundle(*, maxima, output_path, native_source, raw_inputs,
                              prior_bundle, unit, declarations, live_bytes):
    """Exercise the opaque output capacity without inventing a witness body.

    The allocation is scratch, never published as RCV6 or as proof evidence.
    The complete native source and all seven immutable authority inputs are
    authenticated. The actual serializer, every proof, SHA and EOF stay pending
    until the single authorized native capture and its actual consumers run.
    """
    maximum=_role_maximum(68,maxima)
    records.canonical_path(output_path)
    fixed._pin_tuple(native_source);fixed._pin_tuple(prior_bundle)
    if native_source['path']!='/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_shared_window.rs':
        raise ValueError('fixed40 native bundle exact implementation source')
    if type(raw_inputs) is not list or len(raw_inputs)!=7:
        raise ValueError('fixed40 full native bundle authority inputs')
    pins=[native_source,*raw_inputs]
    for pin in pins:fixed._pin_tuple(pin)
    if raw_inputs[-1]!=prior_bundle or len(declarations)!=8:
        raise ValueError('fixed40 complete native bundle source/prior binding')
    inputs=(output_path,native_source,raw_inputs,prior_bundle,declarations)
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    caller=live_bytes+fixed.graph_charge(inputs)
    if owner+caller+maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 complete maximum native output allocation')
    for index,(pin,declaration) in enumerate(zip(pins,declarations)):
        occurrence,physical,kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(pin) or kind!=('source' if index==0 else 'evidence') or mode!='bytes':
            raise ValueError('fixed40 native output layout exact source declaration')
        unit.consume(occurrence,pin,kind=kind,live_bytes=caller)
    scratch=bytearray(maximum)
    actual_capacity=len(scratch)
    del scratch
    value=dict(schema=PROSPECTIVE_MODE,status=PROSPECTIVE_MODE,role=68,
        accepted=False,ready=False,new_facts=0,emitted_native_body=False,
        output_path=output_path,native_format='RCV6',native_source=native_source,
        raw_inputs=raw_inputs,prior_bundle=prior_bundle,
        maximum_output_bytes=maximum,actual_capacity_exercised=actual_capacity,
        payload_tuples=40,constructor_proofs=51,current_proofs=91,fresh_prior_proofs=71,
        mandatory_actual_consumers=['native serializer','full body SHA256 and exact EOF',
            'all91 current proofs','fresh71 prior proofs','both full replays',
            'independent tuple/fact/seal acceptance'])
    return _constructor_output(value,maximum)


def _prospective_bundle_capacity(bundle,authority,vrf_sources):
    """Bind the source-backed capacity record without inventing a witness hash."""
    if type(bundle) is not dict or bundle.get('schema')!=PROSPECTIVE_MODE:
        # Preserve the independently reviewed metadata-only input contract.
        # Actual production constructors never call this prospective function.
        fixed._pin_tuple(bundle)
        if bundle['bytes']!=ROLE_MAXIMA[68]:
            raise ValueError('fixed40 prospective full bundle maximum')
        return bundle['bytes']
    expected={'schema','status','role','accepted','ready','new_facts','emitted_native_body',
        'output_path','native_format','native_source','raw_inputs','prior_bundle',
        'maximum_output_bytes','actual_capacity_exercised','payload_tuples',
        'constructor_proofs','current_proofs','fresh_prior_proofs','mandatory_actual_consumers'}
    raw_inputs=[*authority['six_input_owner']['inputs'],authority['prior_first_d']['input']]
    if (set(bundle)!=expected or bundle['status']!=PROSPECTIVE_MODE or bundle['role']!=68
            or bundle['accepted'] is not False or bundle['ready'] is not False
            or bundle['new_facts']!=0 or bundle['emitted_native_body'] is not False
            or bundle['native_format']!='RCV6' or bundle['payload_tuples']!=40
            or bundle['constructor_proofs']!=51 or bundle['current_proofs']!=91
            or bundle['fresh_prior_proofs']!=71
            or bundle['maximum_output_bytes']!=ROLE_MAXIMA[68]
            or bundle['actual_capacity_exercised']!=ROLE_MAXIMA[68]
            or bundle['raw_inputs']!=raw_inputs or bundle['prior_bundle']!=raw_inputs[-1]
            or sum(row.get('path')=='vrf_shared_window.rs'
                and all(row.get(key)==bundle['native_source'][key] for key in ('bytes','sha256'))
                for row in vrf_sources)!=1
            or bundle['native_source']['path']!='/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/src/vrf_shared_window.rs'
            or bundle['mandatory_actual_consumers']!=['native serializer',
                'full body SHA256 and exact EOF','all91 current proofs','fresh71 prior proofs',
                'both full replays','independent tuple/fact/seal acceptance']):
        raise ValueError('fixed40 complete source-backed pending bundle capacity')
    records.canonical_path(bundle['output_path'])
    return bundle['maximum_output_bytes']


def guard_deferred_binding(*, reference_source, inner_argv, template_path):
    """Closed non-executable outer-guard plan; never an installed command."""
    fixed._pin_tuple(reference_source);_command_argv(inner_argv)
    records.canonical_path(template_path)
    if reference_source['path']!='/workspaces/rustaxa-evm/experiments/evm_feasibility/snapshot_qualifier/tools/pa1_fixed40.py':
        raise ValueError('fixed40 deferred guard exact source reference')
    return dict(schema='fixed40-deferred-outer-guard-v1',profile=FIXED40_PROFILE,
        implementation_present=False,executable=False,accepted=False,
        reference_source=reference_source,
        reference_scope='REFERENCE_ONLY_NO_INSTALLED_E946_GUARD',
        inner_argv=inner_argv,cwd='/workspaces/rustaxa-evm',
        environment=dict(FIXED40_ENVIRONMENT),
        invocation_template=dict(path=template_path,input_channel='--request-file',
            ready_input_slot=dict(name='ready',type='complete-authenticated-pin',
                binds=['template','freeze','inner_argv']),
            request_fields=['template','freeze','ready','source_acceptance','preflight',
                'attempt_marker','schedule','counter','inner_argv']),
        bounds=dict(request_bytes=131072,result_bytes=16384,log_bytes=65536,
            guard_source_bytes=1048576,current_source_bytes=4194304,
            semantic_bytes=2097152,chunk_bytes=65536,deadline_seconds=300,
            binary_file_bytes=268435456,binary_pair_bytes=440401920,
            json_depth=8,json_nodes=8192,path_bytes=4096),
        required_checks=['full source and frozen gate acceptance',
            'full template/freeze/READY/inner-argv equality',
            'full original bodies, parsers, hashes and EOF',
            'fresh native preflight and prior71 proofs',
            'all16 accepted offline fits and unchanged finite read schedule',
            'normal-hook source commit and independent READY',
            'exact source-copy metadata before and after',
            'atomic one-use durable attempt admission before child launch',
            'before-open counter debit for every original occurrence',
            'bounded actual child stdout/stderr/exit/failure/closeout capture'])


def _checked_guard_deferred(value):
    if type(value) is not dict:raise ValueError('fixed40 closed deferred guard')
    expected=guard_deferred_binding(reference_source=value.get('reference_source'),
        inner_argv=value.get('inner_argv'),
        template_path=value.get('invocation_template',{}).get('path'))
    if value!=expected:raise ValueError('fixed40 closed deferred guard')
    return value


def prospective_exact_commands(*,maxima,freeze,freeze_pin,build,capture,
        lead_replay,independent_replay,outcomes,guard_deferred,live_bytes):
    """Keep every role59 field while separating absent outer and native inner."""
    guard=_checked_guard_deferred(guard_deferred)
    if capture is not None or guard['inner_argv']!=freeze['capture_command']:
        raise ValueError('fixed40 absent outer command cannot be executable argv')
    # Reuse all existing field/pin/limit checks; the temporary inner argv is
    # never published as an outer command or execution observation.
    raw=exact_commands(maxima=maxima,freeze=freeze,freeze_pin=freeze_pin,
        build=build,capture=freeze['capture_command'],lead_replay=lead_replay,
        independent_replay=independent_replay,outcomes=outcomes,
        live_bytes=live_bytes+fixed.graph_charge(guard))
    if (live_bytes+fixed.graph_charge((guard,raw,freeze,outcomes))
            +3*len(raw)+160*core._wire_nodes(raw)+3*_role_maximum(59,maxima)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 deferred commands decode lifetime')
    value=records.decode(raw)
    value.update(schema=PROSPECTIVE_MODE,capture=None,guardDeferred=guard)
    return _prospective_output(59,value,(freeze,freeze_pin,build,capture,
        lead_replay,independent_replay,outcomes,guard),maxima,live_bytes)


def prospective_runtime_command_result(role, *, maxima, live_bytes, **planned):
    """Keep every command-result field, with an impossible successful exit."""
    if any(k in planned for k in ('exit_code','child_exit','seconds','wall_timeout','output_overflow')):
        raise ValueError('fixed40 prospective command cannot supply observed completion')
    guard=planned.pop('guard_deferred',None)
    if guard is not None:
        _checked_guard_deferred(guard)
        if role not in (64,73) or planned.get('command') is not None:
            raise ValueError('fixed40 deferred outer result role/command')
        planned['command']=guard['inner_argv']
    raw=runtime_command_result(role,maxima=maxima,live_bytes=live_bytes+fixed.graph_charge(guard),
        exit_code=2147483647,child_exit=2147483647,seconds=300,
        wall_timeout=False,output_overflow=False,**planned)
    value=records.decode(raw,16384)
    value.update(mode=PROSPECTIVE_MODE,executed=False)
    if guard is not None:
        value.update(command=None,guardDeferred=guard)
    if (live_bytes+fixed.graph_charge((planned,raw,value,guard))+3*_role_maximum(role,maxima)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 complete prospective runtime result lifetime')
    return _constructor_output(value,_role_maximum(role,maxima))


PROSPECTIVE_LEAD_FIELDS=frozenset(('schema','status','accepted','verified','new_facts',
    'profile','bundle','capture_report','lead_report','independent_report',
    'independent_verification','constructor_proofs','payload_proofs','current_proofs',
    'fresh_prior_proofs','prior_payload_bindings','omitted_delayed_indices',
    'all40_members_usable','before_after_metadata_equal','full_database_preservation_proven',
    'report_byte_equal','selection_sha256','qualification','capture_usage','replay_usage',
    'physical_ceilings','phase_peaks','value_bytes','value_hex','value_sha256'))
PROSPECTIVE_REVERSE_FIELDS=frozenset(('schema','kind','status','outcomes','pins',
    'total_occurrences','total_bytes','binary_pair_bytes','source_commit',
    'current_binary_source_commit','historical_binary_source_commit','source_count',
    'tool_count','source_provenance','capture_parent','historical_boundary',
    'physical_read_scope','read_accounting','semantic_peaks','caps',
    'binary_post_stat_equal','physical_capture','fact_acceptance','new_facts'))


def prospective_verification_record(role, *, maxima, fields, reports, source_scope,
                                    historical_raw, unit, declarations, live_bytes):
    """Run the closed pending consumers for76/77/78, retaining their full graphs.

    These records have the complete original field roster but no observed native
    completion or proof verdict. All known input relations and full body reads
    still run. Production constructors remain separate and reject this schema.
    """
    if role not in (76,77,78):raise ValueError('fixed40 pending verification role')
    maximum=_role_maximum(role,maxima)
    expected=(INDEPENDENT_VERIFICATION_FIELDS if role==76 else
              PROSPECTIVE_LEAD_FIELDS if role==77 else PROSPECTIVE_REVERSE_FIELDS)
    inputs=(fields,reports,source_scope,historical_raw,declarations)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 pending verification caller')
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    if (type(fields) is not dict or set(fields)!=expected
            or fields['schema']!=PROSPECTIVE_MODE or fields['status']!=PROSPECTIVE_MODE
            or fields['new_facts']!=0):
        raise ValueError('fixed40 full pending verification schema')
    if owner+live_bytes+fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 pending verification input lifetime')
    pins=[];kinds=[]
    if role in (76,77):
        if (fields['accepted'] is not False or fields['verified'] is not False
                or fields['profile']!=FIXED40_PROFILE
                or fields['constructor_proofs']!=51 or fields['payload_proofs']!=40
                or fields['current_proofs']!=91 or fields['fresh_prior_proofs']!=71
                or fields['prior_payload_bindings']!=20
                or fields['omitted_delayed_indices']!=list(fixed.OMISSIONS)
                or any(fields['qualification'].values())
                or fields['full_database_preservation_proven'] is not False):
            raise ValueError('fixed40 pending proof expectations grant no verification')
        names=(('capture_report','lead_report','independent_report') if role==76 else
               ('capture_report','lead_report','independent_report','independent_verification'))
        if type(reports) is not dict or set(reports)!=set(names):
            raise ValueError('fixed40 full three pending native reports')
        if reports['lead_report']!=reports['independent_report']:
            raise ValueError('fixed40 pending complete replay graphs differ')
        for name in names:
            raw=reports[name];pin=fields[name];fixed._pin_tuple(pin)
            if (type(raw) is not bytes or len(raw)!=pin['bytes']
                    or hashlib.sha256(raw).hexdigest()!=pin['sha256']):
                raise ValueError('fixed40 complete pending native report identity')
            pins.append(pin);kinds.append('evidence')
        if role==76:
            _source_lists(source_scope['sources'],source_scope['tool_sources'])
            observations=fields['source_observations']
            if (type(observations) is not list or len(observations)!=63
                    or fields['source_occurrences']!=63
                    or fields['source_occurrence_bytes']!=sum(r['canonical']['bytes'] for r in observations)
                    or fields['source_count']!=90 or fields['tool_count']!=11
                    or fields['binary_count']!=2
                    or fields['exact_eof_verified'] is not False
                    or fields['canonical_reports_byte_equal'] is not False
                    or fields['metadata_before_after_equal'] is not False
                    or fields['guarded_capture'] is not False):
                raise ValueError('fixed40 complete pending source/native observations')
            controls=[source_scope['source_freeze'],source_scope['tool_source_freeze'],
                *(fields[n] for n in ('freeze','independent_result','independent_log','live_result'))]
            pins.extend(controls);kinds.extend(['evidence']*len(controls))
            pins.append(fields['replay_binary']);kinds.append('binary')
            pins.extend(r['canonical'] for r in observations);kinds.extend(['source']*63)
            raw_inputs=fields['raw_inputs']
            if (type(raw_inputs) is not list or len(raw_inputs)!=7
                    or fields['raw_artifact_count']!=7 or fields['raw_artifact_bytes']!=6785636):
                raise ValueError('fixed40 complete pending raw authority')
            pins.extend(raw_inputs);kinds.extend(['evidence']*7)
        else:
            for name in ('all40_members_usable','before_after_metadata_equal','report_byte_equal'):
                if fields[name] is not False:raise ValueError('fixed40 pending lead cannot claim proof outcome')
    else:
        if (type(historical_raw) is not dict or set(historical_raw)!={'predecessor','matrix'}
                or fields['physical_capture'] is not False or fields['fact_acceptance'] is not False
                or fields['binary_post_stat_equal'] is not False
                or fields['source_count']!=90 or fields['tool_count']!=11
                or type(fields['outcomes']) is not list or len(fields['outcomes'])!=5
                or type(fields['pins']) is not list or len(fields['pins'])!=24):
            raise ValueError('fixed40 complete five pending reverse outcomes')
        for name,pin in (('predecessor',REVERSE_PREDECESSOR),('matrix',REVERSE_MATRIX)):
            raw=historical_raw[name]
            if type(raw) is not bytes or len(raw)!=pin['bytes'] or hashlib.sha256(raw).hexdigest()!=pin['sha256']:
                raise ValueError('fixed40 exact full pending reverse source')
            pins.append(pin);kinds.append('evidence')
        for row in fields['outcomes']:
            if (type(row) is not dict or row.get('status')!=PROSPECTIVE_MODE
                    or row.get('executed') is not False):
                raise ValueError('fixed40 future reverse is never observed execution')
        # The whole retained historical boundary remains mandatory. Its exact
        # source pin is authenticated above; every original nonbinary route is
        # charged again, followed by all24 current occurrences and two binaries.
        historical=reverse_historical_body_pins(fields['historical_boundary'])
        pins.extend(historical);kinds.extend(['evidence']*len(historical))
        pins.extend(fields['pins']);kinds.extend(['binary']*2+['evidence']*22)
    if len(declarations)!=len(pins):raise ValueError('fixed40 full pending verifier read closure')
    for pin,kind,declaration in zip(pins,kinds,declarations):
        fixed._pin_tuple(pin)
        occurrence,physical,actual_kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(pin) or actual_kind!=kind or mode!='bytes':
            raise ValueError('fixed40 exact pending verifier source/body declaration')
        unit.consume(occurrence,pin,kind=kind,live_bytes=live_bytes+fixed.graph_charge(inputs))
    if role in (76,77):
        # Decode one complete report graph at a time, under its own occurrence.
        for name,index in zip(('capture_report','lead_report'),(0,1)):
            raw=reports[name]
            report=fixed._inline_decode(unit,declarations[index][0],raw,
                live_bytes+fixed.graph_charge(inputs))
            if (report.get('schema')!=PROSPECTIVE_MODE or report.get('status')!=PROSPECTIVE_MODE
                    or report['profile']!=FIXED40_PROFILE or len(report['selection'])!=40
                    or report['selection_sha256']!=_native_selection_digest(report['selection'])
                    or fields['selection_sha256']!=report['selection_sha256']
                    or fields['bundle']!=report['bundle']
                    or fields['qualification']!=report['qualification']
                    or any(report['qualification'].values())):
                raise ValueError('fixed40 pending full native report semantics')
            if name=='lead_report':
                if fields['selection_sha256']!=report['selection_sha256']:
                    raise ValueError('fixed40 pending verifier full selection binding')
                if role==76 and (fields['values']!=report['values']
                        or fields['replay_usage']!=report['actual']
                        or fields['prior_usage']!=report['authority']['prior_first_d']['actual']):
                    raise ValueError('fixed40 pending complete native value/resource binding')
                if role==76:
                    frozen=source_scope['sources']+source_scope['tool_sources']
                    position=0
                    for group,originals in (('vrf_sources',report['vrf_sources']),
                            ('owner_sources',report['authority']['six_input_owner']['sources']),
                            ('aspen_sources',report['authority']['six_input_owner']['aspen_sources'])):
                        for original in originals:
                            if position>=63:raise ValueError('fixed40 pending source occurrence excess')
                            observation=fields['source_observations'][position];position+=1
                            if (set(observation)!={'canonical','direct_frozen','group','original'}
                                    or observation['original']!=original or observation['group']!=group
                                    or type(observation['direct_frozen']) is not bool
                                    or observation['direct_frozen']!=(observation['canonical'] in frozen)
                                    or any(observation['canonical'][key]!=original[key] for key in ('bytes','sha256'))):
                                raise ValueError('fixed40 exact pending63 source bindings')
                    expected_raw=[*report['authority']['six_input_owner']['inputs'],
                                  report['authority']['prior_first_d']['input']]
                    if position!=63 or fields['raw_inputs']!=expected_raw:
                        raise ValueError('fixed40 exact pending seven authority inputs')
                    del frozen,expected_raw
                else:
                    verification=fixed._inline_decode(unit,declarations[3][0],
                        reports['independent_verification'],live_bytes+fixed.graph_charge((inputs,report)))
                    if (set(verification)!=INDEPENDENT_VERIFICATION_FIELDS
                            or verification['schema']!=PROSPECTIVE_MODE or verification['status']!=PROSPECTIVE_MODE
                            or verification['verified'] is not False or verification['accepted'] is not False
                            or verification['new_facts']!=0 or verification['source_count']!=90
                            or verification['tool_count']!=11 or verification['binary_count']!=2
                            or any(verification[key]!=fields[key] for key in
                                ('bundle','capture_report','lead_report','independent_report','selection_sha256'))
                            or verification['values']!=report['values']
                            or verification['replay_usage']!=report['actual']
                            or verification['prior_usage']!=report['authority']['prior_first_d']['actual']
                            or verification['qualification']!=report['qualification']):
                        raise ValueError('fixed40 complete pending independent-to-lead dependency')
                    del verification
            del report
    else:
        parent_raw=historical_raw['predecessor']
        begin,end=fixed._field(parent_raw,'historical_boundary')
        boundary=fixed._inline_decode(unit,declarations[0][0],parent_raw[begin:end],
            live_bytes+fixed.graph_charge(inputs))
        if fields['historical_boundary']!=boundary:
            raise ValueError('fixed40 complete original reverse boundary retained')
        del boundary
        begin,end=fixed._field(parent_raw,'pins')
        original_pins=fixed._inline_decode(unit,declarations[0][0],parent_raw[begin:end],
            live_bytes+fixed.graph_charge(inputs))
        if fields['pins'][1]!=original_pins[1]:
            raise ValueError('fixed40 original Fbb active binary binding')
        del original_pins
        matrix=fixed._inline_decode(unit,declarations[1][0],historical_raw['matrix'],
            live_bytes+fixed.graph_charge(inputs))
        if (matrix.get('schema')!='fixed40-five-reverse-instance-matrix-v1'
                or len(matrix.get('matrix',()))!=5
                or any(row.get('name')!=rule['new_name']
                    for row,rule in zip(fields['outcomes'],matrix['matrix']))):
            raise ValueError('fixed40 all five source-backed pending reverse identities')
        del matrix
    return _prospective_output(role,fields,inputs,maxima,owner+live_bytes)


def _native_selection_digest(selection):
    """Match Rust fields::normalized_digest: sorted compact UTF8, no newline."""
    raw=json.dumps(selection,sort_keys=True,separators=(',',':'),
                   ensure_ascii=False,allow_nan=False).encode('utf-8')
    return hashlib.sha256(raw).hexdigest()


def _native_window_value(role, *, maxima, selection, authority, vrf_sources,
                         bounds, slot, context=None, bundle=None, actual=None,
                         values=None, live_bytes, prospective=False):
    """Build complete successful native reports from actual producer fields.

    The Rust producer owns authentication and proof execution. These complete
    reports preserve its authority/source/bound bodies and all40 bindings.
    They do not accept facts or authorize capture. Stopped reports require a
    separate diagnostic constructor; no failed prefix is promoted here.
    """
    if role not in (54,69,72,75):raise ValueError('fixed40 native report role')
    inputs=(selection,authority,vrf_sources,bounds,slot,context,bundle,actual,values)
    if prospective:
        maximum=_role_maximum(role,maxima)
        core.integer(live_bytes,core.SEMANTIC,'fixed40 prospective native caller')
        # The complete native root references these retained input graphs;
        # it does not deep-copy them. Reserve three full role buffers for the
        # new root/scalars and encoder, then check the complete encoder graph.
        if (live_bytes+fixed.graph_charge(inputs)+3*maximum
                +core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 prospective native input lifetime')
    else:
        maximum=_constructor_lifetime(role,maxima,inputs,live_bytes)
    if (type(selection) is not list or len(selection)!=40
            or type(slot) is not dict or len(slot.get('rows',()))!=40
            or slot.get('voter')!=fixed.VOTER or slot.get('payloads')!=40
            or slot.get('full_proofs')!=91):
        raise ValueError('fixed40 exact native selection')
    for row,expected in zip(selection,slot['rows']):
        full=expected['full_tuple'];t=row.get('tuple',{});source=row.get('source_row',{})
        if (set(row)!={'parent_manifest_index','source_row','tuple'}
                or row['parent_manifest_index']!=expected['parent_manifest_index']
                or t!={'identity':{'period':full['period'],'state_root_hex':full['state_root_hex']},
                       'address_hex':full['account_hex'],
                       'logical_key_hex':full['logical_key_hex'],
                       'key_preimage_hex':full['key_preimage_hex']}
                or set(source)!={'delayed_period','proof','qualified','semantic_key',
                                 'source_references','value','voter_hex'}
                or source['delayed_period']!=full['period']
                or source['voter_hex']!=fixed.VOTER or source['qualified'] is not False
                or source['proof'] is not None or source['value'] is not None
                or source['semantic_key']!='fe_vrf_00_04_address'
                or source['source_references']!=expected['source_references']):
            raise ValueError('fixed40 complete six-field native tuple binding')
    if (type(authority) is not dict or set(authority)!=
            {'prior_first_d','six_input_owner','raw_artifact_reads','raw_artifact_bytes'}
            or authority['raw_artifact_reads']!=7 or authority['raw_artifact_bytes']!=6785636):
        raise ValueError('fixed40 complete native raw-seven authority')
    prior=authority['prior_first_d'];selected=prior.get('selected',{})
    if (set(prior)!={'actual','input','selected','semantic_sha256'}
            or selected.get('proof_index')!=68 or selected.get('voter_hex')!=fixed.VOTER
            or selected.get('usable_vrf_key') is not True
            or selected.get('identity',{}).get('period')!=25706894):
        raise ValueError('fixed40 fresh selected prior proof binding')
    _native_usage(prior['actual']);fixed._pin_tuple(prior['input'])
    if (prior['input']['bytes']!=417594 or prior['input']['sha256']!=
            '6382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61'
            or selected.get('value_hex')!='31905ba6089466e9b8aabd880d54b4a80f4d38ba9e9db942a4f64aa68f9caeb5'):
        raise ValueError('fixed40 exact native prior input/value')
    owner=authority['six_input_owner']
    if (type(owner) is not dict or set(owner)!=
            {'actual','aspen_sources','bounds','executable','inputs','sources','voter_manifest'}
            or len(owner['inputs'])!=6 or len(owner['sources'])!=25
            or len(owner['aspen_sources'])!=16):
        raise ValueError('fixed40 complete native six-input source authority')
    if type(vrf_sources) is not list or len(vrf_sources)!=22:
        raise ValueError('fixed40 complete22 embedded native sources')
    for pin in vrf_sources:
        if (type(pin) is not dict or set(pin)!={'path','bytes','sha256'}
                or type(pin['path']) is not str or not pin['path']
                or re.fullmatch('[0-9a-f]{64}',pin['sha256']) is None):
            raise ValueError('fixed40 complete embedded source pin')
        core.integer(pin['bytes'],1048576,'fixed40 embedded source file cap')
    if (type(bounds) is not dict or set(bounds)!=
            {'authority_input_bytes','bundle_bytes','exclusions','executable_bytes',
             'executable_hash_buffer_bytes','payload_tuples_exact','proofs_exact',
             'raw_artifacts_exact','report_bytes','storage_resources'}
            or bounds.get('payload_tuples_exact')!=40 or bounds.get('proofs_exact')!=91
            or bounds.get('raw_artifacts_exact')!=7
            or bounds.get('authority_input_bytes')!=6785636
            or bounds.get('report_bytes')!=131072 or bounds.get('bundle_bytes')!=134217728
            or bounds.get('executable_bytes')!=268435456
            or bounds.get('executable_hash_buffer_bytes')!=8192
            or bounds['storage_resources']!=[dict(resource=n,limit=v) for n,v in NATIVE_RESOURCE_LIMITS]):
        raise ValueError('fixed40 unchanged native producer bounds')
    qualification=dict(vrf_family_qualified=not prospective and role in (72,75),voter_weights_qualified=False,
        reward_cache_equal=False,owner_ready=False,producer_verified=False,
        production_routing_authorized=False)
    value=dict(schema=1,profile=FIXED40_PROFILE,selection=selection,
        selection_sha256=_native_selection_digest(selection),
        authority=authority,vrf_sources=vrf_sources,bounds=bounds,
        qualification=qualification)
    if role==54:
        if (type(context) is not dict or set(context)!=
                {'head','parent','delayed','genesis_hash_hex','head_header_sha256'}
                or len(context['delayed'])!=48 or any(x is not None for x in (bundle,actual,values))):
            raise ValueError('fixed40 complete native preflight context')
        value.update(context);value['status']='authority_inputs_authenticated_no_database_open'
    else:
        if context is not None:raise ValueError('fixed40 native report context placement')
        if prospective:
            _prospective_bundle_capacity(bundle,authority,vrf_sources)
        else:
            fixed._pin_tuple(bundle)
        _native_usage(actual)
        value.update(bundle=bundle,actual=actual)
        if role==69:
            if values is not None:raise ValueError('fixed40 capture cannot invent replay values')
            value['status']='captured_requires_offline_replay'
        else:
            if type(values) is not list or len(values)!=40:
                raise ValueError('fixed40 complete40 replay values')
            for index,item in enumerate(values):
                if (set(item)!={'logical','physical','proof_index','usable_vrf_key','value_hex'}
                        or item['proof_index']!=51+index or type(item['usable_vrf_key']) is not bool):
                    raise ValueError('fixed40 complete native replay value binding')
            value.update(status='all_forty_residual_vrf_inputs_authenticated',values=values)
    if prospective:
        value['schema']=PROSPECTIVE_MODE
        value['status']=PROSPECTIVE_MODE
    return value


def native_window_report(role, *, maxima, selection, authority, vrf_sources,
                         bounds, slot, context=None, bundle=None, actual=None,
                         values=None, live_bytes):
    """Encode the unchanged complete actual native producer contract."""
    value=_native_window_value(role,maxima=maxima,selection=selection,authority=authority,
        vrf_sources=vrf_sources,bounds=bounds,slot=slot,context=context,bundle=bundle,
        actual=actual,values=values,live_bytes=live_bytes)
    return _constructor_output(value,_role_maximum(role,maxima))




def _prospective_replay_values(selection):
    """Choose the widest complete valid replay-value shape for each fixed row.

    vrf_witness::decode_values permits member32 or nonmember. The physical
    provenance in vrf_shared_window::replay_values can be present32, tombstone
    or HistoryUnavailable with the full requested identity. The unavailable
    nonmember row is wider than the member row for this fixed40 selection.
    These are unexecuted capacity records; they grant no usable-key proof.
    """
    if type(selection) is not list or len(selection)!=40:
        raise ValueError('fixed40 prospective full40 value domains')
    values=[]
    for i,row in enumerate(selection):
        identity=row.get('tuple',{}).get('identity',{})
        if (type(identity) is not dict or set(identity)!={'period','state_root_hex'}
                or type(identity['period']) is not int or not 0<=identity['period']<2**64
                or type(identity['state_root_hex']) is not str
                or re.fullmatch('[0-9a-f]{64}',identity['state_root_hex']) is None):
            raise ValueError('fixed40 prospective physical history identity')
        member=dict(logical='member',physical=dict(kind='present',bytes=32,sha256='0'*64),
            proof_index=51+i,usable_vrf_key=False,value_hex='0'*64)
        tombstone=dict(logical='nonmember',physical=dict(kind='tombstone'),
            proof_index=51+i,usable_vrf_key=False,value_hex=None)
        unavailable=dict(logical='nonmember',physical=dict(
            kind='successful_seek_without_matching_version',identity=dict(identity)),
            proof_index=51+i,usable_vrf_key=False,value_hex=None)
        # Complete canonical records, not whitespace or width-only substitutes.
        values.append(max((member,tombstone,unavailable),key=lambda v:len(core.canonical(v))))
    return values

def _prospective_native_value(role,*,maxima,selection,authority,vrf_sources,bounds,
                              slot,context,bundle,source_observations,frozen_sources,
                              unit,declarations,live_bytes):
    """Construct complete native capacity fields with zero proof authority.

    All seven immutable raw inputs and all63 source occurrences are full reads,
    including duplicate sources. The same native input/selection/bound checks
    serve both encoders. Capacity resource counters are ceilings, not measured
    use. Replay value widths exercise the complete40-row field graph; false
    usability and the prospective schema/status cannot certify any proof.
    """
    if role not in (54,69,72,75):raise ValueError('fixed40 prospective native role')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 prospective native external caller')
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    inputs=(selection,authority,vrf_sources,bounds,slot,context,bundle,
            source_observations,frozen_sources,declarations)
    # Inputs are retained references, not a deep copy. Reserve the complete
    # retained graph once and three full role buffers before any input read.
    # The native field builder and encoder separately check their live graphs.
    if (full+fixed.graph_charge(inputs)+3*_role_maximum(role,maxima)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective native complete input preallocation')
    if type(source_observations) is not list or len(source_observations)!=63:
        raise ValueError('fixed40 prospective all63 source occurrences')
    if type(frozen_sources) is not list or len(frozen_sources)!=101:
        raise ValueError('fixed40 prospective current101 source closure')
    for pin in frozen_sources:fixed._pin_tuple(pin)
    position=0
    for group,rows in (('vrf_sources',vrf_sources),
            ('owner_sources',authority['six_input_owner']['sources']),
            ('aspen_sources',authority['six_input_owner']['aspen_sources'])):
        for original in rows:
            if position>=63:raise ValueError('fixed40 prospective native source excess')
            row=source_observations[position];position+=1
            if (set(row)!={'canonical','direct_frozen','group','original'}
                    or row['group']!=group or row['original']!=original
                    or type(row['direct_frozen']) is not bool
                    or row['direct_frozen']!=(row['canonical'] in frozen_sources)
                    or any(row['canonical'][k]!=original[k] for k in ('bytes','sha256'))):
                raise ValueError('fixed40 prospective complete source occurrence binding')
    if position!=63:raise ValueError('fixed40 prospective native source count')
    raw_inputs=(*authority['six_input_owner']['inputs'],authority['prior_first_d']['input'])
    pins=(*raw_inputs,*(row['canonical'] for row in source_observations))
    if len(raw_inputs)!=7 or sum(p['bytes'] for p in raw_inputs)!=6785636:
        raise ValueError('fixed40 prospective full raw-seven input binding')
    _prospective_reads(unit,pins,declarations,live_bytes+fixed.graph_charge(inputs))
    source_hold=fixed.graph_charge((source_observations,frozen_sources,declarations))
    actual=values=None
    if role!=54:
        capacity=_prospective_bundle_capacity(bundle,authority,vrf_sources)
        actual=dict(admitted_input_bytes=capacity,resources=[
            dict(resource=name,used=limit) for name,limit in NATIVE_RESOURCE_LIMITS])
        if role in (72,75):
            values=_prospective_replay_values(selection)
    value=_native_window_value(role,maxima=maxima,selection=selection,authority=authority,
        vrf_sources=vrf_sources,bounds=bounds,slot=slot,context=context,bundle=bundle,
        actual=actual,values=values,prospective=True,
        live_bytes=full+source_hold)
    return value


def _prospective_native_owned_value(role,owned,*,maxima,unit,declarations,live_bytes):
    """Construct complete native capacity fields with zero proof authority.

    All seven immutable raw inputs and all63 source occurrences are full reads,
    including duplicate sources. The same native input/selection/bound checks
    serve both encoders. Capacity resource counters are ceilings, not measured
    use. Replay value widths exercise the complete40-row field graph; false
    usability and the prospective schema/status cannot certify any proof.
    """
    if (type(owned) is not dict
            or set(owned)!=set(NATIVE_PROSPECTIVE_WIRE_FIELDS)):
        raise ValueError('fixed40 exact native owned argument mapping')
    selection=owned.pop('selection')
    authority=owned.pop('authority')
    vrf_sources=owned.pop('vrf_sources')
    bounds=owned.pop('bounds')
    slot=owned.pop('slot')
    context=owned.pop('context')
    bundle=owned.pop('bundle')
    source_observations=owned.pop('source_observations')
    frozen_sources=owned.pop('frozen_sources')
    if role not in (54,69,72,75):raise ValueError('fixed40 prospective native role')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 prospective native external caller')
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    inputs=(selection,authority,vrf_sources,bounds,slot,context,bundle,
            source_observations,frozen_sources,declarations)
    # Inputs are retained references, not a deep copy. Reserve the complete
    # retained graph once and three full role buffers before any input read.
    # The native field builder and encoder separately check their live graphs.
    if (full+fixed.graph_charge(inputs)+3*_role_maximum(role,maxima)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective native complete input preallocation')
    if type(source_observations) is not list or len(source_observations)!=63:
        raise ValueError('fixed40 prospective all63 source occurrences')
    if type(frozen_sources) is not list or len(frozen_sources)!=101:
        raise ValueError('fixed40 prospective current101 source closure')
    for pin in frozen_sources:fixed._pin_tuple(pin)
    position=0
    for group,rows in (('vrf_sources',vrf_sources),
            ('owner_sources',authority['six_input_owner']['sources']),
            ('aspen_sources',authority['six_input_owner']['aspen_sources'])):
        for original in rows:
            if position>=63:raise ValueError('fixed40 prospective native source excess')
            row=source_observations[position];position+=1
            if (set(row)!={'canonical','direct_frozen','group','original'}
                    or row['group']!=group or row['original']!=original
                    or type(row['direct_frozen']) is not bool
                    or row['direct_frozen']!=(row['canonical'] in frozen_sources)
                    or any(row['canonical'][k]!=original[k] for k in ('bytes','sha256'))):
                raise ValueError('fixed40 prospective complete source occurrence binding')
    if position!=63:raise ValueError('fixed40 prospective native source count')
    raw_inputs=(*authority['six_input_owner']['inputs'],authority['prior_first_d']['input'])
    pins=(*raw_inputs,*(row['canonical'] for row in source_observations))
    if len(raw_inputs)!=7 or sum(p['bytes'] for p in raw_inputs)!=6785636:
        raise ValueError('fixed40 prospective full raw-seven input binding')
    _prospective_reads(unit,pins,declarations,live_bytes+fixed.graph_charge(inputs))
    source_hold=fixed.graph_charge((source_observations,frozen_sources,declarations))
    # The sole owned mapping was emptied at entry. No expanded kwargs
    # packet or caller decoded roots can retain these two source graphs.
    source_observations=frozen_sources=None
    del inputs,pins,raw_inputs,row
    source_hold=fixed.graph_charge(declarations)
    # The immutable slot wire is unused by all70 source/input checks. Decode
    # only after the source-only graphs have actually left this owned frame.
    # argument_wires remains retained and fully charged by the caller.
    slot_raw=slot
    if (type(slot_raw) is not bytes or not slot_raw
            or len(slot_raw)>records.WRAPPER_CAP
            or full+fixed.graph_charge((selection,authority,vrf_sources,bounds,
                slot_raw,context,bundle,declarations))+2*len(slot_raw)
                +128*core._wire_nodes(slot_raw)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 native deferred slot predecode allocation')
    slot=records.decode(slot_raw,records.WRAPPER_CAP)
    del slot_raw
    actual=values=None
    if role!=54:
        capacity=_prospective_bundle_capacity(bundle,authority,vrf_sources)
        actual=dict(admitted_input_bytes=capacity,resources=[
            dict(resource=name,used=limit) for name,limit in NATIVE_RESOURCE_LIMITS])
        if role in (72,75):
            values=_prospective_replay_values(selection)
    value=_native_window_value(role,maxima=maxima,selection=selection,authority=authority,
        vrf_sources=vrf_sources,bounds=bounds,slot=slot,context=context,bundle=bundle,
        actual=actual,values=values,prospective=True,
        live_bytes=full+source_hold)
    return value


def prospective_native_report(role,*,maxima,selection,authority,vrf_sources,bounds,
                              slot,context,bundle,source_observations,frozen_sources,
                              unit,declarations,live_bytes):
    """Retain the original direct prospective API and its full live guards."""
    value=_prospective_native_value(role,maxima=maxima,selection=selection,
        authority=authority,vrf_sources=vrf_sources,bounds=bounds,slot=slot,
        context=context,bundle=bundle,source_observations=source_observations,
        frozen_sources=frozen_sources,unit=unit,declarations=declarations,
        live_bytes=live_bytes)
    inputs=(selection,authority,vrf_sources,bounds,slot,context,bundle,
            source_observations,frozen_sources,declarations)
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    return _prospective_output(role,value,inputs,maxima,full)


NATIVE_PROSPECTIVE_WIRE_FIELDS=(
    'selection','authority','vrf_sources','bounds','slot','context','bundle',
    'source_observations','frozen_sources')




def _prospective_role68_stopped(role,value,observed_bytes):
    """Model the complete E946 envelope-stop report, never a capture result.

    Capture encoding is at most404+RawCopyBytes. Replay rejects a u64 file
    length before reading it. Original native ceilings and all full authority
    fields remain; the prospective schema/status and false qualification stay.
    """
    maximum_observed=67109268 if role==69 else 18446744073709551615
    if (role not in (69,75) or type(observed_bytes) is not int
            or not ROLE_MAXIMA[68]<observed_bytes<=maximum_observed
            or value.get('schema')!=PROSPECTIVE_MODE
            or value.get('status')!=PROSPECTIVE_MODE
            or any(value['qualification'].values())):
        raise ValueError('fixed40 exact prospective role68 stopped domain')
    value.pop('bundle')
    value['failure']=dict(phase='Fixed40RoleEnvelope',
        reason='bundle exceeds fixed40 role68 envelope',
        observed_bytes=observed_bytes,limit_bytes=ROLE_MAXIMA[68])
    value['values']=None
    if role==75:
        # The source guard has not opened/admitted any replay bundle bytes.
        # Prior raw/proof/source usage remains complete in authority.
        value['actual']=dict(admitted_input_bytes=0,resources=[
            dict(resource=name,used=0) for name,limit in NATIVE_RESOURCE_LIMITS])
    else:
        # Explicit capacity ceiling, not an observed fresh execution counter.
        value['actual']['admitted_input_bytes']=134217728
    return value

def prospective_native_report_from_wires(role,*,argument_wires,maxima,unit,
                                         declarations,live_bytes,role68_stop=None):
    """Consume closed immutable input wires and release owned decoded roots.

    The caller must release its decoded argument graphs before this call, or
    include them in live_bytes. Only this function owns the decoded fields.
    Full seven-input and63-source reads are unchanged. No native proof or
    measured resource observation is generated by this format-only path.
    """
    core.integer(live_bytes,core.SEMANTIC,'fixed40 native wire external caller')
    if role68_stop is not None:
        upper=67109268 if role==69 else 18446744073709551615
        if (role not in (69,75) or type(role68_stop) is not int
                or not ROLE_MAXIMA[68]<role68_stop<=upper):
            raise ValueError('fixed40 exact prospective role68 stopped domain')
        live_bytes+=fixed.graph_charge(role68_stop)
    maximum=_role_maximum(role,maxima)
    if (role not in (54,69,72,75) or type(argument_wires) is not dict
            or set(argument_wires)!=set(NATIVE_PROSPECTIVE_WIRE_FIELDS)):
        raise ValueError('fixed40 exact nine native input wires')
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    external= fixed.graph_charge((argument_wires,declarations))
    owned={}
    for field in NATIVE_PROSPECTIVE_WIRE_FIELDS:
        raw=argument_wires[field]
        if (type(raw) is not bytes or not raw or len(raw)>records.WRAPPER_CAP
                or full+external+fixed.graph_charge(owned)+2*len(raw)
                    +128*core._wire_nodes(raw)+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 native owned field predecode allocation')
        # Retain the complete wire through the full source phase. The owned
        # callee performs the same strict decode after releasing source roots.
        owned[field]=raw if field=='slot' else records.decode(raw,records.WRAPPER_CAP)
    del raw,field
    value=_prospective_native_owned_value(role,owned,maxima=maxima,unit=unit,
        declarations=declarations,live_bytes=live_bytes+external)
    # This is an actual ownership transfer, not an accounting discount:
    # caller supplied immutable wires, and every callee argument root is now
    # removed. Fields retained by value remain fully charged below.
    owned.clear()
    del owned
    if role68_stop is not None:
        # The complete new fixed scalar graph is smaller than the already
        # reserved native output. Source-only roots have actually been released.
        value=_prospective_role68_stopped(role,value,role68_stop)
    return _prospective_output(role,value,(argument_wires,declarations,role68_stop),maxima,full)


def consume_prospective_native_report_from_wires(role,raw,*,report_declaration,
                                                 argument_wires,maxima,unit,
                                                 declarations,live_bytes,role68_stop=None):
    """Reconstruct the owned-wire report and read its complete output."""
    occurrence,physical,kind,mode=report_declaration
    pin=fixed._pin(physical)
    maximum=_role_maximum(role,maxima)
    if (kind!='evidence' or mode!='bytes' or type(raw) is not bytes
            or len(raw)>maximum or len(raw)!=pin['bytes']
            or hashlib.sha256(raw).hexdigest()!=pin['sha256']):
        raise ValueError('fixed40 prospective full native output binding')
    retained=fixed.graph_charge((raw,report_declaration,role68_stop))
    rebuilt=prospective_native_report_from_wires(role,argument_wires=argument_wires,
        maxima=maxima,unit=unit,declarations=declarations,
        live_bytes=live_bytes+retained,role68_stop=role68_stop)
    if rebuilt!=raw:
        raise ValueError('fixed40 prospective complete native reconstruction')
    live=live_bytes+fixed.graph_charge((raw,rebuilt,report_declaration,
                                      argument_wires,declarations,role68_stop))
    unit.consume(occurrence,pin,kind=kind,live_bytes=live)
    value=fixed._inline_decode(unit,occurrence,raw,live)
    if (value['schema']!=PROSPECTIVE_MODE or value['status']!=PROSPECTIVE_MODE
            or any(value['qualification'].values())):
        raise ValueError('fixed40 prospective native output has no proof authority')
    return value




def consume_prospective_native_report(role,raw,*,report_declaration,**arguments):
    """Authenticate and reconstruct the full closed prospective native body.

    The producer re-reads all seven raw inputs and all63 source occurrences;
    this consumer additionally reads the complete output occurrence. This is
    source-format validation only. It cannot authenticate a native proof or
    turn prospective resources into measured execution observations.
    """
    unit=arguments['unit'];live=arguments['live_bytes']
    maximum=_role_maximum(role,arguments['maxima'])
    occurrence,physical,kind,mode=report_declaration
    pin=fixed._pin(physical)
    if (kind!='evidence' or mode!='bytes' or type(raw) is not bytes
            or len(raw)>maximum or len(raw)!=pin['bytes']
            or hashlib.sha256(raw).hexdigest()!=pin['sha256']):
        raise ValueError('fixed40 prospective full native output binding')
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live
    if (full+fixed.graph_charge((arguments,raw,report_declaration))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective native consumer preallocation')
    retained=fixed.graph_charge((raw,report_declaration))
    rebuilt=prospective_native_report(role,**dict(arguments,live_bytes=live+retained))
    if rebuilt!=raw:
        raise ValueError('fixed40 prospective complete native reconstruction')
    unit.consume(occurrence,pin,kind=kind,
        live_bytes=live+fixed.graph_charge((arguments,raw,rebuilt,report_declaration)))
    value=fixed._inline_decode(unit,occurrence,raw,
        live+fixed.graph_charge((arguments,raw,rebuilt,report_declaration)))
    if (value['schema']!=PROSPECTIVE_MODE or value['status']!=PROSPECTIVE_MODE
            or any(value['qualification'].values())
            or (role in (72,75) and (len(value['values'])!=40
                or any(row['usable_vrf_key'] for row in value['values'])))):
        raise ValueError('fixed40 prospective native output has no proof authority')
    return value


def candidate_review(*, maxima, candidate, candidate_pin, preparation,
                     candidate_inputs, live_bytes):
    """Reconstruct all40 candidate tuples and all20 prior bindings for review.

    This complete offline comparison is a component result. Independent
    authority/layout acceptance and runtime proof execution remain pending.
    """
    maximum=_role_maximum(41,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 candidate review caller')
    # The output is a small scalar/pin review, while candidate_inputs retains
    # the complete map/prior context. Do not pretend that entire context is
    # copied into the review; reserve the separately reconstructed candidate.
    if (live_bytes+fixed.graph_charge((candidate,candidate_pin,preparation,candidate_inputs))
            +fixed.graph_charge(candidate)+3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 candidate review caller lifetime')
    fixed._pin_tuple(candidate_pin);fixed._pin_tuple(preparation)
    rebuilt=selected40_candidate(maxima=maxima,
        live_bytes=live_bytes+fixed.graph_charge((candidate,candidate_pin,preparation)),
        **candidate_inputs)
    if (type(candidate) is not dict or core.canonical(candidate)!=rebuilt
            or candidate_pin['bytes']!=len(rebuilt)
            or candidate_pin['sha256']!=hashlib.sha256(rebuilt).hexdigest()):
        raise ValueError('fixed40 complete reconstructed candidate equality')
    chosen=candidate['prior_context']['selected_prior']
    tuples=candidate['full_tuples']
    value=dict(status='OFFLINE_FIXED40_CANDIDATE_VERIFIED_AUTHORITY_PENDING',
        candidate=candidate_pin,preparation=preparation,
        voter=fixed.VOTER,original=914,accepted=805,residual=109,
        residual_if_later_proven=69,candidate_tuples=40,constructor_proofs=51,
        payload_proofs=40,bundle_proofs=91,fresh_prior_proofs=71,
        full_context_requests=48,distinct_roots=len({t['state_root_hex'] for t in tuples}),
        omissions=list(fixed.OMISSIONS),selected_prior_0_based=17,
        selected_prior_1_based=18,prior_parent_manifest_index=53,prior_proof_index=68,
        prior_value_hex=chosen['value_hex'],key_preimage_hex='0004'+fixed.VOTER,
        logical_key_hex=candidate['logical_key_hex'],
        all40_full_tuple_refs_match_raw=True,all20_prior_bindings_exactly_reconstructed=True,
        no_new_accepted_facts=True,no_source_or_command_authority=True,
        required_before_implementation='Complete original authority/body fit, independent source/command review and runtime proof acceptance remain mandatory.')
    return _constructor_output(value,maximum)


def independent_review_document(role, *, maxima, review_raw, review_pin,
                                evidence, unit, declarations, live_bytes):
    """Render the full authenticated review and every named evidence binding.

    Both document roles remain pending until their original independent
    semantic consumer accepts the exact commands or final fact/seal scope.
    Complete reviewer text is retained once, including diagnostics. No READY
    authority is generated by this renderer. A closed actual independent
    result can retain its verdict; the runtime must still verify its scope.
    """
    if role not in (60,79):raise ValueError('fixed40 independent review document role')
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    maximum=_constructor_lifetime(role,maxima,
        (review_raw,review_pin,evidence,declarations),owner+live_bytes)
    fixed._pin_tuple(review_pin)
    if (type(review_raw) is not bytes or len(review_raw)!=review_pin['bytes']
            or hashlib.sha256(review_raw).hexdigest()!=review_pin['sha256']):
        raise ValueError('fixed40 complete independent review body')
    if type(evidence) is not dict:
        raise ValueError('fixed40 complete review evidence bindings')
    expected=(('exact_commands','command_verification','freeze','execution_helper')
              if role==60 else ('lead_verification','independent_verification',
                               'reverse_outcome','fact_seal_inputs'))
    if set(evidence)!=set(expected):
        raise ValueError('fixed40 exact review evidence roles')
    pins=[review_pin,*[evidence[k] for k in expected]];kinds=['evidence']*len(pins)
    structured=review_raw.lstrip().startswith(b'{')
    authoritative=False
    review_verdict=None
    if structured:
        if (owner+live_bytes+fixed.graph_charge((review_raw,review_pin,evidence,declarations))
                +3*len(review_raw)+160*core._wire_nodes(review_raw)
                +3*maximum+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 independent actual review predecode lifetime')
        if not declarations:raise ValueError('fixed40 full review source declaration')
        result=fixed._inline_decode(unit,declarations[0][0],review_raw,
            live_bytes+fixed.graph_charge((review_raw,review_pin,evidence,declarations)))
        fields={'schema','role','profile','verdict','evidence','source_freeze',
                'required_gates','record_bounds','role_maxima','proofs','body'}
        if type(result) is not dict or set(result)!=fields:
            raise ValueError('fixed40 complete closed independent review result')
        authoritative=result['schema']=='fixed40-independent-review-result-v1'
        prospective=result['schema']=='fixed40-prospective-independent-review-scope-v1'
        expected_verdict=('READY' if role==60 else 'ACCEPT_FINAL_FAMILY') if authoritative else 'NOT_EXECUTED'
        if not (authoritative or prospective) or result['verdict']!=expected_verdict:
            raise ValueError('fixed40 complete closed independent review result')
        if (result['role']!=role or result['profile']!=FIXED40_PROFILE
                or result['evidence']!=evidence or result['role_maxima']!=list(maxima)
                or type(result['body']) is not str or not result['body']):
            raise ValueError('fixed40 complete closed independent review result')
        bounds=dict(aggregate_bytes=471859200,manifest_bytes=262144,max_json_depth=8,
            max_json_nodes=8192,max_path_bytes=4096,other_total_bytes=31457280,
            reference_body_bytes=1048576,reference_count_per_record=32,
            reference_count_total=64,reference_total_bytes=1048576,
            semantic_allocation_bytes=2097152,source_tool_total_bytes=4194304,
            stream_chunk_bytes=65536,wrapper_bytes=131072)
        if result['record_bounds']!=bounds:
            raise ValueError('fixed40 unchanged original complete review bounds')
        sources=result['source_freeze']
        if type(sources) is not list:raise ValueError('fixed40 full ordered101 review sources')
        _source_lists(sources[:90],sources[90:])
        gates=result['required_gates']
        if type(gates) is not list or len(gates)!=14:
            raise ValueError('fixed40 all14 actual review gate observations')
        pins.extend(sources);kinds.extend(['source']*101)
        for row,gate_role in zip(gates,VALIDATION_RESULTS):
            if set(row)!={'role','result','log'} or type(row['role']) is not int or row['role']!=gate_role:
                raise ValueError('fixed40 exact full gate observation order')
            pins.extend((row['result'],row['log']));kinds.extend(('evidence','evidence'))
        proof_names=(('preflight','candidate_review','candidate','prior_bundle','prior_report',
            'source_review','command_verification','command_closure','normal_hook','gate_equivalence')
            if role==60 else ('bundle','capture_report','lead_report','independent_report',
            'independent_verification','lead_verification','reverse_outcome','fact_seal_inputs'))
        if type(result['proofs']) is not dict or set(result['proofs'])!=set(proof_names):
            raise ValueError('fixed40 complete independent review proof bindings')
        for name in proof_names:pins.append(result['proofs'][name]);kinds.append('evidence')
        if (owner+live_bytes+fixed.graph_charge((review_raw,result,pins,declarations))
                +3*maximum+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 full review result caller lifetime')
        review_verdict=result['verdict']
        # All fields and original bindings have been checked. The original
        # authenticated body remains in review_raw; only its verdict is needed
        # after the body reads. Release the decoded parent/source/gate graphs.
        del result,bounds,proof_names,sources,gates,row,gate_role,name
    if len(declarations)!=len(pins):
        raise ValueError('fixed40 complete review original declarations')
    compact_extra=(128+sum(128+sum(fixed.graph_charge(pin[key])
        for key in ('path','bytes','sha256')) for pin in pins)+128+len(kinds))
    transfer_caller=live_bytes+fixed.graph_charge((review_raw,review_pin,evidence,
        declarations,pins,kinds,expected,review_verdict))
    if owner+transfer_caller+compact_extra+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 complete review pin closure transfer lifetime')
    pins=[fixed._pin_tuple(pin) for pin in pins]
    kinds=bytearray(kind=='source' for kind in kinds)
    body_caller=live_bytes+fixed.graph_charge((review_raw,review_pin,evidence,
        declarations,pins,kinds,expected,review_verdict))+4096
    if owner+body_caller+3*maximum+core.CHUNK+262144>core.SEMANTIC:
        raise ValueError('fixed40 complete retained review consume lifetime')
    for expected_pin,is_source,declaration in zip(pins,kinds,declarations):
        pin=dict(zip(('path','bytes','sha256'),expected_pin))
        kind_expected='source' if is_source else 'evidence'
        fixed._pin_tuple(pin)
        occurrence,physical,kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(pin) or kind!=kind_expected or mode!='bytes':
            raise ValueError('fixed40 exact review original declaration')
        unit.consume(occurrence,pin,kind=kind,
                     live_bytes=body_caller)
    if authoritative:
        if owner+body_caller+2*len(review_raw)+512+core.CHUNK+262144>core.SEMANTIC:
            raise ValueError('fixed40 review verdict serialization lifetime')
        # This is the actual independent review's verdict, not a caller flag.
        # Runtime must still check all full gates, source and native proof
        # semantics before honoring it. This renderer grants no authority.
        raw=review_verdict.encode()+b'\n'+review_raw
        if len(raw)>maximum:raise ValueError('fixed40 full actual review document cap')
        return raw
    text=review_raw.decode('utf-8')
    header=core.canonical(dict(schema=1,role=role,status='COMPONENT_PENDING_SEMANTIC_REVIEW',
        ready=False,accepted=False,new_facts=0,review=review_pin,evidence=evidence))
    raw=b'PENDING\n'+header+b'\n\n'+text.encode('utf-8')
    if len(raw)>maximum:raise ValueError('fixed40 full independent review document cap')
    return raw




def source_successor_mapping(*, maxima, accepted_freeze_raw, accepted_freeze,
                             accepted_review, source_commit, sources, tool_sources,
                             source_freeze, tool_source_freeze, cargo_inputs,
                             original_gate_pins, replay_input_proof,
                             tracked_dependency_proof, unresolved, live_bytes,
                             historical_sources=None,replay_input_source_raw=None):
    """Bind the unchanged accepted Rust90 scope and retain all reuse proof bodies.

    The current Python closure has no historical gate acceptance. Original
    gate records and tracked/build/replay proof bodies remain complete inputs
    for the later semantic consumers. This record grants no gate reuse itself.
    """
    inputs=(accepted_freeze_raw,accepted_freeze,accepted_review,source_commit,
        sources,tool_sources,source_freeze,tool_source_freeze,cargo_inputs,
        original_gate_pins,replay_input_proof,tracked_dependency_proof,unresolved,historical_sources,replay_input_source_raw)
    maximum=_constructor_lifetime(33,maxima,inputs,live_bytes)
    _source_lists(sources,tool_sources)
    for pin in (accepted_freeze,accepted_review,source_freeze,tool_source_freeze):
        fixed._pin_tuple(pin)
    if (type(accepted_freeze_raw) is not bytes or len(accepted_freeze_raw)!=17908
            or hashlib.sha256(accepted_freeze_raw).hexdigest()!=
                'be283dccbab3448bb727749b83615ac3326cbfe4ae0cb44772d5a43d6baf48ae'
            or accepted_freeze['bytes']!=len(accepted_freeze_raw)
            or accepted_freeze['sha256']!=hashlib.sha256(accepted_freeze_raw).hexdigest()
            or accepted_review['bytes']!=2821 or accepted_review['sha256']!=
                '1bc1edd4924538f692faa892d4cd8badba547db7b2ce773530c5eb8cb340e07f'):
        raise ValueError('fixed40 exact accepted Rust source component')
    if (live_bytes+fixed.graph_charge(inputs)+3*len(accepted_freeze_raw)
            +160*core._wire_nodes(accepted_freeze_raw)+3*maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 source successor predecode lifetime')
    accepted_sources=records.decode(accepted_freeze_raw)
    historical = sources if historical_sources is None else historical_sources
    if accepted_sources!=historical:
        raise ValueError('fixed40 unchanged complete accepted Rust90 scope')
    changed_scope = historical_sources is not None
    if changed_scope:
        if (type(historical_sources) is not list or len(historical_sources)!=90
                or [p['path'] for p in historical_sources]!=[p['path'] for p in sources]):
            raise ValueError('fixed40 ordered historical/current Rust scopes')
        for pin in historical_sources:fixed._pin_tuple(pin)
        # The replay-input proof predates TEN19. Its exact Rust88 roster is a
        # third historical scope; it must not be relabelled as accepted Rust90.
        replay_pin=dict(path='/home/fry/artifacts/vrf-round3-01a141ba-20261016/source/source-freeze.json',
            bytes=16958,sha256='f4dfbae8174ce3f876cbbe8875b6e17bb279e07632b54b5d2d89758e4a591cf7')
        if (replay_input_proof.get('source_freeze')!=replay_pin
                or type(replay_input_source_raw) is not bytes
                or len(replay_input_source_raw)!=replay_pin['bytes']
                or hashlib.sha256(replay_input_source_raw).hexdigest()!=replay_pin['sha256']):
            raise ValueError('fixed40 exact original historical replay source body')
        if (live_bytes+fixed.graph_charge((inputs,accepted_sources,historical))
                +2*len(replay_input_source_raw)+160*core._wire_nodes(replay_input_source_raw)
                +3*maximum+core.CHUNK+262144>core.SEMANTIC):
            raise ValueError('fixed40 original replay source predecode lifetime')
        replay_sources=records.decode(replay_input_source_raw)
        if type(replay_sources) is not list or len(replay_sources)!=88:
            raise ValueError('fixed40 original replay complete Rust88 scope')
        for pin in replay_sources:fixed._pin_tuple(pin)
        del replay_sources
    if re.fullmatch('[0-9a-f]{40}',source_commit) is None:
        raise ValueError('fixed40 source successor commit')
    if type(cargo_inputs) is not list or len(cargo_inputs)!=4:
        raise ValueError('fixed40 complete four Cargo inputs')
    cargo_paths=('experiments/evm_feasibility/snapshot_qualifier/Cargo.toml',
        'experiments/evm_feasibility/snapshot_qualifier/Cargo.lock','rust/Cargo.toml','rust/Cargo.lock')
    for pin,path in zip(cargo_inputs,cargo_paths):
        absolute='/workspaces/rustaxa-evm/'+path
        if (type(pin) is not dict or set(pin)!={'path','bytes','sha256'}
                or pin['path']!=path or not any(p['path']==absolute
                    and p['bytes']==pin['bytes'] and p['sha256']==pin['sha256'] for p in sources)):
            raise ValueError('fixed40 unchanged actual Cargo source binding')
    if type(original_gate_pins) is not list or len(original_gate_pins)!=7:
        raise ValueError('fixed40 all seven original source gate pairs')
    for gate,roles in zip(original_gate_pins,((0,1),(2,3),(4,5),(8,9),(14,15),(16,17),(18,19))):
        if set(gate)!={'gate','log','result','roles'} or gate['roles']!=list(roles):
            raise ValueError('fixed40 exact original source gate roles')
        fixed._pin_tuple(gate['log']);fixed._pin_tuple(gate['result'])
    if (type(replay_input_proof) is not dict or set(replay_input_proof)!=
            {'accepted','new_facts','no_execution','profiles','ready','schema','source_freeze','status'}
            or replay_input_proof['accepted'] is not False
            or replay_input_proof['ready'] is not False
            or replay_input_proof['new_facts']!=0
            or type(tracked_dependency_proof) is not dict or set(tracked_dependency_proof)!=
            {'accepted_freeze_base','current_head','interpretation','scoped_tracked_diff_from_base',
             'tracked_build_definition_diff_from_base','tracked_build_definition_scope'}
            or type(unresolved) is not dict or set(unresolved)!=
            {'bridge','bridge_roles','fast','fast_roles'}):
        raise ValueError('fixed40 complete original replay/dependency/unresolved bodies')
    value=dict(schema=1,status='COMPONENT_PENDING_SOURCE_REUSE_REVIEW',
        accepted=False,ready=False,new_facts=0,accepted_rust_freeze=accepted_freeze,
        accepted_rust_review=accepted_review,source_commit=source_commit,
        source_freeze=source_freeze,tool_source_freeze=tool_source_freeze,
        cargo_inputs=cargo_inputs,original_gate_pins=original_gate_pins,
        current_build_tree_or_compiler_identity_claimed=False,
        whole_current_python_gate_reused=False,reusable_source_gate_roles=[],
        replay_input_proof=replay_input_proof,tracked_dependency_proof=tracked_dependency_proof,
        source_list_identity=dict(count=90,canonical_sha256=hashlib.sha256(core.canonical(sources)).hexdigest(),
            encoding='existing required role38 list; indices zero based'),
        unresolved=unresolved)
    if changed_scope:
        value.update(status=PROSPECTIVE_MODE,
            historical_source_list_identity=dict(count=90,
                canonical_sha256=hashlib.sha256(core.canonical(historical_sources)).hexdigest()),
            historical_gate_scope='exact accepted old Rust/Cargo/consumer scope only',
            historical_replay_input_source=dict(replay_pin),
            current_gate_state='PROSPECTIVE_NOT_EXECUTED_NOT_ACCEPTED',
            current_source_changed=historical_sources!=sources,
            mandatory_current_validation=['all14 required gates','TEN19 full comparisons',
                'independent frozen-source acceptance','normal-hook commit','exact READY'])
    if (live_bytes+fixed.graph_charge((inputs,accepted_sources,historical,value))
            +2*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 separate source scope serialization lifetime')
    return _constructor_output(value,maximum)


def storage_validation(*, maxima, accepted_source_certificate, source_freeze,
                       source_scope_mapping, original_gates,
                       tracked_build_dependency_proof, failure_retention_amendment,
                       live_bytes):
    """Retain complete bridge gate, dependency and failure-retention inputs.

    Historical storage validation remains scoped to its original source and
    execution. This current component awaits semantic scope review; it makes
    no current compiler, build-tree, binary or execution identity claim.
    """
    maximum=_constructor_lifetime(36,maxima,
        (accepted_source_certificate,source_freeze,source_scope_mapping,original_gates,
         tracked_build_dependency_proof,failure_retention_amendment),live_bytes)
    for pin in (accepted_source_certificate,source_freeze,source_scope_mapping):fixed._pin_tuple(pin)
    if type(original_gates) is not list or len(original_gates)!=2:
        raise ValueError('fixed40 complete original storage gates')
    for gate,roles in zip(original_gates,((10,11),(12,13))):
        if set(gate)!={'log','result','roles'} or gate['roles']!=list(roles):
            raise ValueError('fixed40 exact original storage gate pair')
        fixed._pin_tuple(gate['log']);fixed._pin_tuple(gate['result'])
    if (type(tracked_build_dependency_proof) is not dict or set(tracked_build_dependency_proof)!=
            {'accepted_freeze_base','current_head','interpretation','scoped_tracked_diff_from_base',
             'tracked_build_definition_diff_from_base','tracked_build_definition_scope'}
            or type(failure_retention_amendment) is not dict or set(failure_retention_amendment)!=
            {'instruction_read_aliases','manifest','required_failures','schema','source28_findings_prefix'}):
        raise ValueError('fixed40 complete storage dependency/failure bodies')
    return _constructor_output(dict(schema=1,status='COMPONENT_PENDING_STORAGE_VALIDATION_REVIEW',
        accepted=False,ready=False,new_execution=False,
        accepted_source_certificate=accepted_source_certificate,source_freeze=source_freeze,
        source_scope_mapping=source_scope_mapping,original_gates=original_gates,
        tracked_build_dependency_proof=tracked_build_dependency_proof,
        failure_retention_amendment=failure_retention_amendment,
        claim='Original Rust-ON bridge build and four-gtest source validation; current scope pending review',
        current_artifact_identity_claimed=False,current_build_tree_identity_claimed=False,
        current_compiler_identity_claimed=False),maximum)


def command_verification(*, maxima, freeze, exact_commands_pin, source_commit,
                         rebuilt_freeze_raw, actual_freeze_raw, actual_reads,
                         phase_peaks, history, copy_metadata, child_occurrences,
                         strict_pending, live_bytes):
    """Record a completed full freeze comparison and actual bounded read ledger.

    A matching byte comparison is only one component. All original semantic
    consumers, actual metadata checks and independent READY remain required.
    """
    maximum=_constructor_lifetime(61,maxima,
        (freeze,exact_commands_pin,source_commit,rebuilt_freeze_raw,actual_freeze_raw,
         actual_reads,phase_peaks,history,copy_metadata),live_bytes)
    fixed._pin_tuple(freeze);fixed._pin_tuple(exact_commands_pin);_copy_rows(copy_metadata)
    if (type(actual_freeze_raw) is not bytes or type(rebuilt_freeze_raw) is not bytes
            or len(actual_freeze_raw)!=freeze['bytes']
            or hashlib.sha256(actual_freeze_raw).hexdigest()!=freeze['sha256']
            or actual_freeze_raw!=rebuilt_freeze_raw):
        raise ValueError('fixed40 completed full deterministic freeze comparison')
    read_fields={'actual_binary_bytes','actual_bytes','actual_nonbinary_bytes',
        'actual_source_bytes','charged_nonbinary_bytes','charged_total_bytes',
        'open_occurrences','pending_and_outcome_reserved_bytes'}
    if (type(actual_reads) is not dict or set(actual_reads)!=read_fields
            or any(type(x) is not int or x<0 for x in actual_reads.values())
            or actual_reads['pending_and_outcome_reserved_bytes']!=sum(ROLE_MAXIMA[57:])
            or actual_reads['actual_bytes']!=actual_reads['actual_binary_bytes']+actual_reads['actual_nonbinary_bytes']
            or actual_reads['charged_total_bytes']!=actual_reads['actual_binary_bytes']+actual_reads['charged_nonbinary_bytes']
            or type(phase_peaks) is not dict or not phase_peaks
            or any(type(x) is not int or not 0<=x<=core.SEMANTIC for x in phase_peaks.values())
            or type(history) is not dict or set(history)!=
                {'body_bytes','charged_bytes','containers','members'}
            or any(type(x) is not int or x<0 for x in history.values())
            or child_occurrences!=64 or type(strict_pending) is not int or strict_pending<0):
        raise ValueError('fixed40 complete actual verification ledger')
    return _constructor_output(dict(schema=1,freeze=freeze,exact_commands=exact_commands_pin,
        source_commit=source_commit,actual_reads=actual_reads,phase_peaks=phase_peaks,
        history=history,copy_metadata=copy_metadata,child_occurrences=child_occurrences,
        strict_pending=strict_pending,deterministic_byte_equal=True,
        captured=False,ready=False,new_facts=0,full_database_preservation_proven=False,
        status='COMPONENT_PENDING_FULL_COMMAND_ADMISSION'),maximum)


def capture_execution_decision(*, maxima, source_commit, command, inner_command,
                               freeze, exact_commands_pin, ready, execution_helper,
                               live_bytes):
    """Bind the complete one-use decision inputs without authorizing execution.

    The guard must consume and accept the complete independently reviewed
    READY body and all command/source/proof routes before it can execute.
    A pin to that body, or this pending record, grants no runtime authority.
    """
    maximum=_constructor_lifetime(62,maxima,
        (source_commit,command,inner_command,freeze,exact_commands_pin,ready,execution_helper),live_bytes)
    if re.fullmatch('[0-9a-f]{40}',source_commit) is None:
        raise ValueError('fixed40 capture decision source commit')
    for pin in (freeze,exact_commands_pin,ready,execution_helper):fixed._pin_tuple(pin)
    for argv in (command,inner_command):_command_argv(argv)
    if len(inner_command)!=5 or inner_command[1::2]!=['--bundle-output','--report-output']:
        raise ValueError('fixed40 complete decision native argv')
    return _constructor_output(dict(schema=1,source_commit=source_commit,
        command=command,inner_command=inner_command,freeze=freeze,
        exact_commands=exact_commands_pin,ready=ready,execution_helper=execution_helper,
        accepted_facts_before=805,residual_before=109,new_facts=0,no_retry=True,
        status='COMPONENT_PENDING_INDEPENDENT_READY_AND_GUARD_ADMISSION'),maximum)


def lead_verification(*, maxima, slot, capture_raw, lead_raw, independent_raw,
                      independent_verification_raw, pins, unit, declarations,
                      before, after, phase_peaks, live_bytes):
    """Construct the complete lead summary after authenticated native replay.

    Four complete report bodies and their original declarations are mandatory.
    The independent verification body must already bind the actual source,
    executable, native proof run and all current reports. No caller verdict
    flag is accepted. This function cannot replace native proof execution.
    """
    maximum=_role_maximum(77,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 lead verification caller')
    names=('capture_report','lead_report','independent_report','independent_verification')
    bodies=(capture_raw,lead_raw,independent_raw,independent_verification_raw)
    if type(pins) is not dict or set(pins)!=set((*names,'bundle')):
        raise ValueError('fixed40 complete lead verification pins')
    inputs=(slot,bodies,pins,declarations,before,after,phase_peaks)
    base=live_bytes+fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144
    if base>core.SEMANTIC:raise ValueError('fixed40 lead verification caller lifetime')
    if len(declarations)!=4:raise ValueError('fixed40 complete lead original body declarations')
    for name,raw,declaration in zip(names,bodies,declarations):
        p=pins[name];fixed._pin_tuple(p)
        if (type(raw) is not bytes or len(raw)>ROLE_MAXIMA[{'capture_report':69,
                'lead_report':72,'independent_report':75,'independent_verification':76}[name]]
                or p['bytes']!=len(raw) or p['sha256']!=hashlib.sha256(raw).hexdigest()):
            raise ValueError('fixed40 full authenticated lead input body')
        occurrence,physical,kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(p) or kind!='evidence' or mode!='bytes':
            raise ValueError('fixed40 exact lead input declaration')
        unit.consume(occurrence,p,kind=kind,live_bytes=live_bytes+fixed.graph_charge(inputs))
    fixed._pin_tuple(pins['bundle']);_copy_rows(before);_copy_rows(after)
    if before!=after:raise ValueError('fixed40 actual before/after metadata difference')
    if lead_raw!=independent_raw:
        raise ValueError('fixed40 complete canonical replay reports differ')

    def decode(raw,retained=()):
        if (base+fixed.graph_charge(retained)+3*len(raw)
                +160*core._wire_nodes(raw)>core.SEMANTIC):
            raise ValueError('fixed40 lead full report predecode lifetime')
        occurrence=declarations[1 if raw is lead_raw else (0 if raw is capture_raw else 3)][0]
        return fixed._inline_decode(unit,occurrence,raw,
            live_bytes+fixed.graph_charge((inputs,retained)))

    lead=decode(lead_raw)
    required={'schema','profile','selection','selection_sha256','authority','vrf_sources',
              'bounds','qualification','status','actual','bundle','values'}
    if type(lead) is not dict or set(lead)!=required:
        raise ValueError('fixed40 complete native replay report fields')
    rebuilt=native_window_report(72,maxima=maxima,slot=slot,
        selection=lead['selection'],authority=lead['authority'],vrf_sources=lead['vrf_sources'],
        bounds=lead['bounds'],bundle=lead['bundle'],actual=lead['actual'],values=lead['values'],
        live_bytes=live_bytes+fixed.graph_charge((inputs,lead)))
    if rebuilt!=core.canonical(lead) or lead['bundle']!=pins['bundle']:
        raise ValueError('fixed40 complete native replay semantics')
    del rebuilt
    capture=decode(capture_raw,(lead,))
    if (set(capture)!=required-{'values'}
            or capture['status']!='captured_requires_offline_replay'
            or any(capture[k]!=lead[k] for k in
                ('schema','profile','selection','selection_sha256','authority','vrf_sources','bounds','bundle'))
            or capture['qualification']!={k:False for k in lead['qualification']}):
        raise ValueError('fixed40 complete native capture/replay binding')
    _native_usage(capture['actual'])
    capture_usage={r['resource']:r['used'] for r in capture['actual']['resources']}
    del capture
    verification=decode(independent_verification_raw,(lead,capture_usage))
    bound_names=('bundle','capture_report','lead_report','independent_report')
    if (type(verification) is not dict or set(verification)!=INDEPENDENT_VERIFICATION_FIELDS
            or verification.get('verified') is not True
            or verification.get('accepted') is not False or verification.get('new_facts')!=0
            or any(verification.get(k)!=pins[k] for k in bound_names)
            or verification.get('profile')!=FIXED40_PROFILE
            or verification.get('source_count')!=90 or verification.get('tool_count')!=11
            or verification.get('binary_count')!=2
            or verification.get('constructor_proofs')!=51 or verification.get('payload_proofs')!=40
            or verification.get('current_proofs')!=91 or verification.get('fresh_prior_proofs')!=71
            or verification.get('prior_payload_bindings')!=20
            or verification.get('exact_eof_verified') is not True
            or verification.get('metadata_before_after_equal') is not True
            or verification.get('canonical_reports_byte_equal') is not True
            or verification.get('values')!=lead['values']
            or verification.get('selection_sha256')!=lead['selection_sha256']
            or verification.get('qualification')!=lead['qualification']
            or verification.get('resource_limits')!=lead['bounds']['storage_resources']
            or verification.get('omitted_delayed_indices')!=list(fixed.OMISSIONS)
            or verification.get('selection_delayed_indices')!=
                [s['tuple']['identity']['period']-25706894 for s in lead['selection']]
            or verification.get('prior_usage')!=lead['authority']['prior_first_d']['actual']
            or verification.get('replay_usage')!=lead['actual']
            or verification.get('raw_artifact_count')!=7
            or verification.get('raw_artifact_bytes')!=6785636
            or type(verification.get('raw_inputs')) is not list
            or len(verification['raw_inputs'])!=7
            or type(verification.get('source_observations')) is not list
            or verification.get('source_occurrences')!=len(verification['source_observations'])):
        raise ValueError('fixed40 full independent native execution/source/proof binding')
    prior=lead['authority']['prior_first_d']['selected']
    if verification['selected_prior']!=dict(one_based=18,zero_based=17,
            parent_manifest_index=53,proof_index=68,selected=prior):
        raise ValueError('fixed40 full independent selected prior binding')
    position=source_bytes=0
    for group,rows in (('vrf_sources',lead['vrf_sources']),
            ('owner_sources',lead['authority']['six_input_owner']['sources']),
            ('aspen_sources',lead['authority']['six_input_owner']['aspen_sources'])):
        for original in rows:
            if position>=len(verification['source_observations']):
                raise ValueError('fixed40 complete all63 source occurrences')
            row=verification['source_observations'][position]
            if (set(row)!={'canonical','direct_frozen','group','original'}
                    or row['group']!=group or row['original']!=original
                    or type(row['direct_frozen']) is not bool
                    or any(row['canonical'][k]!=original[k] for k in ('bytes','sha256'))):
                raise ValueError('fixed40 full independent source occurrence binding')
            fixed._pin_tuple(row['canonical']);source_bytes+=row['canonical']['bytes'];position+=1
    if position!=63 or position!=len(verification['source_observations']) or source_bytes!=verification['source_occurrence_bytes']:
        raise ValueError('fixed40 full source occurrence count/byte accounting')
    if (type(phase_peaks) is not dict or set(phase_peaks)!=
            {'independent_verification','lead_report','shared_projection'}
            or any(type(x) is not int or not 0<=x<=core.SEMANTIC for x in phase_peaks.values())):
        raise ValueError('fixed40 complete lead phase peaks')
    value_hex=prior['value_hex']
    value_digest=hashlib.sha256(bytes.fromhex(value_hex)).hexdigest()
    for item in lead['values']:
        if (item['logical']!='member' or item['usable_vrf_key'] is not True
                or item['value_hex']!=value_hex or item['physical']!=
                dict(kind='present',bytes=32,sha256=value_digest)):
            raise ValueError('fixed40 all40 complete usable native values')
    value=dict(schema=1,status='LEAD_FULL_FIXED40_PROOF_VERIFIED_PENDING_REVERSE_AND_FACT_SEAL',
        accepted=False,verified=True,new_facts=0,profile=FIXED40_PROFILE,
        bundle=pins['bundle'],capture_report=pins['capture_report'],lead_report=pins['lead_report'],
        independent_report=pins['independent_report'],independent_verification=pins['independent_verification'],
        constructor_proofs=51,payload_proofs=40,current_proofs=91,fresh_prior_proofs=71,
        prior_payload_bindings=20,omitted_delayed_indices=list(fixed.OMISSIONS),
        all40_members_usable=True,before_after_metadata_equal=True,
        full_database_preservation_proven=False,report_byte_equal=True,
        selection_sha256=lead['selection_sha256'],qualification=lead['qualification'],
        capture_usage=capture_usage,replay_usage={r['resource']:r['used'] for r in lead['actual']['resources']},
        physical_ceilings=lead['bounds']['storage_resources'],phase_peaks=phase_peaks,
        value_bytes=32,value_hex=value_hex,value_sha256=value_digest)
    if (base+fixed.graph_charge((lead,verification,capture_usage,value))>core.SEMANTIC):
        raise ValueError('fixed40 lead summary serialization lifetime')
    return _constructor_output(value,maximum)


AUTHORITY_PARENT=dict(path='/home/fry/artifacts/vrf-round3-01a141ba-20261016/astra/next42-successor-contract-v7.json',
    bytes=117947,sha256='1f29f71d1637754fa27eb126366d85c9bfd25fe045019c6c24aee5310bd58f4e')


def successor_contract(*, maxima, predecessor_raw, unit, parent_declaration,
                       mapping_declaration, projection_read_charge, live_bytes):
    """Build reviewed role40 with the entire original parent embedded verbatim.

    The parent is historical context, with every original predicate retained.
    Only the independently authenticated V5 selected40 slot defines current
    membership. Active role lists declare full current-body obligations; they
    discharge no read, source, proof, gate, fit, READY or acceptance check.

    The mapping's complete read/parse finishes before the full parent graph
    is allocated. The source authentication read and descriptor read are two
    real charged occurrences; neither can be replaced with cached metadata.
    """
    owner=unit.invocation.metadata_charge()+unit.declaration_charge
    maximum=_role_maximum(40,maxima)
    if not callable(projection_read_charge):
        raise ValueError('fixed40 prepaid full mapping projection read required')
    core.integer(live_bytes,core.SEMANTIC,'fixed40 authority constructor caller')
    deferred_parent=predecessor_raw is None
    if not deferred_parent and (type(predecessor_raw) is not bytes
            or len(predecessor_raw)!=AUTHORITY_PARENT['bytes']
            or hashlib.sha256(predecessor_raw).hexdigest()!=AUTHORITY_PARENT['sha256']):
        raise ValueError('fixed40 entire exact historical authority parent')
    if (owner+live_bytes+fixed.graph_charge((predecessor_raw,parent_declaration,mapping_declaration))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 authority initial caller lifetime')
    declarations=((parent_declaration,AUTHORITY_PARENT),(mapping_declaration,fixed.MAPPING))
    for declaration,pin in declarations:
        occurrence,physical,kind,mode=declaration
        if tuple(physical)!=fixed._pin_tuple(pin) or kind!='evidence' or mode!='bytes':
            raise ValueError('fixed40 exact full authority source declaration')
    # The separately prepaid projection may precede the immutable unit reads.
    # Its full mapping body is released before the prescribed parent read.
    if deferred_parent:
        projection_read_charge((dict(fixed.MAPPING),))
        descriptor=fixed.selected_descriptor(live_bytes=owner+live_bytes)
    for declaration,pin in declarations:
        occurrence,physical,kind,mode=declaration
        if deferred_parent and pin is AUTHORITY_PARENT:
            predecessor_raw=unit.consume(occurrence,pin,kind=kind,retain=True,
                live_bytes=live_bytes+fixed.graph_charge(descriptor))
            if (type(predecessor_raw) is not bytes or len(predecessor_raw)!=pin['bytes']
                    or hashlib.sha256(predecessor_raw).hexdigest()!=pin['sha256']):
                raise ValueError('fixed40 entire exact historical authority parent')
        else:
            unit.consume(occurrence,pin,kind=kind,
                live_bytes=live_bytes+fixed.graph_charge((predecessor_raw,
                    descriptor if deferred_parent else None)))
    if not deferred_parent:
        # Additional full body+EOF pass, charged before opening.
        projection_read_charge((dict(fixed.MAPPING),))
        descriptor=fixed.selected_descriptor(live_bytes=owner+live_bytes+fixed.graph_charge(predecessor_raw))
    if descriptor['maxima']!=maxima:
        raise ValueError('fixed40 unchanged full81 authority maxima')
    slot=descriptor['slot']
    shared_fields=('account_hex','key_preimage_hex','logical_key_hex','voter_hex')
    first=slot['rows'][0]['full_tuple'];shared={k:first[k] for k in shared_fields}
    compact=dict(encoding='fixed40-lossless-shared-tuple-columns-v1',
        columns=['period','state_root_hex','parent_manifest_index','source_references'],
        source_reference_columns=['period','vote_hash_hex'],shared_tuple=shared,
        rows=[],**{k:slot[k] for k in slot if k!='rows'})
    for row in slot['rows']:
        full=row['full_tuple']
        if {k:full[k] for k in shared_fields}!=shared:
            raise ValueError('fixed40 exact shared tuple columns')
        compact['rows'].append([full['period'],full['state_root_hex'],row['parent_manifest_index'],
            [[ref['period'],ref['vote_hash_hex']] for ref in row['source_references']]])
    # Verify every full row after lossless expansion, including all references.
    expanded=dict(slot,rows=[dict(full_tuple=dict(shared,period=row[0],state_root_hex=row[1]),
        parent_manifest_index=row[2],source_references=[dict(period=r[0],vote_hash_hex=r[1])
        for r in row[3]]) for row in compact['rows']])
    if expanded!=slot:raise ValueError('fixed40 complete lossless authority slot equality')
    del expanded,slot,descriptor
    retained=(predecessor_raw,compact,parent_declaration,mapping_declaration)
    # The shared decoder checks metadata, declarations, every retained caller
    # object and its raw/token allocation before parsing. Output serialization
    # starts afterward and has its own complete graph guard below.
    parent=fixed._inline_decode(unit,parent_declaration[0],predecessor_raw,
        live_bytes+fixed.graph_charge(retained))
    # Raw authentication already fixes every one of the parent's40 fields.
    if len(parent)!=40 or parent.get('schema')!='n4-next42-finite-successor-v1':
        raise ValueError('fixed40 complete historical parent schema')
    if (owner+live_bytes+fixed.graph_charge((retained,parent))+maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 authority metadata preallocation lifetime')
    meta=dict(schema='fixed40-full-parent-compact-slot-contract-v1',
        profile=FIXED40_PROFILE,mapping_source=fixed.MAPPING,predecessor_pin=AUTHORITY_PARENT,
        predecessor_contract_encoding='verbatim-JSON-value-with-original-terminal-newline',
        selected_slot=compact,active_source_roles=[38,39],active_candidate_roles=[41,42,43,44,55],
        active_command_roles=[54,*range(56,68)],active_proof_roles=list(range(68,80)),
        accepted=False,ready=False,new_facts=0)
    if (owner+live_bytes+fixed.graph_charge((predecessor_raw,meta,parent))
            +16384+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 authority complete shape caller lifetime')
    records.bounded_shape(dict(meta,predecessor_contract=parent))
    del parent
    if (owner+live_bytes+fixed.graph_charge((predecessor_raw,meta))
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 authority output caller lifetime')
    prefix=core.canonical(meta)
    if not prefix.endswith(b'}\n'):raise ValueError('fixed40 authority canonical metadata framing')
    raw=prefix[:-2]+b',"predecessor_contract":'+predecessor_raw+b'}\n'
    if len(raw)>maximum:raise ValueError('fixed40 full verbatim authority role40 cap')
    return raw



REVERSE_PREDECESSOR=dict(path='/home/fry/artifacts/vrf-round3-01a141ba-20261016/next42/reverse-outcome-record.json',
    bytes=98208,sha256='128cfee1325189b8793670d273dd12d2214c418d3a9a9ea1a5d0844d066383aa')
REVERSE_MATRIX=dict(path='/home/fry/artifacts/vrf40-development-20261018/reverse170-instance-matrix-proposed.json',
    bytes=10636,sha256='46d24ff6e25980e76d386b227a3665b0224de2cfb0c3a9b6a09d839d6bb3f2bd')


def reverse_historical_body_pins(boundary):
    """Keep the accepted22 nonbinary rereads; the old24 pin metadata stays whole."""
    ordered=boundary.get('original_ordered24_pins')
    nonbinary=boundary.get('full_nonbinary_routes')
    if (boundary.get('classification')!=
            'HISTORICAL_FULL_LEDGER_AND_NONBINARY_ROUTES; NO_EXTRA_FRESH_HISTORICAL_BINARY_PAIR_CLAIM'
            or boundary.get('fresh_historical_binary_validation') is not False
            or boundary.get('nonbinary_read_occurrences')!=22
            or type(ordered) is not list or len(ordered)!=24
            or type(nonbinary) is not list or nonbinary!=ordered[2:]):
        raise ValueError('fixed40 exact accepted historical nonbinary boundary')
    return [boundary['original_ledger'],boundary['reverse_READY'],
            boundary['reverse_freeze'],boundary['boundary_review'],*nonbinary]


def _reverse_declaration_kinds(expected, declarations, binary_start):
    """Validate exactly the two active binary positions before any body read."""
    if len(expected)!=binary_start+24 or len(declarations)!=len(expected):
        raise ValueError('fixed40 all original and current reverse declarations')
    for index,(p,declaration) in enumerate(zip(expected,declarations)):
        fixed._pin_tuple(p)
        occurrence,physical,kind,mode=declaration
        expected_kind='binary' if binary_start<=index<binary_start+2 else 'evidence'
        if tuple(physical)!=fixed._pin_tuple(p) or kind!=expected_kind or mode!='bytes':
            raise ValueError('fixed40 exact reverse body/binary declaration')


def reverse_outcome_record(*, maxima, predecessor_raw, matrix_raw, runs, slot,
                           historical_fbb_report_raw, binary_before, binary_after,
                           current_pins, source_commit, capture_parent,
                           read_accounting, semantic_peaks, source_provenance,
                           unit, declarations, live_bytes):
    """Build all five actual reverse outcomes and preserve full old boundary.

    Old results are historical inputs only. Every new native result/log/report
    is authenticated and checked; all thirteen explicit family counters must
    be zero, while admitted_input_bytes must equal the full wrong bundle.
    Source/build provenance for the historical Fbb executable stays separate
    from the current E946 executable. Native raw7/prior71 proof execution and
    independently accepted exact reverse READY are required runtime inputs.
    """
    maximum=_role_maximum(78,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 reverse constructor caller')
    for raw,p in ((predecessor_raw,REVERSE_PREDECESSOR),(matrix_raw,REVERSE_MATRIX)):
        if (type(raw) is not bytes or len(raw)!=p['bytes']
                or hashlib.sha256(raw).hexdigest()!=p['sha256']):
            raise ValueError('fixed40 exact complete reverse source bodies')
    if type(runs) is not list or len(runs)!=5 or type(current_pins) is not list or len(current_pins)!=24:
        raise ValueError('fixed40 all five actual reverse runs and24 pins')
    # Two source records, four historical controls,22 historical nonbinary
    # bodies, one historical target report, then the complete current24.
    if len(declarations)!=2+4+22+1+len(current_pins):
        raise ValueError('fixed40 complete reverse declaration layout')
    current_offset=len(declarations)-len(current_pins)
    historical_report_offset=current_offset-1
    inputs=(predecessor_raw,matrix_raw,runs,slot,historical_fbb_report_raw,binary_before,binary_after,current_pins,source_commit,capture_parent,
            read_accounting,semantic_peaks,source_provenance,declarations)
    base=live_bytes+fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144
    if base>core.SEMANTIC:raise ValueError('fixed40 reverse initial caller lifetime')
    def decode(raw,retained=(),cap=131072):
        if (base+fixed.graph_charge(retained)+3*len(raw)
                +160*core._wire_nodes(raw)>core.SEMANTIC):
            raise ValueError('fixed40 reverse full decode lifetime')
        if raw is predecessor_raw:occurrence=declarations[0][0]
        elif raw is matrix_raw:occurrence=declarations[1][0]
        elif raw is historical_fbb_report_raw:occurrence=declarations[historical_report_offset][0]
        else:
            report_pin=next(r['report'] for r in runs if raw is r['report_raw'])
            occurrence=declarations[current_offset+current_pins.index(report_pin)][0]
        return fixed._inline_decode(unit,occurrence,raw,
            live_bytes+fixed.graph_charge((inputs,retained)))
    parent=decode(predecessor_raw);matrix=decode(matrix_raw,(parent,))
    if (parent.get('schema')!=1 or parent.get('kind')!='fixed_reverse_outcome_record'
            or matrix.get('schema')!='fixed40-five-reverse-instance-matrix-v1'
            or len(matrix.get('matrix',()))!=5):
        raise ValueError('fixed40 complete reverse parent/matrix schema')
    boundary=parent['historical_boundary']
    historical=reverse_historical_body_pins(boundary)
    historical_fbb_pin=parent['outcomes'][1]['report']
    if (type(historical_fbb_report_raw) is not bytes
            or len(historical_fbb_report_raw)!=historical_fbb_pin['bytes']
            or hashlib.sha256(historical_fbb_report_raw).hexdigest()!=historical_fbb_pin['sha256']
            or current_pins[1]!=parent['pins'][1]):
        raise ValueError('fixed40 unchanged original Fbb binary/report provenance')
    expected=[REVERSE_PREDECESSOR,REVERSE_MATRIX,*historical,historical_fbb_pin,*current_pins]
    _reverse_declaration_kinds(expected,declarations,2+len(historical)+1)
    for p,declaration in zip(expected,declarations):
        occurrence,physical,kind,mode=declaration
        unit.consume(occurrence,p,kind=kind,live_bytes=live_bytes+fixed.graph_charge((inputs,parent,matrix)))
    # Exact parent authentication preserves the complete inline historical
    # ledger/READY/freeze and ordered24 pins without reconstruction.
    for body_name,pin_name in (('original_ledger_body_utf8','original_ledger'),
                               ('reverse_READY_body_utf8','reverse_READY'),
                               ('reverse_freeze_body_utf8','reverse_freeze')):
        b=boundary[body_name].encode('utf-8');p=boundary[pin_name]
        if len(b)!=p['bytes'] or hashlib.sha256(b).hexdigest()!=p['sha256']:
            raise ValueError('fixed40 full historical reverse boundary body')
    outcomes=[]
    for index,(run,rule) in enumerate(zip(runs,matrix['matrix'])):
        if (type(run) is not dict or set(run)!=
                {'name','report_raw','result_raw','log_raw','report','result','log','binary','bundle','command'}):
            raise ValueError('fixed40 complete actual reverse run fields')
        if run['name']!=rule['new_name']:
            raise ValueError('fixed40 exact five reverse order')
        for name in ('report','result','log'):
            p=run[name];raw=run[name+'_raw'];fixed._pin_tuple(p)
            cap=16384 if name=='result' else 65536
            if (type(raw) is not bytes or len(raw)>cap or len(raw)!=p['bytes']
                    or hashlib.sha256(raw).hexdigest()!=p['sha256'] or p not in current_pins):
                raise ValueError('fixed40 complete actual reverse result/log/report')
        for name in ('binary','bundle'):
            fixed._pin_tuple(run[name])
            if run[name] not in current_pins:raise ValueError('fixed40 actual reverse binary/bundle scope')
        result_raw=run['result_raw']
        if (base+fixed.graph_charge((parent,matrix,outcomes))+3*len(result_raw)
                +320*core._wire_nodes(result_raw)>core.SEMANTIC):
            raise ValueError('fixed40 reverse actual result predecode lifetime')
        # The original completed command record has a finite decimal duration;
        # protocol native reports continue to use the integer-only decoder.
        operation=declarations[current_offset+current_pins.index(run['result'])][0]
        if unit.invocation.operations[operation]>=8:raise ValueError('fixed40 ninth reverse result parse')
        unit.invocation.operations[operation]+=1
        result=json.loads(result_raw,object_pairs_hook=records.unique_object)
        if (type(result) is not dict or set(result)!=
                {'accepted','binary','bundle','child_exit','command','cwd','environment','exit',
                 'log','log_bytes','name','new_facts','output_overflow','schema','seconds','wall_timeout'}):
            raise ValueError('fixed40 complete original reverse result fields')
        records.bounded_shape(dict(result,seconds=0))
        _command_argv(run['command'])
        if (any(type(result[k]) is not int for k in ('schema','exit','child_exit','log_bytes','new_facts'))
                or result.get('command')!=run['command'] or result.get('exit')!=1
                or result.get('child_exit')!=1 or result.get('wall_timeout') is not False
                or result.get('output_overflow') is not False
                or result.get('log')!=run['log'] or result.get('log_bytes')!=run['log']['bytes']
                or type(result.get('seconds')) not in (int,float)
                or not math.isfinite(result['seconds']) or not 0<=result['seconds']<=300
                or result['accepted'] is not False or result['new_facts']!=0
                or result['name']!=run['name'] or result['binary']!=run['binary']
                or result['bundle']!=run['bundle'] or result['schema']!=1
                or result['cwd']!='/workspaces/rustaxa-evm'
                or result['environment']!=FIXED40_ENVIRONMENT):
            raise ValueError('fixed40 actual bounded reverse failure execution')
        if run['command']!=['/usr/local/bin/rtk','proxy',run['binary']['path'],'--bundle-input',run['bundle']['path'],
                            '--report-output',run['report']['path']]:
            raise ValueError('fixed40 exact actual native reverse argv')
        del result
        report=decode(run['report_raw'],(parent,matrix,outcomes))
        required={'schema','profile','selection','selection_sha256','authority','vrf_sources','bounds',
                  'qualification','status','actual','failure','values'}
        expectation=rule['current_expected']
        if (type(report) is not dict or set(report)!=required or type(report['schema']) is not int or report['schema']!=1
                or report['profile']!=expectation['target_profile'] or report['status']!='stopped'
                or report['values'] is not None or report['failure']!=
                    dict(error='Schema',identity=None,index=0,last_read=None,phase='Admission',tuple=None)
                or type(report['failure']['index']) is not int
                or set(report['qualification'])!={'vrf_family_qualified','voter_weights_qualified',
                    'reward_cache_equal','owner_ready','producer_verified','production_routing_authorized'}
                or any(v is not False for v in report['qualification'].values())
                or report['bounds']['payload_tuples_exact']!=expectation['target_payloads']
                or report['bounds']['proofs_exact']!=expectation['target_bundle_proofs']
                or len(report['selection'])!=expectation['target_payloads']
                or report['selection_sha256']!=_native_selection_digest(report['selection'])):
            raise ValueError('fixed40 exact complete Schema Admission reverse report')
        _native_usage(report['actual'])
        if (report['actual']['admitted_input_bytes']!=run['bundle']['bytes']
                or any(r['used']!=0 for r in report['actual']['resources'])):
            raise ValueError('fixed40 explicit all13 family zeros and actual wrong input bytes')
        if index!=1:
            validated=native_window_report(69,maxima=maxima,slot=slot,
                selection=report['selection'],authority=report['authority'],vrf_sources=report['vrf_sources'],
                bounds=report['bounds'],bundle=run['bundle'],actual=report['actual'],
                live_bytes=live_bytes+fixed.graph_charge((inputs,parent,matrix,outcomes,report)))
            del validated
        else:
            old_report=decode(historical_fbb_report_raw,(parent,matrix,outcomes,report))
            if any(report[k]!=old_report[k] for k in required if k!='actual'):
                raise ValueError('fixed40 unchanged full historical Fbb target semantics')
            del old_report
        authority=report['authority'];prior=authority['prior_first_d'];selected=prior['selected']
        _native_usage(prior['actual'])
        if (authority['raw_artifact_reads']!=7 or authority['raw_artifact_bytes']!=6785636
                or selected['proof_index']!=expectation['prior_proof_index']
                or selected['voter_hex']!=expectation['voter']
                or selected['value_hex']!=expectation['prior_value_hex']
                or selected['usable_vrf_key'] is not True
                or len(prior['actual']['resources'])!=13):
            raise ValueError('fixed40 exact fresh reverse raw7/prior71 authority')
        outcomes.append(dict(name=run['name'],expected=expectation,passed=True,
            report=run['report'],result=run['result'],log=run['log'],
            resources=report['actual']['resources'],prior_usage=prior['actual']['resources'],
            selected_prior=selected,fresh_prior_proofs=71,prior_payload_bindings=20,
            fresh_raw_artifacts=7,fresh_raw_bytes=6785636,no_missing_field_default=True,
            global_proof_zero_basis=expectation['global_proof_zero_basis']))
        del report
    caps=parent['caps']
    for p in current_pins[:2]:
        fixed._executable_value(p)
        if p['bytes']>220200960:raise ValueError('fixed40 reverse individual binary cap')
    binary_bytes=sum(p['bytes'] for p in current_pins[:2]);total=sum(p['bytes'] for p in current_pins)
    if binary_bytes>440401920 or total>caps['aggregate_bytes'] or total-binary_bytes>caps['other_bytes']:
        raise ValueError('fixed40 complete reverse aggregate caps')
    if (type(source_provenance) is not str or not source_provenance
            or re.fullmatch('[0-9a-f]{40}',source_commit) is None):
        raise ValueError('fixed40 separate current and historical reverse provenance')
    fixed._pin_tuple(capture_parent)
    for rows in (binary_before,binary_after):
        if type(rows) is not list or len(rows)!=2:
            raise ValueError('fixed40 complete reverse binary before/after metadata')
        for row,p in zip(rows,current_pins[:2]):
            if (type(row) is not dict or set(row)!=
                    {'path','device','inode','bytes','mtime_ns','ctime_ns','mode'}
                    or row['path']!=p['path'] or row['bytes']!=p['bytes']
                    or any(type(row[k]) is not int or row[k]<0 for k in row if k!='path')):
                raise ValueError('fixed40 actual full reverse binary identity')
    if binary_before!=binary_after:raise ValueError('fixed40 reverse binary post-stat changed')
    value=dict(schema=1,kind='fixed_reverse_outcome_record',
        status='ALL_FIVE_FIXED40_REVERSE_CONTROLS_PASS',outcomes=outcomes,
        pins=current_pins,total_occurrences=24,total_bytes=total,binary_pair_bytes=binary_bytes,
        source_commit=source_commit,current_binary_source_commit=source_commit,
        historical_binary_source_commit=parent['historical_binary_source_commit'],
        source_count=90,tool_count=11,source_provenance=source_provenance,
        capture_parent=capture_parent,historical_boundary=boundary,
        physical_read_scope=parent['physical_read_scope'],read_accounting=read_accounting,
        semantic_peaks=semantic_peaks,caps=caps,
        binary_post_stat_equal=True,physical_capture=False,fact_acceptance=False,new_facts=0)
    if (base+fixed.graph_charge((parent,matrix,outcomes,value))>core.SEMANTIC):
        raise ValueError('fixed40 reverse output caller lifetime')
    return _constructor_output(value,maximum)


def independent_replay_verification(*, maxima, slot, review_raw, capture_raw,
                                    lead_raw, replay_raw, command_raw, log_raw,
                                    source_raw, tools_raw, freeze_raw, live_raw,
                                    pins, unit, declarations, native_declarations,
                                    phase_peaks, live_bytes):
    """Render the original56 fields from a completed independent inspection.

    Actual native command, complete log, reports, source closure and freeze
    are authenticated inputs. The independent inspection retains its full
    reviewer provenance, accepted source exceptions and original control
    bindings. Full native/source/value checks run through the lead consumer;
    no caller verdict flag or counter-only proof can create this result.
    This component accepts no facts and supplies no current READY grant.
    """
    maximum=_role_maximum(76,maxima)
    core.integer(live_bytes,core.SEMANTIC,'fixed40 independent verification caller')
    raw_by_name=dict(review_result=review_raw,capture_report=capture_raw,lead_report=lead_raw,
        independent_report=replay_raw,independent_result=command_raw,independent_log=log_raw,
        source_freeze=source_raw,tool_source_freeze=tools_raw,freeze=freeze_raw,live_result=live_raw)
    if type(pins) is not dict or set(pins)!=set((*raw_by_name,'replay_binary','bundle')):
        raise ValueError('fixed40 complete independent native/control/source pins')
    if len(declarations)!=71 or len(native_declarations)!=4:
        raise ValueError('fixed40 all native source/control and report declarations')
    inputs=(slot,raw_by_name,pins,declarations,native_declarations,phase_peaks)
    base=live_bytes+fixed.graph_charge(inputs)+3*maximum+core.CHUNK+262144
    if base>core.SEMANTIC:raise ValueError('fixed40 independent initial caller lifetime')
    for name,raw in raw_by_name.items():
        p=pins[name];fixed._pin_tuple(p)
        cap=65536 if name=='independent_log' else (16384 if name in ('independent_result','live_result') else 131072)
        if (type(raw) is not bytes or len(raw)>cap or len(raw)!=p['bytes']
                or hashlib.sha256(raw).hexdigest()!=p['sha256']):
            raise ValueError('fixed40 full independent completed input body')
    fixed._executable_value(pins['replay_binary']);fixed._pin_tuple(pins['bundle'])
    if pins['bundle']['bytes']>ROLE_MAXIMA[68]:raise ValueError('fixed40 original native bundle role cap')
    def decode(raw,retained=(),admin=False):
        if (base+fixed.graph_charge(retained)+3*len(raw)
                +160*core._wire_nodes(raw)+(16384 if admin else 0)>core.SEMANTIC):
            raise ValueError('fixed40 independent full input predecode lifetime')
        positions={id(source_raw):0,id(tools_raw):1,id(freeze_raw):2,id(command_raw):3,id(live_raw):5}
        occurrence=(declarations[positions[id(raw)]][0] if id(raw) in positions else
            native_declarations[3 if raw is review_raw else 2][0])
        if not admin:return fixed._inline_decode(unit,occurrence,raw,
            live_bytes+fixed.graph_charge((inputs,retained)))
        if unit.invocation.operations[occurrence]>=8:raise ValueError('fixed40 ninth completed native result parse')
        unit.invocation.operations[occurrence]+=1
        value=json.loads(raw,object_pairs_hook=records.unique_object)
        if type(value) is not dict:raise ValueError('fixed40 completed command object')
        records.bounded_shape(dict(value,seconds=0))
        if type(value.get('seconds')) not in (int,float) or not math.isfinite(value['seconds']) or not 0<=value['seconds']<=300:
            raise ValueError('fixed40 actual complete native deadline')
        return value
    source=decode(source_raw);tools=decode(tools_raw,(source,));_source_lists(source,tools)
    frozen=decode(freeze_raw,(source,tools))
    fields={'schema','profile','run','source_commit','cwd','environment','capture_command','pins',
        'pin_count','pinned_file_bytes','source_count','complete_source_bytes','embedded_source_bytes','copy_metadata','accepted'}
    if (type(frozen) is not dict or set(frozen)!=fields or frozen['profile']!=FIXED40_PROFILE
            or frozen['accepted'] is not False or frozen['source_count']!=90
            or frozen['pins'][:101]!=source+tools or frozen['pins'][102]!=pins['replay_binary']
            or frozen['pin_count']!=len(frozen['pins'])
            or frozen['pinned_file_bytes']!=sum(p['bytes'] for p in frozen['pins'])
            or frozen['complete_source_bytes']!=sum(p['bytes'] for p in source+tools)
            or frozen['cwd']!='/workspaces/rustaxa-evm' or frozen['environment']!=FIXED40_ENVIRONMENT):
        raise ValueError('fixed40 complete current freeze/source/executable closure')
    source_commit=frozen['source_commit'];run=frozen['run'];metadata=frozen['copy_metadata']
    del source,tools,frozen
    command=decode(command_raw,admin=True)
    command_fields={'schema','accepted','command','exit','child_exit','seconds','log_bytes',
        'wall_timeout','output_overflow','cwd','environment','log','new_facts','freeze','ready','replay_binary','input'}
    expected_command=['/usr/local/bin/rtk','proxy',pins['replay_binary']['path'],
        '--bundle-input',pins['bundle']['path'],'--report-output',pins['independent_report']['path']]
    if (set(command)!=command_fields
            or any(type(command[k]) is not int for k in ('schema','exit','child_exit','log_bytes','new_facts'))
            or command['command']!=expected_command
            or command['exit']!=0 or command['child_exit']!=0
            or command['wall_timeout'] is not False or command['output_overflow'] is not False
            or command['accepted'] is not False or command['new_facts']!=0
            or command['log']!=pins['independent_log'] or command['log_bytes']!=len(log_raw)
            or command['freeze']!=pins['freeze'] or command['replay_binary']!=pins['replay_binary']
            or command['input']!=pins['bundle'] or command['cwd']!='/workspaces/rustaxa-evm'
            or command['environment']!=FIXED40_ENVIRONMENT):
        raise ValueError('fixed40 actual successful full independent native execution')
    ready=command['ready'];del command
    live=decode(live_raw,admin=True)
    if (live.get('exit')!=0 or live.get('child_exit')!=0
            or live.get('wall_timeout') is not False or live.get('output_overflow') is not False
            or live.get('before')!=metadata or live.get('after')!=metadata):
        raise ValueError('fixed40 actual complete capture metadata/execution')
    del live
    # Current101 full source reads keep their prescribed singleton units in
    # semantic_units; the freeze controls below bind that exact closure here.
    # This original native proof unit retains all63 native source occurrences.
    # Neither this metadata check nor a prior receipt waives any prescribed
    # current-source unit read on the complete runtime route.
    control_names=('source_freeze','tool_source_freeze','freeze','independent_result','independent_log','live_result','replay_binary','bundle')
    source=decode(source_raw);tools=decode(tools_raw,(source,))
    review=decode(review_raw,(source,tools))
    if type(review) is not dict or set(review)!=INDEPENDENT_VERIFICATION_FIELDS:
        raise ValueError('fixed40 complete original56 independent result fields')
    if len(review['source_observations'])!=63:
        raise ValueError('fixed40 all63 mandatory native source declarations')
    if (base+fixed.graph_charge((source,tools,review))+fixed.graph_charge(pins)
            +sum(fixed.graph_charge(row['canonical']) for row in review['source_observations'])
            +128>core.SEMANTIC):
        raise ValueError('fixed40 independent source declaration allocation')
    for row in review['source_observations']:
        if row['direct_frozen'] is True and row['canonical'] not in source and row['canonical'] not in tools:
            raise ValueError('fixed40 exact cross-unit current source identity')
    expected=[*(pins[n] for n in control_names),
              *(row['canonical'] for row in review['source_observations'])]
    if len(declarations)!=len(expected):raise ValueError('fixed40 full independent source/control declarations')
    for index,(p,declaration) in enumerate(zip(expected,declarations)):
        occurrence,physical,kind,mode=declaration
        expected_kind='binary' if index==6 else ('source' if index>=8 else 'evidence')
        if tuple(physical)!=fixed._pin_tuple(p) or kind!=expected_kind or mode!='bytes':
            raise ValueError('fixed40 exact independent full source/control declaration')
        unit.consume(occurrence,p,kind=kind,live_bytes=live_bytes+fixed.graph_charge((inputs,source,tools,expected,review)))
    del source,tools,expected,review
    native_pins={k:pins[k] for k in ('capture_report','lead_report','independent_report','bundle')}
    native_pins['independent_verification']=pins['review_result']
    checked=lead_verification(maxima=maxima,slot=slot,capture_raw=capture_raw,lead_raw=lead_raw,
        independent_raw=replay_raw,independent_verification_raw=review_raw,pins=native_pins,
        unit=unit,declarations=native_declarations,before=metadata,after=metadata,
        phase_peaks=phase_peaks,live_bytes=live_bytes+fixed.graph_charge(
            (inputs,native_pins,metadata,ready,source_commit,run,control_names,row)))
    del checked,native_pins
    review=decode(review_raw)
    for name in ('capture_report','lead_report','independent_report','independent_result','independent_log',
                 'replay_binary','bundle','freeze','live_result'):
        if review[name]!=pins[name]:raise ValueError('fixed40 independent actual control/result binding')
    if review['source_commit']!=source_commit or review['run']!=run or review['ready']!=ready:
        raise ValueError('fixed40 independent actual source/run/READY binding')
    report=decode(replay_raw,(review,))
    # Every original56 field survives; all native-derived values are rebuilt
    # from the actual successful replay, not copied from reviewer counters.
    value={k:review[k] for k in INDEPENDENT_VERIFICATION_FIELDS}
    value.update(schema=1,profile=report['profile'],source_commit=source_commit,run=run,
        source_count=90,tool_count=11,binary_count=2,accepted=False,verified=True,new_facts=0,
        status='ACCEPT_INDEPENDENT_OFFLINE_FIXED40_REPLAY_ONLY',constructor_proofs=51,
        payload_proofs=40,current_proofs=91,fresh_prior_proofs=71,prior_payload_bindings=20,
        exact_eof_verified=True,canonical_reports_byte_equal=True,metadata_before_after_equal=True,
        full_database_preservation_proven=False,values=report['values'],qualification=report['qualification'],
        selection_sha256=report['selection_sha256'],replay_usage=report['actual'],
        prior_usage=report['authority']['prior_first_d']['actual'],raw_artifact_count=7,
        raw_artifact_bytes=6785636,resource_limits=report['bounds']['storage_resources'],
        omitted_delayed_indices=list(fixed.OMISSIONS),
        selection_delayed_indices=[s['tuple']['identity']['period']-25706894 for s in report['selection']],
        selected_prior=dict(one_based=18,zero_based=17,parent_manifest_index=53,proof_index=68,
            selected=report['authority']['prior_first_d']['selected']))
    if (base+fixed.graph_charge((review,report,value))>core.SEMANTIC):
        raise ValueError('fixed40 independent output caller lifetime')
    return _constructor_output(value,maximum)


def prospective_source_commit_record(*, maxima, planned_raw, planned_pin,
                                     source_commit, source_acceptance, log,
                                     owned_sources, unit, declarations, live_bytes):
    """Model role30 with a complete unexecuted commit control and full reads.

    The control has the production field graph, but a nonhistorical schema
    and impossible successful exit. Production commit consumers reject it.
    Every source body, acceptance specimen, log and control is authenticated;
    this model cannot stand for a completed commit or source acceptance.
    """
    maximum=_role_maximum(30,maxima)
    inputs=(planned_raw,planned_pin,source_commit,source_acceptance,log,
            owned_sources,declarations)
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    if type(planned_raw) is not bytes or len(planned_raw)>65536:
        raise ValueError('fixed40 complete prospective commit control')
    if (full+2*fixed.graph_charge(inputs)+3*maximum
            +160*core._wire_nodes(planned_raw)+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective commit predecode lifetime')
    fixed._pin_tuple(planned_pin)
    if (type(planned_raw) is not bytes or len(planned_raw)>65536
            or len(planned_raw)!=planned_pin['bytes']
            or hashlib.sha256(planned_raw).hexdigest()!=planned_pin['sha256']):
        raise ValueError('fixed40 complete prospective commit control')
    value=records.decode(planned_raw)
    fields={'schema','command','exit_code','hooks_path','log','no_push','normal_hook',
            'owned_sources','parent','source_acceptance','source_commit'}
    if (type(value) is not dict or set(value)!=fields
            or value['schema']!=PROSPECTIVE_MODE
            or type(value['exit_code']) is not int or value['exit_code']!=2147483647
            or value['hooks_path']!='.githooks' or value['normal_hook'] is not True
            or value['no_push'] is not True
            or type(value['command']) is not list or len(value['command'])!=6
            or value['command'][:5]!=['rtk','proxy','git','commit','-m']
            or type(value['command'][5]) is not str or not value['command'][5]
            or type(value['parent']) is not str
            or re.fullmatch('[0-9a-f]{40}',value['parent']) is None
            or type(source_commit) is not str
            or re.fullmatch('[0-9a-f]{40}',source_commit) is None
            or value['source_commit']!=source_commit
            or value['source_acceptance']!=source_acceptance or value['log']!=log
            or value['owned_sources']!=owned_sources):
        raise ValueError('fixed40 nonhistorical complete prospective commit fields')
    # The complete current closure is mandatory in this fit path; there is no
    # shorter historical roster substitute and no source-read alias discount.
    _source_lists(owned_sources[:90],owned_sources[90:])
    fixed._pin_tuple(source_acceptance);fixed._pin_tuple(log)
    core.integer(log['bytes'],ROLE_MAXIMA[31],'fixed40 prospective hook log bound')
    pins=(planned_pin,source_acceptance,log,*owned_sources)
    if [row[2] for row in declarations]!=['evidence']*3+['source']*101:
        raise ValueError('fixed40 prospective commit full source read kinds')
    _prospective_reads(unit,pins,declarations,
        live_bytes+fixed.graph_charge((inputs,value)))
    result=dict(schema=PROSPECTIVE_MODE,source_commit=source_commit,
        source_acceptance=source_acceptance,actual_commit=planned_pin,log=log,no_push=True)
    return _prospective_output(30,result,(inputs,value),maxima,full)


def prospective_normal_hook_summary(*, maxima, planned_raw, planned_pin,
                                    full_log, unit, declarations, live_bytes):
    """Model role32 without passing test lines or a completed hook claim.

    The full role31 log is an explicit unexecuted prospective record stream.
    It is hashed and scanned in full with the production byte-line grammar.
    matched_tests is a capacity scalar, never an observed passing test count.
    The production normal_hook_summary rejects the distinctive control schema.
    """
    maximum=_role_maximum(32,maxima)
    inputs=(planned_raw,planned_pin,full_log,declarations)
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    fixed._pin_tuple(planned_pin)
    if (type(planned_raw) is not bytes or len(planned_raw)>65536
            or len(planned_raw)!=planned_pin['bytes']
            or hashlib.sha256(planned_raw).hexdigest()!=planned_pin['sha256']
            or type(full_log) is not bytes or len(full_log)>ROLE_MAXIMA[31]):
        raise ValueError('fixed40 prospective full hook inputs')
    if (full+fixed.graph_charge(inputs)+3*len(planned_raw)
            +160*core._wire_nodes(planned_raw)+3*maximum
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective hook predecode and scan lifetime')
    # The byte-offset scan makes no full-log UTF8/split copies. Full input
    # bodies remain in inputs; planned-control parser/output reserves stay.
    control=records.decode(planned_raw)
    if (type(control) is not dict or control.get('schema')!=PROSPECTIVE_MODE
            or type(control.get('exit_code')) is not int
            or control['exit_code']!=2147483647
            or control.get('normal_hook') is not True or control.get('no_push') is not True
            or control.get('hooks_path')!='.githooks'
            or type(control.get('source_commit')) is not str
            or re.fullmatch('[0-9a-f]{40}',control['source_commit']) is None):
        raise ValueError('fixed40 prospective hook unexecuted control')
    expected={'schema','command','exit_code','hooks_path','log','no_push','normal_hook',
        'owned_sources','parent','source_acceptance','source_commit'}
    if (set(control)!=expected or type(control['command']) is not list
            or len(control['command'])!=6
            or control['command'][:5]!=['rtk','proxy','git','commit','-m']
            or type(control['command'][5]) is not str or not control['command'][5]
            or type(control['parent']) is not str
            or re.fullmatch('[0-9a-f]{40}',control['parent']) is None):
        raise ValueError('fixed40 prospective full hook control fields')
    _source_lists(control['owned_sources'][:90],control['owned_sources'][90:])
    fixed._pin_tuple(control['source_acceptance'])
    log=control['log'];fixed._pin_tuple(log)
    if len(full_log)!=log['bytes'] or hashlib.sha256(full_log).hexdigest()!=log['sha256']:
        raise ValueError('fixed40 prospective complete hook log binding')
    # Require the explicit unexecuted header. Do not manufacture production
    # success lines for the scan or substitute an old successful hook.
    first_end=full_log.find(b'\n')
    if first_end<0:first_end=len(full_log)
    header_nodes=core._wire_nodes(memoryview(full_log)[:first_end])
    if (full+fixed.graph_charge((inputs,control))+3*first_end+160*header_nodes
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError('fixed40 prospective hook header predecode lifetime')
    first=full_log[:first_end]
    header=records.decode(first)
    first=None  # Release the header slice before reads/scan/output; header stays.
    if (type(header) is not dict or header.get('mode')!=PROSPECTIVE_MODE
            or header.get('executed') is not False):
        raise ValueError('fixed40 prospective hook nonhistorical log header')
    _prospective_reads(unit,(planned_pin,log),declarations,
        live_bytes+fixed.graph_charge((inputs,control,header)))
    pattern=re.compile(rb'test result: ok[.] ([0-9]+) passed; 0 failed;.*')
    start=0
    while start<len(full_log):
        stop=full_log.find(b'\n',start)
        if stop<0:stop=len(full_log)
        end=stop-1 if stop>start and full_log[stop-1]==13 else stop
        if pattern.fullmatch(full_log,start,end):
            raise ValueError('fixed40 prospective hook cannot claim passing tests')
        start=stop+1
    value=dict(schema=PROSPECTIVE_MODE,source_commit=control['source_commit'],
        exit_code=2147483647,bypassed=False,matched_tests=2147483647,log=log)
    return _prospective_output(32,value,(inputs,control,header),maxima,full)


def prospective_command_verification(*, maxima, freeze, exact_commands_pin,
        source_commit, rebuilt_freeze_raw, actual_freeze_raw, model_reads,
        phase_peaks, history, copy_metadata, child_occurrences, strict_pending,
        unit, declarations, live_bytes):
    """Model all role61 ledger fields without claiming actual future reads.

    Freeze equality is a real full-byte comparison of the supplied specimen.
    model_reads and phase_peaks describe the reserved prospective schedule.
    Production verification and actual metadata/read checks remain required.
    """
    maximum=_role_maximum(61,maxima)
    inputs=(freeze,exact_commands_pin,source_commit,rebuilt_freeze_raw,
        actual_freeze_raw,model_reads,phase_peaks,history,copy_metadata,declarations)
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    _constructor_lifetime(61,maxima,inputs,full)
    fixed._pin_tuple(freeze);fixed._pin_tuple(exact_commands_pin);_copy_rows(copy_metadata)
    if (type(source_commit) is not str or re.fullmatch('[0-9a-f]{40}',source_commit) is None
            or type(actual_freeze_raw) is not bytes or type(rebuilt_freeze_raw) is not bytes
            or len(actual_freeze_raw)!=freeze['bytes']
            or hashlib.sha256(actual_freeze_raw).hexdigest()!=freeze['sha256']
            or actual_freeze_raw!=rebuilt_freeze_raw):
        raise ValueError('fixed40 prospective full freeze comparison')
    fields={'actual_binary_bytes','actual_bytes','actual_nonbinary_bytes',
        'actual_source_bytes','charged_nonbinary_bytes','charged_total_bytes',
        'open_occurrences','pending_and_outcome_reserved_bytes'}
    if (type(model_reads) is not dict or set(model_reads)!=fields
            or any(type(x) is not int or x<0 for x in model_reads.values())
            or model_reads['pending_and_outcome_reserved_bytes']!=sum(ROLE_MAXIMA[57:])
            or model_reads['actual_bytes']!=model_reads['actual_binary_bytes']+model_reads['actual_nonbinary_bytes']
            or model_reads['charged_total_bytes']!=model_reads['actual_binary_bytes']+model_reads['charged_nonbinary_bytes']
            or type(phase_peaks) is not dict or not phase_peaks
            or any(type(x) is not int or not 0<=x<=core.SEMANTIC for x in phase_peaks.values())
            or type(history) is not dict or set(history)!=
                {'body_bytes','charged_bytes','containers','members'}
            or any(type(x) is not int or x<0 for x in history.values())
            or type(child_occurrences) is not int or child_occurrences!=64
            or type(strict_pending) is not int or strict_pending<0):
        raise ValueError('fixed40 prospective complete verification ledger')
    _prospective_reads(unit,(freeze,exact_commands_pin),declarations,
        live_bytes+fixed.graph_charge(inputs))
    value=dict(schema=PROSPECTIVE_MODE,freeze=freeze,exact_commands=exact_commands_pin,
        source_commit=source_commit,actual_reads=model_reads,phase_peaks=phase_peaks,
        history=history,copy_metadata=copy_metadata,child_occurrences=child_occurrences,
        strict_pending=strict_pending,deterministic_byte_equal=True,captured=False,
        ready=False,new_facts=0,full_database_preservation_proven=False,
        status=PROSPECTIVE_MODE)
    return _prospective_output(61,value,inputs,maxima,full)


def prospective_capture_execution_decision(*, maxima, source_commit, command,
        inner_command, freeze, exact_commands_pin, ready, execution_helper,
        unit, declarations, live_bytes, guard_deferred=None):
    """Model role62 complete command/READY inputs without execution authority.

    All four complete input bodies are read. The original independent READY
    predicates, real source acceptance and runtime guard are still mandatory.
    """
    maximum=_role_maximum(62,maxima)
    inputs=(source_commit,command,inner_command,freeze,exact_commands_pin,ready,
        execution_helper,declarations,guard_deferred)
    full=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    _constructor_lifetime(62,maxima,inputs,full)
    if type(source_commit) is not str or re.fullmatch('[0-9a-f]{40}',source_commit) is None:
        raise ValueError('fixed40 prospective decision source commit')
    for pin in (freeze,exact_commands_pin,ready,execution_helper):fixed._pin_tuple(pin)
    if guard_deferred is None:
        _command_argv(command)
    else:
        guard=_checked_guard_deferred(guard_deferred)
        if (command is not None or guard['inner_argv']!=inner_command
                or guard['reference_source']!=execution_helper):
            raise ValueError('fixed40 deferred decision exact source/inner binding')
    _command_argv(inner_command)
    if len(inner_command)!=5 or inner_command[1::2]!=['--bundle-output','--report-output']:
        raise ValueError('fixed40 prospective complete decision native argv')
    if [row[2] for row in declarations]!=['evidence']*3+['source']:
        raise ValueError('fixed40 prospective decision read kinds')
    _prospective_reads(unit,(freeze,exact_commands_pin,ready,execution_helper),
        declarations,live_bytes+fixed.graph_charge(inputs))
    value=dict(schema=PROSPECTIVE_MODE,source_commit=source_commit,command=command,
        inner_command=inner_command,freeze=freeze,exact_commands=exact_commands_pin,
        ready=ready,execution_helper=execution_helper,accepted_facts_before=805,
        residual_before=109,new_facts=0,no_retry=True,status=PROSPECTIVE_MODE)
    if guard_deferred is not None:value['guardDeferred']=guard_deferred
    return _prospective_output(62,value,inputs,maxima,full)


def prospective_validation_summary(*, maxima, gates, source_commit,
        source_acceptance, source_freeze, tool_source_freeze,
        source_commit_record, actual_hook, hook_summary,
        complete_offline_fit_review, authority, bridge_scope, cache_scope,
        deferred_review_raw, current_profile_raw, source_bindings,
        review_body_maximum, unit, declarations, live_bytes):
    """Render role29 with a closed, nonhistorical future review obligation.

    This input binds an acyclic context derivation, not an observed matrix or
    reviewer verdict. Full gate, profile, reference and source bodies remain
    mandatory. Production validation_summary does not accept this obligation
    as an actual complete-fit review. The maximum is supplied by the frozen
    source-backed review contract and is repeated exactly in the obligation.
    """
    maximum=_role_maximum(29,maxima)
    core.integer(review_body_maximum,records.WRAPPER_CAP,
                 "fixed40 frozen future review body maximum")
    if review_body_maximum!=records.WRAPPER_CAP:
        raise ValueError("fixed40 exact existing wrapper review body maximum")
    inputs=(gates,source_commit,source_acceptance,source_freeze,
        tool_source_freeze,source_commit_record,actual_hook,hook_summary,
        complete_offline_fit_review,authority,bridge_scope,cache_scope,
        deferred_review_raw,current_profile_raw,source_bindings,declarations)
    owner=unit.invocation.metadata_charge()+unit.declaration_charge+live_bytes
    if (type(deferred_review_raw) is not bytes
            or len(deferred_review_raw)>65536
            or type(current_profile_raw) is not bytes
            or len(current_profile_raw)>131072):
        raise ValueError("fixed40 complete deferred review inputs")
    # Decode only the active full document. The other immutable wire stays
    # charged in inputs; no source/declaration/caller owner is discounted.
    if (owner+fixed.graph_charge(inputs)
            +3*len(current_profile_raw)+160*core._wire_nodes(current_profile_raw)
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError(f"fixed40 deferred profile predecode lifetime owner={owner}")
    profile=records.decode(current_profile_raw,131072)
    if type(profile) is not dict or profile.get("sources")!=source_bindings:
        raise ValueError("fixed40 exact current profile and full101 source binding")
    profile_binding=(len(current_profile_raw),
        hashlib.sha256(current_profile_raw).hexdigest(),profile.get("source_identity"))
    core.sha(profile_binding[2])
    del profile
    if (owner+fixed.graph_charge(inputs)+fixed.graph_charge(profile_binding)
            +3*len(deferred_review_raw)+160*core._wire_nodes(deferred_review_raw)
            +3*maximum+core.CHUNK+262144>core.SEMANTIC):
        raise ValueError("fixed40 deferred review predecode lifetime")
    value=records.decode(deferred_review_raw,65536)
    fields={"schema","mode","history","status","accepted","all16_accepted",
        "production_authority","current_profile","source_identity","sources",
        "gates","bindings","context_derivation","descriptor_template",
        "schedule_template","future_review_body_maximum","actual_review_required"}
    if (type(value) is not dict or set(value)!=fields
            or value["schema"]!="fixed40-deferred-complete-fit-review-v1"
            or value["mode"]!=PROSPECTIVE_MODE
            or value["history"]!="NONHISTORICAL"
            or value["status"]!="NOT_EXECUTED"
            or value["accepted"] is not False
            or value["all16_accepted"] is not False
            or value["production_authority"] is not False
            or value["actual_review_required"] is not True
            or value["future_review_body_maximum"]!=review_body_maximum):
        raise ValueError("fixed40 closed nonhistorical complete-fit obligation")
    del fields
    refs=dict(source_acceptance=source_acceptance,source_freeze=source_freeze,
        tool_source_freeze=tool_source_freeze,source_commit_record=source_commit_record,
        actual_hook=actual_hook,hook_summary=hook_summary,
        complete_offline_fit_review=complete_offline_fit_review,authority=authority,
        bridge_scope=bridge_scope,cache_scope=cache_scope)
    # The obligation contains the other nine bindings. Its own pin is not
    # embedded in itself; the resulting role29 record binds that full raw pin.
    acyclic={k:v for k,v in refs.items() if k!="complete_offline_fit_review"}
    if value["bindings"]!=acyclic or value["gates"]!=gates:
        raise ValueError("fixed40 complete deferred gate and reference equality")
    del acyclic
    fixed._pin_tuple(complete_offline_fit_review)
    if (len(deferred_review_raw)!=complete_offline_fit_review["bytes"]
            or hashlib.sha256(deferred_review_raw).hexdigest()
                !=complete_offline_fit_review["sha256"]):
        raise ValueError("fixed40 full deferred obligation raw identity")
    profile_pin=value["current_profile"];fixed._pin_tuple(profile_pin)
    if (profile_binding[0]!=profile_pin["bytes"]
            or profile_binding[1]!=profile_pin["sha256"]
            or profile_binding[2]!=value["source_identity"]
            or value["sources"]!=source_bindings):
        raise ValueError("fixed40 exact current profile and full101 source binding")
    core.sha(value["source_identity"])
    _source_lists(source_bindings[:90],source_bindings[90:])
    derivation=value["context_derivation"]
    context_fields={"run","manifest","source","verifier","executables","phase",
        "route","policy","ledger","setup_policy","setup_ledger",
        "setup_publication","event_history"}
    if (type(derivation) is not dict or set(derivation)!={
            "schema","round_index","routes","context_fields","known",
            "manifest_slots","executable_slots","phase_format"}
            or derivation["schema"]!="fixed40-context-derivation-v1"
            or type(derivation["round_index"]) is not int
            or derivation["round_index"] not in (0,1)
            or derivation["routes"]!=list(fixed.framing_routes())
            or derivation["context_fields"]!=sorted(context_fields)
            or derivation["phase_format"]!="development:{round}:0:{local_ordinal}"
            or type(derivation["known"]) is not dict
            or set(derivation["known"])!=context_fields-{
                "manifest","executables","phase","route"}
            or derivation["known"]["source"]!=core.digest((
                source_bindings[:90],source_bindings[90:]))):
        raise ValueError("fixed40 exact sixteen-context derivation")
    del context_fields
    known=derivation["known"]
    if type(known["run"]) is not str or not known["run"]:
        raise ValueError("fixed40 concrete context run binding")
    for name in ("source","verifier","policy","ledger","setup_policy",
                 "setup_ledger","setup_publication","event_history"):
        core.sha(known[name])
    expected_slots=[dict(route=route,global_index=ordinal,
        local_ordinal=int(route.rsplit(':',1)[1]),
        kind="exact-frozen-manifest-digest",state="RESOLVE_FROM_FULL_PROGRAM")
        for ordinal,route in enumerate(fixed.framing_routes())]
    if derivation["manifest_slots"]!=expected_slots:
        raise ValueError("fixed40 exact acyclic manifest digest slots")
    slots=derivation["executable_slots"]
    if type(slots) is not list or len(slots)!=2:
        raise ValueError("fixed40 exact two owned executable identity slots")
    for slot in slots:
        if (type(slot) is not dict or set(slot)!={"path","bytes","kind","owner"}
                or slot["kind"]!="OWNED_FULL_SHA256_RESOLUTION"
                or slot["owner"]!="accepted-bootstrap-schedule"):
            raise ValueError("fixed40 closed owned executable slot")
        records.canonical_path(slot["path"])
        core.integer(slot["bytes"],core.BINARY_FILE,"fixed40 executable slot file")
        if slot["bytes"]==0:raise ValueError("fixed40 positive executable slot")
    if slots[0]["path"]==slots[1]["path"] or sum(x["bytes"] for x in slots)>core.BINARY_PAIR:
        raise ValueError("fixed40 executable slot pair bound")
    for name in ("descriptor_template","schedule_template"):
        fixed._pin_tuple(value[name])
    summary=validation_summary(maxima=maxima,gates=gates,source_commit=source_commit,
        source_acceptance=source_acceptance,source_freeze=source_freeze,
        tool_source_freeze=tool_source_freeze,source_commit_record=source_commit_record,
        actual_hook=actual_hook,hook_summary=hook_summary,
        complete_offline_fit_review=complete_offline_fit_review,authority=authority,
        bridge_scope=bridge_scope,cache_scope=cache_scope,
        live_bytes=owner+fixed.graph_charge((inputs,value,profile_binding,expected_slots,refs)))
    pins=(complete_offline_fit_review,profile_pin,*refs.values(),
          *(pin for row in gates for pin in (row["result"],row["log"])),
          value["descriptor_template"],value["schedule_template"],*source_bindings)
    del refs
    evidence_count=len(pins)-101
    if [row[2] for row in declarations]!=["evidence"]*evidence_count+["source"]*101:
        raise ValueError("fixed40 deferred review full input classifications")
    _prospective_reads(unit,pins,declarations,
        live_bytes+fixed.graph_charge((inputs,value,profile_binding,expected_slots,summary,pins)))
    # Every validation-only owner is dead before the output graph exists.
    # The retained input roots below include the complete decoded obligation,
    # profile, source roster, gate bindings and original raw summary.
    del derivation,known,expected_slots,slots,slot
    del name,pins,evidence_count,profile_pin
    if (owner+fixed.graph_charge((inputs,value,profile_binding,summary))
            +3*len(summary)+160*core._wire_nodes(summary)
            +core.CHUNK+262144>core.SEMANTIC):
        raise ValueError("fixed40 deferred review final summary predecode lifetime")
    result=records.decode(summary,maximum)
    result.update(schema=PROSPECTIVE_MODE,status="NOT_EXECUTED",history="NONHISTORICAL",
        accepted=False,all16_accepted=False,production_authority=False,
        actual_review_required=True,deferred_review=complete_offline_fit_review)
    return _prospective_output(29,result,(inputs,value,profile_binding,summary),maxima,owner)

# Round152 saved-budget instance; all earlier identities remain unchanged.
QUOTA_BUDGET_IDENTITIES['9f3d646a2d506f7aeb79347c2d1849251ba8393996a16ffe291cc518ef427475'] = '/home/fry/artifacts/vrf40-generation-20261024-round152/quota-budget.json'
QUOTA_BUDGET_WIDTHS['9f3d646a2d506f7aeb79347c2d1849251ba8393996a16ffe291cc518ef427475'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['9f3d646a2d506f7aeb79347c2d1849251ba8393996a16ffe291cc518ef427475'] = 10

# Round153 saved-budget instance; all earlier identities remain unchanged.
QUOTA_BUDGET_IDENTITIES['43757136fef817299f46151d78b9059cb4fd45388476b72874363a7715705c08'] = '/home/fry/artifacts/vrf40-gen17-20261024-round153/quota-budget.json'
QUOTA_BUDGET_WIDTHS['43757136fef817299f46151d78b9059cb4fd45388476b72874363a7715705c08'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['43757136fef817299f46151d78b9059cb4fd45388476b72874363a7715705c08'] = 10

# Round154 saved-budget instance; all previous identities are preserved.
QUOTA_BUDGET_IDENTITIES['267fc4ba27c7c985f82628a34990e2997bcf18bcae0dfe31cc05d7d08e1d7587'] = '/home/fry/artifacts/vrf40-alias577-20261024-round154/quota-budget.json'
QUOTA_BUDGET_WIDTHS['267fc4ba27c7c985f82628a34990e2997bcf18bcae0dfe31cc05d7d08e1d7587'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['267fc4ba27c7c985f82628a34990e2997bcf18bcae0dfe31cc05d7d08e1d7587'] = 10

# Round155 budget instance; all previous identities and rules remain.
QUOTA_BUDGET_IDENTITIES['18eb6f30a422db55b146307ca2fab45bf901532edd6be3ee92589c5b90403773'] = '/home/fry/artifacts/vrf40-gen20-20261024-round155/quota-budget.json'
QUOTA_BUDGET_WIDTHS['18eb6f30a422db55b146307ca2fab45bf901532edd6be3ee92589c5b90403773'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['18eb6f30a422db55b146307ca2fab45bf901532edd6be3ee92589c5b90403773'] = 10

# Round156: separate saved ten-point budget; previous identities remain.
QUOTA_BUDGET_IDENTITIES['13865f51521f56a3f4e121f063614bad057ebf9f947ca1e6c529a90396d8f31e'] = '/home/fry/artifacts/vrf40-role42-20261024-round156/quota-budget.json'
QUOTA_BUDGET_WIDTHS['13865f51521f56a3f4e121f063614bad057ebf9f947ca1e6c529a90396d8f31e'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['13865f51521f56a3f4e121f063614bad057ebf9f947ca1e6c529a90396d8f31e'] = 10

# Round157: separate saved ten-point budget; all older identities remain.
QUOTA_BUDGET_IDENTITIES['20061e7a6d4a74cf59f25113b76f9779e116b5e3d7755dc13988319c5918de4b'] = '/home/fry/artifacts/vrf40-native54-20261025-round157/quota-budget.json'
QUOTA_BUDGET_WIDTHS['20061e7a6d4a74cf59f25113b76f9779e116b5e3d7755dc13988319c5918de4b'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['20061e7a6d4a74cf59f25113b76f9779e116b5e3d7755dc13988319c5918de4b'] = 10

# Round158: independent ten-point budget; every previous identity remains.
QUOTA_BUDGET_IDENTITIES['ebf3890edcdd2e8fd59e2d094722e8d309296b43d44e33ae5b40f863043844df'] = '/home/fry/artifacts/vrf40-launch-20261025-round158/quota-budget.json'
QUOTA_BUDGET_WIDTHS['ebf3890edcdd2e8fd59e2d094722e8d309296b43d44e33ae5b40f863043844df'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['ebf3890edcdd2e8fd59e2d094722e8d309296b43d44e33ae5b40f863043844df'] = 10

# Round159: separate ten-point budget; every older identity remains.
QUOTA_BUDGET_IDENTITIES['54734d020bff9f39a417e8a96816c6ad7fc03207fc2905d4374e437bb98c4efd'] = '/home/fry/artifacts/vrf40-gen25-20261025-round159/quota-budget.json'
QUOTA_BUDGET_WIDTHS['54734d020bff9f39a417e8a96816c6ad7fc03207fc2905d4374e437bb98c4efd'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['54734d020bff9f39a417e8a96816c6ad7fc03207fc2905d4374e437bb98c4efd'] = 10

# Round160: one separate saved20-point allowance; preserve all prior identities.
QUOTA_BUDGET_IDENTITIES['eee18ba4a2aba8901e8ae6b14d2def895f79824df5c9d54696e183fdf7d2379b'] = '/home/fry/artifacts/vrf40-role41-20261025-round160/quota-budget.json'
QUOTA_BUDGET_WIDTHS['eee18ba4a2aba8901e8ae6b14d2def895f79824df5c9d54696e183fdf7d2379b'] = 277
QUOTA_BUDGET_RESET_TOLERANCES['eee18ba4a2aba8901e8ae6b14d2def895f79824df5c9d54696e183fdf7d2379b'] = 10

# Round161: independent saved20-point budget; all old identities remain.
QUOTA_BUDGET_IDENTITIES['6d0d225cf991d81489bc3ede3d936beda4825db83c40dccd195434a9aff46d7a'] = '/home/fry/artifacts/vrf40-gen26-20261025-round161/quota-budget.json'
QUOTA_BUDGET_WIDTHS['6d0d225cf991d81489bc3ede3d936beda4825db83c40dccd195434a9aff46d7a'] = 277
QUOTA_BUDGET_RESET_TOLERANCES['6d0d225cf991d81489bc3ede3d936beda4825db83c40dccd195434a9aff46d7a'] = 10

# Round162: separate saved20-point allowance; preserve every prior identity.
QUOTA_BUDGET_IDENTITIES['4913a624c31661e1e4730a3e5760393be028b3926d9478a77ebe6f3626eddd55'] = '/home/fry/artifacts/vrf40-role61-20261025-round162/quota-budget.json'
QUOTA_BUDGET_WIDTHS['4913a624c31661e1e4730a3e5760393be028b3926d9478a77ebe6f3626eddd55'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['4913a624c31661e1e4730a3e5760393be028b3926d9478a77ebe6f3626eddd55'] = 10

# Round163: separate saved20-point allowance; old budgets remain immutable.
QUOTA_BUDGET_IDENTITIES['646b61ab9c056d8c418ba33826b903a8b844817c39db7c79ebe0040de8cdf575'] = '/home/fry/artifacts/vrf40-gen30-20261026-round163/quota-budget.json'
QUOTA_BUDGET_WIDTHS['646b61ab9c056d8c418ba33826b903a8b844817c39db7c79ebe0040de8cdf575'] = 278
QUOTA_BUDGET_RESET_TOLERANCES['646b61ab9c056d8c418ba33826b903a8b844817c39db7c79ebe0040de8cdf575'] = 10
