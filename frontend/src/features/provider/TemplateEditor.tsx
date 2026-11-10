import {
  FieldError,
  Label,
  Text,
  TextArea as RacTextArea,
  TextField as RacTextField,
} from 'react-aria-components';
import type { ReactNode } from 'react';

/**
 * Editor template JSON request (Penpot F0 / S03, `EditorInner`): permukaan `inverse-surface`
 * bersudut 12 px dengan label di kiri atas dan, bila ada galat, penanda baris di kanan atas.
 * Memakai primitif RAC langsung supaya label, deskripsi, dan galat tetap terhubung ke `textarea`
 * (`aria-describedby`, `aria-invalid`) seperti `TextArea` m3, tetapi dengan warna permukaan gelap.
 */
export function TemplateEditor({
  label,
  lineNote,
  description,
  errorMessage,
  value,
  onChange,
  isInvalid,
}: {
  label: ReactNode;
  /** Penanda singkat di kepala editor, mis. "Baris 4: sintaks error". */
  lineNote?: ReactNode;
  description?: ReactNode;
  errorMessage?: ReactNode;
  value: string;
  onChange: (value: string) => void;
  isInvalid?: boolean;
}) {
  return (
    <RacTextField
      value={value}
      onChange={onChange}
      isInvalid={isInvalid}
      autoComplete="off"
      className="flex flex-col gap-3"
    >
      <div className="flex flex-col gap-3 rounded-medium bg-inverse-surface p-4 text-inverse-on-surface">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <Label className="typescale-label-large text-inverse-on-surface">{label}</Label>
          {lineNote ? (
            <span className="typescale-body-small text-error-container">{lineNote}</span>
          ) : null}
        </div>
        <RacTextArea
          rows={8}
          spellCheck={false}
          className="m3-focus typescale-body-medium min-h-40 w-full resize-y rounded-extra-small bg-transparent font-code text-inverse-on-surface outline-none"
        />
      </div>
      {description ? (
        <Text slot="description" className="typescale-body-medium text-on-surface-variant">
          {description}
        </Text>
      ) : null}
      <FieldError className="typescale-body-small text-error">{errorMessage}</FieldError>
    </RacTextField>
  );
}
