import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { AlertCircle, Check, ChevronDown, ChevronLeft, ChevronRight, Clock3, Layers3, LoaderCircle, Monitor, Moon, Search, Settings, Sun } from 'lucide-react';
import { Button } from './ui/button';
import { NativeSelect } from './ui/native-select';
import { codexStatisticsSources } from '../lib/settings';
import type { AppSettings, CodexStatisticsPreference, ProviderId, ProviderUsage, Theme } from '../types';
import './Preferences.css';

const providers = [{ id: 'codex', name: 'Codex', detail: 'OpenAI' }, { id: 'claude', name: 'Claude', detail: 'Anthropic' }] as const;
const themes: { value: Theme; name: string; icon: typeof Monitor }[] = [
  { value: 'system', name: '跟随系统', icon: Monitor }, { value: 'light', name: '浅色', icon: Sun }, { value: 'dark', name: '深色', icon: Moon },
];
type Page = 'general' | 'providers' | ProviderId;
interface Props {
  draft: AppSettings;
  accounts: ProviderUsage[];
  saving: boolean;
  saved: boolean;
  isDirty: boolean;
  error: string | null;
  desktop: boolean;
  onChange: (patch: Partial<AppSettings>) => void;
  onSave: () => void;
  onReset: () => void;
}
function ProviderIcon({ id }: { id: ProviderId }) {
  return <span className={`preferences-provider-icon provider-${id}`} aria-hidden="true"><span className="provider-icon" /></span>;
}
function Group({ title, children }: { title: string; children: ReactNode }) {
  return <section className="preferences-group" aria-label={title}><h2>{title}</h2><div className="preferences-card">{children}</div></section>;
}

