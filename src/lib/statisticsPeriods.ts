import type { TokenPeriod } from '../types';

export const statisticsPeriods = ['day', 'week', 'month', 'year', 'all'] as const;
export const periodLabels: Record<TokenPeriod['period'], string> = { day: '今日', week: '本周', month: '本月', year: '本年', all: '全部' };
export const accountPeriodLabels: Record<TokenPeriod['period'], string> = { ...periodLabels, day: '昨日' };
