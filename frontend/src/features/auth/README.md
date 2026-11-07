# Layar MFA (S17–S17c)

Empat layar presentasional untuk alur login dua langkah (`f1-mfa-totp`, AC-29–31). Tidak ada
pemanggilan jaringan di dalamnya: pemilik state mengirim data lewat props, sehingga layar yang sama
dipakai form login mana pun.

## Seam ke alur login

Routing layar ini ke alur login (ARC-8) diserahkan ke tiket lanjutan; kontraknya sudah tetap.

| Layar | Respons API yang memicunya | Props masuk | Props keluar |
| --- | --- | --- | --- |
| `MfaCodeScreen` (S17) | `POST /auth/login` → 200 `{langkah: "mfa"}` | `username`, `loading`, `error` | `onSubmit(kode)` → `POST /auth/mfa`; `onBack` |
| `MfaRegistrationCodeScreen` (S17a) | 200 `{langkah: "daftar_mfa"}` | `loading`, `error` | `onSubmit(kodeDaftar)` → `POST /auth/mfa/daftar`; `onBack` |
| `MfaQrScreen` (S17b) | 200 `POST /auth/mfa/daftar` (`otpauth_uri`, `rahasia_base32`) | `otpauthUri`, `setupKey` (dari `rahasia_base32`), `account`, `issuer`, `loading`, `error` | `onConfirm(kode)` → `POST /auth/mfa/daftar/konfirmasi` |
| `MfaRecoveryScreen` (S17c) | 200 `POST /auth/mfa/daftar/konfirmasi` (`kode_pemulihan[]`) | `codes`, `isOpen` | `onAcknowledge()` → buang `codes`, buka shell |

Operator dan client owner tidak pernah melihat layar ini (`langkah` hanya muncul untuk peran
wajib-MFA). S17d (sisa kode pemulihan) adalah banner di dalam shell dan memakai `GET /auth/me`
`mfa.sisa_kode_pemulihan`; itu bagian shell, bukan folder ini.

## Data sensitif (AC-31)

`otpauthUri`, `setupKey`, dan `codes` hanya hidup di state komponen pemanggil: bukan cache Query,
bukan storage/URL, dan tidak pernah masuk `aria-label` atau Snackbar. `MfaRecoveryScreen` tidak bisa
ditutup lewat Esc/scrim; `onAcknowledge` adalah satu-satunya jalan keluar dan pemanggil wajib
membuang `codes` saat itu juga.
