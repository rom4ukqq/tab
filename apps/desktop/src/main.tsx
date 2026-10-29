import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { FluentProvider, webDarkTheme, webLightTheme, Button, Input, Switch, Field, Badge, Spinner, Dropdown, Option } from '@fluentui/react-components';
import './style.css';
import './bridge.css';
import './glass.css';
import { ThemeBackdrop, BackgroundTheme } from './ThemeBackdrop';
import { importTheme, listThemes, ThemeInfo } from './themeStore';
import { AgentIcon, agentNames } from './AgentIcon';
import { AccentPicker, accentColors, defaultAccent, validAccent } from './AccentPicker';
import { LanguageProvider, useLanguage, useT } from './i18n';

type Provider = 'codex' | 'claude' | 'omp' | 'mock';
type Agent = { provider: Provider; executable: string; version: string; enabled: boolean; accept_unsandboxed: boolean };
type Project = { id: string; name: string; root: string };
type Session = { id: string; project_id: string; provider: Provider; title: string; native_id?: string; model?: string };
type Asset = { id: string; name: string; path: string; mime: string; bytes: number; sha256: string };
type Turn = { id: string; session_id: string; state: string; text: string; created_at: number };
type Request = { id: string; turn_id: string; session_id: string; kind: string; data: Record<string, any>; local_only: boolean; expires_at: number };
type Event = { event_id: string; session_id: string; turn_id: string; sequence: number; kind: string; data: Record<string, any> };
type Snapshot = { config: { machine_name: string; installations: Agent[]; projects: Project[]; telegram_user?: number }; sessions: Session[]; turns: Turn[]; requests: Request[]; telegram_status: string; telegram_active: boolean; telegram_authorized: boolean; pairing_code?: { code: string; expires_at: number } };
const pages = ['Агенты', 'Telegram', 'Настройки'];
// Migrate preferences once while preserving imported themes in the same WebView store.
for (const suffix of ['theme', 'accent', 'matrix-motion', 'background']) {
  const legacy = `agent-hub-${suffix}`, current = `tab-${suffix}`;
  const value = localStorage.getItem(legacy);
  if (value !== null) {
    if (localStorage.getItem(current) === null) localStorage.setItem(current, value);
    localStorage.removeItem(legacy);
  }
}
const active = (t: Turn) => ['running', 'queued', 'dispatching'].includes(t.state);
const names: Record<string, string> = { running: 'В работе', queued: 'В очереди', completed: 'Завершено', cancelled: 'Отменено', interrupted: 'Прервано', failed: 'Ошибка', unknown: 'Результат неизвестен' };
function NavIcon({ index }: { index: number }) {
  return <svg className="nav-icon" width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{index === 0 ? <><rect x="4" y="7" width="16" height="13" rx="4"/><path d="M12 3v4M8 12v2m8-2v2M9 17h6"/></> : index === 1 ? <><path d="m3 10 18-7-5 18-5-7-8-4Z"/><path d="m11 14 10-11"/></> : <><path d="M4 6h16M4 12h16M4 18h16"/><circle cx="9" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="8" cy="18" r="2"/></>}</svg>;
}
function App() {
  const t = useT();
  const { language, setLanguage } = useLanguage();
  useEffect(() => { invoke('set_ui_language', { language }).catch(e => setError(String(e))); }, [language]);
  const [page, setPage] = useState('Агенты'), [snap, setSnap] = useState<Snapshot>();
  const [error, setError] = useState(''), [notice, setNotice] = useState(''), [busy, setBusy] = useState(false);
  useEffect(() => { if (!notice) return; const timer = setTimeout(() => setNotice(''), 4000); return () => clearTimeout(timer); }, [notice]);
  const [dark, setDark] = useState(() => { const saved = localStorage.getItem('tab-theme'); return saved ? saved === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches; });
  useEffect(() => { localStorage.setItem('tab-theme', dark ? 'dark' : 'light'); }, [dark]);
  const [accent, setAccent] = useState(() => { const saved = localStorage.getItem('tab-accent'); return validAccent(saved) ? saved : defaultAccent; });
  useEffect(() => { localStorage.setItem('tab-accent', accent); }, [accent]);
  useEffect(() => { invoke('set_window_theme', { dark }).catch(e => setError(String(e))); }, [dark]);
  const [animateBackground, setAnimateBackground] = useState(() => localStorage.getItem('tab-matrix-motion') !== 'off');
  const [background, setBackground] = useState<BackgroundTheme>('matrix');
  const [importedThemes, setImportedThemes] = useState<ThemeInfo[]>([]), [importingTheme, setImportingTheme] = useState(false);
  const themeFile = useRef<HTMLInputElement>(null);
  const savedBackground = useRef(localStorage.getItem('tab-background'));
  const backgroundThemes = [{ id: 'matrix', name: t('Матрица') }, ...importedThemes];
  useEffect(() => {
    let disposed = false;
    listThemes().then(themes => { if (!disposed) { setImportedThemes(themes); if (themes.some(theme => theme.id === savedBackground.current)) setBackground(savedBackground.current!); } }).catch(e => { if (!disposed) setError(String(e)); });
    return () => { disposed = true; };
  }, []);
  const addTheme = async (file: File) => {
    setImportingTheme(true); setError('');
    try { const theme = await importTheme(file); setImportedThemes(await listThemes()); setBackground(theme.id); setNotice(t('Тема импортирована.')); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setImportingTheme(false); }
  };
  const [developer, setDeveloper] = useState(false);
  const [trayIcon, setTrayIcon] = useState<'star' | 'face'>(() => localStorage.getItem('tab-tray-icon') === 'face' ? 'face' : 'star');
  useEffect(() => { invoke('set_tray_icon', { kind: trayIcon }).then(() => localStorage.setItem('tab-tray-icon', trayIcon)).catch(e => setError(String(e))); }, [trayIcon]);
  useEffect(() => {
    let count = 0;
    const unlock = (e: KeyboardEvent) => {
      if (e.repeat || e.ctrlKey || e.altKey || e.metaKey) return;
      if (e.key !== 'Tab') { count = 0; return; }
      if (++count === 10) { setDeveloper(true); setPage('Настройки'); count = 0; }
    };
    window.addEventListener('keydown', unlock);
    return () => window.removeEventListener('keydown', unlock);
  }, []);
  useEffect(() => { localStorage.setItem('tab-background', background); }, [background]);
  useEffect(() => { localStorage.setItem('tab-matrix-motion', animateBackground ? 'on' : 'off'); }, [animateBackground]);
  const [token, setToken] = useState(''), [user, setUser] = useState('');
  const numericUser = user.trim();
  const userIdError = !numericUser ? t("Введите ваш числовой Telegram ID.") :
    !/^\d+$/.test(numericUser) || !Number.isSafeInteger(Number(numericUser)) || Number(numericUser) <= 0 ?
      t("Нужен числовой ID, например 123456789. Имя пользователя или @username не подходит.") : '';
  const telegramFormError = !token.trim() ? t("Введите bot token из BotFather.") : userIdError;
  const [audit, setAudit] = useState<any[]>([]), [autostart, setAutostart] = useState(false);
  const refresh = useCallback(async () => setSnap(await invoke<Snapshot>('snapshot')), []);
  const run = async (fn: () => Promise<unknown>) => {
    setError(''); setNotice(''); setBusy(true);
    try { await fn(); await refresh(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const command = (name: string, args?: Record<string, unknown>) => run(() => invoke(name, args));
  useEffect(() => {
    refresh().catch(e => setError(String(e)));
    invoke<boolean>('get_autostart').then(setAutostart).catch(() => {});
    let disposed = false; const clean: (() => void)[] = [];
    for (const event of ['hub-event', 'hub-resync']) listen(event, () => refresh().catch(() => {})).then(fn => { if (disposed) fn(); else clean.push(fn); });
    const timer = setInterval(() => refresh().catch(() => {}), 2000);
    return () => { disposed = true; clean.forEach(f => f()); clearInterval(timer); };
  }, [refresh]);
  const accentPalette = accentColors(accent, dark);
  const telegramConnected = !!snap?.telegram_active && (!!snap.pairing_code || snap.telegram_authorized);
  const theme = { ...(dark ? webDarkTheme : webLightTheme), colorBrandBackground: accent, colorBrandBackgroundHover: accentPalette.hover, colorBrandBackgroundPressed: accentPalette.pressed, colorNeutralForegroundOnBrand: accentPalette.text, colorBrandForeground1: accentPalette.foreground, colorBrandStroke1: accent, colorCompoundBrandBackground: accent, colorCompoundBrandBackgroundHover: accentPalette.hover, colorCompoundBrandBackgroundPressed: accentPalette.pressed, colorCompoundBrandStroke: accent, colorCompoundBrandStrokeHover: accentPalette.hover, colorCompoundBrandStrokePressed: accentPalette.pressed, borderRadiusMedium: '8px' };
  return <FluentProvider theme={theme}><div className={dark ? 'shell theme-dark' : 'shell theme-light'} style={{ '--hub-accent': accent, '--hub-accent-readable': accentPalette.foreground } as React.CSSProperties}>
    <ThemeBackdrop selected={background} animate={animateBackground} onError={setError}/>
    <aside>
      <div className="brand" title="Telegram Agents Bridge"><span className="brand-mark" aria-hidden="true"/><div>TAB</div></div>
      <nav aria-label={t("Основная навигация")}>{pages.map((p, i) => <button aria-current={p === page ? 'page' : undefined} className={p === page ? 'nav active' : 'nav'} key={p} onClick={() => setPage(p)}><NavIcon index={i}/>{t(p)}</button>)}</nav>
      <div className="sidebar-footer"><span className="dot"/>{snap?.config.machine_name ?? t("Этот компьютер")}</div>
    </aside>
    <main>
      <header><div><div className="eyebrow">TAB / {t(page)}</div><h1>{t(page)}</h1></div></header>
      {error && <div className="banner error" role="alert">{t(error)}<Button appearance="subtle" onClick={() => setError('')}>{t("Закрыть")}</Button></div>}
      {notice && <div className="app-toast" role="status" aria-live="polite"><span>{t(notice)}</span><Button appearance="subtle" size="small" onClick={() => setNotice('')}>{t('Закрыть')}</Button></div>}
      {!snap ? <Spinner label={t("Открываем локальную базу…")}/> : <>
        <Requests requests={snap.requests} act={command}/>
        {page === 'Агенты' && <>
          <div className="intro"><h2>{t("Подключите ваши CLI-агенты")}</h2><p>{t("Отметьте агентов, к которым будет доступ через Telegram.")}</p></div>
          <div className="section-heading"><span>{t("Агенты на этом компьютере")}</span><Button appearance="primary" disabled={busy} onClick={() => command('discover')}>{t("Найти агенты")}</Button></div>
          {!snap.config.installations.length && <Empty title={t("Агенты ещё не найдены")} text={t("Установите и авторизуйте Codex, Claude Code или OMP, затем нажмите «Найти агенты».")}/>}
          {snap.config.installations.map(a => <AgentCard key={a.provider} agent={a} busy={busy} act={command}/>)}
        </>}
        {page === 'Telegram' && <>
          <div className="card">
            <div className="row"><h2 className="grow">{t("Подключение бота")}</h2><Button appearance={telegramConnected ? 'secondary' : 'primary'} disabled={busy || !snap.config.telegram_user || snap.telegram_status === 'Отключается'} onClick={() => command(telegramConnected ? 'disconnect_telegram' : 'connect_telegram')}>{snap.telegram_status === 'Отключается' ? t("Отключение…") : telegramConnected ? t("Отключить") : t("Подключить")}</Button></div>
            {snap.telegram_active && snap.pairing_code && <div className="pairing-code">
              <span>{t('Отправьте этот PIN боту в Telegram')}</span><strong>{snap.pairing_code.code}</strong><span className="muted">{t('Одноразовый код · до')}{' '}{new Date(snap.pairing_code.expires_at * 1000).toLocaleTimeString(language)}</span>
              <Button appearance="subtle" disabled={busy || snap.telegram_status === 'Отключается'} onClick={() => command('refresh_pairing_code')}>{t('Новый PIN')}</Button>
            </div>}
            <p>{t("Используйте отдельного бота из BotFather. Одновременно с TAB у него не должно быть другого Telegram-моста.")}</p>
            <div className="form-grid">
              <Field label="Bot token"><Input type="password" autoComplete="off" value={token} onChange={(_, d) => setToken(d.value)}/></Field>
              <Field label={t("Ваш числовой Telegram ID")} hint={t("Узнать свой ID: откройте @userinfobot в Telegram и нажмите Start. Скопируйте число из строки Id.")} validationState={user && userIdError ? 'error' : 'none'} validationMessage={user ? userIdError : undefined}><Input inputMode="numeric" value={user} placeholder={t("Например, 123456789")} onChange={(_, d) => setUser(d.value)}/></Field>
            </div>
            <Button appearance="primary" disabled={busy || !!telegramFormError} onClick={() => run(async () => {
              if (telegramFormError) throw new Error(telegramFormError);
              await invoke('configure_telegram', { token: token.trim(), userId: Number(numericUser) }); setToken(''); setNotice(t("Настройки сохранены. Теперь подключите бота."));
            })}>{t("Сохранить подключение")}</Button>
            {!busy && telegramFormError && <p role="status">{t("Чтобы сохранить:")}{' '}{telegramFormError}</p>}
            <p className="muted">{t("Bot token хранится в Windows Credential Manager. Принимаются сообщения только от указанного User ID в личном чате.")}</p>
          </div>
          <p className="muted">{t("Telegram cloud chat не использует E2EE. PIN удаляется из чата по возможности; это дополнительный барьер доступа.")}</p>
        </>}
        {page === 'Настройки' && <>
          <div className="card"><h2>{t("Приложение")}</h2>
            <Field label={t('Язык')}><Dropdown aria-label="Language" className="theme-dropdown" listbox={{ className: 'theme-listbox' }} value={language === 'en' ? 'English' : 'Русский'} selectedOptions={[language]} onOptionSelect={(_,d) => { if(d.optionValue === 'en' || d.optionValue === 'ru') setLanguage(d.optionValue); }}><Option value="en">English</Option><Option value="ru">Русский</Option></Dropdown></Field>
            <div className="setting"><Switch checked={autostart} onChange={(_, d) => run(async () => { await invoke('set_autostart', { enabled: d.checked }); setAutostart(d.checked); })} label={t("Запускать в трее при входе в Windows")}/></div>
            <div className="setting"><Switch checked={dark} onChange={(_, d) => setDark(d.checked)} label={t("Тёмная тема")}/></div>
            <AccentPicker value={accent} onChange={setAccent}/>
            <Field label={t("Фон")}><Dropdown aria-label={t("Фон")} className="theme-dropdown" listbox={{ className: 'theme-listbox' }} value={backgroundThemes.find(item => item.id === background)?.name ?? t('Матрица')} selectedOptions={[background]} onOptionSelect={(_, d) => { if (backgroundThemes.some(item => item.id === d.optionValue)) setBackground(d.optionValue!); }}>{backgroundThemes.map(item => <Option key={item.id} value={item.id}>{item.name}</Option>)}</Dropdown></Field>
            <div className="theme-import-row"><Button appearance="subtle" className="theme-import-button" icon={<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5"/></svg>} disabled={importingTheme} onClick={() => themeFile.current?.click()}>{t(importingTheme ? 'Импорт…' : 'Импорт темы')}</Button><input ref={themeFile} type="file" accept=".zip,.tab-theme.zip" hidden onChange={e => { const file = e.currentTarget.files?.[0]; e.currentTarget.value = ''; if (file && !importingTheme) void addTheme(file); }}/><span className="muted">{t('ZIP из TAB Theme Generator')}</span></div>
            <div className="setting"><Switch checked={animateBackground} onChange={(_, d) => setAnimateBackground(d.checked)} label={t("Анимация фона")}/></div>
            <p className="muted">{t("Закрытие окна скрывает его в трей. Команда «Выйти» завершает TAB и его задачи.")}</p>
          </div>
          {developer && <div className="card"><h2>Developer Settings</h2>
            <Field label={t("Иконка трея")}><Dropdown aria-label={t("Иконка трея")} className="theme-dropdown" listbox={{ className: 'theme-listbox' }} value={trayIcon === 'star' ? '*' : ':3'} selectedOptions={[trayIcon]} onOptionSelect={(_, d) => { if (d.optionValue === 'star' || d.optionValue === 'face') setTrayIcon(d.optionValue); }}><Option value="star">*</Option><Option value="face">:3</Option></Dropdown></Field>
            <div className="theme-import-row"><Button appearance="subtle" className="theme-import-button" onClick={() => setDeveloper(false)}>{t("Скрыть Developer Settings")}</Button></div>
          </div>}
          <details className="card"><summary>{t("Безопасность и журнал")}</summary><p>{t("Отзыв доступа останавливает remote-задачи и требует заново ввести PIN.")}</p>
            <div className="row"><Button appearance="subtle" className="theme-import-button" onClick={() => command('revoke_access')}>{t("Отозвать доступ")}</Button><Button appearance="subtle" className="theme-import-button" onClick={() => run(async () => setAudit(await invoke('audit')))}>{t("Показать аудит")}</Button></div>
            {audit.map((a, i) => <div className="audit" key={i}><time>{new Date(a.at * 1000).toLocaleString(language)}</time><strong>{a.action}</strong><code>{a.target}</code></div>)}
          </details>
          <details className="card"><summary>{t("О сборке")}</summary><p>{t("TAB — Telegram Agents Bridge 0.1.0 · первая сборка MVP для одного ПК. Реальные CLI и Telegram требуют приёмки после локальной настройки.")}</p><p>{t("Данные: %LOCALAPPDATA%\\TAB. Multi-PC, голос, Computer Use и подписанные обновления пока не включены.")}</p></details>
        </>}
      </>}
    </main>
  </div></FluentProvider>;
}
function Empty({ title, text }: { title: string; text: string }) { const t = useT(); return <div className="empty"><h3>{t(title)}</h3><p>{t(text)}</p></div>; }
function AgentCard({ agent: a, busy, act }: { agent: Agent; busy: boolean; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) {
  const t = useT();
  const [accept, setAccept] = useState(a.accept_unsandboxed);
  const name = t(agentNames[a.provider] ?? a.provider);
  const needsConsent = !a.enabled && a.provider !== 'codex' && !accept;
  return <div className="card">
    <div className="row">
      <div className="agent-heading grow"><AgentIcon provider={a.provider}/><h2>{name}</h2></div>
      <Switch checked={a.enabled} disabled={busy || needsConsent} aria-label={`${t('Доступ к')} ${name}`} title={needsConsent ? t("Сначала разрешите запуск без OS sandbox ниже") : `${t('Доступ к')} ${name}`} onChange={(_, d) => act('enable_agent', { provider: a.provider, enabled: d.checked, acceptUnsandboxed: accept })}/>
    </div>
    <p>{a.version}</p><code>{a.executable}</code>
    <p>{a.provider === 'codex' ? 'App-server · workspace-write · approvals on-request · steering' : a.provider === 'claude' ? t("Stream JSON · permissions manual · сообщения в очередь") : t("RPC UI v1 · steering · native dialogs подтверждаются на ПК")}</p>
    {a.provider !== 'codex' && <Switch checked={accept} disabled={busy} onChange={(_, d) => setAccept(d.checked)} label={t("Разрешаю запуск без подтверждённой OS sandbox")}/>}
    <div className="row"><small>{t("Runtime qualification: ещё не выполнена")}</small></div>
  </div>;
}
function Requests({ requests, act }: { requests: Request[]; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) { return <>{requests.map(r => <RequestCard key={r.id} r={r} act={act}/>)}</>; }
function RequestCard({ r, act }: { r: Request; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) {
  const t = useT();
  const { language } = useLanguage();
  const questions: any[] = r.data.params?.questions ?? r.data.input?.questions ?? []; const [answers, setAnswers] = useState<Record<string, string>>({}), [value, setValue] = useState('');
  const payload = () => { if (r.kind === 'native_dialog') return value; const result: Record<string, unknown> = {}; questions.forEach(q => { const key = q.id ?? q.question; const v = answers[key] ?? ''; result[key] = r.data.method ? { answers: [v] } : v; }); return result; };
  return <div className="card request"><div className="row"><h3 className="grow">{r.kind === 'question' ? t("Вопрос агента") : r.kind === 'native_dialog' ? t("Native dialog · локально") : t("Требуется решение")}</h3><Badge color="warning">{t("До")}{' '}{new Date(r.expires_at * 1000).toLocaleTimeString(language)}</Badge></div><pre>{JSON.stringify(r.data, null, 2)}</pre>{questions.map((q, i) => { const key = q.id ?? q.question; return <Field key={i} label={q.question ?? q.header ?? q.id}><Input value={answers[key] ?? ''} onChange={(_, d) => setAnswers(a => ({ ...a, [key]: d.value }))}/>{q.options?.length > 0 && <div className="row">{q.options.map((o: any, j: number) => <Button key={j} size="small" onClick={() => setAnswers(a => ({ ...a, [key]: o.label ?? o }))}>{o.label ?? String(o)}</Button>)}</div>}</Field>; })}{r.kind === 'native_dialog' && <Field label={t("Значение ответа / пункт native select")}><Input value={value} onChange={(_, d) => setValue(d.value)}/></Field>}<div className="row"><Button appearance="primary" disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'allow', answers: payload() })}>{r.kind === 'question' ? t("Ответить") : t("Разрешить один раз")}</Button><Button disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'deny', answers: {} })}>{t("Отказать")}</Button><Button onClick={() => act('cancel_turn', { turnId: r.turn_id })}>{t("Остановить задачу")}</Button></div></div>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><LanguageProvider><App/></LanguageProvider></React.StrictMode>);
