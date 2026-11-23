// 同梱 7 本の宣言（Hub の GET /api/providers/declarations の答えと同じ形）。
// テストと fixtures が、Hub から宣言が届いた後の状態を作るための種。画面は起動時に、Hub の答えが
// 届くまで（と取れなかったとき）の控えとしてだけ読む（provider-store.ts の initProviderDeclarations）。
// 値の正本は rust/assets/provider/declarations.json。web/tests/provider-declarations.test.ts が
// この写しと突き合わせ、ずれたら落とす。

import { cacheProviderDeclarations, type ProviderDeclaration } from './provider-declarations.js';

export const BUILTIN_DECLARATIONS_SEED: ProviderDeclaration[] = [
  {
    "id": "claude",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "C",
      "order": 1,
      "handoff_order": 1,
      "label": "Claude",
      "long_label": "Claude Code"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md"
    },
    "permissions": {
      "modes": {
        "*": [
          "--permission-mode",
          "{mode}"
        ],
        "default": [],
        "bounded": []
      },
      "allowed_tools_args": [
        "--allowedTools",
        "{tools}"
      ],
      "choices": [
        "default",
        "plan",
        "acceptEdits",
        "auto",
        "bypassPermissions"
      ]
    },
    "links": {
      "usage_url": "https://claude.ai/settings/usage"
    },
    "input": {
      "attach_format": "at-path"
    }
  },
  {
    "id": "codex",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "X",
      "order": 4,
      "handoff_order": 2,
      "label": "Codex",
      "long_label": "Codex CLI"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md",
      "skill_prefix": "$"
    },
    "links": {
      "usage_url": "https://chatgpt.com/codex/cloud/settings/analytics#usage"
    },
    "input": {
      "slow_paste": true,
      "attach_format": "at-path-slash",
      "full_frame_cursor": true,
      "no_line_clear": true
    },
    "routines": {
      "completion_mode": "native_or_marker"
    }
  },
  {
    "id": "copilot",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "P",
      "order": 5,
      "handoff_order": 4,
      "label": "Copilot",
      "long_label": "GitHub Copilot"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/copilot.md"
    },
    "permissions": {
      "modes": {
        "bypassPermissions": [
          "--allow-all"
        ],
        "bounded": [
          "--no-ask-user"
        ],
        "auto": [
          "--autopilot"
        ]
      },
      "allowed_tools_args": [
        "--allow-tool={tool}"
      ],
      "choices": [
        "default",
        "auto",
        "bypassPermissions"
      ]
    },
    "links": {
      "usage_url": "https://github.com/settings/billing"
    }
  },
  {
    "id": "cursor-agent",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "r",
      "order": 7,
      "handoff_order": 5,
      "label": "Cursor Agent",
      "long_label": "Cursor Agent"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/cursor-agent.md"
    },
    "permissions": {
      "modes": {
        "bypassPermissions": [
          "--force"
        ],
        "auto": [
          "--auto-review"
        ]
      },
      "choices": [
        "default",
        "auto",
        "bypassPermissions"
      ]
    },
    "links": {
      "usage_url": "https://cursor.com/dashboard"
    }
  },
  {
    "id": "opencode",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "O",
      "order": 6,
      "handoff_order": 6,
      "label": "OpenCode",
      "long_label": "OpenCode"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/opencode.md"
    },
    "permissions": {
      "modes": {
        "bounded": [
          "--auto"
        ],
        "bypassPermissions": [
          "--auto"
        ]
      }
    },
    "links": {
      "usage_url": "https://opencode.ai/go"
    },
    "input": {
      "slow_paste": true
    }
  },
  {
    "id": "grok",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "G",
      "order": 8,
      "handoff_order": 3,
      "label": "Grok Build",
      "long_label": "Grok Build"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/grok.md"
    },
    "permissions": {
      "modes": {
        "*": [
          "--permission-mode",
          "{mode}"
        ],
        "default": [],
        "bounded": []
      },
      "choices": [
        "default",
        "plan",
        "acceptEdits",
        "auto",
        "bypassPermissions"
      ]
    },
    "links": {
      "usage_url": "https://grok.com/?_s=usage"
    },
    "input": {
      "interrupt_key": "ctrl-c",
      "full_frame_cursor": true
    }
  },
  {
    "id": "command-code",
    "enabled": true,
    "builtin": true,
    "connection": "pty",
    "presentation": {
      "letter": "M",
      "order": 9,
      "handoff_order": 7,
      "label": "Command Code",
      "long_label": "Command Code"
    },
    "slash_commands": {
      "source": "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/command-code.md"
    },
    "permissions": {
      "modes": {
        "plan": [
          "--permission-mode",
          "plan"
        ],
        "acceptEdits": [
          "--auto-accept"
        ],
        "auto": [
          "--auto-accept"
        ],
        "bypassPermissions": [
          "--yolo"
        ]
      },
      "choices": [
        "default",
        "plan",
        "acceptEdits",
        "bypassPermissions"
      ]
    },
    "links": {
      "usage_url": "https://commandcode.ai/usage"
    }
  }
];

export function seedBuiltinProviderDeclarations(): void {
  cacheProviderDeclarations(BUILTIN_DECLARATIONS_SEED);
}
