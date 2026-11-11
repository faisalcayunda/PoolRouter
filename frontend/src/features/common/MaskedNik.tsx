/**
 * A masked NIK shown as one unbreakable unit (section 4: "NIK tampil 4 digit akhir"; spec ARC-363
 * §6 L6: NIK and codes never break mid-string).
 *
 * A masked value is 16 characters, narrower than any cell it sits in, so the whole run stays on
 * one line; a break inside it (even before the four check digits) splits a value the reader
 * compares digit by digit.
 */
export function MaskedNik({ value }: { value: string }) {
  return <span className="whitespace-nowrap">{value}</span>;
}
