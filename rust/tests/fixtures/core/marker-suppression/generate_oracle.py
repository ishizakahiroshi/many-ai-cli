#!/usr/bin/env python3
"""Pinned-Go suppression call ordering, pure state and legacy predicate evidence.

Only standard-library code runs in a synthetic temporary package. Probe adapters
record eligibility/order, not successful snapshots or dumps. No Hub, filesystem
diagnostic sink, provider, credentials, network or notification service is used.
"""
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import re
import subprocess
import tempfile

BASELINE = '21d0bc7935a2c4696fb89ccff2e324157a528c2d'
FILES = [
    'internal/hub/approval_marker.go',
    'internal/hub/approval_marker_verdict.go',
    'internal/sessionlog/sessionlog.go',
    'internal/hub/approval_native.go',
    'internal/hub/approval_identity.go',
    'internal/hub/approval_record.go',
    'internal/hub/approval_text_question.go',
    'internal/hub/approval_marker_transcript.go',
    'internal/hub/approval_action.go',
    'internal/hub/input_gate.go',
    'internal/hub/ui_broadcast.go',
    'internal/hub/wrapper_loop.go',
    'internal/hub/reattach_state.go',
    'internal/hub/server.go',
    'internal/hub/probe_debug.go',
    'internal/hub/probe_nodebug.go',
    'internal/proto/messages.go',
    'instrumentation.json',
]


def function(source, name):
    match = re.search(r'^func (?:\([^\n]*?\) )?' + re.escape(name) + r'\(', source, re.M)
    if not match:
        raise ValueError(name)
    return source[match.start():source.index('\n}', match.start()) + 2]


