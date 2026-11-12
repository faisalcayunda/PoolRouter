import { createHmac } from 'node:crypto';

/**
 * TOTP (RFC 6238) dari `rahasia_base32` yang dikembalikan `mfaDaftar` (spec smoke-test bagian 3:
 * "TOTP dihitung di test dengan node:crypto, HMAC-SHA1, 30 s, 6 digit; tanpa dependensi baru").
 */
export function totp(rahasiaBase32: string, waktuMs = Date.now()): string {
  const kunci = base32Decode(rahasiaBase32);
  const langkah = Math.floor(waktuMs / 1000 / 30);
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64BE(BigInt(langkah));
  const hmac = createHmac('sha1', kunci).update(buf).digest();
  const offset = (hmac[hmac.length - 1] ?? 0) & 0x0f;
  const kode =
    (((hmac[offset] ?? 0) & 0x7f) << 24) |
    (((hmac[offset + 1] ?? 0) & 0xff) << 16) |
    (((hmac[offset + 2] ?? 0) & 0xff) << 8) |
    ((hmac[offset + 3] ?? 0) & 0xff);
  return String(kode % 1_000_000).padStart(6, '0');
}

const ALFABET_BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

function base32Decode(input: string): Buffer {
  const bersih = input.toUpperCase().replace(/=+$/u, '');
  let bit = 0;
  let nilai = 0;
  const bytes: number[] = [];
  for (const c of bersih) {
    const idx = ALFABET_BASE32.indexOf(c);
    if (idx === -1) continue;
    nilai = (nilai << 5) | idx;
    bit += 5;
    if (bit >= 8) {
      bit -= 8;
      bytes.push((nilai >> bit) & 0xff);
    }
  }
  return Buffer.from(bytes);
}
