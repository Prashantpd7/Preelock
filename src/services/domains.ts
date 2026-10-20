// Domain input helpers: clean up what the user types so the strict
// backend validator rarely has to complain.
const LABEL = '(?!-)[a-z0-9-]{1,63}(?<!-)';
const BARE_DOMAIN = new RegExp('^' + LABEL + '(\\.' + LABEL + ')+$');
const SINGLE_LABEL = new RegExp('^' + LABEL + '$');
// Clean a single domain line. Returns the bare domain, or null when the
// line is not recognizable as a domain at all.
export function cleanDomain(raw: string): string | null {
  let s = raw.trim().toLowerCase();
  if (!s) return null;
  s = s.replace(/^[a-z][a-z0-9+.-]*:\/\//, '');
  s = s.split(/[/?#]/)[0];
  s = s.replace(/:\d+$/, '');
  s = s.replace(/\.$/, '');
  if (!s) return null;
  if (/^(\d+\.)+\d+$/.test(s)) return null;
  if (BARE_DOMAIN.test(s)) return s;
  if (SINGLE_LABEL.test(s)) return s + '.com';
  return null;
}
// Normalize a multi-line textarea. Bad lines are left untouched so the user
// can fix them; errors lists the offending lines.
export function normalizeDomainInput(value: string): {
  text: string;
  errors: string[];
} {
  const errors: string[] = [];
  const lines = value.split('\n').map((line) => {
    if (!line.trim()) return '';
    const cleaned = cleanDomain(line);
    if (cleaned === null) {
      errors.push(line.trim());
      return line;
    }
    return cleaned;
  });
  return { text: lines.join('\n'), errors };
}
// Live validation message for a textarea, without rewriting anything.
export function domainInputError(value: string): string {
  const errors = normalizeDomainInput(value).errors;
  if (!errors.length) return '';
  return 'Use a bare domain, without paths or wildcards: ' + errors[0];
}