def line(source, exact):
    matches = [value.strip() for value in source.splitlines() if exact in value]
    if len(matches) != 1:
        raise ValueError((exact, matches))
    return matches[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', required=True, type=Path)
    parser.add_argument('--work-root', required=True, type=Path)
    args = parser.parse_args()
    if not args.go.is_absolute() or not args.work_root.is_absolute():
        parser.error('explicit absolute tool and scratch paths required')
    destination = Path(__file__).resolve().parent
    repo = destination.parents[4]
    sources = {path: subprocess.check_output(
        ['git', 'show', f'{BASELINE}:{path}'], cwd=repo).decode() for path in FILES}
    marker = sources[FILES[0]]
    verdict = sources[FILES[1]]
    log = sources[FILES[2]]
    native = sources[FILES[3]]
    wrapper = sources['internal/hub/wrapper_loop.go']
    server = sources['internal/hub/server.go']
    pure = function(marker, 'maybeBroadcastApprovalMarkerFrom').replace('proto.', '')
    pure += '\n' + verdict[verdict.index('const ('):].replace('sessionlog.', '')
    pure += '\nvar (\n' + '\n'.join(
        re.search(r'^\s*' + name + r'\s*=.*$', log, re.M).group()
        for name in ['oscRE', 'ansiRE', 'ansiSimpleRE']) + '\n)\n'
    pure += function(log, 'StripANSI')
    constant = line(marker, 'const approvalMarkerSuppressNotifyInterval =')
    ttl = 'const ' + line(server, 'approvalConsumedTTL          =')
    start = native.index('\tlegacyConsumed :=')
    predicate = native[start:native.index('\n\tif answered', start)]
    reset = '\n'.join(line(server, 'ses.' + name + ' =') for name in [
        'approvalMarkerSuppressedSig', 'approvalMarkerSuppressedAt'])
    carry_reads = '\n'.join(line(wrapper, name + ' =') for name in [
        'prevApprovalMarkerSuppressedSig', 'prevApprovalMarkerSuppressedAt'])
    carry_writes = '\n'.join(line(wrapper, name + ':') for name in [
        'approvalMarkerSuppressedSig', 'approvalMarkerSuppressedAt'])
    scaffolding = r'''
package main
import("encoding/json"; "fmt"; "os"; "regexp"; "strconv"; "strings"; "time")
const approvalKindMarker="marker"
type approvalMarkerBlock struct {Block string `json:"block"`; Sig string `json:"sig"`}
type textQuestion struct {Kind,Block,Sig string}
type session struct {Provider string; approvalMarkerSuppressedSig string; approvalMarkerSuppressedAt time.Time; pendingApproval string; nativeApprovalConsumed string; nativeApprovalConsumedAt time.Time; approvalConsumedCandidateKey string}
type Message struct {Type string `json:"type"`; SessionID int `json:"session_id"`; Provider string `json:"provider"`; ApprovalSig string `json:"approval_sig"`; ApprovalSource string `json:"approval_source"`; Reason string `json:"reason"`; DetectedAt string `json:"detected_at"`}
type Time struct {Seconds int64 `json:"seconds"`; Nanos int64 `json:"nanos"`}
func(t Time) value() time.Time{return time.Unix(t.Seconds,t.Nanos).UTC()}
func stamp(t time.Time) Time{return Time{t.Unix(),int64(t.Nanosecond())}}
type mutex struct{s *Server; held bool}
func(m *mutex) Lock(){if m.held{panic("recursive fixture lock")};m.held=true;m.s.trace=append(m.s.trace,"lock")}
func(m *mutex) Unlock(){if !m.held{panic("unheld fixture unlock")};m.held=false;m.s.trace=append(m.s.trace,"unlock")}
type logger struct{s *Server}
func(l *logger) Warn(message string, fields ...any){if l.s.sessionsMu.held{panic("warning under lock")};l.s.trace=append(l.s.trace,"warn");out:=map[string]any{"message":message};for i:=0;i<len(fields);i+=2{out[fields[i].(string)]=fields[i+1]};l.s.warnings=append(l.s.warnings,out)}
type Server struct{sessions map[int]*session; sessionsMu mutex; logger *logger; trace []string; warnings []any; broadcasts []Message; openResult bool}
func(s *Server) probe(channel string,args ...any){
 if channel=="approval-corrupt-snapshot" && !s.sessionsMu.held {panic("snapshot call site outside lock")}
 if channel=="approval-corrupt-dump" && s.sessionsMu.held {panic("dump call site under lock")}
 s.trace=append(s.trace,"probe-call-site:"+channel)
}
func(s *Server) broadcast(m Message){if s.sessionsMu.held{panic("broadcast under lock")};s.trace=append(s.trace,"broadcast");s.broadcasts=append(s.broadcasts,m)}
func(s *Server) openTextQuestion(id int,q *textQuestion,at time.Time,source string)bool{
 if s.sessionsMu.held{panic("open entry under lock")};s.trace=append(s.trace,"open-question-entry");return s.openResult
}
type Step struct{Marker *approvalMarkerBlock `json:"marker"`; At Time `json:"at"`; Source string `json:"source"`; Reset bool `json:"reset"`; Warm bool `json:"warm"`}
type Scenario struct{Name string `json:"name"`; InitialSig string `json:"initial_sig"`; InitialAt Time `json:"initial_at"`; Missing bool `json:"missing"`; Logger bool `json:"logger"`; OpenResult bool `json:"open_result"`; Steps []Step `json:"steps"`}
type Legacy struct{Key string `json:"key"`; ConsumedSig string `json:"consumed_sig"`; CandidateSig string `json:"candidate_sig"`; At Time `json:"at"`; Now Time `json:"now"`}
'''
    helpers = '''
func reset(ses *session){''' + reset + '''}
func warm(cur *session)*session{
 var prevApprovalMarkerSuppressedSig string
 var prevApprovalMarkerSuppressedAt time.Time
''' + carry_reads + '''
 return &session{Provider:cur.Provider,pendingApproval:cur.pendingApproval,
''' + carry_writes + '''
 }
}
func legacy(c Legacy)bool{
 now:=c.Now.value()
 ses:=&session{approvalConsumedCandidateKey:c.Key,nativeApprovalConsumed:c.ConsumedSig,nativeApprovalConsumedAt:c.At.value()}
 approval:=&approvalMarkerBlock{Sig:c.CandidateSig}
''' + predicate + '''
 return legacyConsumed
}
func main(){
 var in struct{Scenarios []Scenario `json:"scenarios"`; Legacy []Legacy `json:"legacy"`}
 data,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};if e=json.Unmarshal(data,&in);e!=nil{panic(e)}
 scenarios:=[]any{}
 for _,c:=range in.Scenarios{
  ses:=&session{Provider:"synthetic",approvalMarkerSuppressedSig:c.InitialSig,approvalMarkerSuppressedAt:c.InitialAt.value(),pendingApproval:"immutable-record-sentinel"}
  s:=&Server{sessions:map[int]*session{},openResult:c.OpenResult};s.sessionsMu.s=s
  if !c.Missing{s.sessions[7]=ses};if c.Logger{s.logger=&logger{s}}
  steps:=[]any{}
  for _,step:=range c.Steps{
   if step.Reset{reset(ses)};if step.Warm{ses=warm(ses);if !c.Missing{s.sessions[7]=ses}}
   s.trace=[]string{};s.warnings=[]any{};s.broadcasts=[]Message{}
   result:=s.maybeBroadcastApprovalMarkerFrom(7,step.Marker,step.At.value(),step.Source)
   reason:="";if step.Marker!=nil && step.Marker.Block!="" && step.Marker.Sig!=""{reason=classifyApprovalMarkerBlock(step.Marker.Block)}
   steps=append(steps,map[string]any{"input":step,"expected":map[string]any{"result":result,"reason":reason,"sig":ses.approvalMarkerSuppressedSig,"at":stamp(ses.approvalMarkerSuppressedAt),"trace":s.trace,"warnings":s.warnings,"broadcasts":s.broadcasts,"record":ses.pendingApproval}})
  }
  scenarios=append(scenarios,map[string]any{"input":c,"steps":steps})
 }
 rows:=[]any{};for _,c:=range in.Legacy{rows=append(rows,map[string]any{"input":c,"expected":legacy(c)})}
 if e=json.NewEncoder(os.Stdout).Encode(map[string]any{"scenarios":scenarios,"legacy":rows});e!=nil{panic(fmt.Sprint(e))}
}
'''
    zero = {'seconds': -62135596800, 'nanos': 0}
    def at(seconds=100, nanos=0):
        return {'seconds': seconds, 'nanos': nanos}
    def step(sig='a', block='3. Missing first option', seconds=100, nanos=0, **extra):
        return {'marker': {'block': block, 'sig': sig}, 'at': at(seconds, nanos),
                'source': 'go_vt', **extra}
    def scenario(name, steps, initial_sig='', initial_at=None, **extra):
        return {'name': name, 'initial_sig': initial_sig, 'initial_at': initial_at or zero,
                'missing': False, 'logger': True, 'open_result': True, 'steps': steps, **extra}
    cases = {'scenarios': [], 'legacy': []}
    for previous, delta, same in itertools.product(
            [zero, at(0), at(100), at(100, 1), at(-62135596800, 1)],
            [-30000000001, -1, 0, 29999999999, 30000000000, 30000000001, 60000000000],
            [False, True]):
        ns = previous['seconds'] * 1000000000 + previous['nanos'] + delta
        seconds, nanos = divmod(ns, 1000000000)
        cases['scenarios'].append(scenario(
            f'interval-{len(cases["scenarios"])}', [step('a', seconds=seconds, nanos=nanos)],
            'a' if same else 'older', previous))
    cases['scenarios'] += [
        scenario('alternating-throttle-and-warm-reset', [step('a'), step('b', seconds=101),
            step('a', seconds=129, nanos=999999999), step('b', seconds=130),
            step('b', seconds=200, warm=True), step('a', seconds=159, nanos=999999999),
            step('c', seconds=160), step('c', seconds=161, reset=True)]),
        scenario('valid-empty-do-not-clear', [step('a'), step('valid', '1. Valid\n2. Other', 150),
            step('', '', 160), {'marker': None, 'at': at(170), 'source': 'go_vt'},
            step('', '3. Still corrupt', 180), step('empty', '', 190), step('a', seconds=200)]),
        scenario('zero-notice-remains-zero', [step('a', seconds=-62135596800),
            step('b', seconds=-62135596801), step('c', seconds=-62135596801, nanos=1)]),
        scenario('logger-absent', [step('a'), step('b', seconds=101), step('c', seconds=130)], logger=False),
        scenario('missing-session', [step('a'), step('b', seconds=130)], missing=True),
        scenario('valid-open-result-is-not-state', [step('valid', '1. Valid')], open_result=False),
        scenario('empty-guards-precede-corruption', [step('', '3. Missing'), step('sig', '')]),
        scenario('transcript-and-vt-share-one-slot', [step('a', source='transcript'),
            step('a', seconds=131), step('b', seconds=132, source='transcript')]),
    ]
    open_marker = '[' + 'MANY-AI-CLI' + ']'
    close_marker = '[/' + 'MANY-AI-CLI' + ']'
    for name, block in [
            ('marker-leak', open_marker + '\n' + open_marker + '\n1. Valid\n' + close_marker),
            ('box-rule', '1. Label ───\n2. Other\n'),
            ('duplicate-option', '1. First\n2. Other\n2. Duplicate'),
            ('ansi-corrupt', '\x1b[31m3. Missing\x1b[0m'),
            ('rule-only-is-valid', '───\n1. First\n2. Other'),
            ('yes-no-is-valid', '(Y:1/N:0)\n3. Explanation'),
            ('ambiguous-is-valid', 'explanation only')]:
        cases['scenarios'].append(scenario(name, [step('0123456789abcdef', block)]))
    for key, consumed, candidate, delta in itertools.product(
            ['', 'canonical-key'], ['sig', ''], ['sig', ''],
            [-10000000001, -1, 0, 9999999999, 10000000000, 10000000001]):
        seconds, nanos = divmod(100000000000 + delta, 1000000000)
        cases['legacy'].append({'key': key, 'consumed_sig': consumed,
            'candidate_sig': candidate, 'at': at(100), 'now': at(seconds, nanos)})
    # Sub saturates at the signed Duration bound; both comparisons retain their
    # sign. No nanosecond wrap or arithmetic overflow is permitted in the port.
    cases['scenarios'] += [scenario('saturated-positive-sub', [step('a', seconds=253402300799)],
        'older', at(-62135596801)), scenario('saturated-negative-sub', [step('a', seconds=-62135596801)],
        'older', at(253402300799))]
    args.work_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='marker-suppression-', dir=args.work_root) as tmp:
        root = Path(tmp)
        (root / 'main.go').write_text(scaffolding + '\n' + constant + '\n' + ttl + '\n' + pure + helpers)
        (root / 'cases.json').write_text(json.dumps(cases, ensure_ascii=False))
        output = subprocess.check_output([str(args.go), 'run', str(root / 'main.go'), str(root / 'cases.json')], cwd=root)
    result = json.loads(output)
    result['baseline'] = BASELINE
    result['source_files'] = [{'path': path, 'sha256': hashlib.sha256(content.encode()).hexdigest()}
                              for path, content in sources.items()]
    sink = next(entry for entry in json.loads(sources['instrumentation.json'])['entries']
                if entry['id'] == 'approval-corrupt-dump')
    result['probe_sink'] = {key: sink[key] for key in [
        'id', 'status', 'gate', 'files', 'sharedFiles', 'channels', 'removedOn']}
    # The repository-wide fixed-source census is data, not a filesystem or
    # security probe. It pins all field references and helper call sites.
    patterns = ['nativeApprovalConsumed', 'approvalMarkerSuppressed',
                'approvalConsumedCandidateKey', 'markApprovalConsumed.*Locked',
                'closeMarkerRecordOnSubmittedTurn', 'closeApprovalMarkerOnTranscriptUserMessage',
                'markApprovalUserTurnBoundaryLocked', 'approval-corrupt-(snapshot|dump)']
    census = subprocess.check_output(['git', 'grep', '-n', '-E', '|'.join(patterns), BASELINE,
                                     '--', 'internal/*.go', ':!*_test.go'], cwd=repo, text=True)
    result['source_census'] = census.splitlines()
    (destination / 'go.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    count = sum(len(row['steps']) for row in result['scenarios'])
    print(f'wrote {len(result["scenarios"])} scenarios, {count} suppression steps, {len(result["legacy"])} hand-seeded legacy predicates')


if __name__ == '__main__':
    main()
