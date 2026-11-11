//! Topologi RabbitMQ untuk antrean job (ADR-0001 bagian 5 lapisan 1 dan
//! bagian 6, ADR-0004 bagian 4.3 W4, spec `f0-controller-worker` AC-3).
//!
//! - Satu exchange direct durable [`EXCHANGE_JOB`] untuk semua job. Setiap
//!   antrean di-bind dengan routing key = nama antrean, jadi publish
//!   `mandatory` ke antrean yang sudah dideklarasikan selalu terarah.
//! - Antrean durable bertipe quorum (ADR-0001 bagian 5: "quorum di
//!   produksi"). Tipe diatur eksplisit lewat argumen, bukan `Config::quorum`
//!   apalis-amqp (W4).
//! - Dead-letter: pesan yang di-reject tanpa requeue masuk exchange
//!   [`EXCHANGE_DLX`] lalu antrean `<antrean>.dlq` (routing key = nama
//!   antrean asal).
//! - DLQ punya retensi ([`DLQ_TTL_MS`], [`DLQ_PANJANG_MAKS`], ARC-228):
//!   pesan di DLQ hanya salinan diagnostik, karena request-nya tetap
//!   `dispatched` di PostgreSQL dan reconciler yang memulihkannya.
//!
//! Argumen antrean dibangun satu fungsi ([`argumen_antrean`]) dan dipakai
//! deklarasi lapin dan deklarasi ulang apalis-amqp, karena argumen yang
//! berbeda membuat broker menjawab 406 dan menutup channel.

use lapin::{
    Channel, ExchangeKind,
    options::{ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions},
    types::{AMQPValue, FieldTable, ShortString},
};

/// Exchange direct untuk semua job antrean PoolRouter.
pub const EXCHANGE_JOB: &str = "pr.jobs";

/// Exchange dead-letter untuk pesan yang gagal permanen.
pub const EXCHANGE_DLX: &str = "pr.jobs.dlx";

/// Umur maksimum pesan di DLQ: 7 hari (`x-message-ttl`, milidetik). Cukup
/// untuk investigasi lewat akhir pekan; lebih lama hanya menyimpan salinan
/// `request_id` yang sudah dipulihkan reconciler.
pub const DLQ_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Panjang maksimum satu DLQ (`x-max-length`). Body pesan hanya `request_id`,
/// jadi 10.000 pesan kecil; penuh berarti pesan tertua dibuang (`drop-head`).
pub const DLQ_PANJANG_MAKS: i64 = 10_000;

/// Nama antrean dead-letter untuk satu antrean job.
pub fn nama_dlq(antrean: &str) -> String {
    format!("{antrean}.dlq")
}

fn teks(nilai: &str) -> AMQPValue {
    AMQPValue::LongString(nilai.into())
}

fn argumen_quorum() -> FieldTable {
    let mut argumen = FieldTable::default();
    argumen.insert(ShortString::from("x-queue-type"), teks("quorum"));
    argumen
}

/// Argumen deklarasi DLQ: quorum + retensi. Terpisah dari antrean job supaya
/// TTL dan batas panjang tidak pernah ikut membuang job yang masih hidup.
/// `drop-head` (bawaan quorum, ditulis eksplisit) membuang pesan tertua saat
/// penuh; `reject-publish` dihindari karena membuat dead-lettering dari
/// antrean job tersangkut.
pub(crate) fn argumen_dlq() -> FieldTable {
    let mut argumen = argumen_quorum();
    argumen.insert(
        ShortString::from("x-message-ttl"),
        AMQPValue::LongLongInt(DLQ_TTL_MS),
    );
    argumen.insert(
        ShortString::from("x-max-length"),
        AMQPValue::LongLongInt(DLQ_PANJANG_MAKS),
    );
    argumen.insert(ShortString::from("x-overflow"), teks("drop-head"));
    argumen
}

/// Argumen deklarasi antrean job: quorum + dead-letter ke [`EXCHANGE_DLX`].
pub(crate) fn argumen_antrean(antrean: &str) -> FieldTable {
    let mut argumen = argumen_quorum();
    argumen.insert(
        ShortString::from("x-dead-letter-exchange"),
        teks(EXCHANGE_DLX),
    );
    argumen.insert(
        ShortString::from("x-dead-letter-routing-key"),
        teks(antrean),
    );
    argumen
}

