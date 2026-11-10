//! Dokumen provider hasil `seed dukcapil` (ARC-237) harus terbaca parser runtime. Uji ini ada di
//! crate adapter karena `poolrouter-core` tidak boleh bergantung ke sini; `baca_dokumen_provider`
//! adalah pembaca yang sama dengan jalur sync (pemanggil.rs) dan async (handler.rs).

use poolrouter_adapter_provider::dokumen::baca_dokumen_provider;
use poolrouter_core::admin::seed_dukcapil::dokumen_provider;

#[test]
fn dokumen_seed_dukcapil_terbaca_parser_runtime() {
    let hasil = baca_dokumen_provider(&dokumen_provider());
    assert!(hasil.is_ok(), "{hasil:?}");
}
