import React, { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { FluentProvider, webDarkTheme, webLightTheme, Button, Input, Switch, Field, Badge, Spinner } from '@fluentui/react-components';
import './style.css';
import './bridge.css';
import './glass.css';
import { ThemeBackdrop, backgroundThemes, validBackground, BackgroundTheme } from './ThemeBackdrop';
import { AgentIcon, agentNames } from './AgentIcon';
import { AccentPicker, accentColors, defaultAccent, validAccent } from './AccentPicker';

type Provider = 'codex' | 'claude' | 'omp' | 'mock';
type Agent = { provider: Provider; executable: string; version: string; enabled: boolean; accept_unsandboxed: boolean };
type Project = { id: string; name: string; root: string };
type Session = { id: string; project_id: string; provider: Provider; title: string; native_id?: string; model?: string };
type Asset = { id: string; name: string; path: string; mime: string; bytes: number; sha256: string };
type Turn = { id: string; session_id: string; state: string; text: string; created_at: number };
type Request = { id: string; turn_id: string; session_id: string; kind: string; data: Record<string, any>; local_only: boolean; expires_at: number };
type Event = { event_id: string; session_id: string; turn_id: string; sequence: number; kind: string; data: Record<string, any> };
type Snapshot = { config: { machine_name: string; installations: Agent[]; projects: Project[]; telegram_user?: number; pin_hash?: string }; sessions: Session[]; turns: Turn[]; requests: Request[]; telegram_status: string };
const pages = ['Агенты', 'Telegram', 'Настройки'];
const active = (t: Turn) => ['running', 'queued', 'dispatching'].includes(t.state);
const names: Record<string, string> = { running: 'В работе', queued: 'В очереди', completed: 'Завершено', cancelled: 'Отменено', interrupted: 'Прервано', failed: 'Ошибка', unknown: 'Результат неизвестен' };
function NavIcon({ index }: { index: number }) {
  return <svg className="nav-icon" width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{index === 0 ? <><rect x="4" y="7" width="16" height="13" rx="4"/><path d="M12 3v4M8 12v2m8-2v2M9 17h6"/></> : index === 1 ? <><path d="m3 10 18-7-5 18-5-7-8-4Z"/><path d="m11 14 10-11"/></> : <><path d="M4 6h16M4 12h16M4 18h16"/><circle cx="9" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="8" cy="18" r="2"/></>}</svg>;
}
function App() {
  const [page, setPage] = useState('Агенты'), [snap, setSnap] = useState<Snapshot>();
  const [error, setError] = useState(''), [notice, setNotice] = useState(''), [busy, setBusy] = useState(false);
  const [dark, setDark] = useState(() => { const saved = localStorage.getItem('agent-hub-theme'); return saved ? saved === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches; });
  useEffect(() => { localStorage.setItem('agent-hub-theme', dark ? 'dark' : 'light'); }, [dark]);
  const [accent, setAccent] = useState(() => { const saved = localStorage.getItem('agent-hub-accent'); return validAccent(saved) ? saved : defaultAccent; });
  useEffect(() => { localStorage.setItem('agent-hub-accent', accent); }, [accent]);
  useEffect(() => { invoke('set_window_theme', { dark }).catch(e => setError(String(e))); }, [dark]);
  const [animateBackground, setAnimateBackground] = useState(() => localStorage.getItem('agent-hub-matrix-motion') !== 'off');
  const [background, setBackground] = useState<BackgroundTheme>(() => { const saved = localStorage.getItem('agent-hub-background'); return validBackground(saved) ? saved : 'matrix'; });
  useEffect(() => { localStorage.setItem('agent-hub-background', background); }, [background]);
  useEffect(() => { localStorage.setItem('agent-hub-matrix-motion', animateBackground ? 'on' : 'off'); }, [animateBackground]);
  const [token, setToken] = useState(''), [user, setUser] = useState(''), [pin, setPin] = useState('');
  const numericUser = user.trim();
  const userIdError = !numericUser ? 'Введите ваш числовой Telegram ID.' :
    !/^\d+$/.test(numericUser) || !Number.isSafeInteger(Number(numericUser)) || Number(numericUser) <= 0 ?
      'Нужен числовой ID, например 123456789. Имя пользователя или @username не подходит.' : '';
  const pinError = !/^\d{4,6}$/.test(pin) ? 'PIN должен содержать от 4 до 6 цифр.' : '';
  const telegramFormError = !token.trim() ? 'Введите bot token из BotFather.' : userIdError || pinError;
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
  const theme = { ...(dark ? webDarkTheme : webLightTheme), colorBrandBackground: accent, colorBrandBackgroundHover: accentPalette.hover, colorBrandBackgroundPressed: accentPalette.pressed, colorNeutralForegroundOnBrand: accentPalette.text, colorBrandForeground1: accentPalette.foreground, colorBrandStroke1: accent, colorCompoundBrandBackground: accent, colorCompoundBrandBackgroundHover: accentPalette.hover, colorCompoundBrandBackgroundPressed: accentPalette.pressed, colorCompoundBrandStroke: accent, colorCompoundBrandStrokeHover: accentPalette.hover, colorCompoundBrandStrokePressed: accentPalette.pressed, borderRadiusMedium: '8px' };
  return <FluentProvider theme={theme}><div className={dark ? 'shell theme-dark' : 'shell theme-light'} style={{ '--hub-accent': accent, '--hub-accent-readable': accentPalette.foreground } as React.CSSProperties}>
    <ThemeBackdrop selected={background} animate={animateBackground} onError={setError}/>
    <aside>
      <div className="brand" title="Telegram Agents Bridge"><span className="brand-mark" aria-hidden="true"/><div>TAB</div></div>
      <nav aria-label="Основная навигация">{pages.map((p, i) => <button aria-current={p === page ? 'page' : undefined} className={p === page ? 'nav active' : 'nav'} key={p} onClick={() => setPage(p)}><NavIcon index={i}/>{p}</button>)}</nav>
      <div className="sidebar-footer"><span className="dot"/>{snap?.config.machine_name ?? 'Этот компьютер'}<small>{snap?.telegram_status ?? 'Запуск…'}</small><small>Активных задач: {snap?.turns.filter(active).length ?? 0}</small></div>
    </aside>
    <main>
      <header><div><div className="eyebrow">TAB / {page}</div><h1>{page}</h1></div></header>
      {error && <div className="banner error" role="alert">{error}<Button appearance="subtle" onClick={() => setError('')}>Закрыть</Button></div>}
      {notice && <div className="banner" role="status">{notice}</div>}
      {!snap ? <Spinner label="Открываем локальную базу…"/> : <>
        <Requests requests={snap.requests} act={command}/>
        {page === 'Агенты' && <>
          <div className="intro"><h2>Подключите ваши CLI-агенты</h2><p>Отметьте агентов, к которым будет доступ через Telegram.</p></div>
          <div className="section-heading"><span>Агенты на этом компьютере</span><Button appearance="primary" disabled={busy} onClick={() => command('discover')}>Найти агенты</Button></div>
          {!snap.config.installations.length && <Empty title="Агенты ещё не найдены" text="Установите и авторизуйте Codex, Claude Code или OMP, затем нажмите «Найти агенты»."/>}
          {snap.config.installations.map(a => <AgentCard key={a.provider} agent={a} busy={busy} act={command}/>)}
        </>}
        {page === 'Telegram' && <>
          <div className="card">
            <div className="row"><h2 className="grow">Подключение бота</h2><Badge appearance="tint">{snap.telegram_status}</Badge></div>
            <p>Используйте отдельного бота из BotFather. Одновременно с TAB у него не должно быть другого Telegram-моста.</p>
            <div className="form-grid">
              <Field label="Bot token"><Input type="password" autoComplete="off" value={token} onChange={(_, d) => setToken(d.value)}/></Field>
              <Field label="Ваш числовой Telegram ID" hint="Узнать свой ID: откройте @userinfobot в Telegram и нажмите Start. Скопируйте число из строки Id." validationState={user && userIdError ? 'error' : 'none'} validationMessage={user ? userIdError : undefined}><Input inputMode="numeric" value={user} placeholder="Например, 123456789" onChange={(_, d) => setUser(d.value)}/></Field>
              <Field label="PIN · рекомендуется 6 цифр" validationState={pin && pinError ? 'error' : 'none'} validationMessage={pin ? pinError : undefined}><Input type="password" inputMode="numeric" autoComplete="new-password" maxLength={6} value={pin} onChange={(_, d) => setPin(d.value)}/></Field>
            </div>
            <Button appearance="primary" disabled={busy || !!telegramFormError} onClick={() => run(async () => {
              if (telegramFormError) throw new Error(telegramFormError);
              await invoke('configure_telegram', { token: token.trim(), userId: Number(numericUser), pin }); setToken(''); setPin(''); setNotice('Настройки сохранены. Теперь подключите бота.');
            })}>Сохранить подключение</Button>
            {!busy && telegramFormError && <p role="status">Чтобы сохранить: {telegramFormError}</p>}
            <p className="muted">Bot token хранится в Windows Credential Manager. Принимаются сообщения только от указанного User ID в личном чате.</p>
          </div>
          <div className="card">
            <h2>Управление с телефона</h2>
            <div className="row"><Button appearance="primary" disabled={busy || !snap.config.pin_hash} onClick={() => command('connect_telegram')}>Подключить</Button><Button onClick={() => command('disconnect_telegram')}>Отключить</Button><Button onClick={() => command('revoke_access')}>Закрыть доступ</Button></div>
            <p>Откройте бота: <strong>/start → PIN → агент → задача</strong>. Проект и нужные папки обсудите с агентом прямо в чате.</p>
            <p className="muted">Бот автоматически подключается при запуске TAB, без сообщения в личку. «Отключить» выключает автоподключение до следующего нажатия «Подключить».</p>
            <p className="muted">Доступ закрывается после 15 минут бездействия, через 8 часов или при перезапуске Hub. Подключение бота сохраняется, PIN-доступ нужно открыть заново.</p>
          </div>
          <p className="muted">Telegram cloud chat не использует E2EE. PIN удаляется из чата по возможности; это дополнительный барьер доступа.</p>
        </>}
        {page === 'Настройки' && <>
          <div className="card"><h2>Приложение</h2>
            <div className="setting"><Switch checked={autostart} onChange={(_, d) => run(async () => { await invoke('set_autostart', { enabled: d.checked }); setAutostart(d.checked); })} label="Запускать в трее при входе в Windows"/></div>
            <div className="setting"><Switch checked={dark} onChange={(_, d) => setDark(d.checked)} label="Тёмная тема"/></div>
            <AccentPicker value={accent} onChange={setAccent}/>
            <Field label="Фон"><select value={background} onChange={e => { const value = e.target.value; if (validBackground(value)) setBackground(value); }}>{backgroundThemes.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select></Field>
            <div className="setting"><Switch checked={animateBackground} onChange={(_, d) => setAnimateBackground(d.checked)} label="Анимация фона"/></div>
            <p className="muted">Закрытие окна скрывает его в трей. Команда «Выйти» завершает TAB и его задачи.</p>
          </div>
          <details className="card"><summary>Безопасность и журнал</summary><p>Отзыв доступа останавливает remote-задачи и требует заново ввести PIN.</p>
            <div className="row"><Button onClick={() => command('revoke_access')}>Отозвать доступ</Button><Button onClick={() => run(async () => setAudit(await invoke('audit')))}>Показать аудит</Button></div>
            {audit.map((a, i) => <div className="audit" key={i}><time>{new Date(a.at * 1000).toLocaleString()}</time><strong>{a.action}</strong><code>{a.target}</code></div>)}
          </details>
          <details className="card"><summary>О сборке</summary><p>TAB — Telegram Agents Bridge 0.1.0 · первая сборка MVP для одного ПК. Реальные CLI и Telegram требуют приёмки после локальной настройки.</p><p>Данные: %LOCALAPPDATA%\AgentHub (сохранены при переименовании). Multi-PC, голос, Computer Use и подписанные обновления пока не включены.</p></details>
        </>}
      </>}
    </main>
  </div></FluentProvider>;
}
function Empty({ title, text }: { title: string; text: string }) { return <div className="empty"><h3>{title}</h3><p>{text}</p></div>; }
function AgentCard({ agent: a, busy, act }: { agent: Agent; busy: boolean; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) {
  const [accept, setAccept] = useState(a.accept_unsandboxed);
  const name = agentNames[a.provider] ?? a.provider;
  const needsConsent = !a.enabled && a.provider !== 'codex' && !accept;
  return <div className="card">
    <div className="row">
      <div className="agent-heading grow"><AgentIcon provider={a.provider}/><h2>{name}</h2></div>
      <Switch checked={a.enabled} disabled={busy || needsConsent} aria-label={`Доступ к ${name}`} title={needsConsent ? 'Сначала разрешите запуск без OS sandbox ниже' : `Доступ к ${name}`} onChange={(_, d) => act('enable_agent', { provider: a.provider, enabled: d.checked, acceptUnsandboxed: accept })}/>
    </div>
    <p>{a.version}</p><code>{a.executable}</code>
    <p>{a.provider === 'codex' ? 'App-server · workspace-write · approvals on-request · steering' : a.provider === 'claude' ? 'Stream JSON · permissions manual · сообщения в очередь' : 'RPC UI v1 · steering · native dialogs подтверждаются на ПК'}</p>
    {a.provider !== 'codex' && <Switch checked={accept} disabled={busy} onChange={(_, d) => setAccept(d.checked)} label="Разрешаю запуск без подтверждённой OS sandbox"/>}
    <div className="row"><small>Runtime qualification: ещё не выполнена</small></div>
  </div>;
}
function Requests({ requests, act }: { requests: Request[]; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) { return <>{requests.map(r => <RequestCard key={r.id} r={r} act={act}/>)}</>; }
function RequestCard({ r, act }: { r: Request; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) {
  const questions: any[] = r.data.params?.questions ?? r.data.input?.questions ?? []; const [answers, setAnswers] = useState<Record<string, string>>({}), [value, setValue] = useState('');
  const payload = () => { if (r.kind === 'native_dialog') return value; const result: Record<string, unknown> = {}; questions.forEach(q => { const key = q.id ?? q.question; const v = answers[key] ?? ''; result[key] = r.data.method ? { answers: [v] } : v; }); return result; };
  return <div className="card request"><div className="row"><h3 className="grow">{r.kind === 'question' ? 'Вопрос агента' : r.kind === 'native_dialog' ? 'Native dialog · локально' : 'Требуется решение'}</h3><Badge color="warning">До {new Date(r.expires_at * 1000).toLocaleTimeString()}</Badge></div><pre>{JSON.stringify(r.data, null, 2)}</pre>{questions.map((q, i) => { const key = q.id ?? q.question; return <Field key={i} label={q.question ?? q.header ?? q.id}><Input value={answers[key] ?? ''} onChange={(_, d) => setAnswers(a => ({ ...a, [key]: d.value }))}/>{q.options?.length > 0 && <div className="row">{q.options.map((o: any, j: number) => <Button key={j} size="small" onClick={() => setAnswers(a => ({ ...a, [key]: o.label ?? o }))}>{o.label ?? String(o)}</Button>)}</div>}</Field>; })}{r.kind === 'native_dialog' && <Field label="Значение ответа / пункт native select"><Input value={value} onChange={(_, d) => setValue(d.value)}/></Field>}<div className="row"><Button appearance="primary" disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'allow', answers: payload() })}>{r.kind === 'question' ? 'Ответить' : 'Разрешить один раз'}</Button><Button disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'deny', answers: {} })}>Отказать</Button><Button onClick={() => act('cancel_turn', { turnId: r.turn_id })}>Остановить задачу</Button></div></div>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App/></React.StrictMode>);
