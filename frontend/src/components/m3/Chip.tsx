import { cx } from './cx';
import { Icon, icons, type IconComponent } from '../../icons';

export type ChipVariant = 'assist' | 'filter' | 'input' | 'suggestion';

export interface ChipProps {
  variant?: ChipVariant;
  /** Filter chips show the selected checkmark when set. */
  selected?: boolean;
  /** Optional leading icon (hidden while selected — M3 filter chip behaviour). */
  icon?: IconComponent;
  /** Input chips only: renders the trailing remove button. */
  onRemove?: () => void;
  onClick?: () => void;
  disabled?: boolean;
  /** Accessible name of the remove button (Addendum A 5.4). */
  removeLabel?: string;
  children: React.ReactNode;
}

const BASE =
  'm3-state m3-focus m3-disabled typescale-label-large inline-flex h-8 items-center gap-2 rounded-full border px-3 select-none';

/** M3 chip. Filter chips toggle via `onClick` + `selected`; input chips remove via `onRemove`. */
export function Chip({
  variant = 'assist',
  selected = false,
  icon: LeadingIcon,
  onRemove,
  onClick,
  disabled = false,
  removeLabel,
  children,
}: ChipProps) {
  const clickable = Boolean(onClick);
  return (
    <span
      role={clickable ? 'button' : undefined}
      tabIndex={clickable && !disabled ? 0 : undefined}
      onClick={clickable && !disabled ? onClick : undefined}
      onKeyDown={
        clickable && !disabled
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onClick?.();
              }
            }
          : undefined
      }
      aria-pressed={variant === 'filter' && clickable ? selected : undefined}
      data-disabled={disabled || undefined}
      className={cx(
        BASE,
        variant === 'filter' && selected
          ? 'border-secondary/30 bg-accent-container text-on-secondary-container'
          : 'border-outline bg-surface-container-lowest text-on-surface data-[disabled]:border-on-surface/12 data-[disabled]:text-on-surface/38',
      )}
    >
      {selected ? <Icon icon={icons.check} className="h-4.5 w-4.5" /> : null}
      {LeadingIcon && !selected ? <Icon icon={LeadingIcon} className="h-4.5 w-4.5" /> : null}
      {children}
      {onRemove ? (
        <button
          type="button"
          aria-label={removeLabel}
          disabled={disabled}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className={cx(
            'm3-state m3-focus -mr-1.5 flex h-4.5 w-4.5 cursor-pointer items-center justify-center rounded-full',
            disabled && 'cursor-default text-on-surface/38',
          )}
        >
          <Icon icon={icons.close} className="h-4 w-4" />
        </button>
      ) : null}
    </span>
  );
}
