import { expect, expectNoAxeViolations, test } from './fixtures';

/**
 * Shell smoke test (scope 1): the built SPA on `vite preview` must render the
 * M3 shell with zero CSP violations and zero axe violations.
 *
 * The `cspViolations` fixture is requested by every test here, so the
 * `securitypolicyviolation` assertion runs even when the body only checks markup.
 */
test.describe('app shell', () => {
  test('renders the shell under the production CSP and passes axe', async ({
    page,
    cspViolations,
  }) => {
    await page.goto('/');

    // The CSP header must actually be served, otherwise the listener is vacuous.
    const csp = (await page.request.get('/')).headers()['content-security-policy'];
    expect(csp, 'vite preview harus mengirim header CSP').toContain("script-src 'self'");
    expect(csp).not.toContain('unsafe-inline');

    await expect(page.locator('main#main-content')).toBeVisible();
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    // Skip link is the first tabbable element (ADR-0003 7.5).
    await page.keyboard.press('Tab');
    await expect(page.getByRole('link', { name: /lompat ke konten/i })).toBeFocused();

    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test('modal drawer opens from the compact menu button, closes on Escape, without CSP violations', async ({
    page,
    cspViolations,
  }) => {
    await page.setViewportSize({ width: 600, height: 800 });
    await page.goto('/');

    const drawer = page.getByRole('dialog', { name: 'Navigasi utama' });
    await expect(drawer).toBeHidden();

    await page.getByRole('button', { name: 'Buka menu navigasi' }).click();
    await expect(drawer).toBeVisible();
    await expect(drawer.getByRole('navigation', { name: 'Navigasi utama' })).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();

    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test('expanded window size class shows the standard drawer', async ({ page, cspViolations }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto('/');

    await expect(page.getByRole('navigation', { name: 'Navigasi utama' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Buka menu navigasi' })).toBeHidden();

    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
});
