import type { Terminal as XtermTerminal, ITerminalOptions } from '@xterm/xterm';
import type { FitAddon as XtermFitAddon } from '@xterm/addon-fit';
import type { Unicode11Addon as XtermUnicode11Addon } from '@xterm/addon-unicode11';
import type { WebLinksAddon as XtermWebLinksAddon } from '@xterm/addon-web-links';
import type { WebglAddon as XtermWebglAddon } from '@xterm/addon-webgl';

declare global {
  const Terminal: {
    new(options?: ITerminalOptions): XtermTerminal;
  };

  const FitAddon: {
    FitAddon: new () => XtermFitAddon;
  };

  const Unicode11Addon: {
    Unicode11Addon: new () => XtermUnicode11Addon;
  };

  const WebLinksAddon: {
    WebLinksAddon: new (...args: any[]) => XtermWebLinksAddon;
  };

  const WebglAddon: {
    WebglAddon: new () => XtermWebglAddon;
  };

  const marked: any;
  const DOMPurify: any;
  const hljs: any;

  // qrcode-generator (vendored, MIT). classic script がグローバル `qrcode` を公開する。
  interface QRCodeModel {
    addData(data: string, mode?: string): void;
    make(): void;
    getModuleCount(): number;
    isDark(row: number, col: number): boolean;
    createImgTag(cellSize?: number, margin?: number): string;
    createSvgTag(cellSize?: number, margin?: number): string;
    createDataURL(cellSize?: number, margin?: number): string;
    createTableTag(cellSize?: number, margin?: number): string;
    createASCII(cellSize?: number, margin?: number): string;
  }
  const qrcode: {
    (typeNumber: number, errorCorrectionLevel: 'L' | 'M' | 'Q' | 'H'): QRCodeModel;
    stringToBytes(s: string): number[];
  };
}

export {};

