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
}

export {};

