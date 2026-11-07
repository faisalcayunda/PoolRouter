import { useEffect, useRef, useState, type FormEvent } from 'react';
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
  /**
   * Field label. Section 13.2 gives the payload dialog's reason step its own label
   * ("Alasan membuka payload"), while S11/S12 keep the NIK search wording.
   */
  fieldLabel?: string;
  /** Supporting hint under the field; defaults to the NIK search hint. */
  fieldHint?: string;
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
  fieldLabel,
  fieldHint,
  onConfirm,
  onCancel,
  isSubmitting = false,
  submitError = null,
}: ReasonDialogProps) {
  const { t } = useTranslation();
  const [reason, setReason] = useState(initialReason);
  const [touched, setTouched] = useState(false);
  // Section 8: pressing submit with an empty reason must show the error. Waiting for a keystroke
  // would leave the button looking inert; this flag reports the problem straight away.
  const [submitAttempted, setSubmitAttempted] = useState(false);
  // One press must confirm once: the footer button drives the press itself, so it calls
  // `confirmPress` directly and the form's `onSubmit` covers Enter. Both share one guard.
  const submittingRef = useRef(false);
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
      setSubmitAttempted(false);
    }
  }

  // The duplicate-press guard is cleared when the dialog opens. A ref may not be written during
  // render (react-hooks/refs), so it is done in an effect instead.
  useEffect(() => {
    if (isOpen) submittingRef.current = false;
  }, [isOpen]);

  // Focus moves into the field when the dialog opens. `autoFocus` is banned by jsx-a11y
  // (no-autofocus) because it also fires on page load; this effect is scoped to `isOpen`.
  // React Aria's `Dialog` also seats focus on the first focusable child, so this is a safety net.
  useEffect(() => {
    if (!isOpen) return;
    const field = document.getElementById(REASON_FIELD_ID);
    field?.focus();
  }, [isOpen]);

  const trimmed = reason.trim();
  // Section 8: the submit stays enabled and validation reports the problem, so both ends of the
  // bound are errors the auditor can see — over-long text is no longer silently truncated.
  const invalid = trimmed.length < REASON_MIN || trimmed.length > REASON_MAX;
  const error =
    !touched && !submitAttempted ? null : invalid ? t('log.payload.reason.error') : null;

  function confirmPress() {
    setTouched(true);
    setSubmitAttempted(true);
    if (invalid || isSubmitting || submittingRef.current) return;
    submittingRef.current = true;
    onConfirm(trimmed);
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    confirmPress();
  }

  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={title}
      closeLabel={t('common.close')}
    >
      {/*
        One form around the body *and* the footer. The footer outlives the body in the DOM, so a
        submit button there cannot reference a form id that has already closed: React Aria's dialog
        renders the footer as a sibling of the content. Enter and the confirm button therefore both
        submit this form, and section 8's "submit stays enabled, validation reports" holds.
      */}
      <form id="reason-form" onSubmit={submit} className="flex flex-col gap-3">
        <p className="typescale-body-medium text-on-surface-variant">{body}</p>
        {submitError ? <InlineAlert tone="error">{submitError}</InlineAlert> : null}
        <TextField
          id={REASON_FIELD_ID}
          label={fieldLabel ?? t('nik.reason.field')}
          value={reason}
          onChange={(value) => {
            setReason(value);
            setTouched(true);
          }}
          isInvalid={Boolean(error)}
          errorMessage={error ?? undefined}
          description={fieldHint ?? t('nik.reason.help')}
          isRequired
        />
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="text" onPress={onCancel}>
            {t('common.action.cancel')}
          </Button>
          {/*
            The press runs the check directly. React Aria drives presses through its own events and
            never triggers a native form submission, so a `type="submit"` button outside the form
            would be inert — and one linked by `form=` warns "form is not connected" once the
            dialog unmounts mid-submit. Enter is handled by the form's own `onSubmit`.
          */}
          <Button variant="filled" onPress={confirmPress} isDisabled={isSubmitting}>
            {confirmLabel}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
