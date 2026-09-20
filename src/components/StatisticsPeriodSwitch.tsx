import { useId } from 'react';
import { periodLabels, statisticsPeriods } from '../lib/statisticsPeriods';
import type { TokenPeriod } from '../types';

export function StatisticsPeriodSwitch({ selectedPeriod, onPeriodChange, labels = periodLabels }: {
  selectedPeriod: TokenPeriod['period'];
  onPeriodChange: (period: TokenPeriod['period']) => void;
  labels?: Record<TokenPeriod['period'], string>;
}) {
  const groupId = useId();
  return <div className="statistics-period-switch mb-3 grid grid-cols-5 gap-[3px] rounded-full border border-solid border-[var(--line)] bg-[var(--track)] p-[3px]" role="radiogroup" aria-label="统计时段">
    {statisticsPeriods.map((value) => <label key={value} className="group relative min-w-0 cursor-pointer">
      <input className="peer sr-only" type="radio" name={groupId} value={value} checked={selectedPeriod === value} onChange={() => onPeriodChange(value)} />
      <span className="block whitespace-nowrap rounded-full px-1 py-1.5 text-center text-[11px] leading-[1.4] text-[var(--secondary)] transition-[background-color,color,box-shadow] [transition-duration:120ms] ease-in-out group-hover:text-[var(--text)] peer-checked:bg-[var(--card-bg)] peer-checked:font-semibold peer-checked:text-[var(--text)] peer-checked:shadow-[0_1px_3px_#0000000d] peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-[var(--focus)] motion-reduce:transition-none">{labels[value]}</span>
    </label>)}
  </div>;
}
