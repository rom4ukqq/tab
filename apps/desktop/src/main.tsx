import React, { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { FluentProvider, webDarkTheme, webLightTheme, Button, Input, Switch, Field, Badge, Spinner } from '@fluentui/react-components';
import './style.css';
import './bridge.css';

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
function App() {
  const [page, setPage] = useState('Агенты'), [snap, setSnap] = useState<Snapshot>();
  const [error, setError] = useState(''), [notice, setNotice] = useState(''), [busy, setBusy] = useState(false);
  const [dark, setDark] = useState(matchMedia('(prefers-color-scheme: dark)').matches);
  const [projectName, setProjectName] = useState(''), [root, setRoot] = useState('');
  const [token, setToken] = useState(''), [user, setUser] = useState(''), [pin, setPin] = useState('');
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
  return <FluentProvider theme={dark ? webDarkTheme : webLightTheme}><div className="shell">
    <aside>
      <div className="brand"><span className="brand-mark">A</span><div>Agent Hub<small>TELEGRAM BRIDGE</small></div></div>
      <nav aria-label="Основная навигация">{pages.map((p, i) => <button className={p === page ? 'nav active' : 'nav'} key={p} onClick={() => setPage(p)}><span className="nav-number">0{i + 1}</span>{p}</button>)}</nav>
      <div className="sidebar-footer"><span className="dot"/>{snap?.config.machine_name ?? 'Этот компьютер'}<small>{snap?.telegram_status ?? 'Запуск…'}</small><small>Активных задач: {snap?.turns.filter(active).length ?? 0}</small></div>
    </aside>
    <main>
      <header><div><div className="eyebrow">УПРАВЛЯЙТЕ АГЕНТАМИ ИЗ TELEGRAM</div><h1>{page}</h1></div><Button className="stop" onClick={() => command('emergency_stop')}>Остановить всё</Button></header>
      {error && <div className="banner error" role="alert">{error}<Button appearance="subtle" onClick={() => setError('')}>Закрыть</Button></div>}
      {notice && <div className="banner" role="status">{notice}</div>}
      {!snap ? <Spinner label="Открываем локальную базу…"/> : <>
        <Requests requests={snap.requests} act={command}/>
        {page === 'Агенты' && <>
          <div className="intro"><Badge appearance="tint" color="informative">Настройте один раз</Badge><h2>Подключите ваши CLI-агенты.</h2><p>Задачи, сессии и файлы — в Telegram. Здесь достаточно выбрать агенты и разрешить их запуск.</p></div>
          <div className="section-heading"><span>Агенты на этом компьютере</span><Button appearance="primary" disabled={busy} onClick={() => command('discover')}>Найти агенты</Button></div>
          {!snap.config.installations.length && <Empty title="Агенты ещё не найдены" text="Установите и авторизуйте Codex, Claude Code или OMP, затем нажмите «Найти агенты»."/>}
          {snap.config.installations.map(a => <AgentCard key={a.provider} agent={a} act={command}/>)}
          <div className="card row"><div className="grow"><strong>Дальше — подключение бота</strong><p>Укажите bot token, ваш Telegram User ID и PIN.</p></div><Button onClick={() => setPage('Telegram')}>Открыть Telegram</Button></div>
        </>}
        {page === 'Telegram' && <>
          <div className="card">
            <div className="row"><h2 className="grow">Подключение бота</h2><Badge appearance="tint">{snap.telegram_status}</Badge></div>
            <p>Используйте отдельного бота из BotFather. Одновременно с Agent Hub у него не должно быть другого Telegram-моста.</p>
            <div className="form-grid">
              <Field label="Bot token"><Input type="password" autoComplete="off" value={token} onChange={(_, d) => setToken(d.value)}/></Field>
              <Field label="Ваш Telegram User ID"><Input value={user} placeholder={String(snap.config.telegram_user ?? '')} onChange={(_, d) => setUser(d.value)}/></Field>
              <Field label="PIN · рекомендуется 6 цифр"><Input type="password" autoComplete="new-password" maxLength={6} value={pin} onChange={(_, d) => setPin(d.value)}/></Field>
            </div>
            <Button appearance="primary" disabled={busy || !token || !/^\d+$/.test(user) || !/^\d{4,6}$/.test(pin)} onClick={() => run(async () => {
              await invoke('configure_telegram', { token, userId: Number(user), pin }); setToken(''); setPin(''); setNotice('Настройки сохранены. Теперь подключите бота.');
            })}>Сохранить подключение</Button>
            <p className="muted">Bot token хранится в Windows Credential Manager. Принимаются сообщения только от указанного User ID в личном чате.</p>
          </div>
          <div className="card">
            <h2>Управление с телефона</h2>
            <div className="row"><Button appearance="primary" disabled={busy || !snap.config.pin_hash} onClick={() => command('connect_telegram')}>Подключить</Button><Button onClick={() => command('disconnect_telegram')}>Отключить</Button><Button onClick={() => command('revoke_access')}>Закрыть доступ</Button></div>
            <p>Откройте бота: <strong>/start → PIN → агент → папка → сессия</strong>. После этого отправляйте задачи и вложения.</p>
            {!snap.config.projects.length && <div className="banner">Сначала добавьте разрешённую папку.<Button appearance="subtle" onClick={() => setPage('Настройки')}>Открыть настройки</Button></div>}
            <p className="muted">Доступ закрывается после 15 минут бездействия, через 8 часов или при перезапуске Hub.</p>
          </div>
          <p className="muted">Telegram cloud chat не использует E2EE. PIN удаляется из чата по возможности; это дополнительный барьер доступа.</p>
        </>}
        {page === 'Настройки' && <>
          <div className="card">
            <h2>Разрешённые папки</h2><p>Агент работает в папке, выбранной в Telegram. Новые папки можно разрешить только здесь.</p>
            {snap.config.projects.map(p => <div className="folder" key={p.id}><strong>{p.name}</strong><code>{p.root}</code></div>)}
            <details open={!snap.config.projects.length}><summary>Добавить папку</summary><div className="form-grid">
              <Field label="Название для Telegram"><Input value={projectName} onChange={(_, d) => setProjectName(d.value)}/></Field>
              <Field label="Путь"><div className="row"><Input className="grow" value={root} onChange={(_, d) => setRoot(d.value)}/><Button onClick={() => run(async () => { const p = await open({ directory: true, multiple: false }); if (typeof p === 'string') setRoot(p); })}>Выбрать</Button></div></Field>
            </div><Button appearance="primary" disabled={busy || !root || !projectName} onClick={() => run(async () => { await invoke('add_project', { name: projectName, root }); setRoot(''); setProjectName(''); })}>Разрешить папку</Button></details>
            <p className="muted">Список папок ограничивает выбор через Hub. Изоляция самого CLI зависит от агента и его sandbox.</p>
          </div>
          <div className="card"><h2>Приложение</h2>
            <div className="setting"><Switch checked={autostart} onChange={(_, d) => run(async () => { await invoke('set_autostart', { enabled: d.checked }); setAutostart(d.checked); })} label="Запускать в трее при входе в Windows"/></div>
            <div className="setting"><Switch checked={dark} onChange={(_, d) => setDark(d.checked)} label="Тёмная тема"/></div>
            <p className="muted">Закрытие окна скрывает его в трей. Команда «Выйти» завершает Hub и его задачи.</p>
          </div>
          <details className="card"><summary>Безопасность и журнал</summary><p>Отзыв доступа останавливает remote-задачи и требует заново ввести PIN.</p>
            <div className="row"><Button onClick={() => command('revoke_access')}>Отозвать доступ</Button><Button onClick={() => run(async () => setAudit(await invoke('audit')))}>Показать аудит</Button></div>
            {audit.map((a, i) => <div className="audit" key={i}><time>{new Date(a.at * 1000).toLocaleString()}</time><strong>{a.action}</strong><code>{a.target}</code></div>)}
          </details>
          <details className="card"><summary>О сборке</summary><p>Agent Hub 0.1.0 · первая сборка MVP для одного ПК. Реальные CLI и Telegram требуют приёмки после локальной настройки.</p><p>Данные: %LOCALAPPDATA%\AgentHub. Multi-PC, голос, Computer Use и подписанные обновления пока не включены.</p></details>
        </>}
      </>}
    </main>
  </div></FluentProvider>;
}
function Empty({ title, text }: { title: string; text: string }) { return <div className="empty"><h3>{title}</h3><p>{text}</p></div>; }
function AgentCard({ agent: a, act }: { agent: Agent; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) { const [accept, setAccept] = useState(a.accept_unsandboxed); return <div className="card"><div className="row"><h2 className="grow">{a.provider}</h2><Badge>{a.enabled ? 'Включён' : 'Отключён'}</Badge></div><p>{a.version}</p><code>{a.executable}</code><p>{a.provider === 'codex' ? 'App-server · workspace-write · approvals on-request · steering' : a.provider === 'claude' ? 'Stream JSON · permissions manual · сообщения в очередь' : 'RPC UI v1 · steering · native dialogs подтверждаются на ПК'}</p>{a.provider !== 'codex' && <Switch checked={accept} onChange={(_, d) => setAccept(d.checked)} label="Разрешаю запуск без подтверждённой OS sandbox"/>}<div className="row"><Button disabled={!a.enabled && a.provider !== 'codex' && !accept} onClick={() => act('enable_agent', { provider: a.provider, enabled: !a.enabled, acceptUnsandboxed: accept })}>{a.enabled ? 'Отключить' : 'Включить локально'}</Button><small>Runtime qualification: ещё не выполнена</small></div></div>; }
function Requests({ requests, act }: { requests: Request[]; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) { return <>{requests.map(r => <RequestCard key={r.id} r={r} act={act}/>)}</>; }
function RequestCard({ r, act }: { r: Request; act: (n: string, args?: Record<string, unknown>) => Promise<void> }) {
  const questions: any[] = r.data.params?.questions ?? r.data.input?.questions ?? []; const [answers, setAnswers] = useState<Record<string, string>>({}), [value, setValue] = useState('');
  const payload = () => { if (r.kind === 'native_dialog') return value; const result: Record<string, unknown> = {}; questions.forEach(q => { const key = q.id ?? q.question; const v = answers[key] ?? ''; result[key] = r.data.method ? { answers: [v] } : v; }); return result; };
  return <div className="card request"><div className="row"><h3 className="grow">{r.kind === 'question' ? 'Вопрос агента' : r.kind === 'native_dialog' ? 'Native dialog · локально' : 'Требуется решение'}</h3><Badge color="warning">До {new Date(r.expires_at * 1000).toLocaleTimeString()}</Badge></div><pre>{JSON.stringify(r.data, null, 2)}</pre>{questions.map((q, i) => { const key = q.id ?? q.question; return <Field key={i} label={q.question ?? q.header ?? q.id}><Input value={answers[key] ?? ''} onChange={(_, d) => setAnswers(a => ({ ...a, [key]: d.value }))}/>{q.options?.length > 0 && <div className="row">{q.options.map((o: any, j: number) => <Button key={j} size="small" onClick={() => setAnswers(a => ({ ...a, [key]: o.label ?? o }))}>{o.label ?? String(o)}</Button>)}</div>}</Field>; })}{r.kind === 'native_dialog' && <Field label="Значение ответа / пункт native select"><Input value={value} onChange={(_, d) => setValue(d.value)}/></Field>}<div className="row"><Button appearance="primary" disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'allow', answers: payload() })}>{r.kind === 'question' ? 'Ответить' : 'Разрешить один раз'}</Button><Button disabled={r.kind === 'unsupported'} onClick={() => act('answer_request', { requestId: r.id, decision: 'deny', answers: {} })}>Отказать</Button><Button onClick={() => act('cancel_turn', { turnId: r.turn_id })}>Остановить задачу</Button></div></div>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><App/></React.StrictMode>);
