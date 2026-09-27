import { test, expect } from 'bun:test';
import { isRoutineRunActive, routineRunFromURL, routineSessionMatches } from '../src/app/routines-model';
import type { RoutineRun } from '../src/app/routines-model';
test('old run cannot open a reused live session ID',()=>{
  const run={session_id:4,session_label:'routine-synthetic'} as RoutineRun;
  expect(routineSessionMatches(run,{id:4,label:'other'})).toBe(false);
  expect(routineSessionMatches(run,{id:4,label:'routine-synthetic'})).toBe(true);
  expect(routineSessionMatches({...run,session_label:''},{id:4,label:''})).toBe(false);
});
test('notification preserves exact run and rejects malformed IDs',()=>{
  expect(routineRunFromURL('https://example.test/?routine_run=run-123')).toBe('run-123');
  expect(routineRunFromURL('https://example.test/?routine_run=../latest')).toBeNull();
  expect(routineRunFromURL('https://example.test/')).toBeNull();
});
test('only active runs are polled; finished is not a success claim',()=>{
  for(const status of ['starting','running','waiting'])expect(isRoutineRunActive({status})).toBe(true);
  for(const status of ['finished','failed','interrupted','skipped'])expect(isRoutineRunActive({status})).toBe(false);
});
