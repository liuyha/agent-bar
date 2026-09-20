import { useId, useState, type KeyboardEvent } from 'react';
import { formatCount, formatTokens } from '../lib/format';
import type { TrendData } from '../lib/statisticsTrend';
import './StatisticsTrendChart.css';

const LEFT = 48;
const TOP = 12;
const BOTTOM = 132;

/** Calendar buckets stay separate even for long histories; the plot scrolls horizontally. */
export function StatisticsTrendChart({ data }: { data: TrendData }) {
  const headingId = useId();
  const helpId = useId();
  const [kind, setKind] = useState<'line' | 'bar'>('line');
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const { points, granularity, message } = data;
  const width = Math.max(260, LEFT + points.length * 10 + 12);
  const plotWidth = width - LEFT - 10;
  const step = plotWidth / Math.max(1, points.length);
  const peak = points.reduce((max, point) => Math.max(max, point.tokens ?? 0), 0);
  const scale = peak || 1;
  const x = (index: number) => LEFT + step * (index + 0.5);
  const y = (value: number) => BOTTOM - value / scale * (BOTTOM - TOP);
  const activeIndex = points.findIndex((point) => point.key === activeKey);
  const active = points[activeIndex];
  const hasData = points.some((point) => point.tokens !== null);
  const path = points.map((point, index) => point.tokens === null ? ''
    : `${index === 0 || points[index - 1].tokens === null ? 'M' : 'L'}${x(index)},${y(point.tokens)}`).join(' ');
  const tickStep = Math.max(1, Math.ceil(58 / step));
  const tickLabel = (label: string) => granularity === 'hour'
    ? label.match(/\d{2}:\d{2}/)?.[0] ?? label
    : label.slice(5).replace('-', '/');

  function navigate(event: KeyboardEvent<HTMLDivElement>) {
    if (!points.length || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? points.length - 1
      : activeIndex < 0 ? (event.key === 'ArrowLeft' ? points.length - 1 : 0)
      : Math.max(0, Math.min(points.length - 1, activeIndex + (event.key === 'ArrowRight' ? 1 : -1)));
    setActiveKey(points[index].key);
    const svg = event.currentTarget.querySelector('svg');
    const transform = svg?.getScreenCTM();
    if (transform) {
      const screenPosition = new DOMPoint(x(index), 0).matrixTransform(transform);
      const position = screenPosition.x - event.currentTarget.getBoundingClientRect().left + event.currentTarget.scrollLeft;
      if (position < event.currentTarget.scrollLeft + LEFT || position > event.currentTarget.scrollLeft + event.currentTarget.clientWidth - 20) {
        event.currentTarget.scrollLeft = Math.max(0, position - event.currentTarget.clientWidth / 2);
      }
    }
  }

  return <section className="statistics-trend" aria-labelledby={headingId}>
    <div className="statistics-trend-heading">
      <div><h3 id={headingId}>Token 用量趋势</h3><span>{granularity === 'hour' ? '按小时' : '按天'}</span></div>
      <div className="statistics-chart-switch" role="group" aria-label="图表类型">
        <button type="button" aria-pressed={kind === 'line'} onClick={() => setKind('line')}>折线</button>
        <button type="button" aria-pressed={kind === 'bar'} onClick={() => setKind('bar')}>柱状</button>
      </div>
    </div>
    {hasData ? <>
      <div className="statistics-trend-plot" tabIndex={0} role="group" aria-label={`${granularity === 'hour' ? '每小时' : '每日'} Token ${kind === 'line' ? '折线图' : '柱状图'}`} aria-describedby={helpId}
        onKeyDown={navigate} onBlur={() => setActiveKey(null)} onMouseLeave={() => setActiveKey(null)}>
        <svg viewBox={`0 0 ${width} 157`} style={{ minWidth: points.length > 31 ? width : undefined }} aria-hidden="true"
          onMouseMove={(event) => {
            const transform = event.currentTarget.getScreenCTM();
            if (!transform) return;
            const position = new DOMPoint(event.clientX, event.clientY).matrixTransform(transform.inverse()).x;
            const index = Math.max(0, Math.min(points.length - 1, Math.floor((position - LEFT) / step)));
            setActiveKey(points[index].key);
          }}>
          {[scale, scale / 2, 0].map((value) => <g key={value} className="statistics-chart-grid">
            <line x1={LEFT} x2={width - 10} y1={y(value)} y2={y(value)} />
            <text x={LEFT - 6} y={y(value) + 3} textAnchor="end">{formatTokens(value)}</text>
          </g>)}
          {kind === 'line' && <path className="statistics-chart-line" d={path} />}
          {points.map((point, index) => <g key={point.key}>
            {point.tokens !== null && (kind === 'bar'
              ? <rect className="statistics-chart-bar" x={x(index) - Math.min(16, step * 0.65) / 2} y={y(point.tokens) - (point.tokens === 0 ? 1 : 0)} width={Math.min(16, step * 0.65)} height={Math.max(1, BOTTOM - y(point.tokens))} rx="1.5" />
              : <circle className="statistics-chart-dot" cx={x(index)} cy={y(point.tokens)} r={points.length > 31 ? 1.6 : 2.5} />)}
            {(index % tickStep === 0 || index === points.length - 1 && points.length % tickStep > tickStep / 2) && <text className="statistics-chart-tick" x={x(index)} y="151" textAnchor="middle">{tickLabel(point.label)}</text>}
          </g>)}
          {active && <g className="statistics-chart-active"><line x1={x(activeIndex)} x2={x(activeIndex)} y1={TOP} y2={BOTTOM} />{active.tokens !== null && <circle cx={x(activeIndex)} cy={y(active.tokens)} r="4" />}</g>}
        </svg>
      </div>
      <p className="statistics-chart-detail" id={helpId} aria-live="polite">{active ? <><span>{active.label}</span><strong>{active.tokens === null ? '暂无记录' : `${formatCount(active.tokens)} Token`}</strong></> : <span>悬停或用方向键查看用量{points.length > 31 ? ' · 可横向滚动' : ''}</span>}</p>
      {message && <p className="statistics-footnote">{message}</p>}
    </> : <p className="statistics-chart-empty" role="status">{message || '暂无可展示的用量记录。'}</p>}
  </section>;
}
