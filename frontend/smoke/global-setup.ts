/**
 * Global setup untuk smoke tests (ARC-430): tunggu nginx dan backend-webserver siap
 * sebelum Playwright mulai suite (hindari 502 errors dan EAI_AGAIN akibat race condition).
 *
 * Dijalankan dari host, bukan netns frontend, jadi hanya bisa check nginx. Downstream API
 * (backend-webserver:8081) butuh diakses dari dalam container (DNS internal compose), jadi
 * Playwright yang handle – keduanya diperiksa di bawah, tapi hanya nginx dari setup ini.
 */
async function globalSetup() {
  const baseURL = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080';
  const maxAttempts = 30;
  const delayMs = 1000;

  console.log(`[smoke-setup] Menunggu nginx siap di ${baseURL}`);

  for (let i = 1; i <= maxAttempts; i++) {
    try {
      const res = await fetch(`${baseURL}/`, { method: 'HEAD' });
      if (res.status !== 502) {
        console.log(`[smoke-setup] nginx siap (attempt ${i}/${maxAttempts})`);
        return;
      }
    } catch (err) {
      // Network error, belum siap
    }

    if (i < maxAttempts) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }

  throw new Error(`nginx tidak siap setelah ${maxAttempts} attempt (${maxAttempts * delayMs}ms)`);
}

export default globalSetup;
