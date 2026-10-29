import React, { createContext, useContext, useEffect, useState } from 'react';
export type Language = 'en' | 'ru';
const strings: Record<string, string> = {
  'Отправьте этот PIN боту в Telegram':'Send this PIN to your Telegram bot',
  'Одноразовый код · до':'One-time code · expires at',
  'Для нового подтверждения получите PIN и отправьте его боту.':'To verify again, generate a PIN and send it to your bot.',
  'Новый PIN':'New PIN',
  'Импорт темы':'Import theme', 'Импорт…':'Importing…', 'Тема импортирована.':'Theme imported.', 'ZIP из TAB Theme Generator':'ZIP from TAB Theme Generator',
  'Некорректный архив темы. Экспортируйте его из TAB Theme Generator.':'Invalid theme archive. Export it from TAB Theme Generator.',
  'Архив темы превышает 64 МиБ.':'Theme archive exceeds 64 MiB.',
  'Контрольная сумма темы не совпала. Экспортируйте тему заново.':'Theme checksum mismatch. Export the theme again.',
  'Достигнут лимит хранилища тем (16 тем / 256 МиБ).':'Theme storage limit reached (16 themes / 256 MiB).',
  'Агенты':'Agents', 'Настройки':'Settings', 'Основная навигация':'Main navigation', 'Этот компьютер':'This computer', 'Запуск…':'Starting…', 'Активных задач:':'Active tasks:', 'Закрыть':'Close',
  'Открываем локальную базу…':'Opening local database…', 'Подключите ваши CLI-агенты':'Connect your CLI agents', 'Отметьте агентов, к которым будет доступ через Telegram.':'Select the agents you want to access through Telegram.',
  'Агенты на этом компьютере':'Agents on this computer', 'Найти агенты':'Find agents', 'Агенты ещё не найдены':'No agents found yet', 'Установите и авторизуйте Codex, Claude Code или OMP, затем нажмите «Найти агенты».':'Install and sign in to Codex, Claude Code or OMP, then click “Find agents”.',
  'Подключение бота':'Bot connection', 'Подключить':'Connect', 'Отключить':'Disconnect', 'Отключение…':'Disconnecting…', 'Отключается':'Disconnecting', 'Подключается':'Connecting', 'Подключён · один ПК':'Connected · single PC', 'Отключён':'Disconnected', 'Сеть недоступна; переподключение…':'Network unavailable; reconnecting…',
  'Используйте отдельного бота из BotFather. Одновременно с TAB у него не должно быть другого Telegram-моста.':'Use a dedicated bot from BotFather. Do not run another Telegram bridge with the same bot while TAB is connected.',
  'Ваш числовой Telegram ID':'Your numeric Telegram ID', 'Узнать свой ID: откройте @userinfobot в Telegram и нажмите Start. Скопируйте число из строки Id.':'To find your ID, open @userinfobot in Telegram and press Start. Copy the number from the Id line.',
  'Например, 123456789':'For example, 123456789', 'PIN · рекомендуется 6 цифр':'PIN · 6 digits recommended', 'Сохранить подключение':'Save connection', 'Чтобы сохранить:':'To save:',
  'Введите ваш числовой Telegram ID.':'Enter your numeric Telegram ID.', 'Нужен числовой ID, например 123456789. Имя пользователя или @username не подходит.':'Use a numeric ID, such as 123456789, not a name or @username.', 'PIN должен содержать от 4 до 6 цифр.':'PIN must contain 4–6 digits.', 'Введите bot token из BotFather.':'Enter the bot token from BotFather.', 'Настройки сохранены. Теперь подключите бота.':'Settings saved. You can now connect the bot.',
  'Bot token хранится в Windows Credential Manager. Принимаются сообщения только от указанного User ID в личном чате.':'The bot token is stored in Windows Credential Manager. Only private messages from the specified user ID are accepted.',
  'Telegram cloud chat не использует E2EE. PIN удаляется из чата по возможности; это дополнительный барьер доступа.':'Telegram cloud chats are not end-to-end encrypted. TAB deletes PIN messages when possible; the PIN is an additional access barrier.',
  'Приложение':'Application', 'Язык':'Language', 'Запускать в трее при входе в Windows':'Start in the tray when signing in to Windows', 'Тёмная тема':'Dark mode', 'Фон':'Background', 'Матрица':'Matrix', 'Анимация фона':'Animate background',
  'Закрытие окна скрывает его в трей. Команда «Выйти» завершает TAB и его задачи.':'Closing the window hides TAB in the tray. “Exit” closes TAB and stops its tasks.',
  'Иконка трея':'Tray icon', 'Секретные фоны':'Secret backgrounds', 'Скрыть Developer Settings':'Hide Developer Settings', 'Безопасность и журнал':'Security and audit', 'Отзыв доступа останавливает remote-задачи и требует заново ввести PIN.':'Revoking access stops remote tasks and requires entering the PIN again.', 'Отозвать доступ':'Revoke access', 'Показать аудит':'Show audit log', 'О сборке':'About',
  'TAB — Telegram Agents Bridge 0.1.0 · первая сборка MVP для одного ПК. Реальные CLI и Telegram требуют приёмки после локальной настройки.':'TAB — Telegram Agents Bridge 0.1.0 · single-PC MVP. Real CLI and Telegram workflows require verification after local setup.',
  'Данные: %LOCALAPPDATA%\\AgentHub (сохранены при переименовании). Multi-PC, голос, Computer Use и подписанные обновления пока не включены.':'Data: %LOCALAPPDATA%\\AgentHub (preserved after renaming). Multi-PC, voice, Computer Use and signed updates are not included yet.',
  'Сначала разрешите запуск без OS sandbox ниже':'First allow execution without an OS sandbox below', 'Доступ к':'Access to', 'Stream JSON · permissions manual · сообщения в очередь':'Stream JSON · manual permissions · queued messages', 'RPC UI v1 · steering · native dialogs подтверждаются на ПК':'RPC UI v1 · steering · native dialogs confirmed on this PC', 'Разрешаю запуск без подтверждённой OS sandbox':'Allow execution without a verified OS sandbox', 'Runtime qualification: ещё не выполнена':'Runtime qualification: not completed yet',
  'Вопрос агента':'Agent question', 'Native dialog · локально':'Native dialog · local', 'Требуется решение':'Decision required', 'До':'Until', 'Значение ответа / пункт native select':'Response value / native selection', 'Ответить':'Answer', 'Разрешить один раз':'Allow once', 'Отказать':'Deny', 'Остановить задачу':'Stop task',
  'Акцентный цвет':'Accent color', 'Свой цвет':'Custom color', 'Выбрать свой акцентный цвет':'Choose a custom accent color', 'Текущий цвет':'Current color', 'Жёлтый':'Yellow', 'Мятный':'Mint', 'Голубой':'Blue', 'Фиолетовый':'Purple', 'Розовый':'Pink', 'Коралловый':'Coral', 'Тестовый агент':'Test agent',
  'Не удалось загрузить фон. Предыдущая тема сохранена.':'Could not load the background. The previous theme is still active.',
  'В работе':'Running', 'В очереди':'Queued', 'Завершено':'Completed', 'Отменено':'Cancelled', 'Прервано':'Interrupted', 'Ошибка':'Error', 'Результат неизвестен':'Outcome unknown',
};
const reverse = Object.fromEntries(Object.entries(strings).map(([ru,en]) => [en,ru]));
const Context = createContext<{language: Language; setLanguage: (language: Language) => void}>({language:'en',setLanguage:()=>{}});
export function LanguageProvider({children}: {children: React.ReactNode}) {
  const [language,setLanguage]=useState<Language>(()=>localStorage.getItem('tab-language')==='ru'?'ru':'en');
  useEffect(()=>{localStorage.setItem('tab-language',language);document.documentElement.lang=language},[language]);
  return <Context.Provider value={{language,setLanguage}}>{children}</Context.Provider>;
}
export function useLanguage(){return useContext(Context)}
export function useT(){const {language}=useLanguage();return (text: string):string => language==='en'?(strings[text]??text):(reverse[text]??text)}
