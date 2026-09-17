// Adapted from shadcn/ui Native Select; keeps menus inside the native WebView.
import * as React from 'react';
import { ChevronDown } from 'lucide-react';
import { cn } from '@/lib/utils';

type NativeSelectProps = React.ComponentPropsWithoutRef<'select'> & { wrapperClassName?: string };

const NativeSelect = React.forwardRef<HTMLSelectElement, NativeSelectProps>(
  ({ className, wrapperClassName, ...props }, ref) => (
    <div className={cn('relative min-w-0', wrapperClassName)} data-slot="native-select-wrapper">
      <select
        ref={ref}
        data-slot="native-select"
        className={cn(
          'w-full min-w-0 cursor-pointer appearance-none rounded-[5px] border border-solid border-input bg-background py-[5px] pl-2 pr-7 text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-50',
          className,
        )}
        {...props}
      />
      <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-3 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
    </div>
  ),
);
NativeSelect.displayName = 'NativeSelect';

export { NativeSelect };
