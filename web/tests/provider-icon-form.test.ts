import { describe, expect, test } from 'bun:test';
import { checkIconImageFile, MAX_ICON_IMAGE_BYTES, mergeIconPresentation, presentationWithoutIconFields } from '../src/app/provider-manager-view.ts';

// plan_provider-icon-single-source.md C3。設定画面のアイコン欄と、高度な定義（JSON）の presentation を
// どう合成するかを固定する純関数テスト。DOM には触れない。

describe('mergeIconPresentation', () => {
  test('both fields empty and no JSON leaves no presentation at all', () => {
    expect(mergeIconPresentation(undefined, { icon_text: '', color: '' })).toBeUndefined();
    expect(mergeIconPresentation({}, {})).toBeUndefined();
  });

  test('filled fields are sent, an empty one is left out', () => {
    expect(mergeIconPresentation(undefined, { icon_text: 'A', color: '#1E90FF' })).toEqual({ icon_text: 'A', color: '#1E90FF' });
    expect(mergeIconPresentation(undefined, { icon_text: 'A', color: '' })).toEqual({ icon_text: 'A' });
    expect(mergeIconPresentation(undefined, { icon_text: '', color: '#1E90FF' })).toEqual({ color: '#1E90FF' });
  });

  test('a filled field wins over the JSON, an empty one lets the JSON value through to the Hub', () => {
    expect(mergeIconPresentation({ icon_text: 'X', color: 'red;}' }, { icon_text: 'A', color: '#1E90FF' }))
      .toEqual({ icon_text: 'A', color: '#1E90FF' });
    expect(mergeIconPresentation({ color: 'red;}' }, { icon_text: 'A', color: '' }))
      .toEqual({ icon_text: 'A', color: 'red;}' });
  });

  test('a cleared form field does not bring back an earlier value: the JSON preview never carries them', () => {
    // 編集の開始時、JSON 欄には icon_text / color を出さない。欄を空にして保存すれば何も残らない。
    const preview = presentationWithoutIconFields({ icon_text: 'A', color: '#1E90FF' });
    expect(preview).toBeUndefined();
    expect(mergeIconPresentation(preview, { icon_text: '', color: '' })).toBeUndefined();
  });

  test('other presentation keys in the JSON survive', () => {
    expect(mergeIconPresentation({ future_key: 1 }, { icon_text: 'A', color: '' })).toEqual({ future_key: 1, icon_text: 'A' });
  });

  test('non-object JSON values are treated as absent', () => {
    expect(mergeIconPresentation('x', { icon_text: 'A' })).toEqual({ icon_text: 'A' });
    expect(mergeIconPresentation([1], {})).toBeUndefined();
    expect(mergeIconPresentation(null, {})).toBeUndefined();
  });
});

// C4: the form checks a picked file before sending it, so the reason is on screen at once.
describe('checkIconImageFile', () => {
  test('PNG, JPEG, GIF and WebP up to 512 KB pass', () => {
    for (const type of ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'IMAGE/PNG']) {
      expect(checkIconImageFile({ type, size: 1024 })).toEqual({ ok: true });
    }
    expect(checkIconImageFile({ type: 'image/png', size: MAX_ICON_IMAGE_BYTES })).toEqual({ ok: true });
    expect(MAX_ICON_IMAGE_BYTES).toBe(512 * 1024);
  });

  test('SVG and other types are refused', () => {
    for (const type of ['image/svg+xml', 'image/x-icon', 'image/bmp', 'text/html', 'application/octet-stream']) {
      expect(checkIconImageFile({ type, size: 1024 })).toEqual({ ok: false, reason: 'type' });
    }
  });

  test('one byte over the limit is too large; an empty file is not an image', () => {
    expect(checkIconImageFile({ type: 'image/png', size: MAX_ICON_IMAGE_BYTES + 1 })).toEqual({ ok: false, reason: 'size' });
    expect(checkIconImageFile({ type: 'image/png', size: 0 })).toEqual({ ok: false, reason: 'type' });
  });

  test('a browser that reports no type is let through: the Hub judges the bytes', () => {
    expect(checkIconImageFile({ type: '', size: 1024 })).toEqual({ ok: true });
    expect(checkIconImageFile({ size: 1024 })).toEqual({ ok: true });
    expect(checkIconImageFile({ type: '', size: MAX_ICON_IMAGE_BYTES + 1 })).toEqual({ ok: false, reason: 'size' });
  });
});

describe('presentationWithoutIconFields', () => {
  test('drops only the two icon keys', () => {
    expect(presentationWithoutIconFields({ icon_text: 'A', color: '#1E90FF', future_key: 1 })).toEqual({ future_key: 1 });
    expect(presentationWithoutIconFields({ icon_text: 'A' })).toBeUndefined();
    expect(presentationWithoutIconFields(undefined)).toBeUndefined();
    expect(presentationWithoutIconFields(['x'])).toBeUndefined();
  });

  test('does not modify its input', () => {
    const input = { icon_text: 'A', color: '#1E90FF' };
    presentationWithoutIconFields(input);
    expect(input).toEqual({ icon_text: 'A', color: '#1E90FF' });
  });
});
