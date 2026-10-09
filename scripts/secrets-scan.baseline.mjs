// A one-line edit must not turn unchanged minified content into a new secret.
// Unsupported hunks and occurrences touching the changed region fail closed.
export function unchangedReplacementHit(diff, lineNumber, line, needle) {
  if (!needle || typeof line !== 'string') return false;
  const hunks = diff.split(/(?=^@@ )/m);
  for (const hunk of hunks) {
    const rows = hunk.split('\n');
    const header = /^@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(rows[0]);
    if (!header || Number(header[2]) !== lineNumber) continue;
    if (Number(header[1] ?? 1) !== 1 || Number(header[3] ?? 1) !== 1) return false;
    const body = rows.slice(1).filter(row => row && !row.startsWith('\\ No newline'));
    if (body.length !== 2 || !body[0].startsWith('-') || !body[1].startsWith('+')) return false;
    const before = body[0].slice(1);
    const after = body[1].slice(1);
    // The scanner currently reads the worktree. Never exempt a different index line.
    if (line !== after) return false;
    let prefix = 0;
    while (prefix < before.length && prefix < after.length && before[prefix] === after[prefix]) prefix++;
    let suffix = 0;
    while (suffix < before.length - prefix && suffix < after.length - prefix
      && before[before.length - suffix - 1] === after[after.length - suffix - 1]) suffix++;
    let found = false;
    for (let from = 0;;) {
      const start = after.indexOf(needle, from);
      if (start < 0) return found;
      found = true;
      if (start + needle.length > prefix && start < after.length - suffix) return false;
      from = start + 1;
    }
  }
  return false;
}
