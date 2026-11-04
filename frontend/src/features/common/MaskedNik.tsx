/**
 * A masked NIK that keeps its four trailing digits together (section 4: "NIK tampil 4 digit akhir").
 *
 * As a plain text run the value breaks wherever the cell needs it, so on a 320 px card the check
 * digits split ("…****0" / "001") and the digits the screen is required to show stop being a unit.
 * The `<wbr>` gives the line one place to break, and `whitespace-nowrap` holds the tail intact.
 */
export function MaskedNik({ value }: { value: string }) {
  // A value shorter than the mask (a stub, an empty string) has no tail to protect.
  if (value.length <= 4) return <>{value}</>;
  return (
    <>
      {value.slice(0, -4)}
      <wbr />
      <span className="whitespace-nowrap">{value.slice(-4)}</span>
    </>
  );
}
