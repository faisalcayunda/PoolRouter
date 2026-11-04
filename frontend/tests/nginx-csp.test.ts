import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cspPolicy } from '../vite.csp';

/**
 * ADR-0003 section 3.9: the nginx image serves the production CSP, copied from `vite.csp.ts`.
 * This keeps the copy in `deploy/security-headers.conf` from drifting.
 */
const HERE = dirname(fileURLToPath(import.meta.url));

describe('nginx security headers', () => {
  const conf = readFileSync(join(HERE, '../deploy/security-headers.conf'), 'utf8');

  it('serves the same CSP as vite preview (production policy)', () => {
    const match = /add_header Content-Security-Policy "([^"]+)" always;/.exec(conf);
    expect(match?.[1]).toBe(cspPolicy(false));
  });

  it.each([
    'X-Content-Type-Options "nosniff"',
    'Referrer-Policy "no-referrer"',
    'Permissions-Policy "camera=(), microphone=(), geolocation=()"',
  ])('sets %s', (header) => {
    expect(conf).toContain(`add_header ${header} always;`);
  });
});
