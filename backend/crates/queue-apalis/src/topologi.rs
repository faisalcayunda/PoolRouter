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
        .queue_declare(dlq.as_str().into(), opsi_antrean(), argumen_quorum())
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
    fn dlq_quorum_tanpa_dead_letter_lanjutan() {
        let argumen = argumen_quorum();
        assert_eq!(nilai(&argumen, "x-queue-type"), Some(&teks("quorum")));
        assert_eq!(argumen.inner().len(), 1);
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
