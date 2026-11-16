/**
 * Global setup untuk smoke tests (ARC-430, AC1a): tunggu nginx dan admin API siap
 * sebelum Playwright mulai suite (hindari 502 errors dari race condition backend belum
 * siap pada port 8082 meski healthcheck lulus di port 9090).
 *
 * Dijalankan dari host dan memeriksa admin API lewat nginx. Downstream API (backend-
 * webserver:8081, AC1b) butuh DNS internal compose yang hanya ada di netns frontend, jadi
 * test-level ini di 09-downstream-api.spec.ts dengan retry logic untuk transien EAI_AGAIN.
 */
async function globalSetup() {
  const baseURL = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080';
  const maxAttempts = 30;
  const delayMs = 1000;
  const timeoutMs = 5000;

  console.log(`[smoke-setup] Menunggu nginx dan admin API siap di ${baseURL}`);

  for (let i = 1; i <= maxAttempts; i++) {
    const controller = new AbortController();
    const timeoutHandle = setTimeout(() => controller.abort(), timeoutMs);

    try {
      // Periksa nginx (HEAD /): 200 atau redirect OK, 502-599 adalah error
      const headRes = await fetch(`${baseURL}/`, {
        method: 'HEAD',
        signal: controller.signal,
      });
      // Periksa admin API (/admin/v1/auth/me): 401 OK (auth error jika tak ada token),
      // 502-599 adalah backend error
      const apiRes = await fetch(`${baseURL}/admin/v1/auth/me`, {
        signal: controller.signal,
      });

      const headOk = headRes.status < 500;
      const apiOk = apiRes.status < 500;

      if (headOk && apiOk) {
        clearTimeout(timeoutHandle);
        console.log(
          `[smoke-setup] nginx dan admin API siap (attempt ${i}/${maxAttempts})`
        );
        return;
      }
    } catch {
      // Network error atau timeout, belum siap
    } finally {
      clearTimeout(timeoutHandle);
    }

    if (i < maxAttempts) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }

  throw new Error(
    `nginx/admin API tidak siap setelah ${maxAttempts} attempt `
      + `(${maxAttempts * delayMs}ms total)`
  );
}

export default globalSetup;
