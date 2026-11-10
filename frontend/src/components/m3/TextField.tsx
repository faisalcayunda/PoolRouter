import {
  FieldError,
  Input,
  Label,
  TextArea as RacTextArea,
  TextField as RacTextField,
  Text,
  type FieldErrorProps,
  type InputProps,
  type LabelProps,
  type TextAreaProps as RacTextAreaProps,
  type TextFieldProps as RacTextFieldProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

const FIELD =
  'm3-focus typescale-body-large h-14 w-full rounded-extra-small border border-outline bg-surface px-4 py-4 text-on-surface ' +
  'placeholder:text-on-surface-variant data-[focused]:border-2 data-[focused]:border-primary data-[focused]:px-[15px]';

export interface TextFieldProps extends Omit<RacTextFieldProps, 'className'> {
  className?: string;
  /** Label content comes from the catalog at the usage site; it is never hardcoded here. */
  label?: ReactNode;
  description?: ReactNode;
  /** Hint shown inside an empty field (Penpot S11 search box); never a substitute for the label. */
  placeholder?: string;
  /** RAC renders FieldError only when validation fails (ADR-0003 7.6). */
  errorMessage?: ReactNode;
}

/** M3 outlined text field. The F0 catalog uses outlined, not filled. */
export function TextField({
  label,
  description,
  errorMessage,
  placeholder,
  className,
  ...props
}: TextFieldProps) {
  return (
    <RacTextField {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label>{label}</Label> : null}
      <Input className={FIELD} placeholder={placeholder} />
      {description ? <Text slot="description">{description}</Text> : null}
      <FieldError>{errorMessage}</FieldError>
    </RacTextField>
  );
}

export interface TextAreaProps extends Omit<RacTextFieldProps, 'className'> {
  className?: string;
  label?: ReactNode;
  description?: ReactNode;
  errorMessage?: ReactNode;
  /** Tinggi awal dalam baris teks. */
  rows?: RacTextAreaProps['rows'];
  /** Kelas tambahan untuk elemen `textarea`, mis. `font-code` untuk JSON (ADR-0008 2.3). */
  textAreaClassName?: string;
}

/**
 * M3 multiline text field (outlined), used for notes and long values in F0.
 * Prop field (value, onChange, isInvalid, name) diteruskan ke `TextField` RAC, bukan ke
 * `textarea`-nya, supaya validasi, `aria-describedby`, dan `FieldError` bekerja seperti `TextField`.
 */
export function TextArea({
  label,
  description,
  errorMessage,
  className,
  rows,
  textAreaClassName,
  ...props
}: TextAreaProps) {
  return (
    <RacTextField {...props} className={cx('flex flex-col gap-1', className)}>
      {label ? <Label>{label}</Label> : null}
      <RacTextArea
        rows={rows}
        className={cx(FIELD, 'h-auto min-h-24 resize-y', textAreaClassName)}
      />
      {description ? <Text slot="description">{description}</Text> : null}
      <FieldError>{errorMessage}</FieldError>
    </RacTextField>
  );
}

/** Slot passthroughs so usage sites can compose without re-importing RAC. */
export const TextFieldLabel = Label as (p: LabelProps) => ReactNode;
export const TextFieldInput = Input as (p: InputProps) => ReactNode;
export const TextFieldError = FieldError as (p: FieldErrorProps) => ReactNode;