export function Preferences({ draft, accounts, saving, saved, isDirty, error, desktop, onChange, onSave, onReset }: Props) {
  const [history, setHistory] = useState<{ pages: Page[]; index: number }>({ pages: ['general'], index: 0 });
  const [expanded, setExpanded] = useState(true);
  const [search, setSearch] = useState('');
  const content = useRef<HTMLDivElement>(null);
  const page = history.pages[history.index];
  useLayoutEffect(() => { content.current?.scrollTo({ top: 0 }); }, [page]);
  const provider = providers.find(({ id }) => page === id);
  const account = accounts.find(({ id }) => id === provider?.id);
  const title = page === 'general' ? '通用' : page === 'providers' ? '供应商' : provider?.name ?? '供应商';
  const query = search.trim().toLowerCase();
  const matches = (text: string) => !query || text.toLowerCase().includes(query);
  const showGeneral = matches('通用 外观 浅色 深色 跟随系统 自动刷新 刷新间隔');
  const filteredProviders = providers.filter(({ name, detail }) => matches(`供应商 ${name} ${detail} 账号与显示 使用统计 本机记录 服务端`));
  function navigate(next: Page) {
    setHistory((current) => current.pages[current.index] === next ? current : { pages: [...current.pages.slice(0, current.index + 1), next], index: current.index + 1 });
    if (next !== 'general') setExpanded(true);
  }
  function toggleProvider(id: ProviderId) {
    onChange({ enabledProviders: draft.enabledProviders.includes(id) ? draft.enabledProviders.filter((value) => value !== id) : providers.filter((value) => value.id === id || draft.enabledProviders.includes(value.id)).map((value) => value.id) });
  }
  function navButton(target: Page, label: string, icon: ReactNode, nested = false) {
    return <button type="button" className={`preferences-nav-item${page === target ? ' is-selected' : ''}${nested ? ' is-nested' : ''}`} aria-current={page === target ? 'page' : undefined} onClick={() => navigate(target)}>{icon}<span>{label}</span></button>;
  }

  return <div className="preferences-layout">
    <aside className="preferences-sidebar" aria-label="设置菜单">
      <div className="preferences-sidebar-titlebar" data-tauri-drag-region={desktop || undefined} aria-hidden="true" />
      <div className="preferences-sidebar-scroll"><div className="preferences-sidebar-content">
      <div className="preferences-brand"><Settings size={20} aria-hidden="true" /><div><strong>AgentBar</strong><span>偏好设置</span></div></div>
      <label className="preferences-search"><Search size={14} aria-hidden="true" /><input type="search" aria-label="搜索设置" placeholder="搜索设置" value={search} onChange={(event) => setSearch(event.target.value)} /></label>
      <nav aria-label="偏好设置导航">
        {showGeneral && navButton('general', '通用', <span className="preferences-nav-icon"><Settings size={16} /></span>)}
        {filteredProviders.length > 0 && <>
          <div className="preferences-nav-branch">
            {navButton('providers', '供应商', <span className="preferences-nav-icon suppliers"><Layers3 size={16} /></span>)}
            <button type="button" className="preferences-disclosure" aria-label={expanded ? '折叠供应商' : '展开供应商'} aria-expanded={expanded || !!query} aria-controls="provider-navigation" onClick={() => setExpanded(!expanded)}>{expanded || query ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</button>
          </div>
          {(expanded || query) && <ul id="provider-navigation" className="preferences-provider-tree">{filteredProviders.map(({ id, name }) => <li key={id}>
            {navButton(id, name, <ProviderIcon id={id} />, true)}
          </li>)}</ul>}
        </>}
        {!showGeneral && filteredProviders.length === 0 && <p className="preferences-no-results">没有匹配的设置</p>}
      </nav>
    </div></div></aside>
    <form className="preferences-detail" onSubmit={(event) => { event.preventDefault(); onSave(); }}>
      <header className="preferences-toolbar" data-tauri-drag-region={desktop || undefined}>
        <button type="button" aria-label="返回上一页" disabled={history.index === 0} onClick={() => setHistory((current) => ({ ...current, index: current.index - 1 }))}><ChevronLeft size={19} /></button>
        <button type="button" aria-label="前进一页" disabled={history.index === history.pages.length - 1} onClick={() => setHistory((current) => ({ ...current, index: current.index + 1 }))}><ChevronRight size={19} /></button>
        <span data-tauri-drag-region={desktop || undefined}>{provider ? `供应商 / ${provider.name}` : title}</span>
      </header>
      <div className="preferences-content" ref={content} role="region" aria-label="设置内容" tabIndex={0}><div className="preferences-content-inner">
        <div className="preferences-hero">
          <div className={`preferences-hero-icon${provider ? ' has-provider' : ''}`} aria-hidden="true">{provider ? <ProviderIcon id={provider.id} /> : page === 'general' ? <Settings size={39} strokeWidth={1.5} /> : <Layers3 size={36} strokeWidth={1.5} />}</div>
          <h1>{title}</h1>
          <p>{page === 'general' ? '管理 AgentBar 的整体设置和偏好。' : page === 'providers' ? '管理供应商的显示、账号和使用统计。' : `管理 ${provider?.name} 的显示、账号和使用统计。`}</p>
        </div>
        <fieldset className="preferences-fields" disabled={saving}>
          {page === 'general' && <>
            <Group title="外观"><div className="preferences-row"><span className="preferences-row-label"><Monitor size={17} />外观模式</span><span className="preferences-value">{themes.find(({ value }) => value === draft.theme)?.name}</span></div><div className="preferences-themes">{themes.map(({ value, name, icon: Icon }) => <label key={value} className={`preferences-theme${draft.theme === value ? ' is-selected' : ''}`}><input type="radio" name="theme" value={value} checked={draft.theme === value} onChange={() => onChange({ theme: value })} /><Icon size={22} strokeWidth={1.5} /><span>{name}</span></label>)}</div></Group>
            <Group title="自动刷新"><div className="preferences-row"><label className="preferences-row-label" htmlFor="refresh-interval"><Clock3 size={17} />刷新间隔</label><NativeSelect id="refresh-interval" value={draft.refreshIntervalSeconds} onChange={(event) => onChange({ refreshIntervalSeconds: Number(event.target.value) })}><option value={60}>1 分钟</option><option value={300}>5 分钟</option><option value={900}>15 分钟</option></NativeSelect></div></Group>
            <p className="preferences-note">按设定间隔自动更新已启用供应商的账号用量。</p>
          </>}
          {page === 'providers' && <Group title="已支持的供应商">{providers.map(({ id, name, detail }) => <button key={id} type="button" className="preferences-row preferences-link-row" onClick={() => navigate(id)}><ProviderIcon id={id} /><span className="preferences-row-copy"><strong>{name}</strong><small>{detail}</small></span><span className="preferences-value">{draft.enabledProviders.includes(id) ? '已启用' : '未启用'}</span><ChevronRight size={15} /></button>)}</Group>}
          {provider && <>
            <Group title="显示"><div className="preferences-row"><label htmlFor={`enable-${provider.id}`} className="preferences-row-copy"><strong>在用量面板中显示</strong><small>显示 {provider.name} 的账号用量和统计入口</small></label><input id={`enable-${provider.id}`} className="preferences-switch" type="checkbox" role="switch" checked={draft.enabledProviders.includes(provider.id)} onChange={() => toggleProvider(provider.id)} /></div></Group>
            <Group title="本机账号"><div className="preferences-row"><span>账号</span><span className="preferences-value preferences-account">{account?.account || '尚未读取到账号'}</span></div>{account?.plan && <div className="preferences-row"><span>订阅方案</span><span className="preferences-value">{account.plan}</span></div>}</Group>
            <p className="preferences-note">{desktop ? `自动读取本机已登录的 ${provider.name} 账号。` : '读取本机已登录的账号，请使用 AgentBar 桌面应用。'}</p>
            <Group title="使用统计"><div className="preferences-row"><label htmlFor={provider.id === 'codex' ? 'codex-statistics-source' : undefined}>统计来源</label>{provider.id === 'codex' ? <NativeSelect id="codex-statistics-source" value={draft.codexStatisticsSource} onChange={(event) => onChange({ codexStatisticsSource: event.target.value as CodexStatisticsPreference })}>{codexStatisticsSources.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}</NativeSelect> : <span className="preferences-value">本机记录</span>}</div></Group>
            <p className="preferences-note">本机记录提供日、周、月、年及全部统计。{provider.id === 'codex' ? '服务端查询账号汇总和每日 Token，由程序自动选择可用的连接方式。' : 'Claude 使用本机会话记录统计 Token 用量与约等金额。'}</p>
          </>}
        </fieldset>
      </div></div>
      <footer className="preferences-save">
        {error && <div className="error-notice" role="alert"><AlertCircle size={15} /><span>{error}</span></div>}
        <div className="preferences-save-row"><span role="status">{saved ? <><Check size={14} />已保存</> : isDirty ? '有未保存的修改' : '设置已同步'}</span><div><Button type="button" variant="ghost" size="sm" disabled={!isDirty || saving} onClick={onReset}>还原</Button><Button type="submit" size="sm" disabled={!isDirty || saving}>{saving && <LoaderCircle size={13} className="spin" />}{saving ? '保存中…' : '保存设置'}</Button></div></div>
      </footer>
    </form>
  </div>;
}
