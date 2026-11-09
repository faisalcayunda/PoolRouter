import { expect, expectNoAxeViolations, stubSesi, test } from './fixtures';

/**
 * Shell smoke test (scope 1): the built SPA on `vite preview` must render the
 * M3 shell with zero CSP violations and zero axe violations.
 *
 * The `cspViolations` fixture is requested by every test here, so the
 * `securitypolicyviolation` assertion runs even when the body only checks markup.
 */
test.describe('app shell', () => {
  // AppRoot memanggil GET me dan getSesi dulu; tanpa stub halaman jatuh ke layar login.
  test.beforeEach(async ({ page }) => {
    await stubSesi(page);
  });

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

    // Exactly one level-1 heading per page, and it lives in `main` (spec section 13.1/13.2); the
    // app-bar title is plain text.
    const h1 = page.getByRole('heading', { level: 1 });
    await expect(h1).toHaveCount(1);
    await expect(page.locator('main#main-content h1')).toHaveCount(1);

    // Skip link is the first focusable element in DOM order (ADR-0003 7.5). Asserting the order
    // rather than "first Tab after load" keeps this stable: screens focus their own `h1` on mount
    // (section 8), which would consume the initial Tab.
    const skipLink = page.getByRole('link', { name: /lompat ke konten/i });
    await expect(skipLink).toHaveAttribute('href', '#main-content');
    const firstFocusable = await page.evaluate(() => {
      const candidates = document.querySelectorAll<HTMLElement>(
        'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      return candidates[0]?.textContent?.trim() ?? null;
    });
    expect(firstFocusable).toBe('Lompat ke konten');
    // Focusing it reveals the link and lands on `main`.
    await skipLink.focus();
    await expect(skipLink).toBeFocused();

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
