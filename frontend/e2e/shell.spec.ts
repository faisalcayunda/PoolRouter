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
    await page.evaluate(() => document.fonts.ready);

    const origin = new URL(page.url()).origin;
    expect(fontRequests.length, 'font harus dimuat').toBeGreaterThan(0);
    for (const url of fontRequests) expect(new URL(url).origin, url).toBe(origin);
    const loaded = await page.evaluate(() =>
      [...document.fonts].filter((f) => f.status === 'loaded').map((f) => f.family),
    );
    expect(loaded.join(',')).toMatch(/Hanken Grotesk Variable/);
    expect(loaded.join(',')).toMatch(/Inter Variable/);
    expect(loaded.join(',')).not.toMatch(/Open Sans/);

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

  test('ARC-370: Pengaturan hanya di menu profil, tanpa grup akun di sidebar', async ({
    page,
    cspViolations,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');

    const drawer = page.getByRole('complementary', { name: 'Navigasi utama' });
    await expect(drawer.getByText('Akun')).toHaveCount(0);
    await expect(drawer.getByRole('link', { name: 'Pengaturan' })).toHaveCount(0);

    await drawer.getByRole('button', { name: /Menu akun/ }).click();
    const drawerMenu = page.getByRole('menu');
    const pengaturan = drawerMenu.getByRole('menuitem', { name: 'Pengaturan' });
    await expect(pengaturan).toHaveAttribute('href', '/settings');
    await expect(drawerMenu.getByRole('menuitem', { name: 'Keluar' })).toBeVisible();
    await page.keyboard.press('Escape');

    await page
      .getByRole('banner')
      .getByRole('button', { name: /Menu akun/ })
      .click();
    const barMenu = page.getByRole('menu');
    await expect(barMenu.getByRole('menuitem', { name: 'Pengaturan' })).toHaveAttribute(
      'href',
      '/settings',
    );
    await page.keyboard.press('Escape');

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

  test('ARC-370 AC3: target sentuh menu profil (avatar bar atas dan ciutkan) >= 44 px', async ({
    page,
    cspViolations,
  }) => {
    // Lingkaran tetap 36/40 px (ukuran visual tidak berubah); hit area diperluas lewat
    // pseudo-element `::before` tak terlihat (`-inset-1`), dicek lewat computed style karena
    // `::before` tidak punya `getBoundingClientRect()` sendiri.
    const hitArea = (selector: string) =>
      page.evaluate((sel) => {
        const el = document.querySelector(sel) as HTMLElement;
        const box = el.getBoundingClientRect();
        const before = getComputedStyle(el, '::before');
        const inset = parseFloat(before.top || '0');
        return { width: box.width - inset * 2, height: box.height - inset * 2 };
      }, selector);

    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');
    const avatar = await hitArea('header button[aria-label*="Menu akun"]');
    expect(avatar.width, 'avatar bar atas lebar').toBeGreaterThanOrEqual(44);
    expect(avatar.height, 'avatar bar atas tinggi').toBeGreaterThanOrEqual(44);

    await page.getByRole('button', { name: 'Ciutkan sidebar' }).click();
    const compact = await hitArea('aside button[aria-label*="Menu akun"]');
    expect(compact.width, 'avatar ciutkan lebar').toBeGreaterThanOrEqual(44);
    expect(compact.height, 'avatar ciutkan tinggi').toBeGreaterThanOrEqual(44);

    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
});
