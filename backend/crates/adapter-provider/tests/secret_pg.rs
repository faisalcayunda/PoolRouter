//! AC-23 (PG asli, `pr_api`): secret provider dibuka dari `provider_secret`
//! dengan KEK sintetis, dan nilainya tidak muncul di `Debug`, `Display`, error,
//! tracing, maupun byte yang tersimpan. Butuh `POOLROUTER_TEST_DATABASE_URL_API`
//! + `POOLROUTER_TEST_DATABASE_URL_SU`; dijalankan oleh `make -C backend uji-pg`.
//!
//! Nilai di sini sintetis. Tidak ada kredensial Dukcapil asli.
//!
//! shortcut: secret module (AC-23) belum diimplementasikan, test ditunda sampai
//! modul `poolrouter_adapter_provider::secret` ada.

// placeholder: test akan ditambahkan saat secret module terimplementasi
