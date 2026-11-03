import {
  Tooltip as RacTooltip,
  TooltipTrigger as RacTooltipTrigger,
  type TooltipProps as RacTooltipProps,
  type TooltipTriggerComponentProps,
} from 'react-aria-components';
import type { ReactNode } from 'react';
import { cx } from './cx';

export function TooltipTrigger(props: TooltipTriggerComponentProps) {
  return <RacTooltipTrigger {...props} delay={200} closeDelay={0} />;
}

/** Plain tooltip: short helper text on hover/focus (ADR-0003 7.5). */
export function Tooltip({
  className,
  ...props
}: Omit<RacTooltipProps, 'className'> & { className?: string }) {
  return (
    <RacTooltip
      {...props}
      className={cx(
        'typescale-body-small rounded-extra-small bg-inverse-surface px-2.5 py-1.5 text-inverse-on-surface shadow-level2',
        className,
      )}
    />
  );
}

export type { TooltipProps } from 'react-aria-components';

// ReactNode re-export guard for downstream usage typing.
export type TooltipContent = ReactNode;
