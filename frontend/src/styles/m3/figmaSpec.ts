/**
 * Nilai UI v3 dari spec `products/poolrouter/specs/ui-v3-figma/spec.md` §3 (KB `main`, ARC-364 dan
 * lanjutan ARC-369) dan `design/tokens.json` (ARC-365). Hanya dipakai test: sumber kebenaran runtime
 * tetap `color.css` dan `tokens.css`; berkas ini yang dibandingkan dengan keduanya.
 */

/** 49 role warna: 48 role lama plus ekstensi `primary-text`. */
export const FIGMA_COLOR_ROLES: Record<string, string> = {
  primary: '#a7c513',
  'on-primary': '#101928',
  'primary-container': '#eff9be',
  'on-primary-container': '#101928',
  secondary: '#0d5eba',
  'on-secondary': '#ffffff',
  'secondary-container': '#ebf8ab',
  'on-secondary-container': '#101928',
  tertiary: '#802f00',
  'on-tertiary': '#ffffff',
  'tertiary-container': '#a74000',
  'on-tertiary-container': '#ffd3c1',
  error: '#cb1a14',
  'on-error': '#ffffff',
  'error-container': '#fbeae9',
  'on-error-container': '#9e0a05',
  success: '#04802e',
  'on-success': '#ffffff',
  'success-container': '#e7f6ec',
  'on-success-container': '#036b26',
  warning: '#8a5a00',
  'on-warning': '#ffffff',
  'warning-container': '#f5b546',
  'on-warning-container': '#101928',
  info: '#0d5eba',
  'on-info': '#ffffff',
  'info-container': '#ebeef5',
  'on-info-container': '#052243',
  surface: '#ffffff',
  'surface-bright': '#ffffff',
  'surface-container-lowest': '#ffffff',
  'surface-container-low': '#f9fafb',
  'surface-container': '#f0f2f5',
  'surface-container-high': '#e4e7ec',
  'surface-container-highest': '#d0d5dd',
  'surface-dim': '#d0d5dd',
  'on-surface': '#101928',
  'on-surface-variant': '#475367',
  'inverse-surface': '#101928',
  'inverse-on-surface': '#f9fafb',
  'inverse-primary': '#d0ee3c',
  outline: '#7e8899',
  'outline-variant': '#e4e7ec',
  'outline-soft': '#d0d5dd',
  scrim: '#101928',
  shadow: '#101928',
  'primary-text': '#5a6b0a',
  accent: '#5a6b0a',
  'accent-container': '#eff9be',
};

/** Typescale yang diubah spec §3: [ukuran, line-height, weight]; typeface `brand` atau `plain`. */
export const FIGMA_TYPESCALE: Record<
  string,
  { size: string; lineHeight: string; weight: string; font: 'brand' | 'plain' }
> = {
  'headline-large': { size: '25px', lineHeight: '32px', weight: '600', font: 'brand' },
  'headline-small': { size: '20px', lineHeight: '28px', weight: '700', font: 'brand' },
  'title-large': { size: '20px', lineHeight: '28px', weight: '600', font: 'brand' },
  'title-medium': { size: '18px', lineHeight: '24px', weight: '700', font: 'brand' },
  'title-small': { size: '14px', lineHeight: '20px', weight: '600', font: 'brand' },
  'body-large': { size: '16px', lineHeight: '23px', weight: '400', font: 'plain' },
  'body-medium': { size: '14px', lineHeight: '20px', weight: '400', font: 'plain' },
  'label-large': { size: '14px', lineHeight: '20px', weight: '600', font: 'brand' },
  'label-medium': { size: '14px', lineHeight: '20px', weight: '500', font: 'brand' },
};

/** Shape spec §3: hanya large 16 dan extra-large 20 berubah; extra-small tetap 6 (Input radius 6). */
export const FIGMA_SHAPE: Record<string, string> = {
  none: '0px',
  'extra-small': '6px',
  small: '8px',
  medium: '12px',
  large: '16px',
  'extra-large': '20px',
  full: '9999px',
};

/** Elevation level1 = Figma Shadow/xs `2:19626` (`tokens.json`). */
export const FIGMA_ELEVATION_LEVEL1 = '0px 1px 2px 0px rgba(16, 24, 40, 0.05)';

/** Pasangan §3.1: [depan, latar, ambang]. 4,5 untuk teks, 3 untuk kontrol dan fokus. */
export const CONTRAST_PAIRS: ReadonlyArray<readonly [string, string, number]> = [
  // Teks
  ['on-primary', 'primary', 4.5],
  ['on-primary-container', 'primary-container', 4.5],
  ['on-secondary-container', 'secondary-container', 4.5],
  ['primary-text', 'surface', 4.5],
  ['primary-text', 'primary-container', 4.5],
  ['on-surface-variant', 'surface-container-low', 4.5],
  ['on-error-container', 'error-container', 4.5],
  ['on-success-container', 'success-container', 4.5],
  ['on-warning-container', 'warning-container', 4.5],
  ['on-info-container', 'info-container', 4.5],
  ['on-error', 'error', 4.5],
  ['on-success', 'success', 4.5],
  ['on-secondary', 'secondary', 4.5],
  ['inverse-on-surface', 'inverse-surface', 4.5],
  ['error-container', 'inverse-surface', 4.5],
  ['error', 'surface-container-low', 4.5],
  // Kontrol dan fokus
  ['outline', 'surface', 3],
  ['outline', 'surface-container-low', 3],
  ['secondary', 'surface', 3],
  ['secondary', 'surface-container-low', 3],
  ['secondary', 'primary-container', 3],
  ['secondary', 'primary', 3],
  ['inverse-primary', 'inverse-surface', 3],
  ['accent', 'surface-container', 3],
  ['primary-text', 'surface', 3],
  ['warning', 'warning-container', 3],
  ['on-surface-variant', 'surface-container-highest', 3],
];

/** Rasio yang ditulis spec §3.1, dibulatkan dua desimal; test memastikan nilai kode sama. */
export const CONTRAST_CLAIMS: Record<string, number> = {
  'on-primary/primary': 8.93,
  'on-primary-container/primary-container': 15.91,
  'on-secondary-container/secondary-container': 15.56,
  'primary-text/surface': 5.93,
  'primary-text/primary-container': 5.35,
  'on-surface-variant/surface-container-low': 7.44,
  'on-error-container/error-container': 7.22,
  'on-success-container/success-container': 5.99,
  'on-warning-container/warning-container': 9.72,
  'on-info-container/info-container': 13.74,
  'on-error/error': 5.68,
  'on-success/success': 5.08,
  'on-secondary/secondary': 6.31,
  'inverse-on-surface/inverse-surface': 16.86,
  'error-container/inverse-surface': 15.14,
  'error/surface-container-low': 5.44,
  'outline/surface': 3.58,
  'outline/surface-container-low': 3.42,
  'secondary/surface': 6.31,
  'secondary/surface-container-low': 6.04,
  'secondary/primary-container': 5.7,
  'secondary/primary': 3.2,
  'inverse-primary/inverse-surface': 13.41,
  'accent/surface-container': 5.29,
  'warning/warning-container': 3.27,
  'on-surface-variant/surface-container-highest': 5.27,
};
