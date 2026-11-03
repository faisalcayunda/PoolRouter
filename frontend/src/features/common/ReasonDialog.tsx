import { useEffect, useState, type FormEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { TextField } from '../../components/m3/TextField';

const REASON_FIELD_ID = 'alasan-auditor';

/** Reason bounds shared by the auditor endpoints (section 18.3 and the `nik.reason` copy). */
export const REASON_MIN = 10;
export const REASON_MAX = 500;

export interface ReasonDialogProps {
  isOpen: boolean;
  title: string;
  body: string;
  /** Pre-filled value; the caller decides whether to carry the search reason over. */
  initialReason?: string;
  confirmLabel: string;
  onConfirm: (reason: string) => void;
  onCancel: () => void;
  isSubmitting?: boolean;
  /** Server rejection of the request as a whole (never of the reason text itself). */
  submitError?: string | null;
}

/**
 * Auditor reason dialog (S12 / S12b, `Dialog basic`). The reason is required and never remembered
 * between openings: the field starts from `initialReason` and the caller drops it after the
 * request, so a second opening needs a fresh reason (section 18.3).
 */
export function ReasonDialog({
  isOpen,
  title,
  body,
  initialReason = '',
  confirmLabel,
  onConfirm,
  onCancel,
  isSubmitting = false,
  submitError = null,
}: ReasonDialogProps) {
  const { t } = useTranslation();
  const [reason, setReason] = useState(initialReason);
  const [touched, setTouched] = useState(false);
  // Opening is when the caller's `initialReason` becomes current: the field may have mounted long
  // before that value existed (the auditor screen sets it after the search), and
  // `useState(initialReason)` only reads it on the first render. Adjusting state during render is
  // React's documented pattern for "reset state when a prop changes" — an effect would render the
  // stale value once first and is flagged as a cascading update.
  const [seededFrom, setSeededFrom] = useState<{ isOpen: boolean; reason: string }>({
    isOpen,
    reason: initialReason,
  });
  if (seededFrom.isOpen !== isOpen || seededFrom.reason !== initialReason) {
    setSeededFrom({ isOpen, reason: initialReason });
    if (isOpen) {
      setReason(initialReason);
      setTouched(false);
    }
  }

  // Focus moves into the field when the dialog opens. `autoFocus` is banned by jsx-a11y
  // (no-autofocus) because it also fires on page load; this effect is scoped to `isOpen`.
  // React Aria's `Dialog` also seats focus on the first focusable child, so this is a safety net.
  useEffect(() => {
    if (!isOpen) return;
    const field = document.getElementById(REASON_FIELD_ID);
    field?.focus();
  }, [isOpen]);

  const trimmed = reason.trim();
  const error = !touched
    ? null
    : trimmed.length === 0
      ? t('nik.reason.error', { min: REASON_MIN })
      : trimmed.length < REASON_MIN
        ? t('nik.reason.error', { min: REASON_MIN })
        : null;

  function submit(event: FormEvent) {
    event.preventDefault();
    setTouched(true);
    if (trimmed.length < REASON_MIN) return;
    onConfirm(trimmed);
  }

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={title}
      closeLabel={t('common.action.close')}
      footer={
        <>
          <Button variant="text" onPress={onCancel}>
            {t('common.action.cancel')}
          </Button>
          <Button
            variant="filled"
            type="submit"
            form="reason-form"
            isDisabled={isSubmitting || trimmed.length < REASON_MIN}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      <form id="reason-form" onSubmit={submit} className="flex flex-col gap-3">
        <p className="typescale-body-medium text-on-surface-variant">{body}</p>
        {submitError ? <InlineAlert tone="error">{submitError}</InlineAlert> : null}
        <TextField
          id={REASON_FIELD_ID}
          label={t('nik.reason.field')}
          value={reason}
          onChange={(value) => {
            setReason(value);
            setTouched(true);
          }}
          isInvalid={Boolean(error)}
          errorMessage={error ?? undefined}
          description={t('nik.reason.help')}
          isRequired
          maxLength={REASON_MAX}
        />
      </form>
    </Dialog>
  );
}
