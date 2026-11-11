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

  test('ui-v3: self-hosted Hanken Grotesk and Inter, 272 px drawer, 44 px items, no search (ARC-366)', async ({
    page,
    cspViolations,
  }, testInfo) => {
    const fontRequests: string[] = [];
    page.on('request', (r) => {
      if (r.resourceType() === 'font') fontRequests.push(r.url());
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');
    await expect(page.locator('main#main-content')).toBeVisible();
    // Font dimuat malas per glyph; paksa muat kedua keluarga, lalu cek apa yang benar-benar terpakai.
    const loadedFaces = await page.evaluate(async () => {
      const faces = [
        ...(await document.fonts.load('600 14px "Hanken Grotesk Variable"', 'PoolRouter')),
        ...(await document.fonts.load('400 14px "Inter Variable"', 'PoolRouter')),
      ];
      return faces.map((f) => `${f.family}|${f.status}`);
    });
    expect(loadedFaces).toContain('Hanken Grotesk Variable|loaded');
    expect(loadedFaces).toContain('Inter Variable|loaded');

    const origin = new URL(page.url()).origin;
    expect(fontRequests.length, 'font harus dimuat').toBeGreaterThan(0);
    for (const url of fontRequests) expect(new URL(url).origin, url).toBe(origin);
    expect(fontRequests.join(',')).not.toMatch(/open-?sans/i);
    expect(
      await page
        .locator('.typescale-title-large')
        .first()
        .evaluate((el) => getComputedStyle(el).fontFamily),
    ).toMatch(/^"Hanken Grotesk Variable"/);

    const drawer = page.getByRole('complementary', { name: 'Navigasi utama' });
    expect((await drawer.boundingBox())?.width).toBe(272);
    const item = drawer.getByRole('link').first();
    expect((await item.boundingBox())?.height).toBe(44);
    await expect(page.getByRole('banner').locator('input')).toHaveCount(0);

    // Fokus selalu secondary (spec §3): rgb(13, 94, 186).
    await item.focus();
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Tab');
    await expect(item).toBeFocused();
    expect(await item.evaluate((el) => getComputedStyle(el).outlineColor)).toBe('rgb(13, 94, 186)');
    await item.blur();

    await page.screenshot({ path: testInfo.outputPath('shell@1440.png'), fullPage: true });
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test('ui-v3: compact shell at 390 px', async ({ page, cspViolations }, testInfo) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/');
    await expect(page.locator('main#main-content')).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: testInfo.outputPath('shell@390.png'), fullPage: true });
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
});
