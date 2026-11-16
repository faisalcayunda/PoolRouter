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

  console.log(`[smoke-setup] Menunggu nginx dan admin API siap di ${baseURL}`);

  for (let i = 1; i <= maxAttempts; i++) {
    try {
      // Periksa nginx dengan HEAD (cepat)
      const headRes = await fetch(`${baseURL}/`, { method: 'HEAD' });
      // Periksa admin API backend dengan GET yang akan 401 jika auth gagal, tapi bukan 502
      const apiRes = await fetch(`${baseURL}/admin/v1/auth/me`);

      if (headRes.status !== 502 && apiRes.status !== 502) {
        console.log(`[smoke-setup] nginx dan admin API siap (attempt ${i}/${maxAttempts})`);
        return;
      }
    } catch (err) {
      // Network error, belum siap
    }

    if (i < maxAttempts) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }

  throw new Error(`nginx/admin API tidak siap setelah ${maxAttempts} attempt (${maxAttempts * delayMs}ms)`);
}

export default globalSetup;
