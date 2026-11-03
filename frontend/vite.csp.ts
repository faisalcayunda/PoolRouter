import { createHash } from 'node:crypto';
import type { Plugin } from 'vite';

/**
 * Content-Security-Policy for `vite dev` and `vite preview` (ADR-0003 section 3.9).
 *
 * In production nginx serves the same policy as a response header. Development
 * and the e2e run need it too: without it the `securitypolicyviolation`
 * listener in `e2e/fixtures.ts` would pass vacuously, and a component that needs
 * `style-src 'unsafe-inline'` would only be discovered at deploy time.
 *
 * The policy is the ADR string verbatim, with two documented deviations:
 *
 * 1. `script-src` gains `'unsafe-inline'` under `vite dev` only: Vite injects the
 *    HMR client and the React Refresh preamble as inline scripts. `vite preview`
 *    serves the built bundle in production mode and keeps `script-src 'self'`,
 *    so the e2e gate tests the production policy.
 * 2. `style-src` carries the hash of the one stylesheet React Aria injects at
 *    runtime (below). A hash is not `'unsafe-inline'`: it allows exactly that
 *    one stylesheet and nothing else.
 */

/**
 * The stylesheet `@react-aria/interactions` `usePress` prepends to `<head>` so
 * that pressables can pan and pinch. It is a template literal in the package
 * (`usePress.mjs`), trimmed, so the text below is byte-identical to what lands
 * in the DOM. `tests/csp.test.ts` asserts that against the installed package:
 * if a React Aria upgrade changes it, that test fails before the CSP gate does.
 */
export const RAC_PRESSABLE_STYLE = `@layer {
  [data-react-aria-pressable] {
    touch-action: pan-x pan-y pinch-zoom;
  }
}`;

/** CSP source expression for an inline stylesheet: its sha256 of the exact bytes. */
export function styleHash(text: string): string {
  return `'sha256-${createHash('sha256').update(text, 'utf8').digest('base64')}'`;
}

export const CSP_DIRECTIVES = [
  "default-src 'self'",
  "script-src 'self'",
  `style-src 'self' ${styleHash(RAC_PRESSABLE_STYLE)}`,
  "img-src 'self' data:",
  "font-src 'self'",
  "connect-src 'self'",
  "frame-ancestors 'none'",
  "base-uri 'none'",
  "form-action 'self'",
  "object-src 'none'",
] as const;

export function cspPolicy(isDev: boolean): string {
  return CSP_DIRECTIVES.map((directive) =>
    isDev && directive.startsWith('script-src') ? "script-src 'self' 'unsafe-inline'" : directive,
  ).join('; ');
}

export function cspPlugin(): Plugin {
  return {
    name: 'poolrouter-csp',
    configureServer(server) {
      server.middlewares.use((_req, res, next) => {
        res.setHeader('Content-Security-Policy', cspPolicy(true));
        next();
      });
    },
    // `vite preview` always serves a production build, so the strict policy applies.
    configurePreviewServer(server) {
      server.middlewares.use((_req, res, next) => {
        res.setHeader('Content-Security-Policy', cspPolicy(false));
        next();
      });
    },
  };
}
