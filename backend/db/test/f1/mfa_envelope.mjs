// Envelope encryption untuk secret TOTP uji (SHAA-335, ADR-0006 D7 §9 butir 1).
// Dipanggil oleh load_f1.py dengan stdin JSON; secret tidak pernah masuk argumen proses.
//   node mfa_envelope.mjs            -> stdin {seed, kek_hex, kek_id, pengguna_ids}; stdout JSON per akun
//   node mfa_envelope.mjs --self-test -> round-trip, tamper, dan KEK salah harus ditolak
// CATATAN: bentuk envelope ini adalah bentuk uji buatan loader F1, bukan format produksi. Bila implementasi
// aplikasi sudah ada, bentuk ini harus diselaraskan (urutan nonce/tag, AAD, dan nama kek_id) agar fixture tidak
// menyimpang dari format produksi (review Ridley, SHAA-335 butir 2).
// Format: DEK acak 32 byte; secret_ciphertext = AES-256-GCM(DEK, nonce 12 byte, seed) + tag 16 byte;
// secret_nonce = nonce itu; secret_dek_terbungkus = nonce 12 byte || AES-256-GCM(KEK, nonce, DEK) + tag.
import crypto from 'node:crypto';

function seal(key, plaintext) {
  const nonce = crypto.randomBytes(12);
  const c = crypto.createCipheriv('aes-256-gcm', key, nonce);
  const ct = Buffer.concat([c.update(plaintext), c.final()]);
  return { nonce, sealed: Buffer.concat([ct, c.getAuthTag()]) };
}

function open(key, nonce, sealed) {
  const tag = sealed.subarray(sealed.length - 16);
  const ct = sealed.subarray(0, sealed.length - 16);
  const d = crypto.createDecipheriv('aes-256-gcm', key, nonce);
  d.setAuthTag(tag);
  return Buffer.concat([d.update(ct), d.final()]);
}

export function encryptSecret(kek, seed) {
  const dek = crypto.randomBytes(32);
  const s = seal(dek, Buffer.from(seed, 'utf8'));
  const w = seal(kek, dek);
  return {
    secret_ciphertext: s.sealed,
    secret_nonce: s.nonce,
    secret_dek_terbungkus: Buffer.concat([w.nonce, w.sealed]),
  };
}

export function decryptSecret(kek, rec) {
  const dek = open(kek, rec.secret_dek_terbungkus.subarray(0, 12), rec.secret_dek_terbungkus.subarray(12));
  return open(dek, rec.secret_nonce, rec.secret_ciphertext).toString('utf8');
}

function selfTest() {
  const kek = crypto.randomBytes(32);
  const seed = 'JBSWY3DPEHPK3PXP';
  const rec = encryptSecret(kek, seed);
  if (decryptSecret(kek, rec) !== seed) throw new Error('round-trip gagal');
  const bad = { ...rec, secret_ciphertext: Buffer.from(rec.secret_ciphertext) };
  bad.secret_ciphertext[0] ^= 0x01;
  try { decryptSecret(kek, bad); throw new Error('ciphertext yang diubah diterima'); } catch (e) { if (e.message === 'ciphertext yang diubah diterima') throw e; }
  try { decryptSecret(crypto.randomBytes(32), rec); throw new Error('KEK salah diterima'); } catch (e) { if (e.message === 'KEK salah diterima') throw e; }
  console.log('mfa_envelope self-test: round-trip ok, tamper ditolak, KEK salah ditolak');
}

if (process.argv.includes('--self-test')) {
  selfTest();
} else {
  let raw = '';
  process.stdin.setEncoding('utf8');
  for await (const chunk of process.stdin) raw += chunk;
  const input = JSON.parse(raw);
  const kek = Buffer.from(input.kek_hex, 'hex');
  if (kek.length !== 32) throw new Error('KEK harus 32 byte');
  const out = input.pengguna_ids.map((id) => {
    const r = encryptSecret(kek, input.seed);
    return {
      pengguna_id: id,
      secret_ciphertext: r.secret_ciphertext.toString('hex'),
      secret_nonce: r.secret_nonce.toString('hex'),
      secret_dek_terbungkus: r.secret_dek_terbungkus.toString('hex'),
      kek_id: input.kek_id,
    };
  });
  process.stdout.write(JSON.stringify(out));
}
