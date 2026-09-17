// Adapted from shadcn/ui (new-york), retaining the visible remaining-quota width.
import * as React from 'react';
import * as ProgressPrimitive from '@radix-ui/react-progress';
import { cn } from '@/lib/utils';

type ProgressProps = React.ComponentPropsWithoutRef<typeof ProgressPrimitive.Root> & { indicatorClassName?: string };

const Progress = React.forwardRef<React.ComponentRef<typeof ProgressPrimitive.Root>, ProgressProps>(
  ({ className, value, max = 100, indicatorClassName, ...props }, ref) => {
    const limit = Number.isFinite(max) && max > 0 ? max : 100;
    const current = typeof value === 'number' && Number.isFinite(value) ? Math.min(limit, Math.max(0, value)) : null;
    return (
      <ProgressPrimitive.Root
        ref={ref}
        data-slot="progress"
        className={cn('relative h-[5px] w-full overflow-hidden rounded-full bg-[var(--track)]', className)}
        value={current}
        max={limit}
        {...props}
      >
        <ProgressPrimitive.Indicator
          className={cn('h-full rounded-full bg-[var(--progress-fill)] transition-[width] [transition-duration:450ms]', indicatorClassName)}
          style={{ width: `${current === null ? 0 : current / limit * 100}%` }}
        />
      </ProgressPrimitive.Root>
    );
  },
);
Progress.displayName = ProgressPrimitive.Root.displayName;

export { Progress };
