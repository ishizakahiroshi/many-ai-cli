import assert from 'node:assert/strict';
import test from 'node:test';
import {
  LAUNCH_CHOICE_ASK,
  LAUNCH_CHOICE_LAST,
  LAUNCH_CHOICE_LOCAL,
  MAX_REMOTE_PROFILES,
  normalizeLaunchChoice,
  parseRemoteLast,
  parseRemoteProfiles,
  profileChoiceValue,
  removeRemoteProfile,
  resolveLaunchChoice,
  serializeRemoteLast,
  serializeRemoteProfiles,
  upsertRemoteProfile,
  type RemoteAttachProfile,
} from './remote-attach-profiles.js';

const dev: RemoteAttachProfile = { name: '開発', target: 'devuser@dev-host', identity: '' };
const prod: RemoteAttachProfile = { name: '本番', target: 'opsuser@ops-host', identity: '~/.ssh/ops' };

test('parseRemoteProfiles: 壊れた入力は空、規則に合わない要素と同名の 2 件目は捨てる', () => {
  assert.deepEqual(parseRemoteProfiles(undefined), []);
  assert.deepEqual(parseRemoteProfiles('not json'), []);
  assert.deepEqual(parseRemoteProfiles('{"a":1}'), []);
  const raw = JSON.stringify([
    dev,
    { name: '開発', target: 'otheruser@other-host', identity: '' },
    { name: 'bad', target: '-oProxyCommand=x', identity: '' },
    { name: 'badkey', target: 'h', identity: '-i' },
    { name: '', target: 'h', identity: '' },
    { target: 'h' },
    null,
    { name: 'noid', target: 'h' },
  ]);
  assert.deepEqual(parseRemoteProfiles(raw), [dev, { name: 'noid', target: 'h', identity: '' }]);
});

test('parseRemoteProfiles: 上限を超えた分は読まない', () => {
  const many = Array.from({ length: MAX_REMOTE_PROFILES + 5 }, (_, i) => ({ name: `p${i}`, target: `h${i}`, identity: '' }));
  assert.equal(parseRemoteProfiles(JSON.stringify(many)).length, MAX_REMOTE_PROFILES);
});

test('serialize と parse は往復で変わらない', () => {
  assert.deepEqual(parseRemoteProfiles(serializeRemoteProfiles([dev, prod])), [dev, prod]);
  const last = { target: 'ubuntu@192.0.2.1', identity: '' };
  assert.deepEqual(parseRemoteLast(serializeRemoteLast(last)), last);
});

test('upsertRemoteProfile: 同名は置き換え、新名は末尾、規則違反と上限超過は null', () => {
  assert.deepEqual(upsertRemoteProfile([], dev), [dev]);
  assert.deepEqual(upsertRemoteProfile([dev, prod], { ...dev, target: 'newuser@new-host' }), [
    { ...dev, target: 'newuser@new-host' },
    prod,
  ]);
  assert.deepEqual(upsertRemoteProfile([dev], prod), [dev, prod]);
  assert.equal(upsertRemoteProfile([], { name: 'x', target: '-bad', identity: '' }), null);
  assert.equal(upsertRemoteProfile([], { name: '', target: 'h', identity: '' }), null);
  const full = Array.from({ length: MAX_REMOTE_PROFILES }, (_, i) => ({ name: `p${i}`, target: `h${i}`, identity: '' }));
  assert.equal(upsertRemoteProfile(full, dev), null);
  // 上限いっぱいでも、既にある名前の上書きはできる。
  assert.notEqual(upsertRemoteProfile(full, { name: 'p0', target: 'z', identity: '' }), null);
});

test('removeRemoteProfile', () => {
  assert.deepEqual(removeRemoteProfile([dev, prod], '開発'), [prod]);
  assert.deepEqual(removeRemoteProfile([dev], 'none'), [dev]);
});

test('parseRemoteLast: 規則に合わない接続先は読まない', () => {
  assert.equal(parseRemoteLast(undefined), null);
  assert.equal(parseRemoteLast('{"target":"-x"}'), null);
  assert.equal(parseRemoteLast('{"target":"h","identity":"-i"}'), null);
  assert.deepEqual(parseRemoteLast('{"target":"h"}'), { target: 'h', identity: '' });
});

test('resolveLaunchChoice: 指した先が無いときは別の先にせず「聞く」へ戻す', () => {
  const last = { target: 'ubuntu@192.0.2.1', identity: '' };
  assert.deepEqual(resolveLaunchChoice(LAUNCH_CHOICE_ASK, [dev], last), { kind: 'ask' });
  assert.deepEqual(resolveLaunchChoice(LAUNCH_CHOICE_LOCAL, [dev], last), { kind: 'local' });
  assert.deepEqual(resolveLaunchChoice(LAUNCH_CHOICE_LAST, [dev], last), { kind: 'target', ...last });
  assert.deepEqual(resolveLaunchChoice(LAUNCH_CHOICE_LAST, [dev], null), { kind: 'ask' });
  assert.deepEqual(resolveLaunchChoice(profileChoiceValue('本番'), [dev, prod], null), {
    kind: 'target',
    target: prod.target,
    identity: prod.identity,
  });
  assert.deepEqual(resolveLaunchChoice(profileChoiceValue('消した'), [dev], last), { kind: 'ask' });
  assert.deepEqual(resolveLaunchChoice('unknown', [dev], last), { kind: 'ask' });
});

test('normalizeLaunchChoice: 読めない保存値は「聞く」', () => {
  assert.equal(normalizeLaunchChoice(undefined), LAUNCH_CHOICE_ASK);
  assert.equal(normalizeLaunchChoice('local'), 'local');
  assert.equal(normalizeLaunchChoice('last'), 'last');
  assert.equal(normalizeLaunchChoice('profile:開発'), 'profile:開発');
  assert.equal(normalizeLaunchChoice('profile:'), LAUNCH_CHOICE_ASK);
  assert.equal(normalizeLaunchChoice('x'), LAUNCH_CHOICE_ASK);
});