/// Opsi deklarasi antrean job dan DLQ: durable, tidak eksklusif, tidak
/// auto-delete.
pub(crate) fn opsi_antrean() -> QueueDeclareOptions {
    QueueDeclareOptions {
        durable: true,
        ..QueueDeclareOptions::default()
    }
}

fn opsi_exchange() -> ExchangeDeclareOptions {
    ExchangeDeclareOptions {
        durable: true,
        ..ExchangeDeclareOptions::default()
    }
}

/// Deklarasikan exchange, DLX, DLQ, antrean, dan binding untuk `antrean`.
/// Idempoten: deklarasi ulang dengan argumen yang sama tidak mengubah apa pun.
pub(crate) async fn deklarasi(kanal: &Channel, antrean: &str) -> Result<(), lapin::Error> {
    kanal
        .exchange_declare(
            EXCHANGE_JOB.into(),
            ExchangeKind::Direct,
            opsi_exchange(),
            FieldTable::default(),
        )
        .await?;
    kanal
        .exchange_declare(
            EXCHANGE_DLX.into(),
            ExchangeKind::Direct,
            opsi_exchange(),
            FieldTable::default(),
        )
        .await?;
    let dlq = nama_dlq(antrean);
    kanal
        .queue_declare(dlq.as_str().into(), opsi_antrean(), argumen_dlq())
        .await?;
    kanal
        .queue_bind(
            dlq.as_str().into(),
            EXCHANGE_DLX.into(),
            antrean.into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    kanal
        .queue_declare(antrean.into(), opsi_antrean(), argumen_antrean(antrean))
        .await?;
    kanal
        .queue_bind(
            antrean.into(),
            EXCHANGE_JOB.into(),
            antrean.into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nilai<'a>(argumen: &'a FieldTable, kunci: &str) -> Option<&'a AMQPValue> {
        argumen.inner().get(&ShortString::from(kunci))
    }

    #[test]
    fn dlq_diturunkan_dari_nama_antrean() {
        assert_eq!(
            nama_dlq("pr.jobs.upstream.0190a000-0000-7000-8000-000000000001"),
            "pr.jobs.upstream.0190a000-0000-7000-8000-000000000001.dlq"
        );
    }

    #[test]
    fn antrean_quorum_dengan_dead_letter_ke_dlx() {
        let argumen = argumen_antrean("pr.jobs.upstream.x");
        assert_eq!(nilai(&argumen, "x-queue-type"), Some(&teks("quorum")));
        assert_eq!(
            nilai(&argumen, "x-dead-letter-exchange"),
            Some(&teks(EXCHANGE_DLX))
        );
        assert_eq!(
            nilai(&argumen, "x-dead-letter-routing-key"),
            Some(&teks("pr.jobs.upstream.x"))
        );
        assert_eq!(argumen.inner().len(), 3);
    }

    #[test]
    fn argumen_quorum_hanya_tipe_antrean() {
        let argumen = argumen_quorum();
        assert_eq!(nilai(&argumen, "x-queue-type"), Some(&teks("quorum")));
        assert_eq!(argumen.inner().len(), 1);
    }

    #[test]
    fn dlq_quorum_dengan_retensi_tanpa_dead_letter_lanjutan() {
        let argumen = argumen_dlq();
        assert_eq!(nilai(&argumen, "x-queue-type"), Some(&teks("quorum")));
        assert_eq!(
            nilai(&argumen, "x-message-ttl"),
            Some(&AMQPValue::LongLongInt(7 * 24 * 60 * 60 * 1000))
        );
        assert_eq!(
            nilai(&argumen, "x-max-length"),
            Some(&AMQPValue::LongLongInt(10_000))
        );
        assert_eq!(nilai(&argumen, "x-overflow"), Some(&teks("drop-head")));
        assert_eq!(argumen.inner().len(), 4);
    }

    #[test]
    fn antrean_job_tanpa_retensi() {
        let argumen = argumen_antrean("pr.jobs.upstream.x");
        assert!(nilai(&argumen, "x-message-ttl").is_none());
        assert!(nilai(&argumen, "x-max-length").is_none());
    }

    #[test]
    fn antrean_durable_dan_tidak_sementara() {
        let opsi = opsi_antrean();
        assert!(opsi.durable);
        assert!(!opsi.exclusive);
        assert!(!opsi.auto_delete);
        assert!(!opsi.passive);
    }
}
