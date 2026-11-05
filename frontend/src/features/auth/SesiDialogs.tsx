import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { UNSTABLE_ToastQueue } from 'react-aria-components';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Snackbar } from '../../components/m3/Snackbar';
import { useFormatter } from '../../i18n/useFormatter';
import type { SesiState } from './useSesi';

interface Toast {
  message: string;
}

/**
 * The two shell-level session warnings (handoff spec section 12). They sit above every screen,
 * never touch typed values, and only the idle warning can be extended; the 8-hour limit cannot.
 */
export function SesiDialogs({ sesi }: { sesi: SesiState }) {
  const { t } = useTranslation();
  const format = useFormatter();
  const queue = useMemo(() => new UNSTABLE_ToastQueue<Toast>({ maxVisibleToasts: 1 }), []);
  const [dibacaUntuk, setDibacaUntuk] = useState<string | null>(null);
  const [mengirim, setMengirim] = useState(false);

  // Keyed by the absolute deadline: a fresh expiry re-arms the dialog and its banner.
  const absolutDibaca = dibacaUntuk === sesi.sesi?.absolut_berakhir_pada;

  const perpanjang = async () => {
    setMengirim(true);
    const sukses = await sesi.perpanjang();
    setMengirim(false);
    if (sukses) queue.add({ message: t('sesi.idle_dialog.extend_success') }, { timeout: 5000 });
  };

  const absolutJam = sesi.sesi ? format.jam(new Date(sesi.sesi.absolut_berakhir_pada)) : '';

  return (
    <>
      <Dialog
        role="alertdialog"
        dismissable={false}
        hideClose
        isOpen={sesi.fase === 'peringatan_idle'}
        title={t('sesi.idle_dialog.title')}
        description={t('sesi.idle_dialog.body', { jam: 0.5, detik: sesi.detikIdle })}
        closeLabel={t('common.action.close')}
        footer={
          <>
            <Button isDisabled={mengirim} onPress={() => void perpanjang()}>
              {t('sesi.idle_dialog.btn_extend')}
            </Button>
            <Button variant="text" onPress={() => void sesi.keluar()}>
              {t('sesi.idle_dialog.btn_exit')}
            </Button>
          </>
        }
      >
        {sesi.perpanjangGalat ? (
          <InlineAlert tone="error" live="assertive">
            {t('common.error.network.load')}
          </InlineAlert>
        ) : null}
      </Dialog>

      <Dialog
        role="alertdialog"
        isOpen={sesi.fase === 'peringatan_absolut' && !absolutDibaca}
        title={t('sesi.absolute_dialog.title')}
        description={t('sesi.absolute_dialog.body', { waktu: absolutJam })}
        closeLabel={t('common.action.close')}
        onOpenChange={(open) => {
          if (!open) setDibacaUntuk(sesi.sesi?.absolut_berakhir_pada ?? null);
        }}
        footer={
          <Button onPress={() => setDibacaUntuk(sesi.sesi?.absolut_berakhir_pada ?? null)}>
            {t('sesi.absolute_dialog.btn_ok')}
          </Button>
        }
      >
        <span />
      </Dialog>

      {sesi.fase === 'peringatan_absolut' && absolutDibaca ? (
        <div className="mx-auto w-full max-w-[1200px] px-4 pt-4">
          <InlineAlert tone="warning" live="polite">
            {t('sesi.banner_absolute.message', { waktu: absolutJam })}
          </InlineAlert>
        </div>
      ) : null}

      <Snackbar
        queue={queue}
        closeLabel={t('common.action.close')}
        renderContent={(toast) => <span>{toast.message}</span>}
      />
    </>
  );
}
