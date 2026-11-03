import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { CSP_DIRECTIVES, RAC_PRESSABLE_STYLE, cspPolicy, styleHash } from '../vite.csp';

/**
 * The CSP hash in `vite.csp.ts` must match the stylesheet React Aria actually
 * injects. Pin it here so a dependency bump fails the fast unit suite instead
 * of only the e2e CSP gate.
 */
const HERE = dirname(fileURLToPath(import.meta.url));

function installedUsePressSource(): string {
  const store = join(HERE, '../node_modules/.pnpm');
  const dir = readdirSync(store).find((entry) => entry.startsWith('react-aria@'));
  if (!dir) throw new Error('react-aria is not installed in node_modules/.pnpm');
  const file = join(store, dir, 'node_modules/react-aria/dist/private/interactions/usePress.mjs');
  return readFileSync(file, 'utf8');
}

/** The `@layer` block React Aria assigns to `style.textContent`, trimmed as in the package. */
function injectedStyle(source: string): string {
  const match = /style\.textContent = `([\s\S]*?)`\.trim\(\)/.exec(source);
  if (!match) throw new Error('usePress: injected stylesheet not found');
  return match[1]!
    .replace('${$d27d541f9569d26d$var$PRESSABLE_ATTRIBUTE}', 'data-react-aria-pressable')
    .trim();
}

describe('CSP', () => {
  it('hashes the stylesheet React Aria injects for pressables', () => {
    expect(injectedStyle(installedUsePressSource())).toBe(RAC_PRESSABLE_STYLE);
  });

  it('allows that stylesheet by hash instead of unsafe-inline', () => {
    const policy = cspPolicy(false);
    expect(policy).toContain(`style-src 'self' ${styleHash(RAC_PRESSABLE_STYLE)}`);
    expect(policy).not.toContain("style-src 'self' 'unsafe-inline'");
  });

  it('keeps the production script policy strict', () => {
    expect(cspPolicy(false)).toContain("script-src 'self'");
    expect(cspPolicy(false)).not.toContain('unsafe-inline');
    expect(cspPolicy(false)).not.toContain('unsafe-eval');
  });

  it('relaxes only script-src, and only for vite dev', () => {
    const dev = cspPolicy(true);
    expect(dev).toContain("script-src 'self' 'unsafe-inline'");
    for (const directive of CSP_DIRECTIVES.filter((d) => !d.startsWith('script-src'))) {
      expect(dev).toContain(directive);
    }
  });

  it('matches the policy documented in ADR-0003 section 3.9', () => {
    // The nginx image copies the header from the ADR; if the two drift, the
    // production policy silently stops matching what e2e verified.
    const adr = readFileSync(join(HERE, '../../docs/adr/0003-stack-web-ui.md'), 'utf8');
    const documented = /`Content-Security-Policy: ([^`]+)`/.exec(adr)?.[1];
    expect(documented, 'ADR-0003 harus memuat header CSP di §3.9').toBeDefined();
    expect(documented).toBe(cspPolicy(false));
  });

  it('carries the ADR-0003 directives', () => {
    const policy = cspPolicy(false);
    for (const expected of [
      "default-src 'self'",
      "img-src 'self' data:",
      "font-src 'self'",
      "connect-src 'self'",
      "frame-ancestors 'none'",
      "base-uri 'none'",
      "form-action 'self'",
      "object-src 'none'",
    ]) {
      expect(policy).toContain(expected);
    }
  });
});
