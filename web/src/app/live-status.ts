// Provider-specific live status extraction from rendered terminal lines.
// Keep this file DOM/xterm independent so fixtures can verify provider formats.

function compactLine(line: string): string {
  return String(line || '').replace(/\s+/g, ' ').trim();
}

function stripLeadingSpinner(line: string): string {
  return line.replace(/^[•·●◉◎○◐◑◒◓◴◵◶◷⠁-⣿]\s*/, '').trim();
}

function findLastModelUsage(lines: string[], startIndex: number): string {
  for (let i = startIndex; i >= 0; i--) {
    const raw = compactLine(lines[i]);
    const m = raw.match(/\b(?:Auto|[A-Za-z0-9._-]+)\s*(?:→|->|·)\s*[A-Za-z0-9._+:-]+(?:\s*%)?/);
    if (m) return compactLine(m[0]);
    const percent = raw.match(/\b(?:Auto|[A-Za-z0-9._-]+)\s*·\s*\d+(?:\.\d+)?%/);
    if (percent) return compactLine(percent[0]);
  }
  return '';
}

export function extractCodexLiveStatusFromLines(lines: string[]): string {
  let action = '';
  for (let i = (lines || []).length - 1; i >= 0; i--) {
    const stripped = stripLeadingSpinner(compactLine(lines[i]));
    if (!stripped) continue;
    if (/^Working\b/i.test(stripped)) return stripped;
    if (!action && /^(Running|Ran|Reading|Read|Editing|Edited|Searching|Thinking)\b/i.test(stripped)) {
      action = stripped;
    }
  }
  return action;
}

export function extractCopilotLiveStatusFromLines(lines: string[]): string {
  for (let i = (lines || []).length - 1; i >= 0; i--) {
    const raw = compactLine(lines[i]);
    if (!raw) continue;
    const m = raw.match(/[●◉◎○]?\s*Working\s+esc\s+cancel\b/i);
    if (!m) continue;
    let status = stripLeadingSpinner(raw.slice(m.index || 0));
    status = status.split('❯')[0].trim();
    const model = findLastModelUsage(lines, i);
    if (model) status = `${status.replace(/\s+(?:Auto|[A-Za-z0-9._-]+)\s*(?:→|->)\s*[A-Za-z0-9._+:-]+.*$/i, '').trim()} · ${model}`;
    return status;
  }
  return '';
}

export function extractCursorAgentLiveStatusFromLines(lines: string[]): string {
  let usage = '';
  let followUp = '';
  let action = '';
  for (let i = (lines || []).length - 1; i >= 0; i--) {
    const raw = compactLine(lines[i]);
    if (!raw) continue;
    if (!usage) {
      const m = raw.match(/\b(?:Auto|[A-Za-z0-9._-]+)\s*·\s*\d+(?:\.\d+)?%/);
      if (m) usage = compactLine(m[0]);
    }
    if (!followUp && /(?:^|\s)→\s*Add a follow-up\b/i.test(raw)) {
      followUp = raw.replace(/^.*?→\s*/i, '').replace(/\s*ctrl\+c to stop\b/i, '').trim();
    }
    if (!action && /^(WebSearch|Read|Edit|Search|Run|Apply|Create|Update|Delete)\b/i.test(raw)) {
      action = raw;
    }
    if (followUp && usage) return `${followUp} · ${usage}`;
  }
  if (followUp) return usage ? `${followUp} · ${usage}` : followUp;
  return usage || action;
}
