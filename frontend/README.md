# Frontend (PoolRouter)

Kerangka SPA M3 PoolRouter: React + React Aria Components (RAC) di atas Vite, TypeScript, i18next,
Tailwind, dan Vitest/Playwright. Kontrak acuan: ADR-0003 dan ADR-0008. Token warna berasal dari seed
UI v2 ARC-11; typeface Inter dan JetBrains Mono di-host lokal.

## Gate

Semua target dijalankan lewat container image Node yang di-pin, jadi tidak ada toolchain yang
dipasang di host:

```sh
make -C frontend setup test lint build e2e CONTAINER_LABEL=arctiva.issue=ARC-197
```

`CONTAINER_LABEL` wajib untuk `FRONTEND_RUNNER=podman` (playbook bagian 6); tanpa itu `check-label`
keluar dengan exit 2. Untuk mesin atau job CI yang Node-nya sudah sesuai `.nvmrc`, pakai
`FRONTEND_RUNNER=local`.

## Kunci build berat (K-41)

VM Podman hanya boleh menjalankan satu build berat dalam satu waktu. Kunci bersama ada di
`scripts/build-lock.sh` dan `scripts/build-lock.mk` (SHAA-194); `frontend/Makefile` memakai include
yang sama dengan `backend/Makefile`, tanpa implementasi kedua.

| Target                      | Dikunci | Alasan                                                        |
| --------------------------- | ------- | ------------------------------------------------------------- |
| `setup`                     | ya      | `pnpm install` menulis `node_modules` dan `.cache/pnpm-store` |
| `build`                     | ya      | `vite build` me-resolve seluruh graf modul                    |
| `test`                      | ya      | Vitest men-transpile seluruh suite                            |
| `e2e`                       | ya      | Playwright + Chromium (lewat `HEAVY_PW`, image Playwright)    |
| `lint`, `fmt`, `i18n-check` | tidak   | hanya membaca `node_modules`, tidak menulis                   |
| `gen-api`, `gen-api-check`  | tidak   | hanya satu berkas hasil generate                              |
| `clean`                     | tidak   | menghapus artefak                                             |

Kunci dipegang berarti target keluar dengan **exit 75** (`EX_TEMPFAIL`). Pindah ke pekerjaan
non-compile lalu coba lagi; jangan menunggu dalam loop dan jangan memakai nama container lain.

```sh
make -C frontend build-lock-status
make -C frontend build-lock-clean     # hanya kunci basi; kunci yang masih jalan tidak disentuh
```

## Katalog komponen M3

Sumber kebenaran nama komponen adalah ADR-0003 §7.1: **23 komponen katalog + 6 ekstensi lokal**.
`src/components/m3/catalog.test.ts` membaca tabel §7.1 dan gagal bila ada nama katalog tanpa
komponen, atau ada berkas komponen yang tidak berakar pada katalog/ekstensi.

### 23 komponen katalog

| ADR §7.1                 | Berkas                                       | Ekspor yang dipakai handoff          |
| ------------------------ | -------------------------------------------- | ------------------------------------ |
| Common buttons           | `Button.tsx`                                 | `Button`                             |
| Icon buttons             | `IconButton.tsx`                             | `IconButton`                         |
| Text fields              | `TextField.tsx`                              | `TextField`, `TextArea`              |
| Menu dengan text field   | `Select.tsx`                                 | `Select`                             |
| Autocomplete             | `ComboBox.tsx`                               | `ComboBox`                           |
| Checkbox                 | `Checkbox.tsx`                               | `Checkbox`, `CheckboxGroup`          |
| Radio button             | `Radio.tsx`                                  | `Radio`, `RadioGroup`                |
| Switch                   | `Switch.tsx`                                 | `Switch`                             |
| Chips                    | `Chip.tsx`                                   | `Chip`                               |
| Dialogs                  | `Dialog.tsx`                                 | `Dialog`                             |
| Side sheet               | `SideSheet.tsx`                              | `SideSheet`                          |
| Menus                    | `Menu.tsx`                                   | `Menu`, `MenuItem`                   |
| Tabs                     | `Tabs.tsx`                                   | `Tabs`, `TabList`, `Tab`, `TabPanel` |
| Navigation drawer        | `NavigationDrawer.tsx`                       | `NavigationDrawer`                   |
| Top app bar              | `TopAppBar.tsx`                              | `TopAppBar`                          |
| Lists                    | `List.tsx`                                   | `List`, `ListItem`                   |
| Cards                    | `Card.tsx`                                   | `Card`                               |
| Snackbar                 | `Snackbar.tsx`                               | `Snackbar`                           |
| Tooltips                 | `Tooltip.tsx`                                | `Tooltip`, `TooltipTrigger`          |
| Progress indicators      | `LinearProgress.tsx`, `CircularProgress.tsx` | `LinearProgress`, `CircularProgress` |
| Divider                  | `Divider.tsx`                                | `Divider`                            |
| Date pickers             | `DatePicker.tsx`                             | `DatePicker`, `DateRangePicker`      |
| Time picker (mode input) | `TimeField.tsx`                              | `TimeField`                          |

### 6 ekstensi lokal

| Ekstensi      | Berkas            |
| ------------- | ----------------- |
| `DataTable`   | `DataTable.tsx`   |
| `Pagination`  | `Pagination.tsx`  |
| `StatusLabel` | `StatusLabel.tsx` |
| `InlineAlert` | `InlineAlert.tsx` |
| `Skeleton`    | `Skeleton.tsx`    |
| `Breadcrumbs` | `Breadcrumbs.tsx` |

Jumlah berkas komponen (30) lebih besar dari 29 nama karena `LinearProgress` dan `CircularProgress`
berbagi satu baris katalog di §7.1, dan `DateRangePicker` satu berkas dengan `DatePicker`.

## Token

Token M3 ada di `src/styles/m3/`. Hex/rgb literal hanya boleh muncul di berkas token
(`color.css`, `theme.json`); komponen memakai nama token, dan `src/styles/m3/*.test.ts` menjaga
aturan itu. Header "sementara, DV-2, diganti di SHAA-187" wajib tetap ada sampai SHAA-187 mendarat.

## Uji

- `src/components/m3/*.test.tsx` — perilaku dan aksesibilitas komponen (RAC + ARIA).
- `src/components/m3/catalog.test.ts` — kesesuaian katalog dengan ADR §7.1.
- `tests/csp.test.ts` — bentuk header CSP.
- `e2e/shell.spec.ts` — smoke app shell di Chromium, plus listener `securitypolicyviolation`
  (ADR-0003 §3.5) yang menggagalkan test saat ada pelanggaran CSP.
- `e2e/visual.spec.ts` — bukti visual di lima lebar (1440/1024/768/375/320). Tangkapan layar
  ditulis ke `e2e/screenshots/<layar>@<lebar>.png`; nama berkas itu kontraknya, jadi reviewer bisa
  membuka foldernya dan memeriksa satu layar sekilas. Suite ini memakai build `--mode playwright`
  (`make build-e2e`) karena peran di-pin lewat `?peran=`, dan seam itu tidak boleh ada di bundel
  produksi.
