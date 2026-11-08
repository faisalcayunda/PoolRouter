import { test as base, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

/**
 * CSP listener (ADR-0003 section 3.5, scope 1).
 *
 * `securitypolicyviolation` fires on `document`; it does not bubble, so the
 * listener is registered in an init script before any app script runs. A
 * violation is collected with its directive and blocked URI, and every test
 * fails at the end if the page produced one — a silent workaround (inline
 * style, data: image, @import) must not slip through as a passing smoke test.
 */
export interface CspViolation {
  directive: string;
  blockedURI: string;
  source: string;
}

declare global {
  interface Window {
    __cspViolations: CspViolation[];
  }
}

export const test = base.extend<{ cspViolations: CspViolation[] }>({
  page: async ({ page }, use) => {
    // Stub auth endpoints so e2e tests load the shell without a real backend.
    await page.route('**/admin/v1/auth/me', (route) =>
      route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: 'test', peran: ['auditor'] }),
      }),
    );
    await page.route('**/admin/v1/auth/sesi', (route) =>
      route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ batas_idle_detik: 3600, batas_absolut_detik: 28800 }),
      }),
    );
    await use(page);
  },
  cspViolations: async ({ page }, use) => {
    await page.addInitScript(() => {
      window.__cspViolations = [];
      document.addEventListener('securitypolicyviolation', (event) => {
        window.__cspViolations.push({
          directive: event.effectiveDirective || event.violatedDirective,
          blockedURI: event.blockedURI,
          source: event.sourceFile ? `${event.sourceFile}:${event.lineNumber}` : '',
        });
      });
    });
    const violations: CspViolation[] = [];
    await use(violations);
    const fromPage = await page.evaluate(() => window.__cspViolations ?? []);
    violations.push(...fromPage);
    expect(violations, 'pelanggaran CSP tidak boleh terjadi').toEqual([]);
  },
});

export { expect };

/** Fails the test with the axe violations, listing selector and rule id. */
export async function expectNoAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
    .analyze();
  expect(
    results.violations.map((v) => ({
      id: v.id,
      impact: v.impact,
      nodes: v.nodes.map((n) => n.target),
    })),
  ).toEqual([]);
}
