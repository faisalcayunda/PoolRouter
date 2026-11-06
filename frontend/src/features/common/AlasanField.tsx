import { TextArea } from '../../components/m3/TextField';

export function AlasanField(props: {
  value: string;
  onChange: (v: string) => void;
  error?: string;
}) {
  return (
    <TextArea
      label="Alasan"
      value={props.value}
      onChange={(e) => props.onChange(e.target.value)}
      errorMessage={props.error}
    />
  );
}
