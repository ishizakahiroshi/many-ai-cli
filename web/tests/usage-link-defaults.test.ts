import { describe, expect, test } from 'bun:test';
import { httpsUrlOrEmpty } from '../src/app/url-detect.ts';

// 監査 N36: Hub が GitHub から取ってきた既定のリンクは、https の URL だけを既定値にする。
describe('取得した既定のリンク', () => {
  test('https の URL はそのまま通す', () => {
    expect(httpsUrlOrEmpty('https://example.com/usage')).toBe('https://example.com/usage');
    expect(httpsUrlOrEmpty('  HTTPS://example.com/usage#x ')).toBe('https://example.com/usage#x');
  });

  test('https 以外の scheme・URL でない値・文字列でない値は空にする', () => {
    for (const value of [
      'javascript:alert(1)', 'ms-settings:privacy', 'http://example.com/usage', 'file:///synthetic',
      'data:text/html,synthetic', '//example.com/usage', 'https://', 'example.com', '', null, undefined, 1, {},
    ]) {
      expect(httpsUrlOrEmpty(value)).toBe('');
    }
  });
});
